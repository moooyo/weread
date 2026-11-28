"""Run exactly one frozen A/B stock-card visit, or only verify or recover that visit."""

import argparse
from contextlib import ExitStack, contextmanager
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

from load_progress import HEADER_RECEIVED, checked_progress
from qualified_submit import PORTRAIT_DIAGNOSTIC_PURPOSE
from run_daily_render import LABEL, SHARED, Refused, now, public_reason
from stock_ab_transaction import (LOAD_CLOSE_RECEIPTS, METADATA, WireRefusal, bind_syscall_refusal, checkpoint,
                                  drive, fresh_journal, initial, valid_installation_receipt, valid_journal)


TIMES = dict(install=240, prepare=120, tap=100, claim=40, stop=100, restore=125, close=60)
REFS = ('document-reference', 'launch-reference')
TRUST = Path('/root/.config/kindle-physical')
WORKER_RUNNER = Path('/opt/kindle-toolchain/ubuntu-run')
OPERATION_LOCK_MODULE = Path('/usr/local/lib/kindle-physical/device_lock.py')
MAX_HEADER = 120 * 1024  # Leave room below Linux's single-argument exec limit.
PORTRAIT_READ = 'portrait_diagnostic'
PORTRAIT_DEVICE_SECONDS = 12
PORTRAIT_HOST_SECONDS = 25


@contextmanager
def physical_operation():
    lock = runpy.run_path(str(OPERATION_LOCK_MODULE))
    with ExitStack() as stack:
        try:
            stack.enter_context(lock['operation_lock']())
        except lock['DeviceBusy']:
            raise Refused('physical_operation_busy') from None
        yield


def invocation(body, mode, label):
    allowed = {'admit','stage','install','original','audit','prepare','activate','arm','request','tap','claim',
               'witness','stop','restore','close','absent', PORTRAIT_READ}
    header = body.rstrip(b'\n')
    if (mode not in allowed or not LABEL.fullmatch(label) or not header.isascii() or not 0 < len(header) <= MAX_HEADER
            or b'\r' in header or b'\0' in header):
        raise ValueError('Invalid frozen device program or operation')
    acknowledged = "printf '%s\\n' " + HEADER_RECEIVED.decode('ascii') + ' >&2; ' if mode == 'install' else ''
    watchdog = f'timeout -s KILL {PORTRAIT_DEVICE_SECONDS} ' if mode == PORTRAIT_READ else ''
    command = (f'export LC_ALL=C; program=$(dd bs=1 count={len(header)} 2>/dev/null); '
               f'[ "${{#program}}" = {len(header)} ] || exit 64; ' + acknowledged + f'exec {watchdog}sh -c "$program" ' +
               shlex.join(['weread-stock-ab', mode, label]))
    return command, header


def staged_wire(archive, header, destination):
    if archive.is_symlink() or not archive.is_file():
        raise Refused('unsafe_install_archive')
    size = archive.stat().st_size
    if not 0 < size <= 512 * 1024**2:
        raise Refused('invalid_install_archive_size')
    with archive.open('rb') as source, destination.open('xb') as wire:
        wire.write(header)
        while chunk := source.read(1024 * 1024):
            wire.write(chunk)
        if wire.tell() != len(header) + size:
            raise Refused('install_archive_changed')
        if os.fstat(source.fileno()).st_size != size:
            raise Refused('install_archive_changed')
    return destination


def transport_failure(mode, code, stderr, *, load_retired=False):
    raw = stderr if isinstance(stderr,bytes) else b''
    seen = HEADER_RECEIVED in raw.splitlines() if mode == 'install' else None
    reason = public_reason(raw)
    if reason == 'unconfirmed':
        refusals = [line[8:].decode('ascii', 'replace') for line in raw.splitlines() if line.startswith(b'refused ')]
        if len(refusals) == 1 and bind_syscall_refusal(refusals[0]):
            return WireRefusal(refusals[0],code,seen,load_retired)
        reason = ('transport_timeout' if code is None else 'ssh_transport_exit' if code == 255 else
                  'transport_signaled' if code < 0 else 'unconfirmed_after_header' if seen else
                  'install_header_unconfirmed' if mode == 'install' else 'unconfirmed')
    return WireRefusal(reason,code,seen,load_retired)


