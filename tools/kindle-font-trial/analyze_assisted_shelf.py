"""Read one released assisted visit; never infer physical input from a final route."""

import argparse
import json
from pathlib import Path
import re
import tarfile

from analyze_orientation import orientation_sample
from analyze_pose_visit import pose, timestamp
from run_assisted_shelf import schedule_for_shelf


CASE = 'B-library'
LIMITS = {'portrait': 4, 'portrait-source': 32, 'sensor': 1024, 'portrait-launch': 4, 'portrait-return': 4,
          'observation': 64, 'request': 96, 'markers': 4096, 'final.metadata.json': 2048, 'shelf': 128}
BASE = {'portrait', 'portrait-source', 'sensor', 'portrait-return', 'observation'}
SEQUENCE = [('admit', 'none'), ('reference', 'none'), ('stage', 'none'), ('case', CASE), ('restore', 'none'),
            ('collect', 'none'), ('close', 'none'), ('absent', 'none')]
OPERATOR_FIELDS = {'held_still_through_visit', 'saw_upright_populated_shelf', 'tapped_profile_tab_once', 'saw_profile_page'}


def records(path, label):
    keys = {f'out/{CASE}/{key}': size for key, size in LIMITS.items()} | {'out/orientation-batch': 17}
    entries = {}
    with tarfile.open(path, 'r|gz') as archive:
        for item in archive:
            if item.name not in keys or item.name in entries or not item.isfile() or not 0 < item.size <= keys[item.name]:
                raise ValueError('Unrecognized public assisted-visit record')
            entries[item.name] = archive.extractfile(item).read()
    if entries.pop('out/orientation-batch', None) != (label + '\n').encode('ascii'):
        raise ValueError('Archive label differs from the leased visit')
    return {key.split('/')[-1]: raw for key, raw in entries.items()}


def samples(raw):
    lines = raw.decode('ascii').splitlines(keepends=True)
    result = []
    if len(lines) != 10:
        raise ValueError('Expected ten consecutive independent samples')
    for index, line in enumerate(lines, 1):
        match = re.fullmatch(r'([1-9][0-9]?) ([1-9][0-9]{0,18}) ([1-9][0-9]{0,18}) (15|16|17|18)\n', line)
        if not match:
            raise ValueError('Invalid native sensor sample')
        number, start, end, pressure = map(int, match.groups())
        if number != index or not (result[-1][1] if result else 0) < start <= end < 2**63:
            raise ValueError('Discontinuous pre-launch sensor samples')
        result.append((start, end, pressure))
    return result


def final_route(raw):
    metadata = json.loads(raw)
    expected = {'profile': 'scribe', 'backend': 'kindle', 'design_logical_size': [930, 1240],
                'logical_size': [930, 1240], 'window_size': [1860, 2480], 'snapshot_size': [1860, 2480], 'scale_factor': 2}
    if (not isinstance(metadata, dict) or set(metadata) != {*expected, 'route'}
            or any(metadata[key] != value for key, value in expected.items())
            or not isinstance(metadata['route'], str) or not re.fullmatch('[a-z][a-z0-9_-]{0,31}', metadata['route'])):
        raise ValueError('Unexpected public candidate capture metadata')
    return metadata['route']


def shelf_scene(raw, sample, startup):
    if raw == b'not_observed\n':
        return {'status': 'not_observed'}
    match = re.fullmatch(rb'SHELF ([1-9][0-9]{0,18}) ([1-9][0-9]{0,18}) ([0-3]) (grid|list|empty|none)\n', raw)
    if not match:
        raise ValueError('Invalid software shelf observation')
    start, end, rotation = map(int, match.groups()[:3])
    kind = match[4].decode('ascii')
    sent = sample['request_ns'] + sample['t_from_request_ns']['first_application_frame_send']
    if rotation != int(startup[-1]) or not sent <= start <= end < 2**63:
        return {'status': 'insufficient_clock_or_rotation'}
    return {'status': 'observed_populated_shelf' if kind in ('grid', 'list') else 'public_entries_not_observed',
            'scene': kind, 'clock': 'CLOCK_MONOTONIC', 'read_ns': [start, end]}


