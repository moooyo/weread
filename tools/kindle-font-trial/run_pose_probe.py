"""Read the physical Scribe's current pose in one immediately released worker lease."""

import argparse
import getpass
import json
import os
from pathlib import Path
import re
import secrets
import signal
import subprocess
import sys
import tempfile

from pose_probe import parse_observation, render_device
from run import LEASE, UTIL, stamp, transport


def transact(call, release, report, save, recovering=False):
    touched = recovering
    raw = None
    try:
        if not recovering:
            call('admit', b'admitted')
            touched = True
            raw = call('probe', None)
    finally:
        try:
            if touched:
                call('close', b'closed')
                call('absent', b'absent')
            receipt = release()
            if not (receipt.get('owner_and_token_checked') and receipt.get('only_owned_owner_json_removed_then_rmdir')):
                raise ValueError('release_unconfirmed')
            report['released'] = receipt['released_at_utc']
            report['owner_and_token_checked'] = True
            report['absent_at_release'] = receipt['lease_directory_absent_after_release']
        finally:
            save()
    if raw is not None:
        report['observation'] = parse_observation(raw.decode('ascii'), expected_label=report['label'])
    report['passed'] = True
    save()


def main(args):
    os.umask(0o077)
    if not Path('/opt/kindle-toolchain/ubuntu-run').is_file():
        raise ValueError('Use the designated worker host')
    runs = args.runs.resolve(strict=True)
    if args.recover:
        attempt = args.recover.resolve(strict=True)
        if attempt.parent != runs or not (attempt / 'private-token').is_file():
            raise ValueError('No matching interrupted pose probe')
        label = (attempt / 'label').read_text().strip()
        if not re.fullmatch('[a-f0-9]{16}', label):
            raise ValueError('Invalid original probe label')
        body = (attempt / 'device.sh').read_text()
        helper = b''
    else:
        if not args.helper or args.helper.is_symlink() or not args.helper.is_file() or not 52 <= args.helper.stat().st_size <= 1024**2:
            raise ValueError('Expected the verified ARM helper')
        helper = args.helper.read_bytes()
        if helper[:7] != b'\x7fELF\x01\x01\x01' or int.from_bytes(helper[18:20], 'little') != 40:
            raise ValueError('Expected an ARM32 little-endian ELF')
        label = secrets.token_hex(8)
        body = render_device(len(helper))
    for mode in ('admit', 'probe', 'close', 'absent'):
        transport(body, mode, label, 'none')
    trust = Path('/root/.config/kindle-physical')
    ssh = UTIL['ssh_command'](trust / 'ssh_config', trust / 'known_hosts', 'kindle-physical')
    lease = UTIL['load_lease'](args.lease_tool.resolve(strict=True))
    if args.preflight:
        print('Worker inputs and pinned SSH configuration ready; no device access or lease.')
        return 0
    secret = getpass.getpass('Device password (not saved): ') if sys.stdin.isatty() else sys.stdin.readline().rstrip('\r\n')
    if not secret:
        raise ValueError('Missing private authentication; no acquisition attempted')
    if not args.recover:
        attempt = Path(tempfile.mkdtemp(prefix='probe-', dir=runs))
        (attempt / 'label').write_text(label + '\n')
        (attempt / 'device.sh').write_text(body)
    report = {'label': label, 'started': stamp(), 'operations': [], 'passed': False, 'mode': 'recover' if args.recover else 'probe',
              'scope': 'stock_native_reference_only'}
    output = attempt / ('recovery.json' if args.recover else 'operations.json')
    request = lease.LeaseRequest(LEASE, attempt / 'private-token', args.owner, 'weread-native-pose-' + label)

    def save():
        pending = output.with_suffix('.next')
        pending.write_text(json.dumps(report, indent=2) + '\n')
        pending.replace(output)

    def interrupt(*_):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupt)
    with tempfile.TemporaryDirectory(dir=attempt) as local:
        askpass = Path(local) / 'askpass'
        askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n')
        askpass.chmod(0o700)
        env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE='force', DISPLAY='unused', WEREAD_TRANSIENT_PASSWORD=secret)

        def call(mode, expected):
            if not lease.status(request)['ownership_verified']:
                raise ValueError('ownership_changed')
            remote, header = transport(body, mode, label, 'none')
            try:
                result = subprocess.run([*ssh, remote], input=header + (helper if mode == 'probe' else b''),
                                        capture_output=True, env=env, timeout=30)
                code = result.returncode
                refusal = re.search(rb'^refused ([a-z0-9_]{1,48})$', result.stderr, re.MULTILINE)
                reason = refusal[1].decode() if refusal else 'transport_or_device'
                if not code and (len(result.stdout) > 8192 or (expected is not None and result.stdout.strip() != expected)):
                    code, reason = 65, 'unexpected_response'
            except subprocess.TimeoutExpired:
                code, reason = 124, 'transport_timeout'
            record = {'mode': mode, 'status': code, 'time': stamp()}
            if code:
                record['reason'] = reason
            report['operations'].append(record)
            save()
            if code:
                raise ValueError(mode + ':' + reason)
            return result.stdout

        print('Private attempt:', attempt, flush=True)
        try:
            if not args.recover:
                report['acquired'] = lease.acquire(request)['acquired_at_utc']
            save()
            transact(call, lambda: lease.release(request, native_restored_and_connections_closed=True), report, save, args.recover)
        except (Exception, KeyboardInterrupt) as error:
            report['error'] = getattr(error, 'code', type(error).__name__)
            changes = getattr(error, 'details', {})
            report['recovery_required'] = not bool(report.get('released')) and (request.token_file.exists() or any(
                changes.get(key) for key in ('lease_directory_created', 'private_token_created', 'owner_record_created')))
            save()
        finally:
            if report.get('released'):
                request.token_file.unlink(missing_ok=True)
    print(json.dumps(report), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runs', type=Path, required=True)
    parser.add_argument('--lease-tool', type=Path, required=True)
    parser.add_argument('--owner', required=True)
    parser.add_argument('--helper', type=Path)
    parser.add_argument('--recover', type=Path)
    parser.add_argument('--preflight', action='store_true')
    raise SystemExit(main(parser.parse_args()))
