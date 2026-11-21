"""Build the follow-up Reader sampler without changing the originally qualified observer."""

from pathlib import Path
from textwrap import indent


HERE = Path(__file__).resolve().parent


def render_observer():
    source = (HERE / 'assisted-wake-observe.sh').read_text(encoding='ascii')
    signature = 'wake_observe() (\n    [ "$#" = 1 ] || exit 1\n'
    condition = '    case "$wake_phase:$wake_anchor:$4" in\n'
    recheck = '    wake_guard "$wake_phase" || exit "$?"\n'
    if not all(source.count(boundary) == 1 for boundary in (signature, condition, recheck)):
        raise ValueError('Review the frozen observation boundaries before composing a new sampler')
    guarded, remainder = source.split(signature)
    reading, predicate = remainder.split(condition)
    predicate, confirmation = predicate.split(recheck)
    if not predicate.endswith('    esac\n') or not source.endswith('\n)\n'):
        raise ValueError('Unexpected frozen sample predicate or function ending')
    body = (guarded + 'wake_emit_observation() (\n    [ "$#" = 2 ] || exit 1\n'
            '    wake_filter=$1; shift\n    case "$wake_filter" in strict|actual) ;; *) exit 1;; esac\n' + reading +
            '    if [ "$wake_filter" = strict ] || [ "$wake_phase" = pre ]; then\n' + indent(condition + predicate, '    ') +
            '    fi\n' + recheck + confirmation + '\nwake_observe() { wake_emit_observation strict "$@"; }\n'
            'wake_observe_any() { wake_emit_observation actual "$@"; }\n')
    return body.encode('ascii')