def analyze_shelf(plan, receipt, path, operator=None):
    schedule_for_shelf(plan)
    if (receipt.get('label') != plan['label'] or receipt.get('purpose') != plan['purpose']
            or receipt.get('scope') != 'transaction_only' or receipt.get('owner_token_checked_on_release') is not True
            or receipt.get('recovery_required') or timestamp(receipt['released']) < timestamp(receipt['acquired'])):
        raise ValueError('Not a matching, released assisted visit')
    if operator is not None and (not isinstance(operator, dict) or operator.get('label') != plan['label']
            or set(operator) != {'label', *OPERATOR_FIELDS} or any(type(operator[key]) is not bool for key in OPERATOR_FIELDS)):
        raise ValueError('Operator observations must be explicitly bound to this visit')
    report = {'label': plan['label'], 'released': receipt['released'], 'device': plan['device'], 'firmware': plan['firmware'],
              'collection': 'not_qualified', 'single_profile_tap': 'not_established', 'operator_report': operator,
              'not_measured': ['exact first complete Library frame', 'optical ink', 'turn during startup',
                               'sleep or wake', 'touch at other targets', 'ordinary card startup latency']}
    operations = receipt.get('operations')
    if (receipt.get('passed') is not True or receipt.get('error') or not isinstance(operations, list)
            or any(not isinstance(op, dict) or type(op.get('status')) is not int or op['status'] != 0 for op in operations)
            or [(op.get('mode'), op.get('case')) for op in operations] != SEQUENCE):
        return report
    initial = receipt.get('initial_native_reference')
    if (initial not in ('U:3', 'D:1') or not timestamp(receipt['acquired']) <= timestamp(receipt['operator_cue_issued']) <= timestamp(receipt['released'])):
        raise ValueError('No valid pre-instruction reference in this lease')
    entries = records(path, plan['label'])
    seen = entries.get('observation')
    if (seen not in (b'not_started pose_gate\n', b'captured candidate_library_exit\n')
            or set(entries) != (BASE if seen.startswith(b'not_started') else set(LIMITS))
            or entries['portrait-source'] != b'native_idle_calibration\n'):
        raise ValueError('Incomplete assisted visit or inconsistent launch evidence')
    native, returned = pose(entries['portrait']), pose(entries['portrait-return'])
    points = samples(entries['sensor'])
    expected_pose = native == 'U:3' and all(point[2] in (15, 17) for point in points)
    report.update(collection='completed_and_released', initial_native_reference=initial, native_before=native, native_return=returned,
                  prelaunch_snapshot_raw=[point[2] for point in points], prelaunch_sensor_ns=[points[0][0], points[-1][1]])
    if seen.startswith(b'not_started'):
        report['application'] = 'not_started_pose_gate'
        return report
    if not expected_pose or pose(entries['portrait-launch']) != native:
        raise ValueError('Candidate launched without the independently confirmed target portrait')
    trace = orientation_sample(entries['request'].decode('ascii'), entries['markers'].decode('ascii'))
    if points[-1][1] >= trace['request_ns']:
        raise ValueError('Independent posture observations must precede the launch request')
    scene = shelf_scene(entries['shelf'], trace, native)
    route = final_route(entries['final.metadata.json'])
    report.update(application='launched', entry_route_requested='library', native_launch=native,
                  orientation_queries={site: trace[site]['outcome'] for site in ('splash', 'event_loop')},
                  final_application_route=route, digital_shelf=scene)
    if route != 'settings':
        report['single_profile_tap'] = 'expected_final_route_not_observed'
    elif operator is None:
        report['single_profile_tap'] = 'settings_observed_operator_unconfirmed'
    elif not all(operator[key] for key in OPERATOR_FIELDS):
        report['single_profile_tap'] = 'operator_observations_do_not_confirm'
    elif returned != native:
        report['single_profile_tap'] = 'return_pose_different'
    else:
        report['single_profile_tap'] = 'operator_and_route_confirmed'
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('plan', 'operations', 'archive', 'output'):
        parser.add_argument(name, type=Path)
    parser.add_argument('--operator-report', type=Path, help='A separate label-bound human observation record, if supplied')
    args = parser.parse_args()
    outcome = analyze_shelf(json.loads(args.plan.read_text()), json.loads(args.operations.read_text()), args.archive,
                            json.loads(args.operator_report.read_text()) if args.operator_report else None)
    args.output.write_text(json.dumps(outcome, indent=2) + '\n')
