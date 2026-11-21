"""Freeze a human-assisted, observed Scribe Reader sleep-turn visit, without visiting the device."""

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
from pose_visit import ARM_A, ARM_B, device_program as previous, once
from prepare_orientation import DEVICE, FIRMWARE, HELPERS, arm_inputs, public_seed, publish, read_record, trial_document


HERE = Path(__file__).resolve().parent
DWELL = 105000
CONTRACT = dict(purpose='startup-orientation-assisted-sleep-change', schedule=['B-reader'], device=DEVICE,
                firmware=FIRMWARE, candidate_features=CANDIDATE, clock='CLOCK_MONOTONIC')
MEMBERS = ('document', 'measure', 'seed.tar.gz', 'pose-snapshot', 'wake-observe', 'B/launch.sh',
           'B/app/weread', *(f'B/app/{name}' for name in HELPERS))
WITNESS = dict(sleep_turn_protocol='witness_window_v1', operator_action_window_seconds=16)


def observer(witness):
    if witness:
        from assisted_wake_samples import render_observer
        return render_observer()
    return (HERE/'assisted-wake-observe.sh').read_bytes()


def device_program(*, witness=False):
    body = previous()
    body = once(body, 'document measure seed.tar.gz pose-splash pose-wake', 'document measure seed.tar.gz pose-snapshot wake-observe')
    body = once(body, 'plain "$stage/pose-splash" && [ -x "$stage/pose-splash" ] && plain "$stage/pose-wake" && [ -x "$stage/pose-wake" ]',
                'plain "$stage/pose-snapshot" && [ -x "$stage/pose-snapshot" ] && plain "$stage/wake-observe"')
    body = once(body, 'route=${name#*-}; dwell=35000', f'route=${{name#*-}}; dwell={DWELL}')
    body = once(body, 'case "$name" in [AB][0-6]|[AB]-library|[AB]-reader) ;; *) exit 64;; esac', '[ "$name" = B-reader ] || exit 64')
    body = once(body, '[AB][0-6]|[AB]-library|[AB]-reader) ;; *) fail unexpected_case', 'B-reader) ;; *) fail unexpected_case')
    body = once(body, '        pose_recover_wake || fail wake_recovery\n', '')
    begin, end = body.index('        leaves='), body.index('        for leaf in $leaves;')
    body = body[:begin] + '''        leaves='request markers portrait portrait-source portrait-launch portrait-return observation initial-diagnostic final.png final.metadata.json'
        for extra in pre.row sleep-pre.row sleep-post.row post.row panel.pgm resumed.pgm image-mapping; do
            absent "$entry/$extra" || leaves="$leaves $extra"
        done
''' + body[end:]
    begin, end = body.index('cycle_marker() {'), body.index('no_trial() {')
    body = body[:begin] + (HERE/'assisted-sleep-change.sh').read_text(encoding='ascii')+'\n'+body[end:]
    body = once(body, 'case) pose_case;;', 'case) as_case;;\n    permit) as_permit || fail permit;;')
    if witness:
        body=once(body,'for extra in pre.row','for extra in sleep-trace turn-window post-status.row pre.row')
        body=once(body,'pose-snapshot wake-observe B/launch.sh','pose-snapshot wake-observe pose-finish B/launch.sh')
        body=once(body,f'NR!={len(MEMBERS)} || bytes>127926272',f'NR!={len(MEMBERS)+1} || bytes>127926272')
        body=once(body,'&& plain "$stage/wake-observe"\n}', '&& plain "$stage/wake-observe" && plain "$stage/pose-finish" && [ -x "$stage/pose-finish" ]\n}')
        old='    as_run_cycle; as_drop\n'
        body=once(body,old,'    witness_cycle; as_drop\n')
        body=once(body,'    for tick in $(seq 1 455);','    if [ "$as_result" = "observed resident_wake" ]; then witness_finish; fi\n    for tick in $(seq 1 455);')
        begin=body.index('as_double() {'); end=body.index('as_observer() {',begin)
        body=body[:begin]+body[begin:end].replace('wake_observe "$1"','wake_observe_any "$1"')+body[end:]
        begin=body.index('as_permit() {'); end=body.index('as_recover() {',begin)
        body=body[:begin]+(HERE/'assisted-sleep-witness.sh').read_text(encoding='ascii')+'\n'+body[end:]
        begin=body.index('as_run_cycle() {'); end=body.index('as_case() {',begin); body=body[:begin]+body[end:]
        body=once(body,'permit) as_permit || fail permit;;','permit) witness_permit || fail permit;;')
    return body


def verify_recovery(plan_dir):
    path=Path(plan_dir); plan=read_record(path/'plan.json'); script=path/'device.sh'
    witness='sleep_turn_protocol' in plan
    if any(key in plan for key in WITNESS) and not all(plan.get(key) == value for key,value in WITNESS.items()):
        raise ValueError('Unsupported operator witness protocol')
    if (not all(plan.get(key) == value for key,value in CONTRACT.items()) or plan.get('initial_diagnostic') != 'bounded_v1' or not re.fullmatch(r'[a-f0-9]{16}',plan['label'])
            or script.is_symlink() or not script.is_file() or script.stat().st_size > 32768
            or script.read_bytes() != device_program(witness=witness).encode('ascii')):
        raise ValueError('Not the frozen bounded assisted sleep-turn program')
    return plan