def load_retirement_observed(mode, label, result):
    if mode not in LOAD_CLOSE_RECEIPTS or result.returncode not in (0, 1):
        return False
    marker = LOAD_CLOSE_RECEIPTS[mode] + label.encode('ascii')
    lines = result.stderr.splitlines(keepends=True)
    marks = [line for line in lines if line.startswith(tuple(LOAD_CLOSE_RECEIPTS.values()))]
    return marks == [marker + b'\n'] and lines[-1:] == marks


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
    def __init__(self, ssh, script, label, archive, reference, env, owned, *, wire=None):
        self.ssh, self.script, self.label = ssh, script, label
        self.archive, self.reference, self.env, self.owned = archive, reference, env, owned
        self.wire = wire
        self.install_receive = None

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
                self.install_receive = None
                if self.wire is None:
                    raise Refused('install_wire_not_prepared')
                with self.wire.open('rb') as source:
                    expected_bytes = os.fstat(source.fileno()).st_size - len(header)
                    result = subprocess.run([*self.ssh, command], stdin=source,
                                            capture_output=True, env=self.env, timeout=TIMES['install'])
            else:
                payload = header + (self.reference if operation == 'audit' else b'')
                result = subprocess.run([*self.ssh, command], input=payload, capture_output=True, env=self.env,
                                        timeout=TIMES.get(mode, 45))
        except subprocess.TimeoutExpired as error:
            raise transport_failure(mode,None,error.stderr) from None
        retired = load_retirement_observed(mode, self.label, result)
        if mode == 'install' and retired:
            self.install_receive = checked_progress(result.stderr, self.label, expected_bytes)
        if result.returncode:
            raise transport_failure(mode,result.returncode,result.stderr,load_retired=retired)
        if mode in LOAD_CLOSE_RECEIPTS and not retired:
            raise WireRefusal('missing_load_retirement_receipt',result.returncode,
                              HEADER_RECEIVED in result.stderr.splitlines() if mode == 'install' else None)
        if mode == 'install' and result.stderr.splitlines().count(HEADER_RECEIVED) != 1:
            raise WireRefusal('missing_install_header_receipt',result.returncode,False,load_retired=retired)
        return result.stdout

    def read_portrait_diagnostic(self, limit):
        self.owned()
        command, header = invocation(self.script, PORTRAIT_READ, self.label)
        with tempfile.TemporaryFile() as source:
            source.write(header)
            source.seek(0)
            with subprocess.Popen([*self.ssh, command], stdin=source, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, env=self.env, start_new_session=True) as process:
                deadline = time.monotonic() + PORTRAIT_HOST_SECONDS
                data = bytearray()
                completed = False
                try:
                    while len(data) <= limit:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
                            raise Refused('diagnostic_read_timeout')
                        chunk = os.read(process.stdout.fileno(), min(4096, limit + 1 - len(data)))
                        if not chunk:
                            break
                        data.extend(chunk)
                    if len(data) > limit:
                        raise Refused('diagnostic_output_limit')
                    status = process.wait(timeout=max(0, deadline - time.monotonic()))
                    completed = True
                    if status:
                        raise Refused('diagnostic_read_timeout' if status in (-9, 124, 137) else 'diagnostic_read_refused')
                    return bytes(data)
                except subprocess.TimeoutExpired:
                    raise Refused('diagnostic_read_timeout') from None
                finally:
                    if not completed:
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass


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


