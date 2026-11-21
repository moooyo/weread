"""A single stock-card visit. Its measurement and its handback are independent decisions."""

import json
import os
import re

from qualified_submit import SUBMITTED, qualify
from run_daily_render import CLOCK, Refused, json_line, now, release_bounds


STATES = dict(admit='admitted', stage='staged', install='installed', prepare='prepared', activate='activated',
              arm='armed', claim='claimed', witness='witnessed', stop='stopped', restore='restored', close='closed')
STAT_FIELDS = ('launch', 'document', 'application')
IDENTITY_PREFIX = r'[0-9]{1,20}:[0-9]{1,20}:[0-9]{1,20}:[0-7]{1,5}:[0-9]{1,10}:[0-9]{1,10}:'
STAT = re.compile(IDENTITY_PREFIX + r'[0-9]{1,20}:[0-9]{1,8}')
DOCUMENT_STAT = re.compile(IDENTITY_PREFIX + r'[0-9]{1,8}')
GUARDS = ('original', 'audit_before', 'audit_after')
METADATA = ('label', 'purpose', 'lane', 'slot', 'role', 'pair')


class WireRefusal(Refused):
    def __init__(self, reason, returncode=None, header_observed=None):
        super().__init__(reason)
        self.returncode = returncode
        self.header_observed = header_observed


def valid_stat(key, value):
    return (key in STAT_FIELDS and isinstance(value, str) and
            (DOCUMENT_STAT if key == 'document' else STAT).fullmatch(value) is not None)


def initial(config, owner):
    return dict({key: config[key] for key in METADATA}, owner=owner, protocol='isolated_stock_launch',
                measurement_build='temporary_measurement_build', fixture='disposable_public', started_at_utc=now(),
                operations=[], guard_checks=[], primary='not_timed', released=False, device_accessed=False,
                recovery_required=False, native_guard_verified=False)


def checkpoint(path, value):
    temporary = path.with_name(path.name + '.next')
    with os.fdopen(os.open(temporary, os.O_CREAT | os.O_WRONLY | os.O_TRUNC | os.O_NOFOLLOW, 0o600), 'wb') as output:
        output.write((json.dumps(value, indent=2, allow_nan=False) + '\n').encode('ascii'))
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, path)
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def checked(mode, raw, label):
    if not isinstance(raw, bytes) or not raw.endswith(b'\n') or raw.count(b'\n') != 1:
        raise Refused('invalid_receipt')
    row = json_line(raw)
    if mode in STATES:
        if row == {'state': STATES[mode]} or (mode in ('stop', 'restore', 'close') and row == {'state':'already_absent'}):
            return row
    elif mode in ('original', 'absent'):
        if (set(row) == {'state', *STAT_FIELDS} and row['state'] == mode and
                all(valid_stat(key,row[key]) for key in STAT_FIELDS)):
            return row
    elif mode in ('audit_before', 'audit_after'):
        if row == {'state':'audited'}:
            return row
    elif mode == 'request_armed':
        if (set(row) == {'state','label','request_clock','request_armed_ns','request_deadline_ns'} and
                (row['state'], row['label'], row['request_clock']) == ('armed', label, CLOCK) and
                all(type(row[key]) is int for key in ('request_armed_ns','request_deadline_ns')) and
                0 < row['request_armed_ns'] < row['request_deadline_ns'] < 2**63 and
                row['request_deadline_ns'] - row['request_armed_ns'] <= 60_000_000_000):
            return row
    elif mode == 'tap':
        release_bounds(raw)
        return row
    elif mode == 'library_submit':
        if (set(row) == {'label','marker','tile_receipt','observed_ns'} and row['label'] == label and
                type(row['observed_ns']) is int and 0 < row['observed_ns'] < 2**63 and isinstance(row['marker'], str)
                and (match := SUBMITTED.fullmatch(row['marker'])) and match[1] == label):
            release_bounds(json.dumps(row['tile_receipt'], separators=(',', ':')).encode('ascii'))
            return row
    raise Refused('unexpected_' + mode + '_receipt')


def bind_syscall_refusal(code):
    match = re.fullmatch(r'bind_(?:un)?mount_call_([1-9][0-9]{0,3})', code)
    return bool(match and int(match[1]) <= 4095)


def public_error(error):
    code = str(error) if isinstance(error, Refused) else ''
    scope, separator, reason = code.partition(':')
    if re.fullmatch(r'[a-z][a-z_]{0,64}(?::[a-z][a-z_]{0,64})?', code) or (
            (not separator or re.fullmatch(r'[a-z][a-z_]{0,64}', scope)) and bind_syscall_refusal(reason if separator else scope)):
        return code
    return 'interrupted' if isinstance(error, KeyboardInterrupt) else 'transport_or_worker_error'


def fresh_journal(label):
    return dict(label=label, phase='not_started', stage_possible=False, navigation_possible=False, restored=False,
                original=None, audit_before=False, guard_complete=False)