def verify_plan(plan_dir):
    path=Path(plan_dir); plan=verify_recovery(path); sizes=plan['members']; payload=path/'inputs.tgz'
    witness='sleep_turn_protocol' in plan; names=(*MEMBERS[:5],'pose-finish',*MEMBERS[5:]) if witness else MEMBERS
    if (set(sizes) != set(names) or sizes['B/app/weread'] != ARM_B or
            any(type(size) is not int or size <= 0 for size in sizes.values()) or sum(sizes.values()) > 127926272 or
            payload.is_symlink() or not payload.is_file() or payload.stat().st_size != plan['archive_bytes']):
        raise ValueError('Missing, changed or unexpected assisted payload')
    with tarfile.open(payload,'r:gz') as archive:
        members=list(archive)
        if [entry.name for entry in members] != list(names) or any(not entry.isfile() or entry.size != sizes[entry.name] or
                entry.mode != (0o600 if entry.name in ('seed.tar.gz','wake-observe') else 0o755) for entry in members):
            raise ValueError('Unexpected archive member')
        reference=path/'document.reference'
        if (reference.is_symlink() or not reference.is_file() or reference.stat().st_size>65536
                or archive.extractfile('document').read() != reference.read_bytes()
                or archive.extractfile('wake-observe').read() != observer('sleep_turn_protocol' in plan)):
            raise ValueError('Entry or frozen observer changed')
    return plan


def freeze(retained, plan_dir, sensor, *, witness=False, finish=None):
    retained=retained.resolve(strict=True); source=retained/'src'; helpers=retained/'orientation-inputs/helpers'; seed=retained/'orientation-inputs/seed.tar.gz'
    arms=arm_inputs(retained); public_seed(source,seed)
    if (arms['A']['bytes'],arms['B']['bytes'])!=(ARM_A,ARM_B) or sensor.is_symlink() or not sensor.is_file() or sensor.stat().st_size>524288:
        raise ValueError('Expected the reviewed hardware candidate and a bounded sensor helper')
    inspect=runpy.run_path(str(source/'docs/review/library-startup-latency-20261119/rebuild-arm.py'))['inspect_elf']; inspect(sensor)
    if witness:
        if finish is None or finish.is_symlink() or not finish.is_file() or finish.stat().st_size>524288:
            raise ValueError('Require the static, process-identity guarded finish helper')
        inspect(finish)
    label=secrets.token_hex(8); doc=trial_document(source,label).decode('ascii')
    doc=once(doc,'[AB][0-6]|[AB]-library|[AB]-reader) ;;','B-reader) ;;')
    doc=once(doc,'WEREAD_TRIAL_DWELL=12000',f'WEREAD_TRIAL_DWELL={DWELL}')
    doc=once(doc,'seq 1 65','seq 1 148').encode('ascii')
    supervisor=launcher((source/'packaging/kindle/launch.sh').read_text()).encode('ascii')
    sensor_source=observer(witness); device=device_program(witness=witness).encode('ascii')
    if len(device)>32768 or len(sensor_source)>16384: raise ValueError('Oversized test program')
    for program in (doc,supervisor,sensor_source,device): subprocess.run(['sh','-n'],input=program,check=True)
    entries=dict(zip(MEMBERS,[doc,helpers/'measure',seed,sensor,sensor_source,supervisor,retained/arms['B']['executable'],*(helpers/name for name in HELPERS)],strict=True))
    if witness: entries={key:(finish if key=='pose-finish' else entries[key]) for key in (*MEMBERS[:5],'pose-finish',*MEMBERS[5:])}
    sizes={key:len(value) if isinstance(value,bytes) else value.stat().st_size for key,value in entries.items()}
    plan_dir.mkdir(mode=0o700); (plan_dir/'document.reference').write_bytes(doc); (plan_dir/'device.sh').write_bytes(device)
    payload=plan_dir/'inputs.tgz'
    with tarfile.open(payload,'x:gz',compresslevel=1) as archive:
        for name,value in entries.items():
            member=tarfile.TarInfo(name); member.size=sizes[name]; member.mode=0o600 if name in ('seed.tar.gz','wake-observe') else 0o755
            with (io.BytesIO(value) if isinstance(value,bytes) else value.open('rb')) as stream: archive.addfile(member,stream)
    publish(plan_dir/'plan.json', CONTRACT|(WITNESS if witness else {})|dict(label=label,members=sizes,archive_bytes=payload.stat().st_size,initial_diagnostic='bounded_v1'))
    return verify_plan(plan_dir)


if __name__=='__main__':
    os.umask(0o077); parser=argparse.ArgumentParser(description=__doc__); commands=parser.add_subparsers(dest='action',required=True)
    prep=commands.add_parser('freeze')
    prep.add_argument('--witness-window', action='store_true')
    prep.add_argument('--finish', type=Path)
    for key in ('retained','plan','sensor'): prep.add_argument('--'+key,type=Path,required=True)
    commands.add_parser('verify').add_argument('--plan',type=Path,required=True); args=parser.parse_args()
    plan=verify_plan(args.plan) if args.action=='verify' else freeze(args.retained,args.plan,args.sensor,witness=args.witness_window,finish=args.finish)
    print(f"Prepared {plan['label']}, one manually assisted Reader cycle, no device or lease accessed.")
