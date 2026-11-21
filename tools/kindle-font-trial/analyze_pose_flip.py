"""Qualify the timing of a real Scribe posture change, only after native return and release."""

import argparse
import json
from pathlib import Path
import re
import tarfile

from analyze import TRACE, ImpossibleOrientationTrace, request_ns
from analyze_assisted_shelf import final_route
from analyze_orientation import orientation_sample
from analyze_pose_visit import pose, timestamp
from first_draw_evidence import assess_draw, split_draw_records
from pose_flip_device import CASE, LEAVES
from pose_window import HOLDING, MAX_NS, compare_bracket
from prepare_pose_flip import CONTRACT, DRAW_CONTRACT, RECONCILED_CONTRACT, draw_schedule, reconciled_schedule, schedule


LIMITS = dict(request=96, markers=4096, sensor=20000, shelf=128, portrait=4, observation=64)
LIMITS.update({'sensor-ready': 80, 'app-binding': 160, 'sensor-binding': 160, 'portrait-source': 32,
               'portrait-launch': 32, 'portrait-return': 4, 'final.metadata.json': 2048})
CHECKPOINT = set(LIMITS) - {'portrait-return', 'shelf', 'final.metadata.json'}
PARTIAL_REASON = rb'incomplete_after_pose|scene_cleanup|exit|capture_metadata|native_return|final_publish|presentation_(?:gui_args|gui_identity|generation|owner|log_file|log_writer|log_inode|boot|record|binding)'
SEQUENCE = [('admit','none'), ('reference','none'), ('stage','none'), ('case',CASE), ('restore','none'),
            ('collect','none'), ('close','none'), ('absent','none')]


def archive_records(path, label):
    if set(LEAVES) != set(LIMITS):
        raise ValueError('Device and analyzer disagree about public records')
    expected = {f'out/{CASE}/{leaf}': limit for leaf, limit in LIMITS.items()} | {'out/orientation-batch': 17}
    entries = {}
    with tarfile.open(path, 'r|gz') as archive:
        for member in archive:
            if member.name not in expected or member.name in entries or not member.isfile() or not 0 < member.size <= expected[member.name]:
                raise ValueError('Unexpected, duplicate or oversized startup-turn record')
            entries[member.name] = archive.extractfile(member).read()
    if entries.pop('out/orientation-batch', None) != (label + '\n').encode('ascii'):
        raise ValueError('Wrong physical batch')
    return {name.rsplit('/', 1)[-1]: raw for name, raw in entries.items()}


def binding(raw, label):
    expression = rb'LAUNCH ([a-f0-9]{16}) (B-library) ([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}) ([1-9][0-9]{0,9}) ([1-9][0-9]{0,18}) CLOCK_MONOTONIC\n'
    match = re.fullmatch(expression, raw)
    if not match or match[1].decode() != label or int(match[4]) > 2**31 - 1 or int(match[5]) > MAX_NS:
        raise ValueError('Wrong batch or malformed physical launch binding')
    return dict(label=label, visit=match[2].decode(), boot_id=match[3].decode(), gui_pid=int(match[4]),
                gui_birth=int(match[5]), clock='CLOCK_MONOTONIC')


def trace_rows(raw, ready, *, with_scanout=False):
    rows = []
    pattern = rb'([1-9][0-9]{0,2}) ([1-9][0-9]{0,18}) ([1-9][0-9]{0,18}) (1[5-8])' + (rb' ([0-3])' if with_scanout else b'') + rb'\n'
    for index, line in enumerate(raw.splitlines(keepends=True), 1):
        matched = re.fullmatch(pattern, line)
        if not matched:
            raise ValueError('Invalid sample from the independent pose descriptor')
        row = list(map(int, matched.groups()))
        if row[0] != index or not row[1] <= row[2] <= MAX_NS or (rows and row[1] - rows[-1][2] < CONTRACT['sensor_period_ns']):
            raise ValueError('Incomplete or out-of-order physical pose stream')
        rows.append(row)
    if len(rows) != CONTRACT['sensor_samples'] or rows[-1][2] - rows[0][1] > CONTRACT['sensor_budget_ns']:
        raise ValueError('Wrong physical sample count or budget')
    if ready != ('READY '+ ' '.join(map(str, rows[0][1:])) +'\n').encode('ascii'):
        raise ValueError('READY is not backed by the first independent sensor read')
    return rows


def shelf_result(raw, sent):
    if raw == b'not_observed\n':
        return {'status': 'not_observed'}
    match = re.fullmatch(rb'SHELF ([1-9][0-9]{0,18}) ([1-9][0-9]{0,18}) ([0-3]) (grid|list|empty|none)\n', raw)
    if not match:
        raise ValueError('Malformed digital shelf witness')
    start, end, rotation = map(int, match.groups()[:3]); kind = match[4].decode()
    if not sent <= start <= end <= MAX_NS:
        return {'status': 'insufficient_clock'}
    return dict(status='observed_populated_shelf' if kind in ('grid','list') else 'public_entries_not_observed',
                scene=kind, framebuffer_rotation=rotation, read_ns=[start,end])


