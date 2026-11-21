"""Freeze a single supervised candidate Library launch; never access the physical device."""

import argparse
import io
import os
from pathlib import Path
import re
import runpy
import secrets
import subprocess
import tarfile

from freeze import launcher
from orientation_contract import CANDIDATE
from orientation_device import render_device, section
from pose_visit import ARM_A, ARM_B, once
from prepare_orientation import DEVICE, FIRMWARE, HELPERS, arm_inputs, public_seed, publish, read_record, trial_document


HERE = Path(__file__).resolve().parent
CASE = 'B-library'
DWELL = 35000
CONTRACT = dict(purpose='assisted-stable-shelf-touch', schedule=[CASE], clock='CLOCK_MONOTONIC',
                device=DEVICE, firmware=FIRMWARE, candidate_features=CANDIDATE,
                expected_calibration='U:3', expected_target='settings')
MEMBERS = ('document', 'measure', 'seed.tar.gz', 'pose-snapshot', 'shelf-scene', 'B/launch.sh',
           'B/app/weread', *(f'B/app/{name}' for name in HELPERS))


def device_program():
    body = render_device(ARM_A, ARM_B)
    both = 'document measure seed.tar.gz A/launch.sh B/launch.sh A/app/weread B/app/weread A/app/weread-frame-guard B/app/weread-frame-guard A/app/weread-native-query B/app/weread-native-query'
    body = once(body, both, ' '.join(MEMBERS))
    body = once(body, 'NR!=11 || bytes>230686720', f'NR!={len(MEMBERS)} || bytes>127926272')
    body = once(body, 'for arm in A B; do', 'for arm in B; do')
    first = body.index('    [ "$(stat -c %s "$stage/A/app/weread")')
    end = body.index('\n}', first)
    body = body[:first] + f'    [ "$(stat -c %s "$stage/B/app/weread")" = {ARM_B} ] &&\n' + \
        '        plain "$stage/pose-snapshot" && [ -x "$stage/pose-snapshot" ] && plain "$stage/shelf-scene" && [ -x "$stage/shelf-scene" ]' + body[end:]
    body = once(body, 'route=${name#*-}; dwell=12000', f'route=${{name#*-}}; dwell={DWELL}')
    body = once(body, ' && cmp -s "$out/portrait" "$out/portrait-return"', '')
    body = once(body, 'case "$name" in [AB][0-6]|[AB]-library|[AB]-reader) ;; *) exit 64;; esac', '[ "$name" = B-library ] || exit 64')
    body = once(body, '    native && absent "$stage/armed" || fail launch_admission\n'
                '    printf \'%s\\n\' "$name" >"$stage/next" && mv "$stage/next" "$stage/armed" || fail arm\n', '')
    body = once(body, '[AB][0-6]|[AB]-library|[AB]-reader) ;; *) fail unexpected_case', 'B-library) ;; *) fail unexpected_case')
    first = body.index('        leaves='); end = body.index('        for leaf in $leaves;', first)
    body = body[:first] + '''        leaves='portrait portrait-source sensor portrait-return observation'
        plain "$stage/$entry/observation" || fail public_output
        case "$(cat "$entry/observation")" in
            'not_started pose_gate') ;;
            'captured candidate_library_exit') leaves="$leaves portrait-launch request markers final.metadata.json shelf";;
            *) fail public_output;;
        esac
''' + body[end:]
    common = section((HERE / 'pose_visit.sh').read_text(encoding='ascii'), 'same_gui() {', 'pose_recover_wake() {')
    body = once(body, section(body, 'capture_case() {', 'no_trial() {'), common + '\n' + (HERE / 'assisted-shelf.sh').read_text(encoding='ascii').strip())
    body = once(body, 'case) capture_case;;', 'case) assisted_case;;\n    absent) no_trial && printf \'absent\\n\';;')
    if len(body.encode('ascii')) > 32768:
        raise ValueError('Assisted device program exceeds the established transport')
    return body


def verify_recovery(plan_dir):
    plan = read_record(plan_dir / 'plan.json')
    if any(plan.get(key) != value for key, value in CONTRACT.items()) or not re.fullmatch(r'[0-9a-f]{16}', plan['label']):
        raise ValueError('Not a single supervised shelf plan')
    script = plan_dir / 'device.sh'
    if script.is_symlink() or not script.is_file() or script.stat().st_size > 32768 or script.read_bytes() != device_program().encode('ascii'):
        raise ValueError('Incorrect frozen device program')
    return plan


