"""Freeze an instrumented ARM build and its paired Scribe sensor/scanout witness."""

import argparse
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import tempfile
import tomllib

from assisted_shelf import freeze_shelf, verify_payload
from freeze import ROOT
from orientation_contract import CANDIDATE
from pose_flip_device import CASE, device_program
from prepare_orientation import DEVICE, FACES, FIRMWARE, local, read_record


CONTRACT = dict(purpose='assisted-startup-pose-change', schedule=[CASE], device=DEVICE, firmware=FIRMWARE,
                candidate_features=CANDIDATE, clock='CLOCK_MONOTONIC', sensor_samples=401, sensor_period_ns=50_000_000,
                sensor_budget_ns=27_000_000_000, expected_start_references=['D:1', 'U:3'],
                evidence_policy='bound-pose-checkpoint-independent-of-calibration')
DRAW_CONTRACT = CONTRACT | dict(purpose='assisted-startup-first-draw-turn', evidence_policy='bound-pose-first-draw-and-submit')
RECONCILE = 'startup-orientation-reconcile'
RECONCILED_CONTRACT = DRAW_CONTRACT | dict(purpose='assisted-startup-reconciled-draw-turn',
                                          evidence_policy='bound-reconciled-event-loop-first-draw-and-submit',
                                          candidate_features=[*CANDIDATE, RECONCILE])
APP_POLICY = 'first-application-render-begin-v1'
SENSOR_POLICY = 'startup-draw 401 50000000 27000000000 CLOCK_MONOTONIC ABS_PRESSURE FBIOGET_VSCREENINFO'
BUILD_RESULT = Path('candidate-final/result.json')
RECONCILED_RESULT = Path('reconcile/result.json')
ABI = 'armv7-unknown-linux-musleabihf'
CARGO_TARGET = f'target/{ABI}/release/weread'
COMMAND = ['cargo', 'zigbuild', '--locked', '--offline', '--release', '--target', ABI, '--no-default-features',
           '--features', ','.join(CANDIDATE), '--bin', 'weread', '--message-format=json-render-diagnostics']
ELF_CHECK = ROOT / 'docs/review/library-startup-latency-20261119/rebuild-arm.py'


def matched_schedule(plan, contract):
    if (not isinstance(plan, dict) or not isinstance(plan.get('label'), str) or not re.fullmatch('[0-9a-f]{16}', plan['label'])
            or any(plan.get(key) != value for key, value in contract.items())):
        raise ValueError('Expected the matching Scribe startup-turn measurement contract')
    return [CASE]


def schedule(plan):
    return matched_schedule(plan, CONTRACT)


def draw_schedule(plan):
    return matched_schedule(plan, DRAW_CONTRACT)


def checked_receipt(record, lane='startup-first-draw'):
    with (ROOT / 'Cargo.toml').open('rb') as manifest:
        product = tomllib.load(manifest)
    tools = read_record(ROOT / 'docs/review/startup-orientation-20261121/arm-worker.json')['toolchain']
    selected = CANDIDATE if lane != 'reconcile' else [*CANDIDATE[:-1], RECONCILE]
    backend = sorted([CANDIDATE[-1], RECONCILE] if lane == 'reconcile' else [CANDIDATE[-1]])
    application = sorted([*CANDIDATE[:-1], *backend])
    command = [','.join(selected) if arg == ','.join(CANDIDATE) else arg for arg in COMMAND]
    expected = dict(source='/work/src', lane=lane, abi=ABI, command=command,
                    release_profile=product['profile']['release'], application_features=application,
                    backend_features=backend, local_dependencies=['i-slint-core','swash','zeno','zip'],
                    generated_receipts=FACES, both_recompiled=True, qemu_cpu='cortex-a8',
                    version_output='weread ' + product['package']['version'], runtime_policy=APP_POLICY,
                    toolchain=dict(rustc=tools['rustc'], cargo=tools['cargo'], zig=tools['zig'], cargo_zigbuild=tools['cargo-zigbuild']))
    if not isinstance(record, dict) or record.get('both_recompiled') is not True:
        raise ValueError('Require a fresh application and backend from the first-draw build')
    if lane in ('baseline','reconcile'):
        provenance = record.get('source_provenance')
        log = record.get('cargo_artifacts')
        if (not isinstance(log, str) or not re.fullmatch(rf'logs/{lane}-[a-z0-9_]{{8}}/events\.jsonl', log)
                or not isinstance(provenance, dict) or not re.fullmatch(r'[a-f0-9]{40}', str(provenance.get('base')))
                or not isinstance(paths := provenance.get('paths'), list) or not 1 <= len(paths) <= 64
                or any(not isinstance(path, str) or not re.fullmatch(r'[A-Za-z0-9_-]+(?:[./][A-Za-z0-9_-]+)*', path) for path in paths)
                or len(set(paths)) != len(paths)):
            raise ValueError('Missing the private source admission or lane-specific Cargo artifact journal')
        expected.update(cargo_target=CARGO_TARGET, cargo_artifacts=log, source_provenance=provenance,
                        cargo_graphs={'weread':application, 'slint-backend-kindle':backend})
    if any(record.get(key) != value for key, value in expected.items()):
        raise ValueError('Unexpected first-draw build features, toolchain, policy or provenance')
    elf = record.get('elf')
    if (not isinstance(elf, dict) or set(elf) != {'bytes','elf_flags','program_types'} or type(elf['bytes']) is not int
            or not 84 <= elf['bytes'] <= 110 * 1024**2 or not isinstance(elf['elf_flags'], str)
            or not re.fullmatch(r'0x[0-9a-f]{1,8}', elf['elf_flags']) or (int(elf['elf_flags'],16) & 0xff000600) != 0x05000400
            or not isinstance(elf['program_types'], list) or not 1 <= len(elf['program_types']) <= 128
            or any(type(kind) is not int or not 0 <= kind < 2**32 for kind in elf['program_types'])
            or 1 not in elf['program_types'] or {2,3}.intersection(elf['program_types'])):
        raise ValueError('Require the bounded static ARM32 hard-float ELF receipt')
    return expected | {'elf': elf}