def launch_anchor(raw, native):
    if re.fullmatch(rb'[UD]:[0-3]\n', raw):
        value = raw.decode('ascii').strip()
        return value, {'status':'observed', 'comparison_to_before':'same' if value == native else 'different'}
    if re.fullmatch(rb'FIXED:[0-3]\n', raw):
        return raw.decode('ascii').strip(), {'status':'fixed', 'comparison_to_before':'not_comparable'}
    if raw in (b'missing\n', b'malformed\n', b'foreign_generation\n'):
        return None, {'status':raw.decode('ascii').strip(), 'comparison_to_before':'unknown'}
    raise ValueError('Unexpected or private launch calibration record')


def event_times(text, request):
    previous = request
    times = {}
    if not text.endswith('\n'):
        return {}
    for line in text.splitlines():
        match = TRACE.fullmatch(line)
        if not match or match[3] in times:
            return {}
        stamp = int(match[1]) * 10**9 + int(match[2])
        if not previous <= stamp <= MAX_NS:
            return {}
        times[match[3]] = previous = stamp
    order = ['main_enter', 'startup_feedback_ready', 'portrait_opened', 'first_application_frame_send']
    present = [stage for stage in order if stage in times]
    if (not present or present[0] != 'main_enter' or [name for name in times if name in order] != present or
            any(times[left] >= times[right] for left, right in zip(present, present[1:]))):
        return {}
    return times


def observed_turn(rows, request, times):
    changes = [(left[1], right[2], HOLDING[left[3]], HOLDING[right[3]]) for left,right in zip(rows, rows[1:])
               if HOLDING[left[3]] != HOLDING[right[3]]]
    motion = dict(directional_changes=len(changes), first=HOLDING[rows[0][3]], last=HOLDING[rows[-1][3]])
    if len(changes) != 1:
        return motion, {'coverage':'insufficient','reason':'no_single_observed_change'}
    change = changes[0]
    earliest, latest = change[:2]
    motion['change_after_request_ms'] = [(value - request)/1e6 for value in (earliest,latest)]
    splash, predraw, sent = (times.get(key) for key in ('startup_feedback_ready','portrait_opened','first_application_frame_send'))
    span = sent if sent else predraw
    if not rows[0][2] < request or (span and span >= rows[-1][1]):
        return motion, {'coverage':'insufficient','reason':'sensor_does_not_span_launch'}
    if not (splash and predraw and sent):
        result = {'coverage':'insufficient','reason':'missing_or_invalid_window_markers'}
        if splash and predraw and splash < earliest and latest < predraw:
            result['phase_relation'] = 'after_splash_before_predraw_no_confirmed_submit'
        elif splash and sent and splash < earliest and latest < sent:
            result['phase_relation'] = 'after_splash_before_submit_log_predraw_unknown'
        return motion, result
    turn = compare_bracket(change, splash, predraw, sent)
    if 'observed_change_ns' in turn:
        turn['change_after_splash_ready_ms'] = [(value-splash)/1e6 for value in turn.pop('observed_change_ns')]
    return motion, turn


def reconciliation_evidence(queries, times, rows, draw):
    if queries is None:
        return {'status': 'invalid_or_missing_site_trace'}
    if queries['event_loop'] != 'reconciled':
        return {'status': 'not_observed', 'event_loop_outcome': queries['event_loop']}
    changes = [(old, new) for old, new in zip(rows, rows[1:]) if HOLDING[old[3]] != HOLDING[new[3]]]
    begin = times.get('event_loop_orientation_begin')
    before = bool(begin and len(changes) == 1 and changes[0][1][2] < begin)
    return {'status': 'event_loop_marker_observed', 'one_change_observed_before_event_loop': before,
            'first_draw_mapping_observation': draw.get('software_mapping', {}).get('status', 'not_assessed')}


