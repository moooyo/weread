"""Freeze or verify one isolated Scribe Reader/stock power-cycle visit. Does not connect to a Kindle."""

import argparse
import io
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import struct
import subprocess
import tarfile

from freeze import launcher
from orientation_contract import CANDIDATE
from orientation_device import render_device
from prepare_orientation import DEVICE, FIRMWARE, HELPERS, arm_inputs, public_seed, read_record, trial_document


HERE = Path(__file__).resolve().parent
ARM_A, ARM_B = 95202712, 95205016
MEMBERS = ('document', 'measure', 'seed.tar.gz', 'pose-splash', 'pose-wake', 'B/launch.sh',
           'B/app/weread', *(f'B/app/{name}' for name in HELPERS))
CONTRACT = dict(purpose='startup-orientation-pose-wake', schedule=['B-reader'], device=DEVICE, firmware=FIRMWARE,
                candidate_features=CANDIDATE, clock='CLOCK_MONOTONIC', expected_panel_alignment_degrees=0)


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('The reviewed collector boundary has changed')
    return text.replace(old, new)


def device_program():
    body = render_device(ARM_A, ARM_B)
    old_members = 'document measure seed.tar.gz A/launch.sh B/launch.sh A/app/weread B/app/weread A/app/weread-frame-guard B/app/weread-frame-guard A/app/weread-native-query B/app/weread-native-query'
    body = once(body, old_members, ' '.join(MEMBERS))
    body = once(body, 'NR!=11 || bytes>230686720', f'NR!={len(MEMBERS)} || bytes>127926272')
    body = once(body, 'for arm in A B; do', 'for arm in B; do')
    begin = body.index('    [ "$(stat -c %s "$stage/A/app/weread")')
    end = body.index('\n}', begin)
    body = body[:begin] + f'    [ "$(stat -c %s "$stage/B/app/weread")" = {ARM_B} ] &&\n' + \
        '        plain "$stage/pose-splash" && [ -x "$stage/pose-splash" ] && plain "$stage/pose-wake" && [ -x "$stage/pose-wake" ]' + body[end:]
    body = once(body, 'route=${name#*-}; dwell=12000', 'route=${name#*-}; dwell=35000')
    body = once(body, ' && cmp -s "$out/portrait" "$out/portrait-return"', '')
    body = once(body, 'leaves="$leaves panel.pgm final.png final.metadata.json"',
                'leaves="$leaves panel.pgm final.png final.metadata.json splash splash-status wake"; '
                'if grep -q "^observed " "$entry/wake"; then leaves="$leaves resumed.pgm"; fi')
    before = body.index('capture_case() {')
    after = body.index('no_trial() {', before)
    body = body[:before] + (HERE / 'pose_visit.sh').read_text() + '\n' + body[after:]
    body = once(body, 'case) capture_case;;', 'case) pose_case;;\n    absent) no_trial && printf \'absent\\n\';;')
    body = once(body, '        rm -f "$stage/armed"', '        pose_recover_wake || fail wake_recovery\n        rm -f "$stage/armed"')
    body = once(body, 'for check in $(seq 1 80); do', 'for check in $(seq 1 256); do')
    if len(body.encode('ascii')) > 32768:
        raise ValueError('Visit program exceeds the established transport')
    return body


def verify_recovery(plan_dir):
    plan_dir = Path(plan_dir)
    plan = read_record(plan_dir / 'plan.json')
    if any(plan.get(key) != value for key, value in CONTRACT.items()) or not re.fullmatch(r'[0-9a-f]{16}', plan['label']):
        raise ValueError('Unrecognized public pose visit')
    script = plan_dir / 'device.sh'
    if script.is_symlink() or not script.is_file() or script.stat().st_size > 32768 or script.read_bytes() != device_program().encode('ascii'):
        raise ValueError('Recovery requires the frozen bounded device program')
    return plan


def verify_plan(plan_dir):
    plan_dir = Path(plan_dir)
    plan = verify_recovery(plan_dir)
    sizes = plan['members']
    if (set(sizes) != set(MEMBERS) or sizes['B/app/weread'] != ARM_B or
            any(type(value) is not int or value <= 0 for value in sizes.values()) or sum(sizes.values()) > 127926272):
        raise ValueError('Unexpected public archive manifest')
    path = plan_dir / 'inputs.tgz'
    if path.is_symlink() or not path.is_file() or path.stat().st_size != plan['archive_bytes']:
        raise ValueError('The frozen visit archive is unavailable')
    with tarfile.open(path, 'r:gz') as archive:
        names = []
        for member in archive:
            if (not member.isfile() or member.name not in sizes or member.size != sizes[member.name] or
                    member.mode != (0o600 if member.name == 'seed.tar.gz' else 0o755)):
                raise ValueError('Unexpected visit member')
            names.append(member.name)
            if member.name == 'document':
                reference = plan_dir / 'document.reference'
                if reference.is_symlink() or member.size > 65536 or archive.extractfile(member).read() != reference.read_bytes():
                    raise ValueError('The temporary public document differs from the frozen reference')
        if names != list(MEMBERS):
            raise ValueError('Incomplete bounded visit archive')
    return plan