def valid_journal(journal, label):
    if not isinstance(journal, dict) or set(journal) != set(fresh_journal(label)) or journal['label'] != label:
        return False
    keys = ('stage_possible', 'navigation_possible', 'audit_before', 'restored', 'guard_complete')
    if any(type(journal[key]) is not bool for key in keys):
        return False
    if journal['original'] is not None:
        try:
            checked('original',(json.dumps(journal['original'])+'\n').encode('ascii'),label)
        except (Refused, TypeError, ValueError):
            return False
    if journal['audit_before'] != journal['navigation_possible'] or journal['navigation_possible'] and journal['original'] is None:
        return False
    phase = journal['phase']
    if phase == 'not_started':
        return not any(journal[key] for key in keys) and journal['original'] is None
    if not journal['stage_possible']:
        return False
    if phase in ('stage_requested','original_observed'):
        return (journal['original'] is not None) == (phase == 'original_observed') and not any(journal[key] for key in keys[1:])
    if phase in ('prepare_requested','activate_requested','arm_requested','request_requested','tap_requested'):
        return journal['navigation_possible'] and not journal['restored'] and not journal['guard_complete']
    if phase == 'restored':
        return journal['restored'] and not journal['guard_complete']
    if phase == 'close_requested':
        return (not journal['navigation_possible'] or journal['restored']) and not journal['guard_complete']
    if phase == 'absent':
        return (not journal['navigation_possible'] or journal['restored']) and journal['guard_complete'] == journal['audit_before']
    return False


def drive(transport, release, report, journal, save, persist, *, recovering=False):
    channel = None
    label = report['label']

    def step(mode, action):
        collection = report['guard_checks' if mode in GUARDS else 'operations']
        try:
            receipt = checked(mode, action(), label)
        except (Exception, KeyboardInterrupt) as error:
            reason = public_error(error)
            failed = dict(mode=mode, status=1, reason=reason, at=now())
            if isinstance(error, WireRefusal):
                if type(error.returncode) is int and -255 <= error.returncode <= 255:
                    failed['ssh_returncode'] = error.returncode
                if mode == 'install' and type(error.header_observed) is bool:
                    failed['install_header_observed'] = error.header_observed
            collection.append(failed)
            save()
            raise Refused(mode + ':' + reason) from None
        collection.append(dict(mode=mode, status=0, reason='', receipt=receipt, at=now()))
        save()
        return receipt

    def call(mode):
        return step(mode, lambda: transport.call(mode))

    def mark(phase, **fields):
        journal.update(phase=phase, **fields)
        persist()

    try:
        if not recovering:
            call('admit')
            mark('stage_requested', stage_possible=True)
            call('stage')
            call('install')
            original = call('original')
            mark('original_observed', original=original)
            call('audit_before')
            mark('prepare_requested', audit_before=True, navigation_possible=True)
            call('prepare')
            mark('activate_requested')
            call('activate')
            mark('arm_requested')
            call('arm')
            mark('request_requested')
            def first_line():
                nonlocal channel
                channel = transport.request()
                return channel.armed()
            step('request_armed', first_line)
            if not channel.running():
                raise Refused('request_exited_before_tap')
            mark('tap_requested')
            tap = call('tap')
            report['synthetic_release_ns'] = [tap['up_before_ns'], tap['up_after_ns']]
            save()
            call('claim')
            call('witness')
            step('library_submit', channel.collect)
    except (Exception, KeyboardInterrupt) as error:
        report['refused'] = public_error(error)
        save()
    finally:
        try:
            if channel is not None:
                channel.close()
            if journal['stage_possible']:
                for mode in ('stop','restore','close'):
                    if mode == 'close' and journal['phase'] != 'absent':
                        mark('close_requested')
                    state = call(mode)['state']
                    if state == 'already_absent' and journal['navigation_possible'] and not journal['restored']:
                        raise Refused('stage_lost_before_native_restore')
                    if mode == 'restore' and state == 'restored':
                        mark('restored', restored=True)
                returned = call('absent')
                if journal['original'] is not None:
                    if any(returned[key] != journal['original'][key] for key in STAT_FIELDS):
                        raise Refused('original_identity_changed')
                if journal['audit_before']:
                    call('audit_after')
                    report['native_guard_verified'] = True
                if journal['navigation_possible'] and not report['native_guard_verified']:
                    raise Refused('missing_original_guard')
                mark('absent', guard_complete=report['native_guard_verified'])
            released_at = release()
            report.update(released_at_utc=released_at, released=True, owner_and_token_checked=True, recovery_required=False)
        except (Exception, KeyboardInterrupt) as error:
            report['close_reason'] = public_error(error)
            report['recovery_required'] = True
        save()
    if not report['released']:
        return 2
    if recovering:
        return 0
    if 'refused' in report or not report['native_guard_verified']:
        return 1
    try:
        result = qualify(report)
        if result['content'] != 'entries':
            raise Refused('unexpected_public_fixture_content')
        report['qualified'] = result
    except (Refused, ValueError, TypeError, KeyError) as error:
        report['qualification_reason'] = public_error(error)
        save()
        return 1
    report['primary'] = 'qualified'
    save()
    return 0
