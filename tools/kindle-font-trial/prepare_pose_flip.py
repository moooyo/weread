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
APP_POLICY = 'first-application-render-begin-v1'
SENSOR_POLICY = 'startup-draw 401 50000000 27000000000 CLOCK_MONOTONIC ABS_PRESSURE FBIOGET_VSCREENINFO'
BUILD_RESULT = Path('candidate-final/result.json')
ABI = 'armv7-unknown-linux-musleabihf'
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


def checked_receipt(record):
    with (ROOT / 'Cargo.toml').open('rb') as manifest:
        product = tomllib.load(manifest)
    tools = read_record(ROOT / 'docs/review/startup-orientation-20261121/arm-worker.json')['toolchain']
    expected = dict(source='/work/src', lane='startup-first-draw', abi=ABI, command=COMMAND,
                    release_profile=product['profile']['release'], application_features=sorted(CANDIDATE),
                    backend_features=['startup-orientation-reuse'], local_dependencies=['i-slint-core','swash','zeno','zip'],
                    generated_receipts=FACES, both_recompiled=True, qemu_cpu='cortex-a8',
                    version_output='weread ' + product['package']['version'], runtime_policy=APP_POLICY,
                    toolchain=dict(rustc=tools['rustc'], cargo=tools['cargo'], zig=tools['zig'], cargo_zigbuild=tools['cargo-zigbuild']))
    if not isinstance(record, dict) or record.get('both_recompiled') is not True:
        raise ValueError('Require a fresh application and backend from the first-draw build')
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


def built_candidate(worker, result):
    if local(worker, result) != worker / BUILD_RESULT:
        raise ValueError('Use this worker\'s independently built candidate-final/result.json')
    receipt = checked_receipt(read_record(result))
    executable = local(worker, result.with_name('weread'))
    if runpy.run_path(str(ELF_CHECK))['inspect_elf'](executable) != receipt['elf']:
        raise ValueError('Actual candidate ELF does not match its build receipt')
    return executable, receipt


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
    draw_schedule(plan)
    proof = checked_receipt(plan.get('draw_build'))
    script = directory / 'device.sh'
    if (script.is_symlink() or not script.is_file() or not 0 < script.stat().st_size <= 32768 or
            script.read_bytes() != device_program(proof['elf']['bytes']).encode('ascii')):
        raise ValueError('The frozen startup-turn transaction does not match the reviewed program')
    return plan


def verify_plan(directory):
    plan = verify_recovery(directory)
    worker = directory.parent.resolve(strict=True)
    candidate, proof = built_candidate(worker, worker / BUILD_RESULT)
    if plan['draw_build'] != proof:
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


def freeze(retained, directory, sensor, scene, build_result):
    worker = directory.parent.resolve(strict=True)
    candidate, receipt = built_candidate(worker, build_result.absolute())
    check_policy(worker, candidate, '--startup-draw-trace-policy', APP_POLICY)
    sensor = local(worker, sensor)
    check_policy(worker, sensor, '--draw-policy', SENSOR_POLICY)
    return freeze_shelf(retained, directory, sensor, scene, candidate, DRAW_CONTRACT | {'draw_build':receipt},
                        device_program(receipt['elf']['bytes']), verify_plan)


if __name__ == '__main__':
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='operation', required=True)
    prep = commands.add_parser('freeze')
    for key in ('retained', 'plan', 'sensor', 'scene', 'build-result'):
        prep.add_argument('--' + key, type=Path, required=True)
    commands.add_parser('verify').add_argument('--plan', type=Path, required=True)
    args = parser.parse_args()
    result = freeze(args.retained, args.plan, args.sensor, args.scene, args.build_result) if args.operation == 'freeze' else verify_plan(args.plan)
    print(json.dumps(dict(label=result['label'], frozen=result['schedule'], device_accessed=False, lease_requested=False)))