def execute(args, *, diagnostic_verifier=None, collect_diagnostic=None):
    if (diagnostic_verifier is None) != (collect_diagnostic is None):
        raise Refused('invalid_diagnostic_scope')
    os.umask(0o077)
    if sys.platform != 'linux' or not WORKER_RUNNER.is_file():
        raise ValueError('Use the designated worker host')
    work, plan = args.work.resolve(strict=True), args.plan.resolve(strict=True)
    if args.work.is_symlink() or args.plan.is_symlink() or not plan.is_relative_to(work) or work == plan:
        raise ValueError('Expected a frozen visit under its original worker')
    if diagnostic_verifier is None:
        from stock_ab_freeze import verify_visit

        config = verify_visit(work, plan, recovery=bool(args.recover))
        body = (plan / 'device.sh').read_bytes()
    else:
        config, body = diagnostic_verifier(work, plan, recovery=bool(args.recover))
    diagnostic = config['purpose'] == PORTRAIT_DIAGNOSTIC_PURPOSE
    if diagnostic != (diagnostic_verifier is not None):
        raise Refused('invalid_diagnostic_scope')
    expected = {key:config[key] for key in METADATA}
    label = config['label']
    invocation(body, 'request', label)
    _, install_header = invocation(body, 'install', label)
    archive = plan / 'inputs.tgz'
    originals = None if args.recover else references(archive)
    if args.preflight:
        print(json.dumps(dict(**expected, validated=True, device_accessed=False, lease_requested=False)))
        return 0
    expected_installation = getattr(args, 'expected_installation', None)
    if expected_installation is not None and not valid_installation_receipt(expected_installation, 'original'):
        raise Refused('invalid_installation_baseline')
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
                    result.get('lease_directory_absent_after_release') is not True or
                    not isinstance(result.get('released_at_utc'), str) or datetime.fromisoformat(result['released_at_utc']).tzinfo is None):
                raise Refused('release_unconfirmed')
            operation.close()
            return result['released_at_utc']
        def interrupt(*_):
            raise KeyboardInterrupt
        with ExitStack() as stack, ExitStack() as operation:
            private = stack.enter_context(tempfile.TemporaryDirectory(dir=attempt))
            askpass = Path(private)/'askpass'
            askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n', encoding='ascii')
            askpass.chmod(0o700)
            env = dict(os.environ, SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE='force', DISPLAY='unused',
                       WEREAD_TRANSIENT_PASSWORD=password)
            wire = None if args.recover else staged_wire(archive,install_header,Path(private)/'install.stream')
            transport = Transport(ssh, body, label, archive, originals, env, owned, wire=wire)
            signal.signal(signal.SIGTERM, interrupt)
            if args.recover:
                original = read_json(attempt/'operations.json')
                report.update(recovery_only=True, acquired_at_utc=original.get('acquired_at_utc'), recovery_required=True)
                save()
                operation.enter_context(physical_operation())
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
                    operation.enter_context(physical_operation())
                    acquired = lease.acquire(request)
                except (Exception, KeyboardInterrupt) as error:
                    details = getattr(error,'details',{})
                    partial = os.path.lexists(request.token_file) or any(details.get(key) for key in
                              ('lease_directory_created','private_token_created','owner_record_created'))
                    busy = (getattr(error,'code','') in ('lease_exists','operation_busy') or
                            isinstance(error,Refused) and str(error) == 'physical_operation_busy') and not partial
                    report.update(acquisition='busy' if busy else 'unconfirmed', recovery_required=bool(partial))
                    save()
                    return 2 if partial else 3 if busy else 1
                report.update(acquisition='acquired', acquired_at_utc=acquired['acquired_at_utc'], recovery_required=True)
                save()
            diagnostic_result = {'status': 'not_attempted'}

            def capture(submission):
                nonlocal diagnostic_result
                try:
                    diagnostic_result = collect_diagnostic(transport, submission)
                except Exception:
                    # No auxiliary read can prevent the normal closeout or
                    # replace the authoritative submission/handback results.
                    diagnostic_result = {'status': 'unavailable', 'reason': 'diagnostic_collection_error'}

            options = {'capture_diagnostic': capture} if diagnostic else {}
            status = drive(transport, release, report, journal, save, persist, recovering=bool(args.recover),
                           expected_installation=expected_installation, **options)
        if report['released']:
            request.token_file.unlink(missing_ok=True)
        summary = dict(label=label, lane=config['lane'], slot=config['slot'], primary=report['primary'],
                       released=report['released'], recovery_required=report['recovery_required'],
                       reason=report.get('close_reason') or report.get('refused') or report.get('qualification_reason'),
                       receipt=str(receipt))
        if diagnostic and not args.recover:
            path = attempt/'portrait-notification.json'
            independent = diagnostic_result | {'label':label, 'purpose':config['purpose'], 'released':report['released'],
                                               'startup_timing_eligible':False}
            # Persist auxiliary evidence after leaving the physical operation.
            # Recovery never overwrites it or reads another launch's record.
            try:
                checkpoint(path, independent)
            except OSError:
                summary['portrait_diagnostic'] = 'receipt_write_failed'
            else:
                summary.update(portrait_diagnostic=independent['status'], diagnostic_receipt=str(path))
        print(json.dumps(summary))
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
        return 3 if str(error) in ('visit_busy', 'physical_operation_busy') else 2 if args.recover else 1
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError, tarfile.TarError, KeyboardInterrupt) as error:
        print('Visit not confirmed:', type(error).__name__, file=sys.stderr)
        return 2 if args.recover or not args.preflight else 1


if __name__ == '__main__':
    raise SystemExit(main())
