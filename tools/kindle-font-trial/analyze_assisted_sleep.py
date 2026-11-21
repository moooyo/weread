"""Separate same-cycle sensor, digital Reader and notification evidence, offline after release."""

import argparse
import io
import json
from pathlib import Path
import re
import tarfile

from PIL import Image, ImageChops, ImageStat

from analyze import cropped, mae
from analyze_orientation import orientation_sample
from analyze_pose_visit import SEQUENCE, pose, timestamp
from run_assisted_sleep import schedule_for_sleep


PHASES = ('pre', 'sleep-pre', 'sleep-post', 'post')
LIMITS = {'request': 96, 'markers': 4096, 'portrait': 4, 'portrait-source': 32, 'portrait-launch': 4,
          'portrait-return': 4, 'observation': 128, 'final.png': 12 * 1024**2, 'final.metadata.json': 4096,
          'image-mapping': 70, 'initial-diagnostic': 80, 'sleep-trace': 4096, 'turn-window': 96, 'post-status.row': 128,
          'panel.pgm': 1860 * 2480 + 128, 'resumed.pgm': 1860 * 2480 + 128,
          **{phase + '.row': 256 for phase in PHASES}}
WAKES = ('initial_reader', 'sleep_unconfirmed', 'turn_unconfirmed', 'wake_unconfirmed', 'resume_unconfirmed', 'restored_without_turn')
OBSERVATION_FAILURES = frozenset('stage_scope entry_state owner_record generation application_identity state_open managed_available '
                                 'executable process_birth arguments peer launch_calibration task_busy task_state manager display input '
                                 'phase_markers sleep_cycle wake_cycle wake_calibration power_query power_state sensor_helper sensor_busy '
                                 'sensor_query sensor_format pose_disagrees unstable_scope sensor_timeout observer_unknown'.split())
INITIAL_FAILURES = {'observe_before': OBSERVATION_FAILURES, 'observe_after': OBSERVATION_FAILURES,
                    'image_read': {'raw_panel'}, 'image_state': {'occupied_output','sensor_pair_changed','anchor_invalid','mapping_unsupported'},
                    'publish': {'publish_failed'}, 'ready': {'ready'}}
OPERATOR_FIELDS = ('saw_screensaver', 'turned_while_screensaver', 'held_pose_through_wake', 'same_readable_page_after_wake')


def initial_diagnostic(raw, initial_failure, has_pre_frame, events):
    if raw is None:
        return {'status': 'not_recorded'}
    matched = re.fullmatch(rb'INITIAL ([a-z_]{1,24}) ([a-z_]{1,32}) ([1-9]|1[0-2])\n', raw)
    if not matched:
        raise ValueError('Malformed initial display diagnostic')
    site, reason = matched[1].decode(), matched[2].decode()
    if (reason not in INITIAL_FAILURES.get(site, ()) or (site == 'ready') != has_pre_frame or
            (site == 'ready') == initial_failure or (initial_failure and events)):
        raise ValueError('Initial display diagnostic conflicts with the observation')
    return {'status': 'captured' if site == 'ready' else 'refused', 'site': site, 'reason': reason, 'attempts': int(matched[3])}


def read_visit(path, label):
    members = {}
    limits = {'out/orientation-batch': 17, **{'out/B-reader/' + name: limit for name, limit in LIMITS.items()}}
    with tarfile.open(path, 'r|gz') as archive:
        for member in archive:
            empty_trace = member.name == 'out/B-reader/sleep-trace' and member.size == 0
            if (not member.isfile() or member.name not in limits or member.name in members
                    or not (empty_trace or 0 < member.size <= limits[member.name])):
                raise ValueError('Unexpected, duplicate or oversized public record')
            members[member.name] = archive.extractfile(member).read()
    if members.pop('out/orientation-batch', b'') != (label + '\n').encode('ascii'):
        raise ValueError('Foreign sleep visit')
    return {name.split('/')[-1]: value for name, value in members.items()}


