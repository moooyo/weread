"""Run one prepared normal-card observation on the worker, then return the device."""

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import getpass
import json
import os
from pathlib import Path
import re
import runpy
import secrets
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time


HERE = Path(__file__).resolve().parent
SHARED = HERE.parents[1] / 'docs/review/library-startup-latency-20261119/arm-admission/cpu-batch.py'
CLOCK = 'CLOCK_MONOTONIC'
STATES = dict(admit='admitted', stage='staged', install='installed', prepare='prepared', arm='armed',
              claim='claimed', witness='witnessed', cancel='cancelled', stop='stopped', restore='restored',
              close='closed', absent='absent')
LIMITS = dict(install=135, prepare=105, tap=85, claim=35, stop=90, restore=100)
LABEL = re.compile(r'[0-9a-f]{16}')


class Refused(Exception):
    pass


def now():
    return datetime.now(timezone.utc).isoformat(timespec='milliseconds')


def public_reason(raw):
    return next((line[8:] for line in raw.decode('ascii', 'replace').splitlines()
                 if re.fullmatch(r'refused [a-z_]{1,48}', line)), 'unconfirmed')


def json_line(raw):
    def unique(pairs):
        value = dict(pairs)
        if len(pairs) != len(value):
            raise ValueError('duplicate key')
        return value

    if len(raw) > 1024 or len(raw.splitlines()) != 1:
        raise Refused('invalid_receipt')
    try:
        record = json.loads(raw, object_pairs_hook=unique)
    except (ValueError, UnicodeError):
        raise Refused('invalid_receipt') from None
    if not isinstance(record, dict):
        raise Refused('invalid_receipt')
    return record


def release_bounds(raw):
    row = json_line(raw)
    flags = ('ok', 'down_confirmed', 'up_sent', 'buttons_clear_after')
    fields = ('down_before_ns', 'down_after_ns', 'up_before_ns', 'up_after_ns')
    if (set(row) != {*flags, *fields, 'mode', 'clock', 'target', 'hold_requested_ms'} or
            any(row[key] is not True for key in flags) or row['mode'] != 'XTest-button-1' or
            row['clock'] != CLOCK or row['target'] != 'open-card' or row['hold_requested_ms'] != 50 or
            any(type(row[key]) is not int or not 0 < row[key] < 2**63 for key in fields)):
        raise Refused('invalid_release')
    down, down_ack, before, after = (row[key] for key in fields)
    if not down <= down_ack <= before - 50_000_000 < before <= after:
        raise Refused('invalid_release_order')
    return before, after


def observation(raw, release):
    row = json_line(raw)
    fields = ('last_negative_ns', 'first_complete_scene_ns', 'confirmed_scene_ns', 'submit_upper_bound_ns')
    if (set(row) != {'clock', 'scene', 'evidence', *fields} or row['clock'] != CLOCK or
            row['scene'] not in ('grid', 'list', 'empty') or
            row['evidence'] not in ('submit_upper_bound', 'memory_only') or
            any(type(row[key]) is not int or not 0 <= row[key] < 2**63 for key in fields)):
        raise Refused('invalid_scene')
    before, after = release
    negative, first, confirmed, witness = (row[key] for key in fields)
    if not 0 < negative <= first < confirmed or first <= after:
        raise Refused('invalid_scene_order')
    if (row['evidence'] == 'memory_only' and witness != 0 or
            row['evidence'] == 'submit_upper_bound' and not first <= witness <= confirmed):
        raise Refused('invalid_submit_witness')
    return {'scene': row['scene'], 'evidence': row['evidence'], 'clock': CLOCK, 'synthetic_release_ns': [before, after],
            **{key: row[key] for key in fields}, 'scene_observation_upper_ns': first - before,
            'submit_observation_upper_ns': witness - before if witness else None}


def drive(call, start_observer, release, report, save, recovering=False, progress='no_input', persist=lambda _: None):
    staged = recovering
    observer = None
    report.update(primary='not_timed', released=False)
    try:
        if not recovering:
            call('admit')
            staged = True
            for mode in ('stage', 'install', 'prepare', 'arm'):
                if mode == 'prepare':
                    progress = 'navigation_possible'
                    persist(progress)
                call(mode)
            observer = start_observer()
            observer.armed()
            if not observer.running():
                raise Refused('observer_exited_before_tap')
            bounds = release_bounds(call('tap'))
            report['synthetic_release_ns'] = list(bounds)
            save()
            call('claim')
            scene = observation(observer.finish(), bounds)
            call('witness')
            report['observation'] = scene
            report['primary_reason'] = 'no_same_frame_completion_marker'
    except (Refused, KeyboardInterrupt) as error:
        report['refused'] = str(error) if isinstance(error, Refused) else 'interrupted'
    finally:
        try:
            if observer is not None and observer.running():
                try:
                    call('cancel')
                finally:
                    observer.close()
        finally:
            try:
                if staged:
                    if recovering:
                        try:
                            call('cancel')
                        except Refused:
                            pass
                    for mode in ('stop', 'restore', 'close', 'absent'):
                        state = json_line(call(mode)).get('state')
                        if state == 'already_absent' and progress == 'navigation_possible':
                            raise Refused('missing_stage_before_native_restore')
                        if mode == 'restore' and state == 'restored':
                            progress = 'restoration_proved'
                            persist(progress)
                report['released_at_utc'] = release()
                report['released'] = True
                report['owner_and_token_checked'] = True
            finally:
                report['recovery_required'] = not report['released']
                save()