def built_candidate(worker, result, lane='startup-first-draw'):
    relative = BUILD_RESULT if lane == 'startup-first-draw' else Path(lane, 'result.json')
    if local(worker, result) != worker / relative:
        raise ValueError(f'Use this worker\'s independently built {relative}')
    receipt = checked_receipt(read_record(result), lane)
    executable = local(worker, result.with_name('weread'))
    if runpy.run_path(str(ELF_CHECK))['inspect_elf'](executable) != receipt['elf']:
        raise ValueError('Actual candidate ELF does not match its build receipt')
    return executable, receipt


def check_cargo_artifacts(worker, receipt):
    path = local(worker, worker / receipt['cargo_artifacts'])
    if not 0 < path.stat().st_size <= 16 * 1024**2:
        raise ValueError('Missing or oversized Cargo artifact journal')
    targets = {('weread', ('bin',)): ('/work/src/Cargo.toml', '/work/' + CARGO_TARGET, receipt['application_features']),
               ('slint_backend_kindle', ('lib',)): ('/work/src/vendor/slint-backend-kindle/Cargo.toml', None, receipt['backend_features'])}
    seen = set()
    last = None
    with path.open(encoding='utf-8') as journal:
        for line in journal:
            last = json.loads(line)
            if last['reason'] != 'compiler-artifact':
                continue
            key = (last['target']['name'], tuple(last['target']['kind']))
            if key not in targets:
                continue
            if (key in seen or last['fresh'] is not False or last['profile']['test'] is not False or
                    (last['manifest_path'], last['executable'], last['features']) != targets[key]):
                raise ValueError('Expected a freshly compiled product target with its exact manifest and feature closure')
            seen.add(key)
    if seen != targets.keys() or last != {'reason':'build-finished', 'success':True}:
        raise ValueError('Missing Cargo product artifacts or successful build completion')


def same_bytes(first, second):
    if first.stat().st_size != second.stat().st_size:
        return False
    with first.open('rb') as left, second.open('rb') as right:
        while chunk := left.read(1024*1024):
            if chunk != right.read(len(chunk)):
                return False
        return not right.read(1)


def paired_builds(worker, result):
    candidate, changed = built_candidate(worker, result, 'reconcile')
    control, stable = built_candidate(worker, worker / 'baseline/result.json', 'baseline')
    admission = read_record(local(worker, worker / 'final-source.json'))
    if admission != changed['source_provenance'] or admission != stable['source_provenance']:
        raise ValueError('Baseline and reconciled candidate must come from the same admitted source')
    for path in admission['paths']:
        local(worker / 'src', worker / 'src' / path)
    for receipt in (stable, changed):
        check_cargo_artifacts(worker, receipt)
    if not same_bytes(local(worker, worker / CARGO_TARGET), candidate) or same_bytes(control, candidate):
        raise ValueError('The candidate must match the Cargo output and differ from the same-source baseline')
    return candidate, {'draw_build':changed, 'control_build':stable}


def reconciled_schedule(plan):
    result = matched_schedule(plan, RECONCILED_CONTRACT)
    changed = checked_receipt(plan.get('draw_build'), 'reconcile')
    stable = checked_receipt(plan.get('control_build'), 'baseline')
    if (changed['source_provenance'] != stable['source_provenance'] or
            not isinstance(plan.get('members'), dict) or plan['members'].get('B/app/weread') != changed['elf']['bytes']):
        raise ValueError('Reconciled measurement lacks matching same-source build evidence')
    return result


def runnable_schedule(plan):
    if plan.get('purpose') == RECONCILED_CONTRACT['purpose']:
        return reconciled_schedule(plan)
    return draw_schedule(plan)