def freeze(retained, plan_dir, splash, wake):
    retained = retained.resolve(strict=True)
    helpers = retained / 'orientation-inputs/helpers'
    seed = retained / 'orientation-inputs/seed.tar.gz'
    arms = arm_inputs(retained)
    if (arms['A']['bytes'], arms['B']['bytes']) != (ARM_A, ARM_B):
        raise ValueError('The retained physical orientation pair differs from the bound builds')
    public_seed(retained / 'src', seed)
    for helper in (splash, wake):
        if helper.is_symlink() or not helper.is_file():
            raise ValueError('Missing physical observer or alarm helper')
        with helper.open('rb') as binary:
            header = binary.read(52)
            if (header[:7] != b'\x7fELF\x01\x01\x01' or struct.unpack_from('<HH',header,16)!=(2,40)
                    or not (struct.unpack_from('<I',header,36)[0] & 0x400)):
                raise ValueError('Expected an ARM32 hard-float helper')
            offset = struct.unpack_from('<I',header,28)[0]
            width, count = struct.unpack_from('<HH',header,42)
            if width != 32 or not 1 <= count <= 64:
                raise ValueError('Unexpected helper program headers')
            binary.seek(offset)
            types = {struct.unpack_from('<I',binary.read(width))[0] for _ in range(count)}
            if 1 not in types or types & {2,3}:
                raise ValueError('Expected statically linked helpers')
    emulator = shutil.which('qemu-arm')
    if emulator:
        check = [emulator,'-cpu','cortex-a8',str(wake),'--policy']
    else:
        parent = plan_dir.parent.resolve(strict=True)
        path = wake.resolve(strict=True).relative_to(parent)
        check = ['/opt/kindle-toolchain/ubuntu-run',str(parent),'qemu-arm','-cpu','cortex-a8','/work/'+str(path),'--policy']
    if subprocess.check_output(check, timeout=15) != b'production CLOCK_BOOTTIME_ALARM\n':
        raise ValueError('Refused a non-waking simulation clock')
    label = secrets.token_hex(8)
    doc = trial_document(retained / 'src', label).decode('ascii')
    doc = once(doc, '[AB][0-6]|[AB]-library|[AB]-reader) ;;', 'B-reader) ;;')
    doc = once(doc, 'WEREAD_TRIAL_DWELL=12000', 'WEREAD_TRIAL_DWELL=35000')
    doc = once(doc, 'seq 1 65', 'seq 1 90').encode('ascii')
    supervisor = launcher((retained / 'src/packaging/kindle/launch.sh').read_text()).encode('ascii')
    entries = dict(zip(MEMBERS, [doc, helpers / 'measure', seed, splash, wake, supervisor,
                                 retained / arms['B']['executable'], *(helpers / name for name in HELPERS)], strict=True))
    for member in entries.values():
        if isinstance(member, Path) and (member.is_symlink() or not member.is_file()):
            raise ValueError('Missing regular visit input')
    sizes = {name: len(value) if isinstance(value, bytes) else value.stat().st_size for name, value in entries.items()}
    device = device_program().encode('ascii')
    for script in (doc, supervisor, device):
        subprocess.run(['sh', '-n'], input=script, check=True)
    plan_dir.mkdir(mode=0o700)
    (plan_dir / 'device.sh').write_bytes(device)
    (plan_dir / 'document.reference').write_bytes(doc)
    path = plan_dir / 'inputs.tgz'
    with tarfile.open(path, 'x:gz', compresslevel=1) as archive:
        for name, value in entries.items():
            member = tarfile.TarInfo(name)
            member.size, member.mode = sizes[name], 0o600 if name == 'seed.tar.gz' else 0o755
            with (io.BytesIO(value) if isinstance(value, bytes) else value.open('rb')) as stream:
                archive.addfile(member, stream)
    (plan_dir / 'plan.json').write_text(json.dumps(CONTRACT | dict(label=label, members=sizes, archive_bytes=path.stat().st_size), indent=2)+'\n')
    return verify_plan(plan_dir)


if __name__ == '__main__':
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='operation', required=True)
    prep = commands.add_parser('freeze')
    for key in ('retained', 'plan', 'splash', 'wake'):
        prep.add_argument('--'+key, type=Path, required=True)
    commands.add_parser('verify').add_argument('--plan', type=Path, required=True)
    args = parser.parse_args()
    record = (freeze(args.retained, args.plan, args.splash, args.wake) if args.operation == 'freeze' else verify_plan(args.plan))
    print(f"Ready: {record['label']}, one B-reader visit. No device access.")