def verify_plan(plan_dir):
    plan = verify_recovery(plan_dir)
    sizes = plan['members']
    if (set(sizes) != set(MEMBERS) or sizes['B/app/weread'] != ARM_B or
            any(type(value) is not int or value <= 0 for value in sizes.values()) or sum(sizes.values()) > 127926272):
        raise ValueError('Invalid member manifest')
    archive_path, document = plan_dir / 'inputs.tgz', plan_dir / 'document.reference'
    if (archive_path.is_symlink() or not archive_path.is_file() or archive_path.stat().st_size != plan['archive_bytes'] or
            document.is_symlink() or not document.is_file() or document.stat().st_size > 65536):
        raise ValueError('Invalid supervised input')
    with tarfile.open(archive_path, 'r:gz') as archive:
        if archive.getnames() != list(MEMBERS):
            raise ValueError('Unexpected supervised visit members')
        for member in archive:
            if not member.isfile() or member.size != sizes[member.name] or member.mode != (0o600 if member.name == 'seed.tar.gz' else 0o755):
                raise ValueError('Unexpected member type, size or permissions')
        if archive.extractfile('document').read() != document.read_bytes():
            raise ValueError('The supervised document does not match the frozen plan')
    return plan


def freeze(retained, plan_dir, sensor, scene):
    retained = retained.resolve(strict=True)
    helpers = retained / 'orientation-inputs/helpers'
    seed = retained / 'orientation-inputs/seed.tar.gz'
    source = retained / 'src'
    arms = arm_inputs(retained)
    if (arms['A']['bytes'], arms['B']['bytes']) != (ARM_A, ARM_B):
        raise ValueError('Unexpected physical comparison builds')
    public_seed(source, seed)
    inspect_elf = runpy.run_path(str(source / 'docs/review/library-startup-latency-20261119/rebuild-arm.py'))['inspect_elf']
    for helper in (sensor, scene):
        if not helper.is_file() or helper.is_symlink() or helper.stat().st_size > 512 * 1024:
            raise ValueError('A regular bounded ARM helper is required')
        inspect_elf(helper)
    label = secrets.token_hex(8)
    document = trial_document(source, label).decode('ascii')
    document = once(document, '[AB][0-6]|[AB]-library|[AB]-reader) ;;', 'B-library) ;;')
    document = once(document, 'WEREAD_TRIAL_DWELL=12000', f'WEREAD_TRIAL_DWELL={DWELL}')
    document = once(document, 'seq 1 65', 'seq 1 90').encode('ascii')
    supervisor = launcher((source / 'packaging/kindle/launch.sh').read_text()).encode('ascii')
    entries = dict(zip(MEMBERS, [document, helpers / 'measure', seed, sensor, scene, supervisor,
                                 retained / arms['B']['executable'], *(helpers / name for name in HELPERS)], strict=True))
    sizes = {name: len(value) if isinstance(value, bytes) else value.stat().st_size for name, value in entries.items()}
    device = device_program().encode('ascii')
    for script in (device, document, supervisor):
        subprocess.run(['sh', '-n'], input=script, check=True)
    plan_dir.mkdir(mode=0o700)
    (plan_dir / 'device.sh').write_bytes(device)
    (plan_dir / 'document.reference').write_bytes(document)
    archive_path = plan_dir / 'inputs.tgz'
    with tarfile.open(archive_path, 'x:gz', compresslevel=1) as archive:
        for name, value in entries.items():
            member = tarfile.TarInfo(name)
            member.size, member.mode = sizes[name], 0o600 if name == 'seed.tar.gz' else 0o755
            with (io.BytesIO(value) if isinstance(value, bytes) else value.open('rb')) as stream:
                archive.addfile(member, stream)
    publish(plan_dir / 'plan.json', CONTRACT | dict(label=label, members=sizes, archive_bytes=archive_path.stat().st_size))
    return verify_plan(plan_dir)


if __name__ == '__main__':
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='operation', required=True)
    prep = commands.add_parser('freeze')
    for key in ('retained', 'plan', 'sensor', 'scene'):
        prep.add_argument('--' + key, type=Path, required=True)
    commands.add_parser('verify').add_argument('--plan', type=Path, required=True)
    args = parser.parse_args()
    plan = freeze(args.retained, args.plan, args.sensor, args.scene) if args.operation == 'freeze' else verify_plan(args.plan)
    print(f"Ready: {plan['label']}, one public B-library visit; no device access.")