def check_policy(worker, program, option, expected):
    if qemu := shutil.which('qemu-arm'):
        command = [qemu, '-cpu', 'cortex-a8', str(program), option]
    else:
        command = ['/opt/kindle-toolchain/ubuntu-run', str(worker), 'qemu-arm', '-cpu', 'cortex-a8',
                   '/work/' + str(program.relative_to(worker)), option]
    result = subprocess.run(command, capture_output=True, timeout=30)
    if result.returncode or result.stderr or result.stdout != (expected + '\n').encode('ascii'):
        raise ValueError(f'The packaged ARM executable does not implement {option}')


def verify_recovery(directory):
    plan = read_record(directory / 'plan.json')
    runnable_schedule(plan)
    reconciled = plan['purpose'] == RECONCILED_CONTRACT['purpose']
    proof = checked_receipt(plan.get('draw_build'), 'reconcile' if reconciled else 'startup-first-draw')
    script = directory / 'device.sh'
    if (script.is_symlink() or not script.is_file() or not 0 < script.stat().st_size <= 32768 or
            script.read_bytes() != device_program(proof['elf']['bytes'], reconciled=reconciled).encode('ascii')):
        raise ValueError('The frozen startup-turn transaction does not match the reviewed program')
    return plan


def verify_plan(directory):
    plan = verify_recovery(directory)
    worker = directory.parent.resolve(strict=True)
    if plan['purpose'] == RECONCILED_CONTRACT['purpose']:
        candidate, pair = paired_builds(worker, worker / RECONCILED_RESULT)
        proof = pair['draw_build']
    else:
        candidate, proof = built_candidate(worker, worker / BUILD_RESULT)
        pair = {'draw_build':proof}
    if any(plan[key] != value for key, value in pair.items()):
        raise ValueError('Frozen first-draw provenance does not match this worker\'s build')

    def check_archive(archive):
        with tempfile.TemporaryDirectory(dir=worker, prefix='verify-draw-') as temporary:
            sensor = Path(temporary) / 'sensor'
            executable = Path(temporary) / 'weread'
            if archive.getmember('pose-snapshot').size > 512 * 1024:
                raise ValueError('Oversized packaged pose sampler')
            with archive.extractfile('pose-snapshot') as input_file, sensor.open('xb') as output:
                shutil.copyfileobj(input_file, output)
            sensor.chmod(0o700)
            runpy.run_path(str(ELF_CHECK))['inspect_elf'](sensor)
            check_policy(worker, sensor, '--draw-policy', SENSOR_POLICY)
            with archive.extractfile('B/app/weread') as packed, candidate.open('rb') as built, executable.open('xb') as output:
                while chunk := packed.read(1024 * 1024):
                    if chunk != built.read(len(chunk)):
                        raise ValueError('Packaged candidate differs from the fresh build')
                    output.write(chunk)
                if built.read(1):
                    raise ValueError('Incomplete packaged candidate')
            executable.chmod(0o700)
            check_policy(worker, executable, '--startup-draw-trace-policy', APP_POLICY)
            check_policy(worker, executable, '--version', proof['version_output'])

    return verify_payload(directory, plan, proof['elf']['bytes'], check_archive=check_archive)


def freeze(retained, directory, sensor, scene, build_result, *, reconciled=False):
    worker = directory.parent.resolve(strict=True)
    if reconciled:
        candidate, builds = paired_builds(worker, build_result.absolute())
        contract = RECONCILED_CONTRACT
    else:
        candidate, receipt = built_candidate(worker, build_result.absolute())
        contract, builds = DRAW_CONTRACT, {'draw_build':receipt}
    check_policy(worker, candidate, '--startup-draw-trace-policy', APP_POLICY)
    sensor = local(worker, sensor)
    check_policy(worker, sensor, '--draw-policy', SENSOR_POLICY)
    return freeze_shelf(retained, directory, sensor, scene, candidate, contract | builds,
                        device_program(builds['draw_build']['elf']['bytes'], reconciled=reconciled), verify_plan)


if __name__ == '__main__':
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='operation', required=True)
    prep = commands.add_parser('freeze')
    prep.add_argument('--reconciled', action='store_true', help='Require the separately compiled baseline/reconcile pair and its Cargo artifacts.')
    for key in ('retained', 'plan', 'sensor', 'scene', 'build-result'):
        prep.add_argument('--' + key, type=Path, required=True)
    commands.add_parser('verify').add_argument('--plan', type=Path, required=True)
    args = parser.parse_args()
    result = (freeze(args.retained, args.plan, args.sensor, args.scene, args.build_result, reconciled=args.reconciled)
              if args.operation == 'freeze' else verify_plan(args.plan))
    print(json.dumps(dict(label=result['label'], frozen=result['schedule'], device_accessed=False, lease_requested=False)))
