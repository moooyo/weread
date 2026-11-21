"""Deliver a pinned range of Scribe stock-card visits without retrying or holding a device lease between visits."""

import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import re
import resource
import select
import signal
import stat
import subprocess
import sys
import time

from run_daily_render import Refused
from stock_ab_freeze import SCOPE, verify_batch
from stock_ab_report import MATRIX, checked_slots, private_record, read_receipts, summarize


LEASE_TOOL = '/root/weread-orientation-20261121.WR7ofkGL/device_lease.py'
STOP = 'stock-range.stop'
APPROVAL = 'stock-range-approval.json'
WORK_PATH = re.compile(r'/root/weread-stock-chain-ab-[0-9]{8}\.[A-Za-z0-9]{8}')


def emit(state, **detail):
    print(json.dumps(dict(state=state, **detail)), flush=True)


def reference(work, batch, first_label):
    labels = [slot['label'] for slot in checked_slots(batch)]
    source = batch.get('source_commit')
    if (labels[0] != first_label or not isinstance(source,str) or not re.fullmatch('[0-9a-f]{40}',source) or
            any(batch.get(key) != value for key,value in SCOPE.items()) or
            any(batch['bundles'][lane]['members'].get('selector-bind') != 66868 for lane in 'AB')):
        raise Refused('not_the_approved_batch')
    return dict(worker=str(work), claim_protocol=batch['claim_protocol'], source_commit=source, selector_bytes=66868, labels=labels)


def seal(work, first_label):
    batch = verify_batch(work)
    data = reference(work,batch,first_label)
    if any(os.path.lexists(work/'stock-library-ab'/slot['visit']/'attempt') for slot in batch['schedule']):
        raise Refused('batch_already_attempted')
    with os.fdopen(os.open(work/APPROVAL, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400), 'w') as stream:
        json.dump(data,stream); stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
    emit('range_sealed',first_label=first_label,device_accessed=False,lease_requested=False)


