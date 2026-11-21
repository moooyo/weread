"""Run exactly one frozen A/B stock-card visit, or only verify or recover that visit."""

import argparse
from contextlib import contextmanager
from datetime import datetime
import fcntl
import json
import os
from pathlib import Path
import re
import runpy
import select
import shlex
import signal
import subprocess
import sys
import tarfile
import tempfile
import time

from run_daily_render import LABEL, SHARED, Refused, now, public_reason
from stock_ab_transaction import METADATA, checkpoint, drive, fresh_journal, initial, valid_journal


TIMES = dict(install=240, prepare=120, tap=100, claim=40, stop=100, restore=125, close=60)
REFS = ('document-reference', 'launch-reference')
TRUST = Path('/root/.config/kindle-physical')
WORKER_RUNNER = Path('/opt/kindle-toolchain/ubuntu-run')


def invocation(body, mode, label):
    allowed = {'admit','stage','install','original','audit','prepare','activate','arm','request','tap','claim',
               'witness','stop','restore','close','absent'}
    header = body.rstrip(b'\n')
    if (mode not in allowed or not LABEL.fullmatch(label) or not header.isascii() or not 0 < len(header) <= 256 * 1024
            or b'\r' in header or b'\0' in header):
        raise ValueError('Invalid frozen device program or operation')
    command = (f'export LC_ALL=C; program=$(dd bs=1 count={len(header)} 2>/dev/null); '
               f'[ "${{#program}}" = {len(header)} ] || exit 64; exec sh -c "$program" ' +
               shlex.join(['weread-stock-ab', mode, label]))
    return command, header


def install_command(body, label):
    _, header = invocation(body, 'install', label)
    delimiter = 'WEREAD_STOCK_AB_INSTALL_BODY'
    if delimiter.encode('ascii') in header.splitlines() or len(header) > 96 * 1024:
        raise ValueError('Frozen installation program exceeds its command transport')
    return ("export LC_ALL=C; program=$(cat <<'" + delimiter + "'\n" + header.decode('ascii') + '\n' + delimiter +
            '\n) || exit 64; exec sh -c "$program" ' + shlex.join(['weread-stock-ab','install',label]))


def references(package):
    content = {}
    with tarfile.open(package, 'r|gz') as archive:
        for member in archive:
            if member.name in REFS:
                if member.name in content or not member.isfile() or not 0 < member.size <= 512 * 1024:
                    raise ValueError('Unsafe archive reference')
                content[member.name] = archive.extractfile(member).read()
    if set(content) != set(REFS):
        raise ValueError('Missing original document or supervisor reference')
    return b''.join(content[name] for name in REFS)


def audit_reference(path, data=None):
    if data is not None:
        with path.open('xb') as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 1024 * 1024 or path.stat().st_mode & 0o077:
        raise Refused('unsafe_audit_reference')
    return path.read_bytes()


class RequestChannel:
    def __init__(self, command, header, env, owned):
        self.owned = owned
        self.pending = b''
        self.sent = False
        self.armed_read = False
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                        env=env, bufsize=0, start_new_session=True)
        try:
            os.set_blocking(self.process.stdin.fileno(), False)
            self.write(header, 24)
        except BaseException:
            self.close(cancel=False)
            raise

    def write(self, data, limit):
        end = time.monotonic() + limit
        offset = 0
        while offset < len(data):
            left = end - time.monotonic()
            if left <= 0 or not select.select([], [self.process.stdin], [], left)[1]:
                raise Refused('request_write_timeout')
            try:
                offset += os.write(self.process.stdin.fileno(), data[offset:offset+4096])
            except BlockingIOError:
                continue

    def read(self, limit):
        end = time.monotonic() + limit
        while b'\n' not in self.pending:
            left = end - time.monotonic()
            if left <= 0 or not select.select([self.process.stdout], [], [], left)[0]:
                raise Refused('request_read_timeout')
            data = os.read(self.process.stdout.fileno(), 1025)
            if not data or len(self.pending) + len(data) > 1024:
                raise Refused('invalid_request_stream')
            self.pending += data
        row, self.pending = self.pending.split(b'\n', 1)
        if self.pending:
            raise Refused('extra_request_output')
        return row + b'\n'

    def armed(self):
        if self.armed_read:
            raise Refused('repeated_request_arm')
        self.owned()
        row = self.read(28)
        self.armed_read = True
        return row

    def running(self):
        return self.process.poll() is None

    def collect(self):
        if self.sent or not self.armed_read or not self.running():
            raise Refused('request_not_live')
        self.owned()
        self.sent = True
        self.write(b'collect\n', 3)
        self.process.stdin.close()
        row = self.read(65)
        if self.process.wait(timeout=9) or os.read(self.process.stdout.fileno(), 1):
            raise Refused('invalid_request_completion')
        return row

    def close(self, *, cancel=True):
        try:
            if self.running() and not self.process.stdin.closed:
                try:
                    if cancel and not self.sent:
                        self.write(b'cancel\n', 1)
                except (OSError, Refused):
                    pass
                finally:
                    self.process.stdin.close()
            try:
                self.process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        finally:
            self.process.stdin.close()
            self.process.stdout.close()