def sensor_rows(records, anchor, after_send, witness=False):
    rows = {}
    previous = after_send
    initial_up = anchor == 'U:3'
    for phase in ('pre','sleep-pre','sleep-trace','sleep-post','post-status','post') if witness else PHASES:
        middle = phase == 'sleep-trace'
        wire_phase = 'sleep-post' if middle else 'post' if phase == 'post-status' else phase
        data = records.get(phase if middle else phase + '.row')
        if data is None:
            continue
        samples = []
        count = 2 if phase.startswith('sleep-') else 1
        if middle and data == b'':
            rows[phase] = []
            continue
        if not data.endswith(b'\n') or not (1 <= len(data.splitlines()) <= 24 if middle else len(data.splitlines()) == count):
            raise ValueError('Invalid number of observations in a phase')
        for line in data.splitlines():
            match = re.fullmatch(rb'WAKE_OBS (pre|sleep-pre|sleep-post|post) ([1-9]\d{0,18}) ([1-9]\d{0,18}) (1[5-8]) ([0-3]) (U:3|D:1|none)', line)
            if not match or match[1].decode() != wire_phase:
                raise ValueError('Unbound sleep sample')
            begin, end, raw, rotation = map(int, match.groups()[1:5])
            calibration = match[6].decode()
            expected_up = initial_up if phase in ('pre', 'sleep-pre') else not initial_up
            if (not previous < begin <= end < 2**63 or (samples and not middle and begin - previous < 100_000_000)
                    or ((not witness or phase == 'pre') and (raw in (15, 17)) != expected_up) or (phase == 'pre' and calibration != anchor)
                    or (phase.startswith('sleep-') and calibration != 'none')):
                raise ValueError('Inconsistent phase calibration, order, or pressure')
            samples.append(dict(begin_ns=begin, end_ns=end, pressure=raw, framebuffer_rotation=rotation, calibration=calibration))
            previous = end
        rows[phase] = samples
    return rows


def turn_window(raw, count):
    if raw is None:
        if count:
            raise ValueError('Unbound sleep trace without a turn window')
        return None
    match = re.fullmatch(rb'WINDOW ([0-9]{1,13}) ([0-9]{1,13}) ([0-9]{1,2}) ([0-9]{1,2}) (elapsed|early_wake|lost_state)\n', raw)
    if not match:
        raise ValueError('Malformed turn window')
    start, end, attempted, skipped = map(int, match.groups()[:4])
    if (not start <= end or not count == attempted - skipped or not 0 <= skipped <= attempted <= 24
            or (match[5] == b'elapsed' and end - start < 2200)):
        raise ValueError('Inconsistent turn-window counts or device uptime')
    return {'clock':'device_uptime_centiseconds', 'first':start, 'last':end, 'attempts':attempted,
            'skipped':skipped, 'recorded':count, 'end_reason':match[5].decode()}


def pose_evidence(rows):
    sleeping = [row for stage in ('sleep-pre','sleep-trace','sleep-post') for row in rows.get(stage, [])]
    inside = ['up' if row['pressure'] in (15,17) else 'down' for row in sleeping]
    resumptions = {phase: 'up' if rows[phase][0]['pressure'] in (15,17) else 'down' for phase in ('post-status','post') if phase in rows}
    endpoints = {'pre': 'up' if rows['pre'][0]['pressure'] in (15,17) else 'down'} if 'pre' in rows else {}
    if resumptions:
        endpoints['post'] = resumptions.get('post-status') or resumptions['post']
    different_ends = len(endpoints) == 2 and endpoints['pre'] != endpoints['post']
    changes = [[left['begin_ns'], right['end_ns']] for left,right in zip(sleeping,sleeping[1:])
               if (left['pressure'] in (15,17)) != (right['pressure'] in (15,17))]
    if changes:
        result = 'opposite_directions_observed_while_stopped'
    elif different_ends:
        result = 'awake_endpoints_differ_timing_unresolved'
    elif len(endpoints) == 2 and inside and len(set([*inside, *endpoints.values(), *resumptions.values()])) == 1:
        result = 'same_direction_in_recorded_samples'
    else:
        result = 'insufficient_to_locate_a_turn'
    return {'status':result, 'sleep_read_count':len(sleeping), 'stopped_change_brackets_ns':changes,
            'awake_endpoints': endpoints, 'awake_endpoints_differ':different_ends, 'after_wake_directions':resumptions}


