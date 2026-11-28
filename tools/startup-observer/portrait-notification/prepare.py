"""Generate private diagnostic sources from the retained v6 trial inputs."""

import argparse
import os
from pathlib import Path


HERE = Path(__file__).resolve().parent
DISPLAY = 'weread_handoff_mark cover_ready\ntake_display\nweread_handoff_mark display_ready\n'


def replace(text, old, new):
    if text.count(old) != 1:
        raise ValueError(f'Unrecognized candidate source boundary: {old.splitlines()[0]}')
    return text.replace(old, new, 1)


def source(path):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 256 * 1024:
        raise ValueError(f'Expected an ordinary, bounded source: {path.name}')
    return path.read_text('ascii')


def helper(text):
    first = 'static int completion_present(int directory) {'
    last = 'int main(int argc, char **argv) {'
    anchors = (first, last, 'static int arm_waiter_watchdog(void)', 'SYS_clock_gettime, CLOCK_MONOTONIC')
    if any(text.count(anchor) != 1 for anchor in anchors):
        raise ValueError('Expected the frozen monotonic notification helper with its independent watchdog')
    start = text.index(first); end = text.index(last, start)
    text = text[:start] + source(HERE/'diagnostic.inc') + '\n' + text[end:]
    text = replace(text, '''    int wait = argc == 3 && !strcmp(argv[1], "--wait-portrait-completion");
    if (!wait && (argc != 2 || strcmp(argv[1], "--session"))) return 64;''', '''    int wait = argc == 4 && !strcmp(argv[1], "--wait-portrait-completion");
    int publish = argc == 3 && !strcmp(argv[1], "--publish-portrait-completion");
    if (!wait && !publish && (argc != 2 || strcmp(argv[1], "--session"))) return 64;''')
    return replace(text, '    if (wait) return wait_portrait_completion(argv[2]);', '''    if (publish) return publish_portrait_completion(argv[2]);
    if (wait) {
        unsigned int slot;
        if (notice_slot(argv[3], &slot)) return 64;
        return wait_portrait_completion(argv[2], slot);
    }''')


def launcher(text):
    patches = (
        ('--wait-portrait-completion "$LOCK_DIR" \\', '--wait-portrait-completion "$LOCK_DIR" "$1" \\'),
        ("    portrait_publish_file=''\n", "    portrait_publish_file=''\n    portrait_trace_query=''\n"),
        ('''        portrait_publish_file=$weread_handoff_stage/portrait-publication
    fi''', '''        portrait_publish_file=$weread_handoff_stage/portrait-publication
        portrait_trace=$LOCK_DIR/portrait-notification.diag
        if [ ! -e "$portrait_trace" ] && [ ! -L "$portrait_trace" ] &&
            (umask 077; set -C; : >"$portrait_trace") 2>/dev/null; then
            portrait_trace_query=$APP_DIR/weread-native-query
        fi
    fi'''),
        ('''        portrait_publish_file=$6
        shift 6''', '''        portrait_publish_file=$6
        portrait_trace_query=$7
        shift 7'''),
        ('''        mv -f "$completed.pending" "$completed"
        portrait_mv_status=$?''', '''        portrait_mv_status=0
        if [ -n "$portrait_trace_query" ]; then
            "$portrait_trace_query" --publish-portrait-completion "${completed%/*}" || portrait_mv_status=$?
        else
            mv -f "$completed.pending" "$completed" || portrait_mv_status=$?
        fi'''),
        ('"$NATIVE_COMMAND_FILE_BLOCKS" "$portrait_publish_file" "$@"', '"$NATIVE_COMMAND_FILE_BLOCKS" "$portrait_publish_file" "$portrait_trace_query" "$@"'),
        ('''                native_theme_wait_status=0
                if [ "$weread_portrait_shell_phase" = permitted ]; then
                    weread_notify_waits=$((weread_notify_waits + 1))
                fi
                wait_portrait_completion_hint || native_theme_wait_status=$?''', '''                native_theme_wait_status=0
                portrait_trace_slot=0
                if [ "$weread_portrait_shell_phase" = permitted ]; then
                    weread_notify_waits=$((weread_notify_waits + 1))
                    if [ -n "$portrait_trace_query" ]; then portrait_trace_slot=$weread_notify_waits; fi
                fi
                wait_portrait_completion_hint "$portrait_trace_slot" || native_theme_wait_status=$?'''),
        ('    if ! rm -f "$LOCK_DIR/pid" "$LOCK_DIR/power-events"',
         '    if ! rm -f "$LOCK_DIR/portrait-notification.diag" "$LOCK_DIR/pid" "$LOCK_DIR/power-events"'),
    )
    for old, new in patches:
        text = replace(text, old, new)
    return text


def prepare(helper_source, arm_a, arm_b, output):
    a, b = source(arm_a), source(arm_b)
    display_b = DISPLAY.replace('take_display\n', 'take_display startup\n')
    if replace(a, DISPLAY, display_b) != b:
        raise ValueError('Expected both frozen arms to differ only in the first display-takeover flag')
    patched_a = launcher(a)
    patched_b = launcher(b)
    if replace(patched_a, DISPLAY, display_b) != patched_b:
        raise ValueError('Diagnostic instrumentation changed the A/B arm difference')
    material = {'native_query.c':helper(source(helper_source)), 'A.sh':patched_a, 'B.sh':patched_b,
                'collect.sh':source(HERE/'collect.sh')}
    output.mkdir(mode=0o700)
    for name,text in material.items():
        with (output/name).open('x',encoding='ascii') as f:
            f.write(text)
        (output/name).chmod(0o400)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('helper_source','arm_a','arm_b','output'):
        p.add_argument(name,type=Path)
    args = p.parse_args()
    os.umask(0o077)
    prepare(args.helper_source,args.arm_a,args.arm_b,args.output)
    print('Diagnostic sources only; no physical roster, production package, or device operation.')


if __name__ == '__main__':
    main()
