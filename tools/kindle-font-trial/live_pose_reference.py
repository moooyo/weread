"""Observe bounded live native portrait references, without staging or launching anything."""

import argparse
from datetime import datetime
import getpass
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

from assisted_shelf import verify_recovery
from run import LEASE, UTIL, stamp, transport


WINDOW = 28
SSH_LIMIT = 7
PROBE = '/mnt/us/weread/app/weread --kindle --device scribe --probe-native-portrait-calibration'
FROZEN_PORTRAIT = '''portrait() {
    observed=$(''' + PROBE + ''') || return 1
    case "$observed" in [UD]:[0-3]) printf '%s\\n' "$observed";; *) return 1;; esac
}'''


def reference_program(frozen):
    original = 'reference) native && state >/dev/null && portrait || fail native_reference;;'
    split = 'reference) native || fail native_guard; state >/dev/null || fail protected_guard; portrait;;'
    if frozen.count(original) != 1 or frozen.count(FROZEN_PORTRAIT) != 1 or frozen.count(PROBE) != 1:
        raise ValueError('Unexpected frozen reference guards')
    return frozen.replace(original, split).replace(FROZEN_PORTRAIT, '''portrait() {
    if observed=$(''' + PROBE + ''' 2>&1); then
        case "$observed" in [UD]:[0-3]|FIXED:[0-3]) printf '%s\\n' "$observed";; *) fail reference_output_invalid;; esac
    else
        status=$?
        if [ "$status" = 1 ]; then
            case "$observed" in
                'weread: device operation failed: The native portrait direction changed during calibration'|\
                'weread: device operation failed: The native framebuffer metadata changed during calibration') fail pose_transient;;
            esac
        fi
        fail calibration_probe_failed
    fi
}''')


def decode_reference(result):
    if result.returncode == 1 and result.stdout == b'' and result.stderr == b'refused pose_transient\n':
        return None
    for reason in ('native_guard', 'protected_guard', 'calibration_probe_failed', 'reference_output_invalid'):
        if result.returncode == 1 and not result.stdout and result.stderr == ('refused ' + reason + '\n').encode('ascii'):
            raise ValueError(reason)
    if result.returncode or result.stderr or not re.fullmatch(rb'(?:[UD]|FIXED):[0-3]\n', result.stdout):
        raise ValueError('reference_guard_or_transport_failure')
    return result.stdout[:-1].decode('ascii')


def measure(read, announce, log, clock=time.monotonic, pause=time.sleep):
    deadline = clock() + WINDOW
    consecutive = 0
    previous = None
    instructed = False
    while clock() < deadline:
        limit = min(SSH_LIMIT, max(0.01, deadline - clock()))
        try:
            value = read(limit)
        except TimeoutError:
            if limit < SSH_LIMIT and clock() >= deadline:
                return 'target_not_confirmed'
            raise ValueError('reference_timeout') from None
        if value is not None and not re.fullmatch('(?:[UD]|FIXED):[0-3]', value):
            raise ValueError('unsupported_reference')
        log(value)
        if value is not None and not instructed:
            announce('TURN_NOW' if value == 'D:1' else 'HOLD_STILL')
            instructed = True
        if previous != value:
            announce('POSE ' + (value or 'unavailable'))
        consecutive = consecutive + 1 if value == 'U:3' else 0
        if consecutive == 2 and clock() <= deadline:
            return 'target_confirmed'
        previous = value
        pause(min(0.30, max(0, deadline - clock())))
    return 'target_not_confirmed'