def witness_report(value, label):
    if value is not None and (not isinstance(value, dict) or value.get('label') != label
            or set(value) != {'label', *OPERATOR_FIELDS} or any(type(value[key]) is not bool for key in OPERATOR_FIELDS)):
        raise ValueError('Expected a separate label-bound boolean operator report')
    return value


def reader_route(records):
    try:
        meta = json.loads(records['final.metadata.json'])
        if all(meta.get(key) == value for key,value in {'backend':'kindle','profile':'scribe','route':'reader',
                                                        'snapshot_size':[1860,2480],'window_size':[1860,2480]}.items()):
            return 'reader'
    except (KeyError, TypeError, ValueError, AttributeError):
        pass
    return 'not_verified'


def mappings(records, rows, anchor):
    angles = {}
    for line in records.get('image-mapping', b'').splitlines():
        match = re.fullmatch(rb'IMAGE (pre|post) (keep|turn) (1[5-8]) ([0-3])', line)
        if not match:
            raise ValueError('Invalid picture mapping')
        phase, declared, raw, rotation = match[1].decode(), match[2], int(match[3]), int(match[4])
        sample = rows.get(phase, [{}])[0]
        if phase in angles or sample.get('pressure') != raw or sample.get('framebuffer_rotation') != rotation:
            raise ValueError('Unbound picture mapping')
        delta = (rotation - int(anchor[-1])) % 4
        if delta not in (0, 2):
            raise ValueError('Unsupported scanout rotation')
        turn = ((raw in (15, 17)) != anchor.startswith('U')) != (delta == 2)
        if declared != (b'turn' if turn else b'keep'):
            raise ValueError('Picture mapping contradicts the physical samples')
        angles[phase] = 180 if turn else 0
    return angles


def distinguishable(expected, opposite):
    return expected <= 8 and opposite - expected >= 3


def reader_images(records, angles):
    result = {'method': 'relative_scanout_v2', 'absolute_orientation': 'not_measured', 'software_content': 'not_established'}
    panels = {}
    try:
        if reader_route(records) != 'reader' or not records['final.png'].startswith(b'\x89PNG\r\n\x1a\n'):
            raise ValueError('Invalid software reference')
        with Image.open(io.BytesIO(records['final.png'])) as raw:
            if raw.size != (1860, 2480) or raw.convert('RGBA').getchannel('A').getextrema() != (255,255):
                raise ValueError('Invalid Reader opacity or geometry')
            software = cropped(raw.convert('L'))
        if ImageStat.Stat(software).stddev[0] < 8:
            raise ValueError('Uninformative Reader reference')
        for phase, name in [('pre', 'panel.pgm'), ('post', 'resumed.pgm')]:
            if phase not in angles or name not in records:
                continue
            if not records[name].startswith(b'P5\n') or angles[phase] not in (0, 180):
                raise ValueError('Invalid Reader panel or mapping')
            with Image.open(io.BytesIO(records[name])) as raw:
                if raw.size != (1860, 2480) or raw.mode != 'L':
                    raise ValueError('Invalid Reader panel geometry')
                panels[phase] = cropped(raw)
            result[phase] = {'degrees': angles[phase]}
        if len(panels) == 2:
            delta = (angles['post'] - angles['pre']) % 360
            before, after = panels['pre'], panels['post']
            half_turn = after.transpose(Image.Transpose.ROTATE_180)
            aligned, alternative = (after, half_turn) if delta == 0 else (half_turn, after)
            histogram = ImageChops.difference(before, aligned).histogram()
            pixels = sum(histogram)
            expected = sum(level * count for level, count in enumerate(histogram)) / pixels
            opposite = mae(before, alternative)
            status = ('expected_relative_alignment' if distinguishable(expected, opposite) else
                      'opposite_relative_alignment' if distinguishable(opposite, expected) else 'inconclusive')
            result['comparison'] = dict(expected_delta_degrees=delta, expected_mae=expected, opposite_mae=opposite,
                                        body_pixels=pixels, different_body_pixels=pixels - histogram[0], status=status)
            turned_software = software.transpose(Image.Transpose.ROTATE_180)
            for phase, panel in [('pre', before), ('post', aligned)]:
                result[phase].update(software_mae=mae(panel, software), turned_software_mae=mae(panel, turned_software))
            content = [(result[phase]['software_mae'], result[phase]['turned_software_mae']) for phase in ('pre', 'post')]
            if (all(distinguishable(direct, turned) for direct, turned in content)
                    or all(distinguishable(turned, direct) for direct, turned in content)):
                result['software_content'] = 'consistent'
    except (OSError, ValueError, KeyError, TypeError, Image.DecompressionBombError):
        result['reference_or_frame_error'] = True
    return result