def analyze(plan, receipt, path):
    reconciled_plan = plan.get('purpose') == RECONCILED_CONTRACT['purpose']
    diagnostic = reconciled_plan or plan.get('purpose') == DRAW_CONTRACT['purpose']
    (reconciled_schedule if reconciled_plan else draw_schedule if diagnostic else schedule)(plan)
    if (receipt.get('label') != plan['label'] or receipt.get('purpose') != plan['purpose'] or receipt.get('scope') != 'transaction_only'
            or receipt.get('owner_token_checked_on_release') is not True or receipt.get('recovery_required')
            or timestamp(receipt['released']) < timestamp(receipt['acquired'])):
        raise ValueError('No matching verified lease release')
    report = dict(label=plan['label'], device=plan['device'], firmware=plan['firmware'], released=receipt['released'],
                  collection='not_completed', startup_turn={'coverage':'insufficient','reason':'incomplete_transaction'},
                  touch='not_tested', visual_uprightness='not_established', first_frame_orientation='not_established')
    report['first_application_draw'] = {'status':'not_assessed' if diagnostic else 'not_in_historical_plan'}
    operations = receipt.get('operations')
    if not isinstance(operations, list) or any(not isinstance(op,dict) or type(op.get('status')) is not int for op in operations):
        return report
    if [(op.get('mode'),op.get('case')) for op in operations] != SEQUENCE:
        return report
    failed = [op for op in operations if op['status'] != 0]
    partial = bool(failed)
    if partial:
        if (len(failed) != 1 or failed[0]['mode'] != 'case' or receipt.get('passed') is not False
                or not re.fullmatch(r'refused [a-z_]+', failed[0].get('diagnostic') or '')):
            return report
        report['case_refusal'] = failed[0]['diagnostic'].split(' ',1)[1]
    elif receipt.get('passed') is not True or receipt.get('error'):
        return report
    initial = receipt['initial_native_reference']
    if initial not in CONTRACT['expected_start_references'] or not timestamp(receipt['acquired']) <= timestamp(receipt['operator_cue_issued']) <= timestamp(receipt['released']):
        raise ValueError('Missing initial reference or cue within the verified lease')
    if diagnostic:
        fields = (operations[2].get('at'), receipt['operator_cue_issued'], receipt.get('operator_cue_acknowledged'), operations[3].get('at'))
        if not all(isinstance(field, str) for field in fields):
            raise ValueError('Missing staged, acknowledged startup cue')
        staged, issued, acknowledged, completed = map(timestamp,fields)
        if not timestamp(receipt['acquired']) <= staged <= issued <= acknowledged <= completed <= timestamp(receipt['released']):
            raise ValueError('The observed launch did not follow the staged, acknowledged startup cue')
    records = archive_records(path, plan['label'])
    if partial and not records:
        return report
    if set(records) != (CHECKPOINT if partial else set(LIMITS)):
        raise ValueError('Unexpected scope for the released evidence')
    if partial:
        evidence = re.fullmatch(rb'partial (' + PARTIAL_REASON + rb')\n', records['observation'])
        if not evidence:
            raise ValueError('Missing bounded checkpoint status')
        report['checkpoint_reason'] = evidence[1].decode('ascii')
    elif records['observation'] != b'captured startup_turn\n':
        raise ValueError('Missing complete visit status')
    if records['portrait-source'] != b'native_idle_calibration\n':
        raise ValueError('Launch and native source are not identified')
    native = pose(records['portrait'])
    launch, anchor = launch_anchor(records['portrait-launch'], native)
    app_id = binding(records['app-binding'], plan['label'])
    sensor_id = binding(records['sensor-binding'], plan['label'])
    if sensor_id != app_id:
        raise ValueError('Sensor and application do not identify the same boot, GUI and visit')
    request, markers = records['request'].decode('ascii'), records['markers'].decode('ascii')
    if diagnostic:
        markers, draw_lines = split_draw_records(markers)
    rows = trace_rows(records['sensor'], records['sensor-ready'], with_scanout=diagnostic)
    origin = request_ns(request)
    if not 0 < origin <= MAX_NS:
        raise ValueError('Invalid device request origin')
    times = event_times(markers, origin)
    if diagnostic:
        report['first_application_draw'] = assess_draw(draws=draw_lines, ordinary=markers, times=times,
                                                     rows=rows, request=origin, initial=initial, native=native, launch=launch)
    motion, turn = observed_turn(rows, origin, times)
    try:
        sample = orientation_sample(request, markers, lane='B', allow_reconciliation=reconciled_plan)
        queries = {site:sample[site]['outcome'] for site in ('splash','event_loop')}
    except ImpossibleOrientationTrace:
        raise
    except ValueError:
        queries = None
    if reconciled_plan:
        report['reconciliation_evidence'] = reconciliation_evidence(queries, times, rows, report['first_application_draw'])
    report.update(collection='pose_checkpoint_only_released' if partial else 'completed_and_released', startup_turn=turn,
                  sensor_direction=motion, initial_reference=initial, native_before=native, native_launch=launch,
                  launch_calibration=anchor, initial_reference_matches_before=initial==native,
                  first_sensor_direction_matches_native=motion['first']==('up' if native.startswith('U:') else 'down'),
                  native_return=None if partial else pose(records['portrait-return']), independent_samples=len(rows),
                  sample_pose_counts={name:sum(HOLDING[row[3]]==name for row in rows) for name in ('up','down')},
                  orientation_queries=queries, final_route=None, digital_shelf={'status':'not_assessed'})
    if not partial:
        try:
            report['final_route'] = final_route(records['final.metadata.json'])
        except (UnicodeError, ValueError, TypeError):
            report['final_route_status'] = 'unusable_metadata'
        sent = times.get('first_application_frame_send')
        if sent:
            try:
                report['digital_shelf'] = shelf_result(records['shelf'], sent)
            except ValueError:
                report['digital_shelf'] = {'status':'unusable_observation'}
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('plan','operations','archive','output'):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    result = analyze(json.loads(args.plan.read_text()), json.loads(args.operations.read_text()), args.archive)
    with args.output.open('x') as output:
        output.write(json.dumps(result, indent=2) + '\n')