class Observer:
    def __init__(self, process, record):
        self.process = process
        self.record = record
        self.tail = b''

    def armed(self):
        deadline = time.monotonic() + 22
        while b'\n' not in self.tail:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.process.stdout], [], [], remaining)[0]:
                raise Refused('observer_arm_timeout')
            chunk = os.read(self.process.stdout.fileno(), 1024)
            if not chunk or len(self.tail) + len(chunk) > 1024:
                raise Refused('observer_not_armed')
            self.tail += chunk
        first, self.tail = self.tail.split(b'\n', 1)
        if first != b'ARMED':
            raise Refused('observer_not_armed')
        self.record('observe_armed', 0)

    def finish(self):
        try:
            output, _ = self.process.communicate(timeout=120)
        except subprocess.TimeoutExpired:
            raise Refused('observer_timeout') from None
        self.record('observe', self.process.returncode)
        if self.process.returncode:
            raise Refused('observer_no_qualified_scene')
        return self.tail + output

    def running(self):
        return self.process.poll() is None

    def close(self):
        try:
            self.process.communicate(timeout=8)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.communicate(timeout=5)


@contextmanager
def owner_operation(attempt):
    descriptor = os.open(attempt, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(descriptor)


def install_transport(body, label):
    header = body.rstrip(b'\n')
    if not LABEL.fullmatch(label) or not header.isascii() or not 0 < len(header) < 64 * 1024:
        raise ValueError('Invalid install transport')
    command = (f'export LC_ALL=C; program=$(dd bs=1 count={len(header)} 2>/dev/null); '
               f'[ "${{#program}}" = {len(header)} ] || exit 64; exec sh -c "$program" ' +
               shlex.join(['weread-render', 'install', label]))
    return command, header


def run(args):
    from prepare_daily_render import validate
    shared = runpy.run_path(str(SHARED))
    os.umask(0o077)
    if not Path('/opt/kindle-toolchain/ubuntu-run').is_file():
        raise ValueError('Run on the designated worker host')
    work = args.work.resolve(strict=True)
    program = HERE / 'daily_render_device.sh'
    body = program.read_bytes()
    if not body.isascii() or not 0 < len(body) < 64 * 1024 or b'\r' in body:
        raise ValueError('Invalid device program')
    if not args.recover:
        validate(work / 'install.tgz')
    trust = Path('/root/.config/kindle-physical')
    ssh = shared['ssh_command'](trust / 'ssh_config', trust / 'known_hosts', 'kindle-physical')
    lease = shared['load_lease'](args.lease_tool.resolve(strict=True))
    if args.check:
        print('Prepared worker inputs validated; no lease or device action.')
        return 0
    if args.recover:
        attempt = args.recover.resolve(strict=True)
        if attempt.parent != work or not (attempt / 'private-token').is_file():
            raise ValueError('Use the original interrupted attempt')
        label = (attempt / 'label').read_text().strip()
    else:
        if os.path.lexists(shared['LEASE']):
            raise Refused('lease_busy')
        label = secrets.token_hex(8)
        attempt = work / ('batch-' + label)
        attempt.mkdir(mode=0o700)
        (attempt / 'label').write_text(label + '\n')
    if not LABEL.fullmatch(label) or attempt.name != 'batch-' + label:
        raise ValueError('Invalid attempt identity')
    password = getpass.getpass('Device password (not saved): ') if sys.stdin.isatty() else sys.stdin.readline().rstrip('\r\n')
    if not password:
        raise ValueError('Missing authentication; no acquisition attempted')
    request = lease.LeaseRequest(shared['LEASE'], attempt / 'private-token', args.owner, 'weread-render-' + label)
    report = dict(label=label, started_at_utc=now(), operations=[], primary='not_timed', measurement_build='ordinary_installation')
    destination = attempt / ('recovery.json' if args.recover else 'operations.json')

    def save():
        pending = destination.with_suffix('.next')
        pending.write_text(json.dumps(report, indent=2) + '\n')
        pending.replace(destination)

    def record(mode, code, reason='', **fields):
        report['operations'].append(dict(mode=mode, status=code, reason=reason, at=now(), **fields))
        save()
        print(mode, code, reason, flush=True)

    def owned():
        if lease.status(request).get('ownership_verified') is not True:
            raise Refused('lease_not_owned')

    def release():
        receipt = lease.release(request, native_restored_and_connections_closed=True)
        if not (receipt.get('owner_and_token_checked') and receipt.get('only_owned_owner_json_removed_then_rmdir')):
            raise Refused('release_unconfirmed')
        request.token_file.unlink(missing_ok=True)
        return receipt['released_at_utc']

    with owner_operation(attempt), tempfile.TemporaryDirectory(dir=attempt) as private:
        askpass = Path(private) / 'askpass'
        askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n')
        askpass.chmod(0o700)
        env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE='force', DISPLAY='unused',
                   WEREAD_TRANSIENT_PASSWORD=password)

        def call(mode):
            owned()
            invocation = shlex.join(['sh', '-s', '--', mode, label])
            with tempfile.TemporaryFile(dir=private) as data:
                if mode == 'install':
                    invocation, header = install_transport(body, label)
                    data.write(header)
                    with (work / 'install.tgz').open('rb') as archive:
                        shutil.copyfileobj(archive, data)
                else:
                    data.write(body)
                data.seek(0)
                try:
                    result = subprocess.run([*ssh, invocation], stdin=data, capture_output=True, env=env,
                                            timeout=LIMITS.get(mode, 35))
                except subprocess.TimeoutExpired:
                    record(mode, 124, 'transport_timeout')
                    raise Refused(mode + ':transport_timeout') from None
            code, reason = result.returncode, public_reason(result.stderr) if result.returncode else ''
            fields = {}
            if not code and mode == 'tap':
                try:
                    release_bounds(result.stdout)
                    fields['receipt'] = json_line(result.stdout)
                except Refused:
                    code, reason = 65, 'invalid_release'
            if not code and mode in STATES:
                try:
                    allowed = [dict(state=STATES[mode])]
                    if mode in ('stop', 'restore', 'close'):
                        allowed.append(dict(state='already_absent'))
                    if json_line(result.stdout) not in allowed:
                        raise Refused('unexpected_state')
                except Refused:
                    code, reason = 65, 'unexpected_state'
            record(mode, code, reason, **fields)
            if code:
                raise Refused(mode + ':' + reason)
            return result.stdout

        def start_observer():
            owned()
            with program.open('rb') as source:
                proc = subprocess.Popen([*ssh, shlex.join(['sh', '-s', '--', 'observe', label])], stdin=source,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, bufsize=0)
            return Observer(proc, record)

        def interrupt(*_):
            raise KeyboardInterrupt

        journal = attempt / 'device-progress'

        def persist(progress):
            temporary = journal.with_suffix('.next')
            temporary.write_text(progress + '\n')
            temporary.replace(journal)

        signal.signal(signal.SIGTERM, interrupt)
        print('Private attempt:', attempt, flush=True)
        if args.recover:
            progress = journal.read_text().strip()
            if progress not in ('no_input', 'navigation_possible', 'restoration_proved'):
                raise Refused('invalid_recovery_journal')
        else:
            progress = 'no_input'
            persist(progress)
        save()
        if not args.recover:
            try:
                lease.acquire(request)
            except (Exception, KeyboardInterrupt) as error:
                changes = getattr(error, 'details', {})
                report.update(refused=getattr(error, 'code', 'acquisition_failed'),
                              recovery_required=request.token_file.exists() or bool(isinstance(changes, dict) and any(
                                  changes.get(key) for key in ('lease_directory_created', 'private_token_created', 'owner_record_created'))))
                save()
                return 1
        report['recovery_required'] = True
        save()
        if args.recover:
            owned()
        drive(call, start_observer, release, report, save, bool(args.recover), progress, persist)
    print('Released exact physical lease.' if report['released'] else 'Close unconfirmed.', flush=True)
    return 0 if report['released'] and (args.recover or 'observation' in report) else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--lease-tool', type=Path, required=True)
    parser.add_argument('--owner', required=True)
    action = parser.add_mutually_exclusive_group()
    action.add_argument('--check', action='store_true')
    action.add_argument('--recover', type=Path)
    try:
        raise SystemExit(run(parser.parse_args()))
    except (Refused, ValueError, OSError) as exc:
        raise SystemExit(f'Worker refused: {exc}') from None
