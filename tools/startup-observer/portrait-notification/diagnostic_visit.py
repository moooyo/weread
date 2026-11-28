"""Bind a notification snapshot to one separately verified diagnostic visit."""

from pathlib import Path

from analyze import LIMIT, WAIT_FIELDS, analyze
from prepare import replace, source
from qualified_submit import PORTRAIT_DIAGNOSTIC_PURPOSE
from run_daily_render import Refused
from run_stock_ab_visit import PORTRAIT_READ, execute as run_visit


HERE = Path(__file__).resolve().parent
SNAPSHOT = b'WEREAD_PORTRAIT_SNAPSHOT'
WIRE_LIMIT = LIMIT + len(SNAPSHOT) + len(b' 0123456789abcdef 0000000000000000000 0000000000000000000\n')
ACK = '*) write_tag stock-ack-seen || return 1;;'
DISPATCH = 'case "$mode" in\n    admit)'
SUBMIT = r'''    printf '{"label":"%s","marker":"%s\\n","tile_receipt":%s,"observed_ns":%s}\n' "$label" "$marker" "$tile_receipt" "$observed"'''
BIND = '''    # Observation is auxiliary: a failed anchor write cannot change SEND.
    case "$marker" in
        *' entries') set -- $marker; record_once portrait-diagnostic-submit "$session $3 $observed" 2>/dev/null || :;;
    esac
'''


def device_program(base):
    """Extend a fresh private trial program, not its inputs or ordinary launcher."""
    text = base.decode('ascii')
    text = replace(text, ACK, '*) [ "${2-}" = read_only ] || write_tag stock-ack-seen || return 1;;')
    text = replace(text, SUBMIT, BIND + SUBMIT)
    branch = f'''case "$mode" in
    {PORTRAIT_READ}) portrait_diagnostic_receipt 2>/dev/null || fail diagnostic_snapshot;;
    trace) exit 64;;
    admit)'''
    return replace(text, DISPATCH, source(HERE/'collect.sh') + '\n' + branch).encode('ascii')


def snapshot(raw, submission):
    stamp = [submission[key] for key in ('submitted_ns', 'observed_ns')]
    expected = SNAPSHOT + f" {submission['label']} {stamp[0]} {stamp[1]}".encode('ascii')
    header, separator, trace = raw.partition(b'\n')
    if len(raw) > WIRE_LIMIT or not separator or header != expected:
        raise ValueError('Unbound snapshot envelope')
    analysis = analyze(trace)
    start = submission['synthetic_release_ns'][0]
    samples = [analysis['publication'][key] for key in ('before_ns', 'after_ns')]
    samples.extend(wait[key] for wait in analysis['waits'] for key in WAIT_FIELDS if key != 'deadline_ns')
    if any(value and not start <= value <= stamp[0] for value in samples):
        raise ValueError('Diagnostic sample outside its startup')
    # Keep only parser-approved rows so the private evidence can be reanalyzed.
    return {'status': 'collected', 'submission': submission, 'analysis': analysis,
            'records': trace.decode('ascii').splitlines()}


def collect(transport, submission):
    try:
        raw = transport.read_portrait_diagnostic(WIRE_LIMIT)
    except Refused as error:
        reason = str(error)
        if reason not in ('diagnostic_read_timeout', 'diagnostic_read_refused', 'diagnostic_output_limit'):
            reason = 'diagnostic_read_refused'
        return {'status': 'unavailable', 'reason': reason, 'submission': submission}
    try:
        return snapshot(raw, submission)
    except ValueError:
        return {'status': 'unavailable', 'reason': 'invalid_diagnostic_snapshot', 'submission': submission}


def execute(args, verify_trial):
    """Use a caller's fresh diagnostic material verifier; there is no live CLI."""
    def verify(work, plan, *, recovery):
        config, base = verify_trial(work, plan, recovery=recovery)
        if config.get('purpose') != PORTRAIT_DIAGNOSTIC_PURPOSE:
            raise Refused('invalid_diagnostic_scope')
        path = plan/'device.sh'
        program = device_program(base)
        if (path.is_symlink() or not path.is_file() or path.stat().st_size != len(program)
                or path.read_bytes() != program):
            raise Refused('invalid_diagnostic_device_program')
        return config, program

    return run_visit(args, diagnostic_verifier=verify, collect_diagnostic=collect)
