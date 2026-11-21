"""Dispatch only freshly verified offers from the live candidate Reader."""

import os
import re
import select
import subprocess
import time


PHASES = ('sleep', 'turn', 'wake', 'recover-wake')
PERMIT_LIMIT = 3.0
CASE_LIMIT = 155


class CueGate:
    def __init__(self, label, permit, record):
        self.label = label
        self.permit = permit
        self.record = record
        self.offered = set()
        self.issued = set()
        self.completed = False

    def offer(self, phase):
        if phase not in PHASES or phase in self.offered:
            raise ValueError('invalid_or_repeated_phase')
        self.offered.add(phase)
        previous = {'sleep': set(), 'turn': {'sleep'}, 'wake': {'sleep','turn'}, 'recover-wake': {'sleep'}}
        if not previous[phase] <= self.issued or self.issued.intersection({'wake', 'recover-wake'}):
            self.record(phase, 'suppressed_order')
            return
        if not self.permit(phase):
            self.record(phase, 'suppressed_no_fresh_permit')
            return
        self.issued.add(phase)
        self.record(phase, 'issued')

    def line(self, raw):
        if self.completed:
            raise ValueError('event_after_completion')
        if raw == b'completed B-reader':
            self.completed = True
            return
        match = re.fullmatch(rb'PHASE ([0-9a-f]{16}) (sleep|turn|wake|recover-wake)', raw)
        if not match or match[1].decode() != self.label:
            raise ValueError('unknown_or_foreign_phase')
        self.offer(match[2].decode())


def permitted(label, phase, query, clock=time.monotonic):
    started = clock()
    try:
        result = query(phase, PERMIT_LIMIT)
    except (OSError, subprocess.TimeoutExpired):
        return False
    elapsed = clock() - started
    return (0 <= elapsed < PERMIT_LIMIT and result.returncode == 0 and
            result.stdout == f'permitted {label} {phase}\n'.encode('ascii'))


def stream_case(command, source, env, gate, timeout=CASE_LIMIT):
    deadline = time.monotonic() + timeout
    pending = b''
    total = 0
    with subprocess.Popen(command, stdin=source, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, bufsize=0) as process:
        try:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
                    raise subprocess.TimeoutExpired(command, timeout)
                data = os.read(process.stdout.fileno(), 512)
                if not data:
                    break
                total += len(data)
                pending += data
                if total > 1024:
                    raise ValueError('oversized_phase_stream')
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    gate.line(line)
            code = process.wait(timeout=max(0, deadline - time.monotonic()))
            if not code and (pending or not gate.completed):
                raise ValueError('missing_exact_case_completion')
            return subprocess.CompletedProcess(command, code, b'completed B-reader\n' if not code else b'', b'')
        except ValueError:
            return subprocess.CompletedProcess(command, 65, b'', b'refused invalid_phase_stream')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
