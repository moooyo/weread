"""Inspect a disposable file bind in the current process views, not an on-demand handler."""

from pathlib import Path
import re


HERE = Path(__file__).resolve().parent
POLICY = 'selector-bind-v1'
REASONS = ('bind_stage bind_files bind_native_service bind_native_view bind_native_proc_permission bind_native_proc_read '
           'bind_native_proc_limit bind_namespace bind_mount_table bind_parent_value bind_parent_scope bind_propagation bind_foreign_mount bind_journal '
           'bind_provenance bind_unknown_state bind_contended bind_mount_unconfirmed bind_unmount_unconfirmed helper_response').split()


def program(size):
    if type(size) is not int or not 52 <= size <= 256 * 1024:
        raise ValueError('invalid_bind_probe_size')
    common = (HERE/'volume-diagnostic.sh').read_text(encoding='ascii')
    if common.count('\nmarker() {') != 1:
        raise ValueError('invalid_shared_native_admission')
    header = common.split('\nmarker() {')[0].replace('weread-exchange-diagnostic-', 'weread-bind-diagnostic-')
    source = (HERE/'scratch-bind.sh').read_text(encoding='ascii')
    if source.count('@HELPER_BYTES@') != 1 or header.count('weread-bind-diagnostic-') != 2:
        raise ValueError('invalid_bind_probe_program')
    return header + '\n' + source.replace('@HELPER_BYTES@', str(size))


def refusal(stage, reason, hint=None):
    result = {'result':'not_qualified', 'stage':stage, 'original_identity_and_bytes_restored':True, 'on_demand_handler_observed':False}
    code = re.fullmatch(r'(bind_(?:un)?mount_call)_([1-9][0-9]{0,3})', reason)
    if code and int(code[2]) <= 4095:
        result.update(reason=code[1], errno=int(code[2]))
    elif reason in REASONS or (reason == 'already_unbound' and stage == 'independent_view'):
        result['reason'] = reason
    else:
        raise ValueError('invalid_bind_probe_reason')
    if hint is not None:
        if (hint != 'before_mount' or stage not in ('prepare', 'bind') or code or
                reason in ('bind_mount_unconfirmed', 'bind_unmount_unconfirmed', 'helper_response')):
            raise ValueError('invalid_bind_probe_phase_hint')
        result['mount_call_not_reached'] = True
    return result


def observation(raw):
    if not isinstance(raw, bytes) or len(raw) > 200 or not raw.endswith(b'\n') or raw.count(b'\n') != 2:
        raise ValueError('invalid_bind_probe_result')
    first, second = raw.decode('ascii').removesuffix('\n').split('\n')
    if first == 'scratch_bind bound' and second == 'independent_view bound':
        return {'result':'current_views_verified', 'independent_connection_saw_bound_file':True,
                'original_identity_and_bytes_restored':True, 'on_demand_handler_observed':False}
    if first == 'scratch_bind bound' and second == 'independent_view unbound':
        return refusal('independent_view', 'already_unbound')
    reason = second.removeprefix('independent_view refused ')
    if first == 'scratch_bind bound' and reason != second:
        return refusal('independent_view', reason)
    if first.startswith('scratch_bind refused ') and second == 'independent_view not_attempted':
        fields = first.split(' ')
        if len(fields) in (4,5) and fields[2] in ('prepare','bind','status'):
            return refusal(*fields[2:])
    raise ValueError('unexpected_bind_probe_value')
