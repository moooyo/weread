"""Run one read-only mount table observation without staging anything on the Kindle."""

import argparse
from contextlib import contextmanager
from datetime import datetime
import fcntl
import json
import os
from pathlib import Path
import re
import secrets
import signal
import subprocess
import sys
import tempfile

from run import LEASE, UTIL, stamp


HERE = Path(__file__).resolve().parent
POLICY = 'mount-table-read-v1'
PRECEDING = '3a578c34b99e748b'
SCOPE = 'read_only_mount_table_diagnosis'
OWNER = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}')
LABEL = re.compile('[0-9a-f]{16}')
GUARD_REFUSALS = {'baseline_roots','baseline_residue','baseline_native'}


def guard():
    common = (HERE/'volume-diagnostic.sh').read_text(encoding='ascii')
    start, end = '\nnative() {', '\nadmission() {'
    if common.count(start) != 1 or common.count(end) != 1:
        raise ValueError('Invalid native baseline definition')
    native = common[common.index(start):common.index(end)]
    return ('''#!/bin/sh
set -eu
export LC_ALL=C
[ "$#" = 2 ] || exit 64
case "$1" in admit|verify) ;; *) exit 64;; esac
label=$2
case "$label" in ''|*[!0-9a-f]*) exit 64;; esac
[ "${#label}" = 16 ] || exit 64
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
directory() { [ -d "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
fail() { printf 'refused %s\\n' "$1" >&2; exit 1; }
''' + native + '''
directory /mnt/us && directory /var/tmp || fail baseline_roots
for visit in "$label" PRECEDING; do
    absent "/mnt/us/.weread-bind-diagnostic-$visit" && absent "/var/tmp/weread-bind-diagnostic-$visit" || fail baseline_residue
done
absent /var/tmp/weread-stock-selector-bind || fail baseline_residue
native || fail baseline_native
printf 'native_clear %s %s\\n' "$1" "$label"
'''.replace('PRECEDING',PRECEDING)).encode('ascii')


def programs():
    script = HERE/'mount-table-diagnose.sh'
    if script.is_symlink() or not script.is_file() or not 100 <= script.stat().st_size <= 32768:
        raise ValueError('Expected the reviewed read-only diagnostic on the worker')
    content = script.read_bytes()
    if not content.isascii() or b'\0' in content or not content.endswith(b'\n'):
        raise ValueError('Invalid diagnostic transport')
    return guard(), content


def checkpoint(path, data):
    pending = path.with_suffix('.next'); pending.write_text(json.dumps(data, indent=2)+'\n',encoding='ascii'); pending.replace(path)


def time_text(value):
    try:
        return isinstance(value,str) and datetime.fromisoformat(value).tzinfo is not None
    except ValueError:
        return False


def exact_release(result):
    flags = ('owner_and_token_checked','only_owned_owner_json_removed_then_rmdir','native_restored_and_connections_closed')
    if (not isinstance(result,dict) or any(result.get(flag) is not True for flag in flags) or
            type(result.get('lease_directory_absent_after_release')) is not bool or not time_text(result.get('released_at_utc'))):
        raise ValueError('release_unconfirmed')
    return {'released_at_utc':result['released_at_utc'],'owner_and_token_checked':True,
            'lease_directory_absent_after_release':result['lease_directory_absent_after_release']}


def transaction(report, call, release, save, decoder, *, recover=False):
    label = report['label']
    def receive(mode):
        try:
            status, stdout, stderr = call(mode)
        except subprocess.TimeoutExpired as error:
            status, stdout, stderr = 124, error.stdout or b'', error.stderr or b''
        entry = {'mode':mode,'status':status,'time':stamp()}
        if mode == 'diagnose':
            try:
                report['observation'] = decoder(stdout)
            except (ValueError, UnicodeError):
                entry['reason'] = 'invalid_or_incomplete_observation'
        elif stdout != f'native_clear {mode} {label}\n'.encode('ascii'):
            entry['reason'] = 'invalid_baseline_reply'
        if status != 0:
            candidates = [b'refused '+reason.encode('ascii') for reason in GUARD_REFUSALS]
            reason = next((line[8:].decode('ascii') for line in stderr.splitlines() if line in candidates),None)
            entry['reason'] = reason or ('transport_timeout' if status == 124 else 'device_or_transport_refused')
        report['operations'].append(entry); save()
        return 'reason' not in entry

    sample_ok = False
    try:
        if not recover and receive('admit'):
            report['admitted'] = True
            report['diagnosis_started'] = True; save()
            sample_ok = receive('diagnose')
    finally:
        if not receive('verify'):
            raise ValueError('independent_baseline_unconfirmed')
        report['native_and_absence_verified'] = True; save()
        report.update(exact_release(release())); save()
    report['completed'] = recover or sample_ok
    save()


def secret_line():
    if sys.stdin.isatty():
        raise ValueError('Use the existing private one-line stdin pipe')
    source = getattr(sys.stdin,'buffer',sys.stdin); line = source.readline(258)
    if isinstance(line,str): line = line.encode('utf-8')
    value = line.removesuffix(b'\n').removesuffix(b'\r')
    if (not line.endswith(b'\n') or not 0 < len(value) <= 255 or any(char in value for char in (b'\r',b'\n',b'\0'))):
        raise ValueError('Expected one complete bounded private authentication line')
    return value.decode('utf-8')