def owned_window(lease, request, report, save, observe):
    try:
        receipt = lease.acquire(request)
    except (Exception, KeyboardInterrupt) as error:
        changes = getattr(error, 'details', {})
        report['error'] = getattr(error, 'code', type(error).__name__)
        report['recovery_required'] = request.token_file.exists() or any(changes.get(key) for key in (
            'lease_directory_created', 'private_token_created', 'owner_record_created'))
        save()
        return 1
    try:
        report['acquired'] = receipt['acquired_at_utc']
        save()
        report['result'] = observe()
    except (Exception, KeyboardInterrupt) as error:
        report['error'] = getattr(error, 'code', str(error) if isinstance(error, ValueError) else type(error).__name__)
    finally:
        try:
            receipt = lease.release(request, native_restored_and_connections_closed=True)
            required = {'ok': True, 'operation': 'release', 'state': 'released', 'project': 'WeRead',
                        'owner_thread': request.owner_thread, 'run_id': request.run_id, 'owner_and_token_checked': True,
                        'native_restored_and_connections_closed': True, 'only_owned_owner_json_removed_then_rmdir': True}
            if (not isinstance(receipt, dict) or any(receipt.get(key) != value or type(receipt[key]) is not type(value)
                    for key,value in required.items()) or type(receipt.get('lease_directory_absent_after_release')) is not bool):
                raise ValueError('release_not_confirmed')
            start = datetime.fromisoformat(report['acquired'])
            end = datetime.fromisoformat(receipt['released_at_utc'])
            if start.utcoffset() is None or end.utcoffset() is None or end < start:
                raise ValueError('release_timestamp_invalid')
            report.update(released=receipt['released_at_utc'], owner_and_token_checked=True,
                          absent_at_release=receipt['lease_directory_absent_after_release'])
            try:
                request.token_file.unlink()
            except OSError:
                report['local_cleanup_error'] = 'private_token_unlink_failed'
        except (Exception, KeyboardInterrupt) as error:
            report['release_error'] = getattr(error, 'code', str(error) if isinstance(error, ValueError) else type(error).__name__)
        finally:
            report['recovery_required'] = 'released' not in report
            save()
    return 1 if any(key in report for key in ('error', 'release_error', 'local_cleanup_error')) else (0 if report['result'] == 'target_confirmed' else 2)


def main(args):
    os.umask(0o077)
    if not Path('/opt/kindle-toolchain/ubuntu-run').is_file():
        raise ValueError('Use the physical device coordinator on the designated worker')
    work = args.work.resolve(strict=True)
    plan = args.plan.resolve(strict=True)
    if work == plan or not plan.is_relative_to(work):
        raise ValueError('Use this worker and its frozen shelf plan')
    label = verify_recovery(plan)['label']
    command, header = transport(reference_program((plan / 'device.sh').read_text(encoding='ascii')), 'reference', label, 'none')
    trust = Path('/root/.config/kindle-physical')
    ssh = UTIL['ssh_command'](trust/'ssh_config', trust/'known_hosts', 'kindle-physical')
    lease = UTIL['load_lease'](args.lease_tool.resolve(strict=True))
    if args.preflight:
        print('Validated reference-only inputs; no lease or device access.')
        return 0
    if not args.operator_ready or not args.owner:
        raise ValueError('A present operator and owner are required')
    password = getpass.getpass('Device password (not saved): ') if sys.stdin.isatty() else sys.stdin.readline().rstrip('\r\n')
    if not password:
        raise ValueError('Missing authentication; no lease requested')
    attempt = Path(tempfile.mkdtemp(prefix='live-reference-', dir=work))
    request = lease.LeaseRequest(LEASE, attempt/'private-token', args.owner, 'weread-' + attempt.name.replace('_', '-'))
    report = {'scope': 'read_only_native_reference', 'source': 'firmware_direction_and_framebuffer_rotation',
              'independent_sensor_sampled': False, 'candidate_launched': False,
              'candidate_plan': label, 'references': [], 'window_seconds': WINDOW}

    def save():
        pending = attempt/'report.next'
        pending.write_text(json.dumps(report, indent=2) + '\n')
        pending.replace(attempt/'report.json')

    def emit(message):
        print(message, flush=True)

    def interrupt(*_):
        raise KeyboardInterrupt

    def record(value):
        report['references'].append({'native': value, 'received_at_utc': stamp()})
        save()

    signal.signal(signal.SIGTERM, interrupt)
    with tempfile.TemporaryDirectory(dir=attempt) as temporary:
        askpass = Path(temporary)/'askpass'
        askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n')
        askpass.chmod(0o700)
        env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE='force', DISPLAY='unused', WEREAD_TRANSIENT_PASSWORD=password)

        def read(limit):
            if not lease.status(request).get('ownership_verified'):
                raise ValueError('ownership_changed')
            try:
                result = subprocess.run([*ssh, command], env=env, input=header, capture_output=True, timeout=limit)
            except subprocess.TimeoutExpired:
                raise TimeoutError from None
            return decode_reference(result)

        emit('Private attempt: ' + str(attempt))
        code = owned_window(lease, request, report, save, lambda: measure(read, emit, record))
    print(json.dumps(report), flush=True)
    return code


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('work', 'plan', 'lease-tool'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--owner')
    parser.add_argument('--operator-ready', action='store_true')
    parser.add_argument('--preflight', action='store_true')
    raise SystemExit(main(parser.parse_args()))
