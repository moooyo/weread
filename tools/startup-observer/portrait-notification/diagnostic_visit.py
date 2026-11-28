"""Bind a notification snapshot to one separately verified diagnostic visit."""

import base64
from pathlib import Path
import re

from analyze import LIMIT, WAIT_FIELDS, analyze
from prepare import replace, source
from qualified_submit import PORTRAIT_DIAGNOSTIC_PURPOSE
from prepare_daily_render import check_arm_elf
from run_daily_render import Refused
from run_stock_ab_visit import MAX_HEADER, PORTRAIT_READ, execute as run_visit


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


def receive_program(text, reader):
    encoded = base64.b64encode(reader).decode('ascii')
    inline = '\n'.join(encoded[i:i+76] for i in range(0,len(encoded),76))
    fragment = source(HERE/'receive-progress.sh').replace('@RECEIVER_BASE64@', inline).replace('@RECEIVER_BYTES@', str(len(reader)))
    text = replace(text, 'receive_archive() {\n', fragment + '\nreceive_archive() {\n')
    text = replace(text, "(exec 7<&- 8<&-; exec timeout -s KILL 150 sh -c '",
                   "(exec 3<&- 4<&- 7<&- 8<&-; exec timeout -s KILL 150 /bin/sh -c '")
    text = replace(text, '''[ "$(stat -Lc "%d:%i" /proc/self/fd/6)" = "$2" ] || exit 76
        exec head -c "$3" >/proc/self/fd/6''', '''[ "$(stat -Lc "%d:%i" /proc/self/fd/6)" = "$2" ] &&
            exec 5<"$4" && [ "$4" -ef /proc/self/fd/5 ] &&
            [ "$(stat -Lc "%d:%i" /proc/self/fd/5)" = "$5" ] || exit 76
        exec /proc/self/fd/5 "$3" "$6" "$7" "$8" >/proc/self/fd/6''')
    argument = re.search(r'(?m)^    \' weread-load-read "\$stage/input\.tgz" "\$receive_id" \$\(\([1-9][0-9]{0,8}\+1\)\)\); then$', text)
    if not argument:
        raise ValueError('Unrecognized frozen archive length argument')
    old = argument[0]
    text = replace(text, old, old.removesuffix('); then') +
                   ' "$receive_reader" "$receive_reader_id" "$label" "$receive_records" "$receive_records_id"); then')
    text = replace(text, '    case "$receive_status" in\n        0) return 0;;',
                   '    receive_joined=1\n    case "$receive_status" in\n        0) return 0;;')
    text = replace(text, '    install_reason=install_receive_failed\n', '''    install_reason=install_receive_reader
    prepare_receive_reader || return 1
    install_reason=install_receive_failed
''')
    return replace(text, '        install 2>/dev/null || load_refused "${bind_reason:-$install_reason}"\n        reply installed; load_terminal;;', '''        if install 2>/dev/null; then
            report_receive_progress
            reply installed; load_terminal
        else
            report_receive_progress
            load_refused "${bind_reason:-$install_reason}"
        fi;;''')


def device_program(base, reader):
    """Extend a fresh private trial program, not its inputs or ordinary launcher."""
    text = receive_program(base.decode('ascii'), reader)
    text = replace(text, ACK, '*) [ "${2-}" = read_only ] || write_tag stock-ack-seen || return 1;;')
    text = replace(text, SUBMIT, BIND + SUBMIT)
    branch = f'''case "$mode" in
    {PORTRAIT_READ}) portrait_diagnostic_receipt 2>/dev/null || fail diagnostic_snapshot;;
    trace) exit 64;;
    admit)'''
    program = replace(text, DISPATCH, source(HERE/'collect.sh') + '\n' + branch).encode('ascii')
    if len(program) > MAX_HEADER:
        raise ValueError('Embedded diagnostic receiver would exceed the frozen shell argument limit')
    return program


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
    """Use a verifier for the exact stock program, archive and embedded reader."""
    def verify(work, plan, *, recovery):
        config, base, reader = verify_trial(work, plan, recovery=recovery)
        if config.get('purpose') != PORTRAIT_DIAGNOSTIC_PURPOSE:
            raise Refused('invalid_diagnostic_scope')
        if not isinstance(reader, bytes) or not 0 < len(reader) <= 64 * 1024:
            raise ValueError('Expected a bounded, freshly verified archive reader')
        check_arm_elf(reader, static=True)
        path = plan/'device.sh'
        program = device_program(base, reader)
        if (path.is_symlink() or not path.is_file() or path.stat().st_size != len(program)
                or path.read_bytes() != program):
            raise Refused('invalid_diagnostic_device_program')
        return config, program

    return run_visit(args, diagnostic_verifier=verify, collect_diagnostic=collect)
