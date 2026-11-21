"""Publish one bounded operator cue from a live, labeled stock diagnostic; never admit the app."""

import os
import re
import selectors
import signal
import subprocess
import time

from pose_probe import parse_display_group


ROW = '''        printf 'read %s %s %s %s\\n' "$sequence" "$left" "$winmgr" "$right"'''
MARKER = b'POSE_CUE '
MAX_CUE_AGE = 7


def cue_program(script):
    if script.count(ROW) != 1 or 'pose_display_trace %s' not in script or 'POSE_CUE' in script:
        raise ValueError('Unexpected display trace program for a first-group cue')
    return script.replace(ROW, ROW + '''
        if [ "$sequence" = 1 ]; then
            printf 'POSE_CUE %s read 1 %s %s %s\\n' "$label" "$left" "$winmgr" "$right" >&2
        fi''')


def first_group(line, label):
    parsed = re.fullmatch(rb'POSE_CUE ([0-9a-f]{16}) (read 1 [^\n]{1,180})', line)
    if not parsed or parsed[1].decode('ascii') != label:
        raise ValueError('Unbound first-group cue')
    raw = parsed[2]
    group = parse_display_group(raw.decode('ascii'), 1)
    return raw, ('TURN_NOW' if group['winmgr']['raw'] == 'D' and
                 all((group[key]['raw'], group[key]['framebuffer_rotation']) == (16, 1)
                     for key in ('before', 'after')) else 'HOLD_STILL')


def streaming_probe(command, payload, env, label, announce, *, timeout=40):
    started = time.monotonic()
    deadline = started + timeout
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             env=env, bufsize=0, start_new_session=True)
    output, pending, errors = bytearray(), bytearray(), bytearray()
    offset, uploaded, receipt = 0, None, None
    try:
        with selectors.DefaultSelector() as selector:
            for stream, event in ((child.stdin, selectors.EVENT_WRITE), (child.stdout, selectors.EVENT_READ),
                                  (child.stderr, selectors.EVENT_READ)):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, event)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise subprocess.TimeoutExpired(command, timeout)
                for key, _ in selector.select(remaining):
                    stream = key.fileobj
                    if stream is child.stdin:
                        try:
                            offset += os.write(key.fd, payload[offset:offset + 16384])
                        except BrokenPipeError:
                            selector.unregister(stream)
                            stream.close()
                            continue
                        if offset == len(payload):
                            selector.unregister(stream)
                            stream.close()
                            uploaded = time.monotonic()
                        continue
                    data = os.read(key.fd, 4096)
                    if not data:
                        selector.unregister(stream)
                        continue
                    if stream is child.stdout:
                        output.extend(data)
                        if len(output) > 8192:
                            raise ValueError('Oversized display trace response')
                        continue
                    pending.extend(data)
                    if len(pending) + len(errors) > 8192:
                        raise ValueError('Oversized trace diagnostic stream')
                    while b'\n' in pending:
                        line, _, rest = pending.partition(b'\n')
                        pending = bytearray(rest)
                        if not line.startswith(MARKER):
                            errors.extend(line + b'\n')
                            continue
                        if receipt is not None:
                            raise ValueError('Duplicate first-group cue')
                        raw, action = first_group(line, label)
                        now = time.monotonic()
                        live = (uploaded is not None and 0 <= now - uploaded <= MAX_CUE_AGE and not output
                                and child.poll() is None and now < deadline)
                        receipt = {'first_group_wire': raw.decode('ascii'), 'action': action if live else 'not_issued'}
                        if live:
                            announce(action)
            code = child.wait(timeout=max(0, deadline - time.monotonic()))
        errors.extend(pending)
        if not code and (receipt is None or output.splitlines()[1:2] != [receipt['first_group_wire'].encode('ascii')]):
            raise ValueError('First group did not match the completed stock trace')
        return subprocess.CompletedProcess(command, code, bytes(output), bytes(errors)), receipt
    finally:
        if child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        for stream in (child.stdin, child.stdout, child.stderr):
            stream.close()
