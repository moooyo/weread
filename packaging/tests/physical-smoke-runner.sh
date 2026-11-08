#!/bin/sh
# Keep initialization bounded independently of the application's event-loop timer.
set -eu
umask 077

WEREAD_HOME=$(CDPATH= cd "$(dirname "$0")" && pwd)
export WEREAD_HOME
DISPLAY=${DISPLAY:-:0.0}
export DISPLAY
test_case=$(cat "$WEREAD_HOME/data/logs/test-case" 2>/dev/null || printf '%s' smoke)
case "$test_case" in
    smoke) watchdog_limit=120 ;;
    interactive|sleep) watchdog_limit=420 ;;
    experience) watchdog_limit=900 ;;
    *) exit 2 ;;
esac
LOG_DIR=$WEREAD_HOME/testlogs/$test_case-$$
mkdir -p "$LOG_DIR"
printf '%s\n' "$LOG_DIR" > "$WEREAD_HOME/testlogs/latest"

[ ! -e /tmp/weread-launcher.lock ] || {
    printf '%s\n' 'An existing display lock must be investigated before testing.' >&2
    exit 1
}
if pidof jobman >/dev/null 2>&1; then
    printf '%s\n' 'Exit the Jobman GUI before testing the display and touch input.' >&2
    exit 1
fi

collect_state() {
    cat /etc/prettyversion.txt
    uname -m
    uname -r
    printf 'DISPLAY=%s\n' "$DISPLAY"
    status statusbar 2>&1 || :
    for wm_pid in $(pidof awesome 2>/dev/null || :); do
        printf 'awesome pid=%s ' "$wm_pid"
        sed -n 's/^State:[[:space:]]*//p' "/proc/$wm_pid/status" 2>/dev/null || :
    done
    if [ -e /tmp/weread-launcher.lock ]; then
        printf '%s\n' 'display_lock=present'
    else
        printf '%s\n' 'display_lock=absent'
    fi
}

collect_state > "$LOG_DIR/before.txt"
if [ -r "$WEREAD_HOME/data/logs/application.log" ]; then
    cp "$WEREAD_HOME/data/logs/application.log" "$LOG_DIR/previous-application.log"
fi
: > "$WEREAD_HOME/data/logs/application.log"
sh "$WEREAD_HOME/launch.sh" --detached > "$LOG_DIR/launcher.log" 2>&1 &
launcher_pid=$!
printf '%s\n' "$launcher_pid" > "$LOG_DIR/launcher.pid"

owns_launcher() {
    if kill -0 "$launcher_pid" 2>/dev/null; then
        command_line=$(tr '\000' ' ' < "/proc/$launcher_pid/cmdline" 2>/dev/null || :)
        case "$command_line" in
            *"$WEREAD_HOME/launch.sh --detached"*)
                return 0
                ;;
        esac
    fi
    return 1
}

terminate_owned_launcher() {
    # The runner and watchdog coordinate one request. Repeated TERM must not
    # interrupt the launcher's already-running restoration sequence.
    if owns_launcher && mkdir "$LOG_DIR/stop-requested" 2>/dev/null; then
        kill -TERM "$launcher_pid" 2>/dev/null || :
    fi
}

trap terminate_owned_launcher HUP INT TERM
(
    trap - HUP INT TERM
    remaining=$watchdog_limit
    while kill -0 "$launcher_pid" 2>/dev/null && [ "$remaining" -gt 0 ]; do
        sleep 1
        remaining=$((remaining - 1))
    done
    if kill -0 "$launcher_pid" 2>/dev/null; then
        printf '%s\n' 'The initialization watchdog requested a managed shutdown.' > "$LOG_DIR/watchdog.txt"
        terminate_owned_launcher
    fi
) &
watchdog_pid=$!

while :; do
    if wait "$launcher_pid"; then
        result=0
    else
        result=$?
    fi
    # A trapped signal can interrupt wait before the launcher has restored UI.
    # Keep waiting for this exact child, rather than snapshotting early.
    if ! owns_launcher; then break; fi
done
wait "$watchdog_pid" 2>/dev/null || :
collect_state > "$LOG_DIR/after.txt"
cp "$WEREAD_HOME/data/logs/application.log" "$LOG_DIR/application.log"
printf '%s\n' "$result" > "$LOG_DIR/exit-code.txt"
printf 'TEST_EXIT_CODE=%s\n' "$result"
cat "$LOG_DIR/before.txt" "$LOG_DIR/after.txt"
exit "$result"