def pinned(work, first_label):
    directory = work/'stock-library-ab'
    if work.resolve(strict=True) != work or directory.resolve(strict=True) != directory:
        raise Refused('unexpected_batch_path')
    batch = private_record(directory/'batch.json')
    path = work/APPROVAL
    if not os.path.lexists(path): raise Refused('range_not_sealed')
    info = path.lstat()
    if (not stat.S_ISREG(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o400 or info.st_uid != os.geteuid() or
            info.st_nlink != 1 or private_record(path) != reference(work,batch,first_label)):
        raise Refused('not_the_approved_batch')
    return batch


def boundary(work, batch, next_slot, owner):
    directory = work/'stock-library-ab'
    receipts = read_receipts(directory, batch)
    samples = summarize(batch, receipts)['samples']
    for slot, receipt, sample in zip(batch['schedule'], receipts, samples, strict=True):
        attempt = directory/slot['visit']/'attempt'
        if slot['slot'] >= next_slot:
            if os.path.lexists(attempt): raise Refused('later_slot_already_attempted')
        elif (not receipt or receipt.get('owner') != owner or receipt.get('acquisition') != 'acquired' or
              receipt.get('primary') != 'qualified' or sample['result'] != 'timed' or os.path.lexists(attempt/'recovery.json')):
            raise Refused('previous_slot_not_qualified_and_returned')


@contextmanager
def controller_guard(work):
    descriptor = os.open(work, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(descriptor)


def stop(work):
    try: os.close(os.open(work/STOP, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600))
    except FileExistsError: pass
    emit('stop_requested')


def clear_stop(work):
    marker = work/STOP
    if os.path.lexists(marker):
        info = marker.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_nlink != 1 or info.st_size or info.st_mode & 0o077:
            raise Refused('unexpected_stop_marker')
        marker.unlink()
    emit('stop_cleared')


@contextmanager
def live_input(descriptor):
    mode = os.fstat(descriptor).st_mode
    if not (stat.S_ISFIFO(mode) or stat.S_ISSOCK(mode)): raise Refused('use_a_private_live_pipe')
    poller = select.poll(); poller.register(descriptor, select.POLLIN | select.POLLHUP | select.POLLERR)
    line = bytearray(); deadline = time.monotonic() + 15
    try:
        while b'\n' not in line:
            remaining = deadline - time.monotonic()
            if len(line) >= 257 or remaining <= 0 or not poller.poll(int(remaining*1000)):
                raise Refused('missing_bounded_credential_line')
            chunk = os.read(descriptor, 258-len(line))
            if not chunk: raise Refused('caller_disconnected')
            line.extend(chunk)
        value = line[:-1].removesuffix(b'\r')
        try:
            if (not line.endswith(b'\n') or not 0 < len(value) <= 255 or any(char in value for char in (0,10,13))):
                raise Refused('expected_one_credential_line')
            value.decode('utf-8')
        finally:
            value[:] = bytes(len(value))
        yield line, lambda: not poller.poll(0)
    finally:
        line[:] = bytes(len(line))


def single_visit(work, slot, owner, line):
    command = [sys.executable,'-B',str(work/'src/tools/startup-observer/run_stock_ab_visit.py'),
               '--work',str(work),'--plan',str(work/'stock-library-ab'/slot['visit']),'--owner',owner,'--lease-tool',LEASE_TOOL]
    return subprocess.run(command, input=line, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                          start_new_session=True).returncode


def deliver(work, batch, first, last, owner, line, connected):
    for index in range(first, last+1):
        slot = batch['schedule'][index]
        boundary(work,batch,index,owner)
        if os.path.lexists(work/STOP) or not connected():
            emit('stopped_before_slot',slot=index); return 4
        emit('dispatch',slot=index,lane=slot['lane'],label=slot['label'])
        if os.path.lexists(work/STOP) or not connected(): return 4
        code = single_visit(work,slot,owner,line)
        if code != 0:
            emit('single_visit_stopped',slot=index,runner_code=code,visit=slot['visit'])
            return code if code in (1,2,3) else 2
        boundary(work,batch,index+1,owner)
        emit('qualified_and_returned',slot=index,label=slot['label'])
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work', required=True, type=Path)
    parser.add_argument('--first-label', required=True)
    operations = parser.add_subparsers(dest='mode',required=True)
    for mode in ('run','check'):
        action = operations.add_parser(mode)
        for name in ('start','end'): action.add_argument('--'+name, type=int, required=True)
        action.add_argument('--owner',required=True)
    for mode in ('seal','stop','clear-stop'): operations.add_parser(mode)
    args = parser.parse_args()
    work = args.work
    if (not WORK_PATH.fullmatch(str(work)) or work.resolve(strict=True) != work or
            Path(__file__).resolve().parent != work/'src/tools/startup-observer' or not re.fullmatch('[0-9a-f]{16}',args.first_label)):
        parser.error('Use the original successor worker path and the first label from its freeze receipt')
    if args.mode in ('run','check') and (not 0 <= args.start <= args.end < len(MATRIX) or
                                        not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}',args.owner)):
        parser.error('Specify an owner and a contiguous range within slots 0 through 13')
    try:
        if args.mode == 'stop': pinned(work,args.first_label); stop(work); return 0
        with controller_guard(work):
            if args.mode == 'seal': seal(work,args.first_label); return 0
            batch = pinned(work,args.first_label)
            if args.mode == 'clear-stop': clear_stop(work); return 0
            if os.path.lexists(work/STOP): emit('stop_latched'); return 4
            boundary(work,batch,args.start,args.owner)
            if args.mode == 'check': emit('range_ready',first=args.start,last=args.end,device_accessed=False); return 0
            interrupted = False
            def cancel(*_):
                nonlocal interrupted
                interrupted = True
            for signum in (signal.SIGHUP,signal.SIGTERM,signal.SIGINT): signal.signal(signum,cancel)
            resource.setrlimit(resource.RLIMIT_CORE,(0,0))
            with live_input(sys.stdin.fileno()) as (line, attached):
                return deliver(work,batch,args.start,args.end,args.owner,line,lambda: not interrupted and attached())
    except BlockingIOError:
        emit('controller_busy'); return 3
    except (Refused, OSError, ValueError, TypeError, KeyError, RuntimeError, OverflowError) as error:
        print('Range halted:', str(error) if isinstance(error,Refused) else type(error).__name__,file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
