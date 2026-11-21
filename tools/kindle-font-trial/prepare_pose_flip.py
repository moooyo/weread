"""Freeze a new isolated candidate launch with a continuous physical pose witness."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

from assisted_shelf import freeze_shelf, verify_payload
from orientation_contract import CANDIDATE
from pose_flip_device import CASE, device_program
from prepare_orientation import DEVICE, FIRMWARE, read_record


CONTRACT = dict(purpose='assisted-startup-pose-change', schedule=[CASE], device=DEVICE, firmware=FIRMWARE,
                candidate_features=CANDIDATE, clock='CLOCK_MONOTONIC', sensor_samples=401, sensor_period_ns=50_000_000,
                sensor_budget_ns=27_000_000_000, expected_start_references=['D:1', 'U:3'],
                evidence_policy='bound-pose-checkpoint-independent-of-calibration')


def schedule(plan):
    if (not isinstance(plan, dict) or not isinstance(plan.get('label'), str) or not re.fullmatch('[0-9a-f]{16}', plan['label'])
            or any(plan.get(key) != value for key, value in CONTRACT.items())):
        raise ValueError('Expected a frozen single Scribe startup turn')
    return [CASE]


def verify_recovery(directory):
    plan = read_record(directory / 'plan.json')
    schedule(plan)
    script = directory / 'device.sh'
    if (script.is_symlink() or not script.is_file() or not 0 < script.stat().st_size <= 32768 or
            script.read_bytes() != device_program().encode('ascii')):
        raise ValueError('The frozen startup-turn transaction does not match the reviewed program')
    return plan


def verify_plan(directory):
    return verify_payload(directory, verify_recovery(directory))


def freeze(retained, directory, sensor, scene):
    worker = directory.parent.resolve(strict=True)
    if sensor.is_symlink() or not sensor.is_file():
        raise ValueError('A private regular production sensor is required')
    sensor = sensor.resolve(strict=True)
    if qemu := shutil.which('qemu-arm'):
        command = [qemu, '-cpu', 'cortex-a8', str(sensor), '--flip-policy']
    else:
        command = ['/opt/kindle-toolchain/ubuntu-run', str(worker), 'qemu-arm', '-cpu', 'cortex-a8',
                   '/work/' + str(sensor.relative_to(worker)), '--flip-policy']
    expected = (f"startup-flip {CONTRACT['sensor_samples']} {CONTRACT['sensor_period_ns']} "
                f"{CONTRACT['sensor_budget_ns']} CLOCK_MONOTONIC\n").encode('ascii')
    if subprocess.check_output(command, timeout=15) != expected:
        raise ValueError('The ARM helper does not implement the startup sampling policy')
    return freeze_shelf(retained, directory, sensor, scene, CONTRACT, device_program(), verify_plan)


if __name__ == '__main__':
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='operation', required=True)
    prep = commands.add_parser('freeze')
    for key in ('retained', 'plan', 'sensor', 'scene'):
        prep.add_argument('--' + key, type=Path, required=True)
    commands.add_parser('verify').add_argument('--plan', type=Path, required=True)
    args = parser.parse_args()
    result = freeze(args.retained, args.plan, args.sensor, args.scene) if args.operation == 'freeze' else verify_plan(args.plan)
    print(json.dumps(dict(label=result['label'], frozen=result['schedule'], device_accessed=False, lease_requested=False)))