@contextmanager
def lock_runs(runs):
    handle = os.open(runs,os.O_NOFOLLOW|os.O_RDONLY|os.O_DIRECTORY)
    try:
        fcntl.flock(handle,fcntl.LOCK_EX|fcntl.LOCK_NB)
        yield
    finally: os.close(handle)


def load_json(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 16384:
        raise ValueError('Missing original protected worker record')
    return json.loads(path.read_bytes())


def run(args):
    from mount_table_observation import observation
    os.umask(0o077)
    if not Path('/opt/kindle-toolchain/ubuntu-run').is_file() or not OWNER.fullmatch(args.owner):
        raise ValueError('Expected a valid owner on the designated worker')
    runs = args.runs.resolve(strict=True)
    if args.runs.is_symlink() or not runs.is_dir() or runs.stat().st_mode & 0o077:
        raise ValueError('Expected a private worker attempt directory')
    baseline, diagnostic = programs()
    trust = Path('/root/.config/kindle-physical')
    ssh = UTIL['ssh_command'](trust/'ssh_config',trust/'known_hosts','kindle-physical')
    lease = UTIL['load_lease'](args.lease_tool.resolve(strict=True))
    if args.preflight:
        print(json.dumps(dict(scope=SCOPE,preflight=True,lease_requested=False,device_accessed=False,bind_tested=False)))
        return 0
    with lock_runs(runs):
        attempt = runs/'attempt'; plan = attempt/'plan.json'; original = attempt/'operations.json'
        token = attempt/'private-token'
        if args.recover:
            identity = load_json(plan)
            label = identity.get('label')
            if (attempt.is_symlink() or not isinstance(label,str) or not LABEL.fullmatch(label) or
                    identity != dict(policy=POLICY,scope=SCOPE,label=label,owner=args.owner) or
                    token.is_symlink() or not token.is_file() or load_json(original).get('released_at_utc') or
                    ((attempt/'recovery.json').exists() and load_json(attempt/'recovery.json').get('released_at_utc'))):
                raise ValueError('Expected an interrupted read-only attempt with its original owner and token')
        else:
            if os.path.lexists(LEASE):
                print(json.dumps(dict(scope=SCOPE,busy=True,lease_requested=False,device_accessed=False))); return 3
            if os.path.lexists(attempt):
                raise ValueError('This one-shot attempt already exists; recovery cannot repeat diagnosis')
            label = secrets.token_hex(8)
        password = secret_line()
        if not args.recover:
            attempt.mkdir(mode=0o700); checkpoint(plan,dict(policy=POLICY,scope=SCOPE,label=label,owner=args.owner))
        request = lease.LeaseRequest(LEASE,token,args.owner,'weread-mount-read-'+label)
        report = dict(policy=POLICY,scope=SCOPE,label=label,owner=args.owner,started=stamp(),operations=[],completed=False,
                      recovery_only=args.recover,bind_tested=False,supports_bind=None,formal_launches=0,formal_pairs=0)
        output = attempt/('recovery.json' if args.recover else 'operations.json')
        def save(): checkpoint(output,report)
        def interrupt(*_): raise KeyboardInterrupt
        signal.signal(signal.SIGTERM,interrupt)
        with tempfile.TemporaryDirectory(dir=attempt) as local:
            ask = Path(local)/'askpass'; ask.write_text('#!/bin/sh\nprintf "%s\\n" "$WEREAD_TRANSIENT_PASSWORD"\n'); ask.chmod(0o700)
            env = dict(os.environ,SSH_ASKPASS=str(ask),SSH_ASKPASS_REQUIRE='force',DISPLAY='unused',WEREAD_TRANSIENT_PASSWORD=password)
            def call(mode):
                if mode not in ('admit','diagnose','verify') or lease.status(request).get('ownership_verified') is not True:
                    raise ValueError('ownership_changed')
                shell = 'export LC_ALL=C; exec sh -s'
                remote, payload = (shell,diagnostic) if mode == 'diagnose' else (shell+' -- '+mode+' '+label,baseline)
                result = subprocess.run([*ssh,remote],input=payload,capture_output=True,env=env,timeout=13)
                return result.returncode,result.stdout,result.stderr
            save()
            try:
                if not args.recover:
                    report['acquired_at_utc'] = lease.acquire(request)['acquired_at_utc']; save()
                transaction(report,call,lambda:lease.release(request,native_restored_and_connections_closed=True),save,observation,recover=args.recover)
            except (Exception,KeyboardInterrupt) as error:
                code = getattr(error,'code','interrupted' if isinstance(error,KeyboardInterrupt) else 'transaction_failed')
                report['failure'] = code if isinstance(code,str) and re.fullmatch(r'[a-z_]{1,48}',code) else 'transaction_failed'
                changes = getattr(error,'details',{})
                report['recovery_required'] = not bool(report.get('released_at_utc')) and (token.exists() or
                        any(changes.get(key) for key in ('lease_directory_created','private_token_created','owner_record_created')))
                save()
            finally:
                if report.get('released_at_utc'): token.unlink(missing_ok=True)
        print(json.dumps(report),flush=True); return 0 if report['completed'] else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runs',type=Path,required=True)
    parser.add_argument('--lease-tool',type=Path,required=True)
    parser.add_argument('--owner',required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--preflight',action='store_true'); mode.add_argument('--recover',action='store_true')
    raise SystemExit(run(parser.parse_args()))
