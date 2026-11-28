"""Validate the separate notification timeline without certifying any startup sample."""

import argparse
import json
from pathlib import Path
import re


LIMIT = 8192
MAX_WAIT = 28
PREFIX = 'WEREAD_PORTRAIT_DIAG'
ERRORS = {'setup_error', 'poll_error', 'read_error', 'watch_lost', 'lookup_error', 'clock_error'}
WAIT_FIELDS = ('started_ns', 'armed_sample_ns', 'deadline_ns', 'wake_sample_ns', 'named_event_sample_ns',
               'visible_sample_ns', 'finished_sample_ns')


def need(ok, problem):
    if not ok:
        raise ValueError(problem)


def number(value, zero=False):
    need(re.fullmatch(r'0|[1-9][0-9]{0,18}',value) is not None, 'Invalid numeric trace field')
    result = int(value)
    need((0 if zero else 1) <= result < 2**63, 'Numeric trace field out of range')
    return result


def wait_row(fields):
    need(len(fields) == 9, 'Invalid wait record length')
    slot, *stamps, reason = fields
    slot = number(slot)
    need(slot <= MAX_WAIT and reason in (ERRORS | {'hint', 'deadline'}), 'Invalid wait slot or outcome')
    begin, arm, deadline, wake, event, visible, end = [number(value, zero=True) for value in stamps]
    need(0 < begin < deadline and (not arm or begin <= arm) and (not wake or 0 < arm <= wake)
         and (not event or 0 < wake <= event) and (not visible or max(arm,wake,event) <= visible and arm > 0)
         and (not end or max(begin,arm,wake,event,visible) <= end), 'Unordered watcher timestamps')
    if reason == 'hint':
        need(0 < visible < deadline, 'Hint did not observe completion before the deadline')
    elif reason == 'deadline':
        need(arm > 0 and (not end or end >= deadline) and (not visible or visible >= deadline), 'Invalid timeout evidence')
    elif reason == 'setup_error':
        need(not any((arm,wake,event,visible)), 'Setup error claimed an armed watch')
    else:
        need(not visible, 'Watch failure claimed validated visibility')
    return dict(slot=slot, **dict(zip(WAIT_FIELDS, (begin,arm,deadline,wake,event,visible,end),strict=True)),outcome=reason)


def classify(wait, bracket):
    if wait['outcome'] in ERRORS:
        return 'watch_failure'
    if wait['outcome'] == 'hint':
        return 'completion_seen_before_deadline'
    deadline=wait['deadline_ns']
    if bracket and bracket[0] >= deadline:
        return 'publication_at_or_after_deadline'
    if bracket and bracket[1] < deadline:
        if wait['visible_sample_ns'] >= deadline or wait['named_event_sample_ns'] >= deadline:
            return 'in_window_publication_processed_after_deadline'
        return 'in_window_publication_no_timely_observation'
    if wait['visible_sample_ns'] >= deadline or wait['named_event_sample_ns'] >= deadline:
        return 'late_observation_publication_order_unknown'
    return 'publication_crosses_deadline' if bracket else 'publication_order_unknown'


def analyze(data, wait_count=None):
    need(0 < len(data) <= LIMIT and data.endswith(b'\n'), 'Invalid trace byte length or missing final newline')
    before = after = None
    write_result = None
    waits = {}
    for raw in data.decode('ascii').split('\n')[:-1]:
        prefix, category, *values = raw.split(' ')
        need(prefix == PREFIX and len(raw) <= 240, 'Foreign trace row')
        if category == 'publish_begin':
            need(before is None and len(values)==1, 'Invalid or duplicate publication begin')
            before = number(values[0])
        elif category == 'publish_end':
            need(after is None and len(values)==2 and values[1] in ('ok','rename_error'), 'Invalid or duplicate publication end')
            after, write_result = number(values[0]), values[1]
        else:
            need(category=='wait', 'Unexpected trace row')
            wait = wait_row(values)
            need(wait['slot'] not in waits, 'Duplicate wait slot')
            waits[wait['slot']] = wait
    need(after is None or before is not None and after >= before, 'Unmatched or reversed publication end')
    ordered=[waits[i] for i in sorted(waits)]
    for earlier,later in zip(ordered,ordered[1:]):
        need(max(earlier[key] for key in WAIT_FIELDS if key != 'deadline_ns') <= later['started_ns'], 'Overlapping wait records')
    if wait_count is not None:
        need(type(wait_count) is int and max(waits,default=0)<=wait_count<=MAX_WAIT, 'Trace exceeds the launcher wait count')
    bracket = [before, after] if write_result=='ok' else None
    return {'clock':'CLOCK_MONOTONIC', 'publication':{'before_ns':before,'after_ns':after,'result':write_result,'bracket_ns':bracket},
            'waits':[wait | {'classification':classify(wait,bracket)} for wait in ordered],
            'missing_wait_slots':None if wait_count is None else sorted(set(range(1,wait_count+1))-waits.keys()),
            'startup_timing_eligible':False}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('trace',type=Path)
    p.add_argument('--wait-count',type=int,help='Exact wait count from the same launch, if independently available')
    args=p.parse_args()
    try:
        with args.trace.open('rb') as f:
            report=analyze(f.read(LIMIT+1),args.wait_count)
    except (OSError,ValueError) as error:
        p.error(str(error))
    print(json.dumps(report,indent=2))


if __name__=='__main__':
    main()