class Transport:
    def __init__(self, ssh, script, label, archive, reference, env, owned):
        self.ssh, self.script, self.label = ssh, script, label
        self.archive, self.reference, self.env, self.owned = archive, reference, env, owned

    def request(self):
        self.owned()
        command, header = invocation(self.script, 'request', self.label)
        return RequestChannel([*self.ssh, command], header, self.env, self.owned)

    def call(self, mode):
        self.owned()
        operation = 'audit' if mode in ('audit_before','audit_after') else mode
        command, header = invocation(self.script, operation, self.label)
        try:
            if mode == 'install':
                with self.archive.open('rb') as source:
                    result = subprocess.run([*self.ssh, install_command(self.script,self.label)], stdin=source,
                                            capture_output=True, env=self.env, timeout=TIMES['install'])
            else:
                payload = header + (self.reference if operation == 'audit' else b'')
                result = subprocess.run([*self.ssh, command], input=payload, capture_output=True, env=self.env,
                                        timeout=TIMES.get(mode, 45))
        except subprocess.TimeoutExpired:
            raise Refused('transport_timeout') from None
        if result.returncode:
            raise Refused(public_reason(result.stderr))
        return result.stdout


@contextmanager
def plan_lock(plan):
    descriptor = os.open(plan, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Refused('visit_busy') from None
        yield
    finally:
        os.close(descriptor)


def read_json(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 192 * 1024:
        raise Refused('missing_original_journal')
    return json.loads(path.read_bytes())


def password_input():
    if sys.stdin.isatty():
        raise ValueError('Use a private one-line stdin pipe, not a terminal or a password argument')
    line = sys.stdin.buffer.readline(258)
    secret = line.rstrip(b'\r\n')
    if not 0 < len(secret) <= 255 or b'\0' in secret or b'\r' in secret or b'\n' in secret:
        raise ValueError('Expected one bounded authentication line on stdin')
    return secret.decode('utf-8')


def original_attempt(plan, config, args):
    attempt = plan / 'attempt'
    if attempt.exists() or attempt.is_symlink():
        if (attempt.is_symlink() or not attempt.is_dir() or attempt.stat().st_mode & 0o077 or
                args.recover and (args.recover.is_symlink() or args.recover.resolve(strict=True) != attempt)):
            raise Refused('unsafe_attempt')
        prior = read_json(attempt / 'operations.json')
        if any(prior.get(key) != config[key] for key in METADATA) or prior.get('owner') != args.owner:
            raise Refused('different_attempt_or_owner')
        token = attempt / 'private-token'
        if args.recover:
            if prior.get('released') or token.is_symlink() or not token.is_file():
                raise Refused('no_original_token_for_recovery')
        elif (prior.get('acquisition') != 'busy' or prior.get('operations') != [] or prior.get('device_accessed') is not False
                or prior.get('recovery_required') is not False or os.path.lexists(token)):
            raise Refused('visit_already_attempted')
    elif args.recover:
        raise Refused('no_original_attempt')
    else:
        attempt.mkdir(mode=0o700)
    return attempt


def execute(args):
    from stock_ab_freeze import verify_visit

    os.umask(0o077)
    if sys.platform != 'linux' or not WORKER_RUNNER.is_file():
        raise ValueError('Use the designated worker host')
    work, plan = args.work.resolve(strict=True), args.plan.resolve(strict=True)
    if args.work.is_symlink() or args.plan.is_symlink() or not plan.is_relative_to(work) or work == plan:
        raise ValueError('Expected a frozen visit under its original worker')
    config = verify_visit(work, plan, recovery=bool(args.recover))
    expected = {key:config[key] for key in METADATA}
    label = config['label']
    body = (plan / 'device.sh').read_bytes()
    invocation(body, 'request', label)
    install_command(body, label)
    archive = plan / 'inputs.tgz'
    originals = None if args.recover else references(archive)
    if args.preflight:
        print(json.dumps(dict(**expected, validated=True, device_accessed=False, lease_requested=False)))
        return 0
    if not args.owner or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}', args.owner) or not args.lease_tool:
        raise ValueError('An owner and approved lease utility are required')
    shared = runpy.run_path(str(SHARED))
    ssh = shared['ssh_command'](TRUST/'ssh_config', TRUST/'known_hosts', 'kindle-physical')
    lease = shared['load_lease'](args.lease_tool.resolve(strict=True))
    with plan_lock(plan):
        if not args.recover and os.path.lexists(shared['LEASE']):
            print(json.dumps(dict(**expected, acquisition='busy', device_accessed=False, lease_requested=False)))
            return 3
        password = password_input()
        attempt = original_attempt(plan, config, args)
        report = initial(config, args.owner)
        receipt = attempt / ('recovery.json' if args.recover else 'operations.json')
        if args.recover and receipt.exists():
            previous = read_json(receipt)
            if previous.get('released'):
                raise Refused('already_recovered')
            receipt.rename(attempt / ('prior-recovery-' + str(time.time_ns()) + '.json'))
        journal_path = attempt/'device-progress.json'
        journal = read_json(journal_path) if args.recover else fresh_journal(label)
        if not valid_journal(journal,label):
            raise Refused('foreign_device_journal')
        refs = attempt/'original-audit-input'
        if args.recover or refs.exists():
            actual = audit_reference(refs)
            if originals is not None and originals != actual:
                raise Refused('different_original_reference')
            originals = actual
        else:
            audit_reference(refs, originals)
        request = lease.LeaseRequest(shared['LEASE'], attempt/'private-token', args.owner, 'weread-stock-ab-' + label)
        def save():
            checkpoint(receipt, report)
        def persist():
            checkpoint(journal_path, journal)
        def owned():
            if lease.status(request).get('ownership_verified') is not True:
                raise Refused('lease_not_owned')
            if not report['device_accessed']:
                report['device_accessed'] = True
                save()
        def release():
            result = lease.release(request, native_restored_and_connections_closed=True)
            if (result.get('owner_and_token_checked') is not True or result.get('only_owned_owner_json_removed_then_rmdir') is not True
                    or result.get('native_restored_and_connections_closed') is not True or
                    not isinstance(result.get('released_at_utc'), str) or datetime.fromisoformat(result['released_at_utc']).tzinfo is None):
                raise Refused('release_unconfirmed')
            return result['released_at_utc']
        def interrupt(*_):
            raise KeyboardInterrupt
        with tempfile.TemporaryDirectory(dir=attempt) as private:
            askpass = Path(private)/'askpass'
            askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n', encoding='ascii')
            askpass.chmod(0o700)
            env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE='force', DISPLAY='unused',
                       WEREAD_TRANSIENT_PASSWORD=password)
            transport = Transport(ssh, body, label, archive, originals, env, owned)
            signal.signal(signal.SIGTERM, interrupt)
            if args.recover:
                original = read_json(attempt/'operations.json')
                report.update(recovery_only=True, acquired_at_utc=original.get('acquired_at_utc'), recovery_required=True)
                save()
                try:
                    if lease.status(request).get('ownership_verified') is not True:
                        raise Refused('lease_not_owned')
                except (Exception, KeyboardInterrupt):
                    report['close_reason'] = 'lease_not_owned'
                    save()
                    return 2
            else:
                persist()
                save()
                try:
                    acquired = lease.acquire(request)
                except (Exception, KeyboardInterrupt) as error:
                    details = getattr(error,'details',{})
                    partial = os.path.lexists(request.token_file) or any(details.get(key) for key in
                              ('lease_directory_created','private_token_created','owner_record_created'))
                    busy = getattr(error,'code','') in ('lease_exists','operation_busy') and not partial
                    report.update(acquisition='busy' if busy else 'unconfirmed', recovery_required=bool(partial))
                    save()
                    return 2 if partial else 3 if busy else 1
                report.update(acquisition='acquired', acquired_at_utc=acquired['acquired_at_utc'], recovery_required=True)
                save()
            status = drive(transport, release, report, journal, save, persist, recovering=bool(args.recover))
            if report['released']:
                request.token_file.unlink(missing_ok=True)
            print(json.dumps(dict(label=label, lane=config['lane'], slot=config['slot'], primary=report['primary'],
                                  released=report['released'], recovery_required=report['recovery_required'],
                                  reason=report.get('close_reason') or report.get('refused') or report.get('qualification_reason'),
                                  receipt=str(receipt))))
            return status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work', required=True, type=Path)
    parser.add_argument('--plan', required=True, type=Path)
    parser.add_argument('--lease-tool', type=Path)
    parser.add_argument('--owner')
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument('--preflight', action='store_true')
    modes.add_argument('--recover', type=Path)
    args = parser.parse_args()
    try:
        return execute(args)
    except Refused as error:
        print('Refused:', error, file=sys.stderr)
        return 3 if str(error) == 'visit_busy' else 2 if args.recover else 1
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError, tarfile.TarError, KeyboardInterrupt) as error:
        print('Visit not confirmed:', type(error).__name__, file=sys.stderr)
        return 2 if args.recover or not args.preflight else 1


if __name__ == '__main__':
    raise SystemExit(main())