def issued_phases(receipt):
    events, seen, issued = receipt.get('operator_phases'), set(), []
    if not isinstance(events, list):
        raise ValueError('Missing authenticated phase ledger')
    previous = timestamp(receipt['acquired'])
    for event in events:
        phase, status, instant = event['phase'], event['status'], timestamp(event['at'])
        if (phase in seen or phase not in ('sleep','turn','wake','recover-wake') or not previous <= instant <= timestamp(receipt['released'])
                or status not in ('issued','suppressed_order','suppressed_no_fresh_permit')):
            raise ValueError('Invalid phase ledger')
        seen.add(phase)
        previous = instant
        if status == 'issued':
            issued.append(phase)
    if issued not in ([], ['sleep'], ['sleep','turn'], ['sleep','turn','wake'], ['sleep','recover-wake'], ['sleep','turn','recover-wake']):
        raise ValueError('Invalid sequence of operator notifications')
    return issued


def analyze(plan, receipt, archive, operator=None):
    schedule_for_sleep(plan)
    protocol = plan.get('sleep_turn_protocol')
    if protocol not in (None, 'witness_window_v1'):
        raise ValueError('Unknown frozen sleep-turn protocol')
    witness = protocol == 'witness_window_v1'
    if witness and (type(plan.get('operator_action_window_seconds')) is not int or plan['operator_action_window_seconds'] != 16):
        raise ValueError('Unexpected frozen operator action window')
    operator = witness_report(operator, plan['label'])
    if (receipt.get('label') != plan['label'] or receipt.get('scope') != 'transaction_only' or receipt.get('purpose') != plan['purpose']
            or receipt.get('owner_token_checked_on_release') is not True or receipt.get('recovery_required')
            or timestamp(receipt['released']) < timestamp(receipt['acquired'])):
        raise ValueError('Missing matching released transaction')
    cues = issued_phases(receipt)
    result = {'label':plan['label'], 'device':plan['device'], 'firmware':plan['firmware'], 'released':receipt['released'],
              'issued_notices':cues, 'sleep_turn':'not_established', 'same_resident_cycle':'not_established', 'reader':'not_established',
              'qualification':'not_established', 'not_measured':['deep suspend','optical ink','physical touch mapping','startup duration']}
    if witness:
        result.update(protocol=protocol, operator_report=operator, resident_wake_qualification='not_established')
    ops = receipt.get('operations', [])
    if (receipt.get('passed') is not True or receipt.get('error') or not isinstance(ops,list) or
            any(type(op.get('status')) is not int or op['status'] != 0 for op in ops) or [(op['mode'],op['case']) for op in ops] != SEQUENCE):
        result['collection'] = 'not_completed'
        return result
    records = read_visit(archive,plan['label'])
    initial, launch, returned = (pose(records[key]) for key in ('portrait','portrait-launch','portrait-return'))
    if records.get('portrait-source') != b'native_idle_calibration\n' or launch != initial or initial not in ('U:3','D:1'):
        raise ValueError('Unsupported or inconsistent launch anchor')
    seen = records.get('observation', b'').decode('ascii')
    success = 'observed resident_wake\n' if witness else 'observed changed_sleep_cycle\n'
    if seen != success and seen not in ['not_qualified '+reason+'\n' for reason in WAKES]:
        raise ValueError('Invalid device outcome')
    if 'initial_diagnostic' in plan and (plan['initial_diagnostic'] != 'bounded_v1' or 'initial-diagnostic' not in records):
        raise ValueError('Missing required or unrecognized frozen initial diagnostic')
    result['initial_reader_diagnostic'] = initial_diagnostic(records.get('initial-diagnostic'), seen == 'not_qualified initial_reader\n',
                                                            {'pre.row','panel.pgm','image-mapping'} <= records.keys(), receipt['operator_phases'])
    trace = orientation_sample(records['request'].decode('ascii'), records['markers'].decode('ascii'))
    rows = sensor_rows(records, initial, trace['request_ns'] + trace['t_from_request_ns']['first_application_frame_send'], witness)
    result.update(collection='completed_and_released', device_observation=seen.strip(), native_before=initial, native_return=returned,
                  orientation_queries={site:trace[site]['outcome'] for site in ('splash','event_loop')}, samples=rows)
    result['both_orientation_sites_reused'] = all(value == 'reused' for value in result['orientation_queries'].values())
    if witness:
        result['turn_window'] = turn_window(records.get('turn-window'), len(rows.get('sleep-trace',[])))
        if 'wake' in cues and (result['turn_window'] or {}).get('end_reason') != 'elapsed':
            raise ValueError('Wake notice has no complete witnessed action window')
        result['pose_evidence'] = pose_evidence(rows)
        result['sleep_turn'] = result['pose_evidence']['status']
        result['final_application_route'] = reader_route(records)
    elif 'sleep-pre' in rows and 'sleep-post' in rows:
        result['sleep_turn'] = 'opposite_pressures_inside_stopped_screenSaver_cycle'
    finished = (all(phase in rows for phase in ('pre','post-status')) and any(rows.get(phase) for phase in ('sleep-pre','sleep-trace','sleep-post'))
                if witness else all(phase in rows for phase in PHASES)) and seen == success
    if finished:
        result['same_resident_cycle'] = 'stopped_then_active_same_reader_identity_and_cycle_records'
    images = reader_images(records, mappings(records, rows, launch))
    result['reader_images'] = images
    comparison = images.get('comparison', {})
    if comparison.get('status') == 'expected_relative_alignment' and images['software_content'] == 'consistent':
        result['reader'] = ('identical_aligned_body' if comparison['different_body_pixels'] == 0
                            else 'approximately_matching_aligned_body')
    if witness and finished and result['reader'] == 'identical_aligned_body':
        result['resident_wake_qualification'] = 'same_cycle_and_same_aligned_reader_body'
    turned_and_held = (not witness or (result['sleep_turn'] == 'opposite_directions_observed_while_stopped' and result['pose_evidence']['awake_endpoints_differ']
                                      and len(set(result['pose_evidence']['after_wake_directions'].values())) == 1))
    if finished and turned_and_held and result['both_orientation_sites_reused'] and cues == ['sleep','turn','wake'] and result['reader'] == 'identical_aligned_body':
        result['qualification'] = 'confirmed_same_reader_and_changed_pose_in_scope'
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ('plan','operations','archive','output'):
        parser.add_argument(arg, type=Path)
    parser.add_argument('--operator-report', type=Path, help='Independent report for this label only; never substitutes for device observations')
    args = parser.parse_args()
    result = analyze(json.loads(args.plan.read_text()),json.loads(args.operations.read_text()),args.archive,
                     json.loads(args.operator_report.read_text()) if args.operator_report else None)
    with args.output.open('x') as output:
        json.dump(result,output,indent=2)
        output.write('\n')
