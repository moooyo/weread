#!/bin/sh
# Name: WeRead for Kindle
# The compatibility allowlist is intentionally empty until device acceptance.

set -eu
umask 077

WEREAD_HOME=${WEREAD_HOME:-/mnt/us/weread}
DISPLAY=${DISPLAY:-:0.0}
export DISPLAY
APP_DIR=$WEREAD_HOME/app
DATA_DIR=$WEREAD_HOME/data
PROFILE_FILE=$DATA_DIR/device-profile
ALLOWLIST_FILE=$DATA_DIR/device-allowlist
EXPERIMENTAL_DISPLAY_FILE=$DATA_DIR/experimental-display
LOCK_DIR=/tmp/weread-launcher.lock
LOCK_OWNED=0
MANAGED_LOGS_ACTIVE=0
MANAGED_LOGS_DISABLED=0
LAUNCHER_LOG_LIMIT=65536

launcher_log() {
    if [ "$MANAGED_LOGS_ACTIVE" -eq 1 ]; then
        [ "$MANAGED_LOGS_DISABLED" -eq 0 ] || return 0
        if ! launcher_log_size=$(wc -c 2>/dev/null <"$DATA_DIR/logs/launcher.log"); then
            MANAGED_LOGS_DISABLED=1
            return 0
        fi
        launcher_log_size=${launcher_log_size#"${launcher_log_size%%[![:space:]]*}"}
        launcher_log_size=${launcher_log_size%"${launcher_log_size##*[![:space:]]}"}
        case "$launcher_log_size" in *[!0-9]*|'') MANAGED_LOGS_DISABLED=1; return 0 ;; esac
        if [ "$launcher_log_size" -ge "$LAUNCHER_LOG_LIMIT" ]; then
            # Keep the inode open on stdout/stderr. Renaming it would leave
            # the supervisor writing an unlinked or archived, unbounded file.
            if ! { : >"$DATA_DIR/logs/launcher.log"; } 2>/dev/null; then
                MANAGED_LOGS_DISABLED=1
                return 0
            fi
        fi
    fi
    printf "$@" >&2 || :
}

fail() {
    launcher_log '%s\n' "weread: $*"
    exit 1
}

case "$WEREAD_HOME" in
    /*) ;;
    *) fail 'WEREAD_HOME must be an absolute path.' ;;
esac

[ -x "$APP_DIR/weread" ] || fail 'The application executable is missing.'
[ -r "$PROFILE_FILE" ] || fail 'Configure data/device-profile before launching.'
[ -r "$ALLOWLIST_FILE" ] || fail 'Configure data/device-allowlist before launching.'

PROFILE=$(sed -n '1{s/\r$//;p;}' "$PROFILE_FILE")
case "$PROFILE" in
    pw5|pw6|scribe|scribe2025) ;;
    *) fail 'Unknown configured device profile.' ;;
esac

[ -r /etc/prettyversion.txt ] || fail 'Kindle firmware information is unavailable.'
FIRMWARE=$(sed -n 's/^Kindle[[:space:]]*\([0-9][0-9.]*\).*$/\1/p' /etc/prettyversion.txt | head -n 1)
[ -n "$FIRMWARE" ] || fail 'The firmware version could not be determined.'

# Match two literal fields; do not evaluate configuration as shell code.
if ! awk -v profile="$PROFILE" -v firmware="$FIRMWARE" '
    { sub(/\r$/, "") }
    /^[[:space:]]*#/ { next }
    NF == 2 && $1 == profile && $2 == firmware { found = 1 }
    END { exit !found }
' "$ALLOWLIST_FILE"; then
    fail "The configured profile/firmware combination has not been accepted: $PROFILE $FIRMWARE."
fi

WEREAD_APPROVED_PROFILE=$PROFILE
WEREAD_APPROVED_FIRMWARE=$FIRMWARE
export WEREAD_APPROVED_PROFILE WEREAD_APPROVED_FIRMWARE

# Experimental display geometry is a named, firmware-bound specification.
# An inherited environment value must not bypass the operator-local file.
unset WEREAD_EXPERIMENTAL_DISPLAY
if [ -e "$EXPERIMENTAL_DISPLAY_FILE" ]; then
    [ -r "$EXPERIMENTAL_DISPLAY_FILE" ] || fail 'The experimental display configuration is unreadable.'
    EXPERIMENTAL_DISPLAY=$(sed 's/\r$//' "$EXPERIMENTAL_DISPLAY_FILE")
    case "$EXPERIMENTAL_DISPLAY" in
        pw6-5201-emulator)
            [ "$PROFILE" = pw6 ] && [ "$FIRMWARE" = 5.20.1 ] ||
                fail 'The experimental display requires pw6 firmware 5.20.1.'
            ;;
        *) fail 'Unknown experimental display specification.' ;;
    esac
    WEREAD_EXPERIMENTAL_DISPLAY=$EXPERIMENTAL_DISPLAY
    export WEREAD_EXPERIMENTAL_DISPLAY
fi

[ -c /dev/fb0 ] || fail 'The framebuffer is unavailable.'
[ -r /dev/fb0 ] && [ -w /dev/fb0 ] || fail 'Framebuffer read/write access is required.'

mkdir -p "$DATA_DIR/logs" "$DATA_DIR/fonts"

# Return control to a document launcher before taking over the display.
if [ "${1:-}" != '--detached' ]; then
    SCRIPT_PATH=$(CDPATH= cd "$(dirname "$0")" && pwd)/$(basename "$0")
    export WEREAD_HOME
    if command -v setsid >/dev/null 2>&1; then
        setsid sh "$SCRIPT_PATH" --detached --managed-logs >"$DATA_DIR/logs/last-launch.log" 2>&1 </dev/null &
    else
        nohup sh "$SCRIPT_PATH" --detached --managed-logs >"$DATA_DIR/logs/last-launch.log" 2>&1 </dev/null &
    fi
    exit 0
fi

APP_RUNNER_PID=''
APP_RUNNER_BIRTH=''
APP_CHILD_PID=''
APP_CHILD_BIRTH=''
APP_GENERATION=0
APP_PHASE=none
APP_STATUS=0
APP_RESUME_INTENT=0
APP_NATIVE_RETURN_INTENT=0
APP_NATIVE_RETURN_RESULT=closed
SLEEP_RESUME_INTENT=0
SLEEP_NATIVE_RETURN_INTENT=0
SLEEP_NATIVE_RETURN_RESULT=closed
LAUNCH_TOKEN=''
EVENT_SOURCE_PID=''
EVENT_SOURCE_BIRTH=''
EVENT_SOURCE_REAPED=0
EVENT_SOURCE_CHILD_PID=''
EVENT_SOURCE_CHILD_BIRTH=''
POWER_OWNER_BIRTH=''
LAUNCH_REGISTRATION_ACTIVE=0
LAUNCH_STARTUP_SIGNAL=0
EVENT_FD_OPEN=0
CLEANUP_RUNNING=0
CLEANUP_FAILED=0
WAKE_PROBE_PID=''
WAKE_PROBE_BIRTH=''
WAKE_PROBE_CYCLE=0
WAKE_PROBE_REAPED=0
WAKE_PROBE_CHILD_PID=''
WAKE_PROBE_CHILD_BIRTH=''
SLEEP_GENERATION=0
MANAGED_SLEEP_CYCLE=0
MANAGED_FROZEN=0
WAKE_PENDING=0
WAKE_PROBES_REMAINING=0
PAUSED_WM_IDENTITIES=''
WM_STOP_PENDING_IDENTITY=''
STATUSBAR_STOPPED=0
STATUSBAR_RESTORE_REQUIRED=0
DISPLAY_OWNED=0
FRAME_GUARD_PID=''
FRAME_GUARD_BIRTH=''
FRAME_GUARD_WINDOW=''
FRAME_GUARD_SEQUENCE=0
FRAME_GUARD_FD_OPEN=0
FRAME_GUARD_LIFETIME_FD_OPEN=0
FRAME_GUARD_RELAY_PID=''
FRAME_GUARD_RELAY_BIRTH=''
NATIVE_EPDC_MODE=''
NATIVE_PORTRAIT_CALIBRATION=''
NATIVE_THEME_CAPTURE_STATUS=unknown
NATIVE_DARK_EXIT_ALLOWED=0
NATIVE_THEME_COMMAND_PID=''
NATIVE_THEME_COMMAND_BIRTH=''
NATIVE_THEME_COMMAND_SEQUENCE=0
NATIVE_THEME_REQUEST_SEQUENCE=0
NATIVE_THEME_REQUEST_TARGET=''
NATIVE_THEME_IN_PROGRESS=0
NATIVE_THEME_STAGE=idle
NATIVE_THEME_TIMEOUT_TICKS=30
NATIVE_COMMAND_EXIT_STATUS=''
NATIVE_COMMAND_FAILURE=''
NATIVE_QUERY_OUTPUT_LIMIT=4096
NATIVE_WINDOW_OUTPUT_LIMIT=65536
NATIVE_COMMAND_FILE_BLOCKS=256
NATIVE_SETTINGS_SUPPORTED=0
NATIVE_OBSERVER_PID=''
NATIVE_GENERATION=0
NATIVE_SLEEPING=0
NATIVE_RETURN_PENDING=''
NATIVE_KIND=''
NATIVE_BASELINE_APP=''
NATIVE_BASELINE_VIEW=''
NATIVE_POWER_FAILURES=0

process_birth() {
    # The comm field may contain spaces or parentheses. Field 22 is the
    # twentieth field after the final closing parenthesis, including state.
    process_stat=$(cat "/proc/$1/stat" 2>/dev/null || :)
    process_tail=${process_stat##*) }
    [ "$process_tail" != "$process_stat" ] || return 1
    set -- $process_tail
    [ "$#" -ge 20 ] || return 1
    shift 19
    case "$1" in *[!0-9]*|'') return 1 ;; esac
    printf '%s\n' "$1"
}

process_parent() {
    sed -n 's/^PPid:[[:space:]]*//p' "/proc/$1/status" 2>/dev/null
}

process_matches() {
    case "$1:$2:$3" in *[!0-9:]*|::*|:*|*:) return 1 ;; esac
    [ "$(process_birth "$1" 2>/dev/null || :)" = "$2" ] &&
        [ "$(process_parent "$1" 2>/dev/null || :)" = "$3" ]
}

application_command_matches() {
    [ -r "/proc/$1/cmdline" ] &&
        tr '\000' '\n' <"/proc/$1/cmdline" |
        grep -F -x -- "$APP_DIR/weread" >/dev/null
}

signal_owned_process() {
    process_matches "$1" "$2" "$3" || return 1
    kill "-$4" "$1" 2>/dev/null
}

signal_original_application() {
    [ -n "$APP_CHILD_PID" ] && [ -n "$APP_CHILD_BIRTH" ] || return 1
    application_command_matches "$APP_CHILD_PID" || return 1
    signal_owned_process "$APP_CHILD_PID" "$APP_CHILD_BIRTH" "$APP_RUNNER_PID" "$1"
}

captured_orphan_is_alive() {
    # Only the immutable identity already captured from this runner is
    # authoritative here. Mutable application-pid/birth records are ignored.
    [ -n "$APP_CHILD_PID" ] && [ -n "$APP_CHILD_BIRTH" ] &&
        [ -n "$APP_RUNNER_PID" ] && [ -n "$APP_RUNNER_BIRTH" ] || return 1
    process_matches "$APP_CHILD_PID" "$APP_CHILD_BIRTH" 1 &&
        application_command_matches "$APP_CHILD_PID" || return 1
    case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$APP_CHILD_PID/status" 2>/dev/null || :)" in
        Z*|X*|'') return 1 ;;
    esac
    if process_matches "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$"; then
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$APP_RUNNER_PID/status" 2>/dev/null || :)" in
            Z*|X*|'') ;;
            *) return 1 ;;
        esac
    fi
}

signal_captured_orphan() {
    # Reparenting is a cleanup exception, never permission to suspend or
    # resume an application after its original runner has been lost.
    case "$1" in TERM|CONT|KILL) ;; *) return 1 ;; esac
    captured_orphan_is_alive || return 1
    signal_owned_process "$APP_CHILD_PID" "$APP_CHILD_BIRTH" 1 "$1"
}

cleanup_captured_orphan() {
    captured_orphan_is_alive || return 0
    signal_captured_orphan TERM || :
    signal_captured_orphan CONT || :
    orphan_cleanup_checks=10
    while captured_orphan_is_alive && [ "$orphan_cleanup_checks" -gt 0 ]; do
        sleep 1
        orphan_cleanup_checks=$((orphan_cleanup_checks - 1))
    done
    if captured_orphan_is_alive; then
        signal_captured_orphan KILL || :
    fi
}

capture_application_identity() {
    process_matches "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$" || return 1
    process_matches "$1" "$2" "$APP_RUNNER_PID" || return 1
    application_command_matches "$1" || return 1
    if [ -n "$APP_CHILD_PID" ]; then
        [ "$APP_CHILD_PID:$APP_CHILD_BIRTH" = "$1:$2" ] || return 1
    fi
    APP_CHILD_PID=$1
    APP_CHILD_BIRTH=$2
    printf '%s:%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$APP_CHILD_PID" "$APP_CHILD_BIRTH" >"$LOCK_DIR/application-owned"
}

capture_runner_identity() {
    [ -n "$APP_RUNNER_PID" ] || return 1
    [ -z "$APP_RUNNER_BIRTH" ] || return 0
    [ "$(process_parent "$APP_RUNNER_PID" 2>/dev/null || :)" = "$$" ] || return 1
    # A supervisor signal can interrupt the first birth read immediately
    # after fork. Recover it only for this launcher's own shell job.
    [ -r "/proc/$APP_RUNNER_PID/cmdline" ] &&
        tr '\000' '\n' <"/proc/$APP_RUNNER_PID/cmdline" |
        grep -F -x -- "$0" >/dev/null || return 1
    capture_wrapper_child_birth "$APP_RUNNER_PID" "$$" 2>/dev/null || return 1
    APP_RUNNER_BIRTH=$WRAPPER_CHILD_BIRTH
    [ -n "$APP_RUNNER_BIRTH" ]
}

managed_application_is_current() {
    [ -n "$APP_CHILD_PID" ] && [ -n "$APP_CHILD_BIRTH" ] || return 1
    [ "$(cat "$LOCK_DIR/pid" 2>/dev/null || :)" = "$$" ] &&
        [ "$(cat "$LOCK_DIR/nonce" 2>/dev/null || :)" = "$LAUNCH_TOKEN" ] &&
        [ "$(cat "$LOCK_DIR/generation" 2>/dev/null || :)" = "$APP_GENERATION:$LAUNCH_TOKEN" ] &&
        [ "$(cat "$LOCK_DIR/application-pid" 2>/dev/null || :)" = "$APP_CHILD_PID" ] &&
        [ "$(cat "$LOCK_DIR/application-birth" 2>/dev/null || :)" = "$APP_CHILD_BIRTH" ] &&
        process_matches "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$" &&
        process_matches "$APP_CHILD_PID" "$APP_CHILD_BIRTH" "$APP_RUNNER_PID" &&
        application_command_matches "$APP_CHILD_PID"
}

signal_managed_application() {
    managed_application_is_current || return 1
    signal_original_application "$1"
}

managed_marker_matches() {
    [ "$(cat "$LOCK_DIR/$1" 2>/dev/null || :)" = "$APP_GENERATION:$LAUNCH_TOKEN:$MANAGED_SLEEP_CYCLE" ]
}

write_managed_request() {
    printf '%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$MANAGED_SLEEP_CYCLE" >"$LOCK_DIR/$1.pending" &&
        mv -f "$LOCK_DIR/$1.pending" "$LOCK_DIR/$1"
}

managed_wait_tick() {
    if command -v usleep >/dev/null 2>&1; then
        usleep 100000
    else
        sleep 1
    fi
}

managed_wait_budget() {
    if command -v usleep >/dev/null 2>&1; then
        printf '%s\n' "$((10 * $1))"
    else
        printf '%s\n' "$1"
    fi
}

# Asynchronous shell functions retain the supervisor's $$ on dash/ash. Read
# procfs with a builtin in the wrapper itself rather than a command substitution.
capture_wrapper_identity() {
    IFS= read -r wrapper_identity_stat </proc/self/stat || return 1
    WRAPPER_PID=${wrapper_identity_stat%% *}
    wrapper_identity_tail=${wrapper_identity_stat##*) }
    set -- $wrapper_identity_tail
    [ "$#" -ge 20 ] || return 1
    shift 19
    WRAPPER_BIRTH=$1
    case "$WRAPPER_PID:$WRAPPER_BIRTH" in *[!0-9:]*|:*|*:) return 1 ;; esac
}

read_wrapper_child_birth() {
    WRAPPER_CHILD_BIRTH=''
    IFS= read -r wrapper_child_stat <"/proc/$1/stat" || return 1
    [ "${wrapper_child_stat%% *}" = "$1" ] || return 1
    wrapper_child_tail=${wrapper_child_stat##*) }
    set -- $wrapper_child_tail
    [ "$#" -ge 20 ] && [ "$2" = "$wrapper_capture_parent" ] || return 1
    shift 19
    case "$1" in *[!0-9]*|'') return 1 ;; esac
    WRAPPER_CHILD_BIRTH=$1
}

capture_wrapper_child_birth() {
    # Keep the original shell job and ancestry in memory across a transient
    # procfs read failure. Never replace them with a mutable published record.
    wrapper_capture_pid=$1
    wrapper_capture_parent=${2:-$WRAPPER_PID}
    wrapper_capture_checks=$(managed_wait_budget 1)
    while :; do
        if read_wrapper_child_birth "$wrapper_capture_pid"; then
            return 0
        fi
        [ -d "/proc/$wrapper_capture_pid" ] || return 1
        [ "$wrapper_capture_checks" -gt 0 ] || return 1
        managed_wait_tick || :
        wrapper_capture_checks=$((wrapper_capture_checks - 1))
    done
}

owned_child_is_running() {
    process_matches "$1" "$2" "$3" || return 1
    case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$1/status" 2>/dev/null || :)" in
        Z*|X*|'') return 1 ;;
    esac
}

owned_child_has_exited() {
    case "$1" in *[!0-9]*|'') return 1 ;; esac
    # An absent original shell job cannot block wait. A still-present PID must
    # match its captured birth and parent before its exited state is trusted.
    [ ! -d "/proc/$1" ] && return 0
    process_matches "$1" "$2" "$3" || return 1
    case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$1/status" 2>/dev/null || :)" in
        Z*|X*) return 0 ;;
        *) return 1 ;;
    esac
}

# The still-live wrapper keeps the child's identity in memory even if publishing
# a PID file fails. Never wait for a live or unproven process without a deadline.
cleanup_wrapper_child() {
    wrapper_cleanup_pid=$1
    wrapper_cleanup_birth=$2
    wrapper_cleanup_parent=$3
    if [ -n "$wrapper_cleanup_pid" ] && [ -z "$wrapper_cleanup_birth" ]; then
        if capture_wrapper_child_birth "$wrapper_cleanup_pid" "$wrapper_cleanup_parent" 2>/dev/null; then
            wrapper_cleanup_birth=$WRAPPER_CHILD_BIRTH
        fi
    fi
    if owned_child_is_running "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent"; then
        signal_owned_process "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent" TERM || :
        signal_owned_process "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent" CONT || :
        wrapper_cleanup_checks=$(managed_wait_budget 2)
        while owned_child_is_running "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent" &&
            [ "$wrapper_cleanup_checks" -gt 0 ]; do
            managed_wait_tick || :
            wrapper_cleanup_checks=$((wrapper_cleanup_checks - 1))
        done
        if owned_child_is_running "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent"; then
            signal_owned_process "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent" KILL || :
        fi
        wrapper_cleanup_checks=$(managed_wait_budget 1)
        while owned_child_is_running "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent" &&
            [ "$wrapper_cleanup_checks" -gt 0 ]; do
            managed_wait_tick || :
            wrapper_cleanup_checks=$((wrapper_cleanup_checks - 1))
        done
    fi
    [ -n "$wrapper_cleanup_pid" ] || return 0
    if owned_child_has_exited "$wrapper_cleanup_pid" "$wrapper_cleanup_birth" "$wrapper_cleanup_parent"; then
        wait "$wrapper_cleanup_pid" 2>/dev/null || :
        return 0
    fi
    launcher_log '%s\n' 'weread: Child cleanup could not confirm an exited owned process; blocking wait was refused.'
    return 1
}

frame_guard_command_matches() {
    [ -r "/proc/$1/cmdline" ] &&
        tr '\000' '\n' <"/proc/$1/cmdline" |
        grep -F -x -- "$APP_DIR/weread-frame-guard" >/dev/null
}

frame_guard_is_current() {
    [ -n "$FRAME_GUARD_PID" ] && [ -n "$FRAME_GUARD_BIRTH" ] &&
        [ "$FRAME_GUARD_FD_OPEN" -eq 1 ] &&
        owned_child_is_running "$FRAME_GUARD_PID" "$FRAME_GUARD_BIRTH" "$$" &&
        frame_guard_command_matches "$FRAME_GUARD_PID" &&
        [ -n "$FRAME_GUARD_RELAY_PID" ] && [ -n "$FRAME_GUARD_RELAY_BIRTH" ] &&
        owned_child_is_running "$FRAME_GUARD_RELAY_PID" "$FRAME_GUARD_RELAY_BIRTH" "$$" &&
        [ "$(cat "$LOCK_DIR/pid" 2>/dev/null || :)" = "$$" ] &&
        [ "$(cat "$LOCK_DIR/nonce" 2>/dev/null || :)" = "$LAUNCH_TOKEN" ] || return 1
    case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$FRAME_GUARD_PID/status" 2>/dev/null || :)" in
        T*|t*|Z*|X*|'') return 1 ;;
    esac
}

frame_guard_ready_matches() {
    [ -f "$LOCK_DIR/frame-guard-ready" ] && [ ! -L "$LOCK_DIR/frame-guard-ready" ] || return 1
    guard_ready_bytes=$(wc -c <"$LOCK_DIR/frame-guard-ready" 2>/dev/null) || return 1
    guard_ready_bytes=${guard_ready_bytes#"${guard_ready_bytes%%[![:space:]]*}"}
    guard_ready_bytes=${guard_ready_bytes%"${guard_ready_bytes##*[![:space:]]}"}
    case "$guard_ready_bytes" in *[!0-9]*|'') return 1 ;; esac
    [ "$guard_ready_bytes" -gt 0 ] && [ "$guard_ready_bytes" -le 128 ] || return 1
    guard_ready_record=$(cat "$LOCK_DIR/frame-guard-ready" 2>/dev/null) || return 1
    [ "$guard_ready_record" = "$1:$2:$LAUNCH_TOKEN:$FRAME_GUARD_PID:$FRAME_GUARD_BIRTH" ]
}

capture_frame_guard_window() {
    frame_guard_is_current || return 1
    [ -f "$LOCK_DIR/frame-guard-window" ] && [ ! -L "$LOCK_DIR/frame-guard-window" ] || return 1
    guard_window_bytes=$(wc -c <"$LOCK_DIR/frame-guard-window" 2>/dev/null) || return 1
    guard_window_bytes=${guard_window_bytes#"${guard_window_bytes%%[![:space:]]*}"}
    guard_window_bytes=${guard_window_bytes%"${guard_window_bytes##*[![:space:]]}"}
    case "$guard_window_bytes" in *[!0-9]*|'') return 1 ;; esac
    [ "$guard_window_bytes" -gt 0 ] && [ "$guard_window_bytes" -le 128 ] || return 1
    IFS=: read -r guard_window_token guard_window_pid guard_window_birth guard_window_id guard_window_extra <"$LOCK_DIR/frame-guard-window" || return 1
    [ "$guard_window_token:$guard_window_pid:$guard_window_birth" = "$LAUNCH_TOKEN:$FRAME_GUARD_PID:$FRAME_GUARD_BIRTH" ] &&
        [ -z "$guard_window_extra" ] || return 1
    case "$guard_window_id" in 0x*) ;; *) return 1 ;; esac
    guard_window_hex=${guard_window_id#0x}
    case "$guard_window_hex" in *[!0-9a-f]*|'') return 1 ;; esac
    [ "${#guard_window_hex}" -le 8 ] && [ "$guard_window_hex" != 0 ] || return 1
    FRAME_GUARD_WINDOW=$guard_window_id
}

register_frame_guard() {
    [ -x "$APP_DIR/weread-frame-guard" ] && [ -n "$POWER_OWNER_BIRTH" ] || return 1
    rm -f "$LOCK_DIR/frame-guard-ready" "$LOCK_DIR/frame-guard-ready.pending" "$LOCK_DIR/frame-guard-window" "$LOCK_DIR/frame-guard-window.pending" || return 1
    mkfifo "$LOCK_DIR/frame-guard-commands" || return 1
    mkfifo "$LOCK_DIR/frame-guard-lifetime" || return 1
    { exec 6<>"$LOCK_DIR/frame-guard-commands"; } || return 1
    FRAME_GUARD_FD_OPEN=1
    # Inherit an already open descriptor into both children: neither can be
    # stranded in a FIFO open if this supervisor dies before registration.
    { exec 7<>"$LOCK_DIR/frame-guard-lifetime"; } || return 1
    FRAME_GUARD_LIFETIME_FD_OPEN=1
    FRAME_GUARD_SEQUENCE=0
    "$APP_DIR/weread-frame-guard" --context "$LOCK_DIR" --token "$LAUNCH_TOKEN" \
        --owner "$$" --owner-birth "$POWER_OWNER_BIRTH" --profile "$PROFILE" \
        --command-fd 6 >&7 7>&- 2>"$DATA_DIR/logs/frame-guard-bootstrap.log" </dev/null 3>&- &
    FRAME_GUARD_PID=$!
    FRAME_GUARD_BIRTH=''
    capture_wrapper_child_birth "$FRAME_GUARD_PID" "$$" 2>/dev/null || return 1
    FRAME_GUARD_BIRTH=$WRAPPER_CHILD_BIRTH
    # A passive, parent-protected reader reports even an uncatchable helper
    # exit. Redirection opens the read side before closing the inherited anchor.
    "$APP_DIR/weread-frame-guard" --context "$LOCK_DIR" --token "$LAUNCH_TOKEN" \
        --owner "$$" --owner-birth "$POWER_OWNER_BIRTH" --watch-helper "$FRAME_GUARD_PID" \
        --helper-birth "$FRAME_GUARD_BIRTH" --lifetime-fd 0 <"$LOCK_DIR/frame-guard-lifetime" 7>&- 6>&- 3>&- >/dev/null 2>"$DATA_DIR/logs/frame-guard-watcher.log" &
    FRAME_GUARD_RELAY_PID=$!
    FRAME_GUARD_RELAY_BIRTH=''
    capture_wrapper_child_birth "$FRAME_GUARD_RELAY_PID" "$$" 2>/dev/null || return 1
    FRAME_GUARD_RELAY_BIRTH=$WRAPPER_CHILD_BIRTH
    { exec 7>&-; } || return 1
    FRAME_GUARD_LIFETIME_FD_OPEN=0
    guard_start_checks=$(managed_wait_budget 5)
    while [ "$guard_start_checks" -gt 0 ]; do
        owned_child_is_running "$FRAME_GUARD_PID" "$FRAME_GUARD_BIRTH" "$$" || return 1
        if ! frame_guard_command_matches "$FRAME_GUARD_PID"; then
            managed_wait_tick || return 1
            guard_start_checks=$((guard_start_checks - 1))
            continue
        fi
        frame_guard_is_current || return 1
        if frame_guard_ready_matches 0 0; then
            capture_frame_guard_window || return 1
            return 0
        fi
        managed_wait_tick || return 1
        guard_start_checks=$((guard_start_checks - 1))
    done
    launcher_log '%s\n' 'weread: Native frame guard admission was not acknowledged.'
    return 1
}

start_frame_guard() {
    if [ -n "$FRAME_GUARD_PID" ]; then frame_guard_is_current; return $?; fi
    begin_owned_registration
    guard_start_status=0
    if register_frame_guard; then :; else guard_start_status=$?; fi
    finish_owned_registration
    return "$guard_start_status"
}

frame_guard_application_is_frozen() {
    managed_application_is_current && frame_guard_is_current &&
        [ "$DISPLAY_OWNED" -eq 1 ] && [ -z "$WM_STOP_PENDING_IDENTITY" ] || return 1
    case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$APP_CHILD_PID/status" 2>/dev/null || :)" in
        T*|t*) ;; *) return 1 ;;
    esac
    for guard_wm_identity in $PAUSED_WM_IDENTITIES; do
        guard_wm_pid=${guard_wm_identity%%:*}
        guard_wm_birth=${guard_wm_identity#*:}
        [ "$(process_birth "$guard_wm_pid" 2>/dev/null || :)" = "$guard_wm_birth" ] || return 1
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$guard_wm_pid/status" 2>/dev/null || :)" in
            T*|t*) ;; *) return 1 ;;
        esac
    done
    if [ "$NATIVE_THEME_IN_PROGRESS" -eq 1 ]; then native_theme_request_is_current || return 1; fi
}

request_frame_guard() {
    case "$1" in capture|align) ;; *) return 1 ;; esac
    frame_guard_application_is_frozen || return 1
    [ "$FRAME_GUARD_SEQUENCE" -lt 2147483647 ] || return 1
    FRAME_GUARD_SEQUENCE=$((FRAME_GUARD_SEQUENCE + 1))
    printf '%s:%s:%s:%s:%s:%s:%s\n' "$FRAME_GUARD_SEQUENCE" "$APP_GENERATION" "$LAUNCH_TOKEN" "$APP_CHILD_PID" "$APP_CHILD_BIRTH" "$APP_RUNNER_PID" "$1" >&6 || return 1
    guard_frame_checks=$(managed_wait_budget 5)
    while [ "$guard_frame_checks" -gt 0 ]; do
        frame_guard_application_is_frozen || return 1
        if frame_guard_ready_matches "$FRAME_GUARD_SEQUENCE" "$APP_GENERATION"; then return 0; fi
        managed_wait_tick || return 1
        guard_frame_checks=$((guard_frame_checks - 1))
    done
    launcher_log '%s\n' 'weread: Native frame guard update was not acknowledged.'
    return 1
}

stop_frame_guard() {
    guard_stop_status=0
    if [ "$FRAME_GUARD_LIFETIME_FD_OPEN" -eq 1 ]; then
        if { exec 7>&-; }; then FRAME_GUARD_LIFETIME_FD_OPEN=0; else guard_stop_status=1; fi
    fi
    if [ -n "$FRAME_GUARD_PID" ]; then
        if cleanup_wrapper_child "$FRAME_GUARD_PID" "$FRAME_GUARD_BIRTH" "$$"; then
            FRAME_GUARD_PID=''
            FRAME_GUARD_BIRTH=''
            FRAME_GUARD_WINDOW=''
            FRAME_GUARD_SEQUENCE=0
        else
            guard_stop_status=1
        fi
    fi
    if [ -n "$FRAME_GUARD_RELAY_PID" ]; then
        if cleanup_wrapper_child "$FRAME_GUARD_RELAY_PID" "$FRAME_GUARD_RELAY_BIRTH" "$$"; then
            FRAME_GUARD_RELAY_PID=''
            FRAME_GUARD_RELAY_BIRTH=''
        else
            guard_stop_status=1
        fi
    fi
    if [ "$FRAME_GUARD_FD_OPEN" -eq 1 ]; then
        if { exec 6>&-; }; then FRAME_GUARD_FD_OPEN=0; else guard_stop_status=1; fi
    fi
    rm -f "$LOCK_DIR/frame-guard-commands" "$LOCK_DIR/frame-guard-lifetime" "$LOCK_DIR/frame-guard-ready" "$LOCK_DIR/frame-guard-ready.pending" "$LOCK_DIR/frame-guard-window" "$LOCK_DIR/frame-guard-window.pending" || guard_stop_status=1
    return "$guard_stop_status"
}

signal_recorded_child() {
    child_record=$1
    child_signal=$2
    recorded_pid=$(sed -n '1p' "$child_record" 2>/dev/null || :)
    case "$recorded_pid" in
        *[!0-9]*|'') return ;;
    esac
    recorded_parent=$(sed -n 's/^PPid:[[:space:]]*//p' "/proc/$recorded_pid/status" 2>/dev/null || :)
    [ "$recorded_parent" = "$3" ] || return 0
    kill "-$child_signal" "$recorded_pid" 2>/dev/null || :
}

native_theme_supported() {
    case "$PROFILE:$FIRMWARE" in
        scribe:5.19.5|scribe:5.20.1) ;;
        *) return 1 ;;
    esac
    command -v lipc-get-prop >/dev/null 2>&1 &&
        command -v setsid >/dev/null 2>&1 &&
        command -v usleep >/dev/null 2>&1 &&
        command -v pidof >/dev/null 2>&1
}

native_theme_group_is_owned() {
    [ -n "$NATIVE_THEME_COMMAND_PID" ] && [ -n "$NATIVE_THEME_COMMAND_BIRTH" ] || return 1
    process_matches "$NATIVE_THEME_COMMAND_PID" "$NATIVE_THEME_COMMAND_BIRTH" "$$" || return 1
    native_theme_group_stat=$(cat "/proc/$NATIVE_THEME_COMMAND_PID/stat" 2>/dev/null || :)
    native_theme_group_tail=${native_theme_group_stat##*) }
    set -- $native_theme_group_tail
    [ "$#" -ge 4 ] && [ "$3:$4" = "$NATIVE_THEME_COMMAND_PID:$NATIVE_THEME_COMMAND_PID" ]
}

stop_native_theme_command() {
    native_theme_stop_status=0
    if native_theme_group_is_owned; then
        # The wrapper remains the verified session/group leader until this
        # cleanup, including after the command finishes. Kill the whole
        # private group so a blocked command cannot leave a late writer.
        # An explicit signal already ends option parsing for dash/ash kill;
        # an extra -- is treated as an invalid PID by some shell builtins.
        kill -KILL "-$NATIVE_THEME_COMMAND_PID" 2>/dev/null ||
            signal_owned_process "$NATIVE_THEME_COMMAND_PID" "$NATIVE_THEME_COMMAND_BIRTH" "$$" KILL || :
    fi
    native_theme_reap_checks=10
    while process_matches "$NATIVE_THEME_COMMAND_PID" "$NATIVE_THEME_COMMAND_BIRTH" "$$" &&
        [ "$native_theme_reap_checks" -gt 0 ]; do
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$NATIVE_THEME_COMMAND_PID/status" 2>/dev/null || :)" in
            Z*|X*|'') break ;;
        esac
        usleep 100000 || native_theme_stop_status=1
        native_theme_reap_checks=$((native_theme_reap_checks - 1))
    done
    if [ -n "$NATIVE_THEME_COMMAND_PID" ]; then
        if [ ! -d "/proc/$NATIVE_THEME_COMMAND_PID" ] ||
            { process_matches "$NATIVE_THEME_COMMAND_PID" "$NATIVE_THEME_COMMAND_BIRTH" "$$" &&
              [ "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$NATIVE_THEME_COMMAND_PID/status" 2>/dev/null || :)" = 'Z (zombie)' ]; }; then
            wait "$NATIVE_THEME_COMMAND_PID" 2>/dev/null || :
        fi
    fi
    NATIVE_THEME_COMMAND_PID=''
    NATIVE_THEME_COMMAND_BIRTH=''
    return "$native_theme_stop_status"
}

owned_native_command() {
    NATIVE_COMMAND_EXIT_STATUS=''
    NATIVE_COMMAND_FAILURE=preparation
    native_output_limit=$1
    shift
    case "$native_output_limit" in *[!0-9]*|'') return 1 ;; esac
    [ "$native_output_limit" -gt 0 ] && [ "$native_output_limit" -le "$NATIVE_WINDOW_OUTPUT_LIMIT" ] || return 1
    command -v setsid >/dev/null 2>&1 && command -v usleep >/dev/null 2>&1 || return 1
    # This function is also used in conditional callers, which suppress shell
    # errexit. Never launch against stale completion or permission records.
    if ! rm -f "$LOCK_DIR/native-theme-output" "$LOCK_DIR/native-theme-done" "$LOCK_DIR/native-theme-done.pending" "$LOCK_DIR/native-theme-go" "$LOCK_DIR/native-theme-go.pending"; then
        if [ "$CLEANUP_RUNNING" -eq 1 ]; then note_cleanup_failure 'native theme command records'; fi
        return 1
    fi
    NATIVE_THEME_COMMAND_SEQUENCE=$((NATIVE_THEME_COMMAND_SEQUENCE + 1))
    setsid sh -c '
        completed=$1
        permission=$2
        sequence=$3
        token=$4
        file_blocks=$5
        shift 5
        wrapper_pid=$$
        # If the supervisor itself is killed, this private helper group
        # still has a finite lifetime instead of retaining a late IPC writer.
        (
            IFS= read -r watchdog_stat </proc/self/stat || exit 1
            watchdog_pid=${watchdog_stat%% *}
            watchdog_tail=${watchdog_stat##*) }
            set -- $watchdog_tail
            [ "$#" -ge 20 ] && [ "$3:$4" = "$wrapper_pid:$wrapper_pid" ] || exit 1
            shift 19
            watchdog_birth=$1
            sleep 6
            IFS= read -r watchdog_stat </proc/self/stat || exit 1
            [ "${watchdog_stat%% *}" = "$watchdog_pid" ] || exit 1
            watchdog_tail=${watchdog_stat##*) }
            set -- $watchdog_tail
            [ "$#" -ge 20 ] && [ "$3:$4" = "$wrapper_pid:$wrapper_pid" ] || exit 1
            shift 19
            [ "$1" = "$watchdog_birth" ] || exit 1
            # This still-live, same-birth member pins the original private
            # group/session; its numeric group ID cannot have been reused.
            kill -KILL "-$wrapper_pid" 2>/dev/null || exit 1
        ) &
        # No IPC is sent until the parent has verified this session leader.
        # The gate is bound to this PID and call, so a later command cannot
        # accidentally release an unproven older wrapper.
        while [ "$(cat "$permission" 2>/dev/null || :)" != "$wrapper_pid:$sequence:$token" ]; do
            usleep 10000
        done
        # Bound capture while the command is running, including a writer that
        # never returns. Keep the small completion records outside this limit.
        # The exact per-query byte quota is checked by the owner before reading.
        if (ulimit -f "$file_blocks" || exit 125; exec "$@"); then
            command_status=0
        else
            command_status=$?
        fi
        printf "%s\n" "$command_status" >"$completed.pending"
        mv -f "$completed.pending" "$completed"
        # Retain this private group anchor until its supervisor reaps it.
        while :; do sleep 1; done
    ' sh "$LOCK_DIR/native-theme-done" "$LOCK_DIR/native-theme-go" "$NATIVE_THEME_COMMAND_SEQUENCE" "$LAUNCH_TOKEN" "$NATIVE_COMMAND_FILE_BLOCKS" "$@" >"$LOCK_DIR/native-theme-output" 2>/dev/null </dev/null 3>&- 6>&- &
    NATIVE_THEME_COMMAND_PID=$!
    NATIVE_THEME_COMMAND_BIRTH=$(process_birth "$NATIVE_THEME_COMMAND_PID" 2>/dev/null || :)
    native_theme_checks=$NATIVE_THEME_TIMEOUT_TICKS
    native_theme_fast_checks=10
    native_theme_status=1
    NATIVE_COMMAND_FAILURE=unconfirmed
    native_theme_released=0
    while [ "$native_theme_checks" -gt 0 ]; do
        if [ -z "$NATIVE_THEME_COMMAND_BIRTH" ] &&
            [ "$(process_parent "$NATIVE_THEME_COMMAND_PID" 2>/dev/null || :)" = "$$" ]; then
            NATIVE_THEME_COMMAND_BIRTH=$(process_birth "$NATIVE_THEME_COMMAND_PID" 2>/dev/null || :)
        fi
        if native_theme_group_is_owned; then
            if [ "$native_theme_released" -eq 0 ]; then
                if ! printf '%s:%s:%s\n' "$NATIVE_THEME_COMMAND_PID" "$NATIVE_THEME_COMMAND_SEQUENCE" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-theme-go.pending" ||
                    ! mv -f "$LOCK_DIR/native-theme-go.pending" "$LOCK_DIR/native-theme-go"; then
                    if [ "$CLEANUP_RUNNING" -eq 1 ]; then note_cleanup_failure 'native theme permission publication'; fi
                    stop_native_theme_command || :
                    return 1
                fi
                native_theme_released=1
            fi
            if [ -r "$LOCK_DIR/native-theme-done" ]; then
                native_completed_status=$(cat "$LOCK_DIR/native-theme-done" 2>/dev/null || :)
                case "$native_completed_status" in
                    0) native_theme_status=0; NATIVE_COMMAND_EXIT_STATUS=0; NATIVE_COMMAND_FAILURE='' ;;
                    *[!0-9]*|'') NATIVE_COMMAND_FAILURE=invalid-completion ;;
                    *)
                        if [ "$native_completed_status" -le 255 ]; then
                            NATIVE_COMMAND_EXIT_STATUS=$native_completed_status
                            NATIVE_COMMAND_FAILURE=command-exit
                        else
                            NATIVE_COMMAND_FAILURE=invalid-completion
                        fi
                        ;;
                esac
                break
            fi
        fi
        if [ "$native_theme_fast_checks" -gt 0 ]; then
            native_theme_fast_checks=$((native_theme_fast_checks - 1))
            # Ten short polls consume the first existing 100 ms timeout tick.
            # Normal queries need not wait an entire display frame for a gate.
            if [ "$native_theme_fast_checks" -eq 0 ]; then
                native_theme_checks=$((native_theme_checks - 1))
            fi
            usleep 10000
        else
            native_theme_checks=$((native_theme_checks - 1))
            usleep 100000
        fi
    done
    if ! stop_native_theme_command; then
        native_theme_status=1
        NATIVE_COMMAND_EXIT_STATUS=''
        NATIVE_COMMAND_FAILURE=helper-cleanup
        if [ "$CLEANUP_RUNNING" -eq 1 ]; then note_cleanup_failure 'native theme command helper'; fi
    fi
    if [ -n "$NATIVE_COMMAND_EXIT_STATUS" ]; then
        if ! native_output_size=$(wc -c 2>/dev/null <"$LOCK_DIR/native-theme-output"); then
            native_theme_status=1
            NATIVE_COMMAND_EXIT_STATUS=''
            NATIVE_COMMAND_FAILURE=output-read
        else
            native_output_size=${native_output_size#"${native_output_size%%[![:space:]]*}"}
            native_output_size=${native_output_size%"${native_output_size##*[![:space:]]}"}
            case "$native_output_size" in
                *[!0-9]*|'') native_theme_status=1; NATIVE_COMMAND_EXIT_STATUS=''; NATIVE_COMMAND_FAILURE=output-size ;;
                *)
                    if [ "$native_output_size" -gt "$native_output_limit" ]; then
                        native_theme_status=1
                        NATIVE_COMMAND_EXIT_STATUS=''
                        NATIVE_COMMAND_FAILURE=output-limit
                    fi
                    ;;
            esac
        fi
    fi
    return "$native_theme_status"
}

log_native_command_failure() {
    # Callers supply a fixed step name; paths, tokens and native output remain
    # private. A missing completion is distinct from a proven command exit.
    launcher_log 'weread: Native step failed step=%s exit=%s reason=%s.\n' \
        "$1" "${NATIVE_COMMAND_EXIT_STATUS:-unconfirmed}" "${NATIVE_COMMAND_FAILURE:-unknown}"
}

native_theme_command() {
    owned_native_command "$NATIVE_QUERY_OUTPUT_LIMIT" "$@"
}

query_native_property() {
    NATIVE_PROPERTY_VALUE=''
    # Invoke the owned helper in the supervisor, never a command substitution:
    # signal/EXIT cleanup must retain its live PID, birth, and group identity.
    native_theme_command lipc-get-prop -s "$1" "$2" || return 1
    NATIVE_PROPERTY_VALUE=$(cat "$LOCK_DIR/native-theme-output" 3>&-) || return 1
}

query_native_processes() {
    NATIVE_PROCESS_IDS=''
    if owned_native_command "$NATIVE_QUERY_OUTPUT_LIMIT" pidof "$@"; then
        NATIVE_PROCESS_IDS=$(cat "$LOCK_DIR/native-theme-output" 3>&-) || return 1
        return 0
    fi
    # pidof's confirmed exit 1 with no bytes is a legitimate empty snapshot.
    # A timeout, failed retirement, or invalid output never publishes exit 1.
    [ "$NATIVE_COMMAND_EXIT_STATUS" = 1 ] && [ ! -s "$LOCK_DIR/native-theme-output" ]
}

capture_native_epdc_mode() {
    NATIVE_EPDC_MODE=''
    NATIVE_PORTRAIT_CALIBRATION=''
    NATIVE_THEME_CAPTURE_STATUS=unsupported
    native_theme_supported || return 0
    native_theme_running_wm=0
    NATIVE_THEME_CAPTURE_STATUS=wm_missing
    if ! query_native_processes awesome; then
        NATIVE_THEME_CAPTURE_STATUS=unknown
        launcher_log '%s\n' 'weread: Native display mode snapshot is unavailable because the process query failed.'
        return 0
    fi
    for native_theme_wm in $NATIVE_PROCESS_IDS; do
        case "$native_theme_wm" in *[!0-9]*|'') continue ;; esac
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$native_theme_wm/status" 2>/dev/null || :)" in
            T*|t*)
                NATIVE_THEME_CAPTURE_STATUS=wm_pre_stopped
                launcher_log '%s\n' 'weread: Native display mode snapshot is unavailable because awesome was already stopped.'
                return 0
                ;;
            Z*|X*|'') ;;
            *) native_theme_running_wm=1 ;;
        esac
    done
    if [ "$native_theme_running_wm" -ne 1 ]; then
        launcher_log '%s\n' 'weread: Native display mode snapshot is unavailable because no running awesome was observed.'
        return 0
    fi
    NATIVE_THEME_CAPTURE_STATUS=unknown
    if native_theme_command lipc-get-prop -s com.lab126.winmgr epdcMode; then
        case "$(cat "$LOCK_DIR/native-theme-output" 2>/dev/null || :)" in
            Y8|Y8INV)
                NATIVE_EPDC_MODE=$(cat "$LOCK_DIR/native-theme-output")
                NATIVE_THEME_CAPTURE_STATUS=known
                ;;
            *) launcher_log '%s\n' 'weread: Native display mode snapshot returned an unknown mode; no mode is published.' ;;
        esac
    else
        launcher_log '%s\n' 'weread: Native display mode snapshot read did not complete; no mode is published.'
    fi
    capture_native_portrait_calibration
}

capture_native_portrait_calibration() {
    NATIVE_PORTRAIT_CALIBRATION=''
    # The caller has confirmed a running native window manager and has not
    # stopped it yet. Pair direction and read-only GETVAR in one owned group,
    # with repeated observations and an active-power check on both sides.
    # A rejected optional calibration preserves the existing fixed mapping.
    if ! native_theme_command sh -c '
        status_file=$1
        application=$2
        profile=$3
        bounded_value() {
            rm -f "$status_file" "$status_file.pending" || return 1
            snapshot_hex=$(
                (
                    if "$@"; then step_status=0; else step_status=$?; fi
                    printf "%s\n" "$step_status" >"$status_file.pending" || exit 1
                    mv -f "$status_file.pending" "$status_file" || exit 1
                ) | dd bs=1 count=129 2>/dev/null | od -An -v -tx1 | tr -d " \n"
            ) || return 1
            IFS= read -r step_status <"$status_file" || return 1
            [ "$step_status" = 0 ] && [ "${#snapshot_hex}" -gt 0 ] && [ "${#snapshot_hex}" -le 256 ]
        }
        portrait_metadata() {
            case "$snapshot_hex" in
                4155544f3a3[0-3]0a) portrait_kind=AUTO; portrait_rotation_hex=${snapshot_hex#4155544f3a} ;;
                46495845443a3[0-3]0a) portrait_kind=FIXED; portrait_rotation_hex=${snapshot_hex#46495845443a} ;;
                *) return 1 ;;
            esac
            case "$portrait_rotation_hex" in
                300a) portrait_rotation=0 ;;
                310a) portrait_rotation=1 ;;
                320a) portrait_rotation=2 ;;
                330a) portrait_rotation=3 ;;
                *) return 1 ;;
            esac
        }
        portrait_direction() {
            case "$snapshot_hex" in
                55|550a) portrait_orientation=U ;;
                44|440a) portrait_orientation=D ;;
                4c|4c0a) [ "$profile" = scribe ] || return 1; portrait_orientation=U ;;
                52|520a) [ "$profile" = scribe ] || return 1; portrait_orientation=D ;;
                *) return 1 ;;
            esac
        }
        power_is_active() {
            case "$snapshot_hex" in 616374697665|6163746976650a) return 0 ;; esac
            return 1
        }
        bounded_value lipc-get-prop -s com.lab126.powerd state || exit 1
        power_is_active || exit 1
        bounded_value "$application" --kindle --probe-native-portrait-framebuffer --device "$profile" || exit 1
        portrait_metadata || exit 1
        first_metadata=$snapshot_hex
        if [ "$portrait_kind" = AUTO ]; then
            bounded_value lipc-get-prop com.lab126.winmgr accelerometer || exit 1
            portrait_direction || exit 1
            first_orientation=$portrait_orientation
        else
            portrait_orientation=FIXED
        fi
        bounded_value "$application" --kindle --probe-native-portrait-framebuffer --device "$profile" || exit 1
        [ "$snapshot_hex" = "$first_metadata" ] || exit 1
        if [ "$portrait_kind" = AUTO ]; then
            bounded_value lipc-get-prop com.lab126.winmgr accelerometer || exit 1
            portrait_direction || exit 1
            [ "$portrait_orientation" = "$first_orientation" ] || exit 1
        fi
        bounded_value lipc-get-prop -s com.lab126.powerd state || exit 1
        power_is_active || exit 1
        printf "%s:%s\n" "$portrait_orientation" "$portrait_rotation" || exit 1
    ' sh "$LOCK_DIR/native-portrait-step" "$APP_DIR/weread" "$PROFILE"; then
        launcher_log '%s\n' 'weread: Native portrait calibration is unavailable; keeping fixed orientation.'
        return 0
    fi
    native_portrait_hex=$(od -An -v -tx1 "$LOCK_DIR/native-theme-output" | tr -d ' \n') || return 0
    case "$native_portrait_hex" in
        553a3[0-3]0a) native_portrait_direction=U; native_portrait_rotation_hex=${native_portrait_hex#553a} ;;
        443a3[0-3]0a) native_portrait_direction=D; native_portrait_rotation_hex=${native_portrait_hex#443a} ;;
        46495845443a3[0-3]0a) native_portrait_direction=FIXED; native_portrait_rotation_hex=${native_portrait_hex#46495845443a} ;;
        *) return 0 ;;
    esac
    case "$native_portrait_rotation_hex" in
        300a) native_portrait_rotation=0 ;;
        310a) native_portrait_rotation=1 ;;
        320a) native_portrait_rotation=2 ;;
        330a) native_portrait_rotation=3 ;;
        *) return 0 ;;
    esac
    NATIVE_PORTRAIT_CALIBRATION="$native_portrait_direction:$native_portrait_rotation"
    return 0
}

publish_native_portrait_calibration() {
    if ! rm -f "$LOCK_DIR/$1" "$LOCK_DIR/$1.pending"; then return 1; fi
    case "$NATIVE_PORTRAIT_CALIBRATION" in U:[0-3]|D:[0-3]|FIXED:[0-3]) ;; *) return 0 ;; esac
    if [ "$1" = managed-native-portrait-calibration ]; then
        printf '%s:%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$MANAGED_SLEEP_CYCLE" "$NATIVE_PORTRAIT_CALIBRATION" >"$LOCK_DIR/$1.pending" || return 1
    elif [ "$1" = native-theme-portrait-calibration ]; then
        printf '%s:%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$NATIVE_THEME_REQUEST_SEQUENCE" "$NATIVE_PORTRAIT_CALIBRATION" >"$LOCK_DIR/$1.pending" || return 1
    else
        printf '%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$NATIVE_PORTRAIT_CALIBRATION" >"$LOCK_DIR/$1.pending" || return 1
    fi
    mv -f "$LOCK_DIR/$1.pending" "$LOCK_DIR/$1"
}

publish_native_epdc_mode() {
    rm -f "$LOCK_DIR/$1" "$LOCK_DIR/$1.pending"
    case "$NATIVE_EPDC_MODE" in Y8|Y8INV) ;; *) return 0 ;; esac
    if [ "$1" = managed-native-epdc-mode ]; then
        printf '%s:%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$MANAGED_SLEEP_CYCLE" "$NATIVE_EPDC_MODE" >"$LOCK_DIR/$1.pending"
    else
        printf '%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$NATIVE_EPDC_MODE" >"$LOCK_DIR/$1.pending"
    fi
    mv -f "$LOCK_DIR/$1.pending" "$LOCK_DIR/$1"
}

synchronize_native_dark_mode() {
    [ "$NATIVE_DARK_EXIT_ALLOWED" -eq 1 ] || return 0
    marker_matches_application ordinary-exit || return 0
    native_theme_target=$(cat "$LOCK_DIR/native-dark-target" 2>/dev/null || :)
    case "$native_theme_target" in
        "$APP_GENERATION:$LAUNCH_TOKEN:1") native_theme_target_mode=Y8INV ;;
        "$APP_GENERATION:$LAUNCH_TOKEN:0") native_theme_target_mode=Y8 ;;
        *) return 0 ;;
    esac
    if ! native_theme_supported || ! command -v lipc-set-prop >/dev/null 2>&1 ||
        [ "$NATIVE_THEME_CAPTURE_STATUS" != known ] || [ "$1" -ne 1 ]; then
        launcher_log '%s\n' 'weread: Native dark mode synchronization is unavailable; native ownership is restored.'
        return 0
    fi
    if ! native_theme_command lipc-get-prop -s com.lab126.powerd state ||
        [ "$(cat "$LOCK_DIR/native-theme-output" 2>/dev/null || :)" != active ]; then
        launcher_log '%s\n' 'weread: Native dark mode synchronization skipped because native power state is not active.'
        return 0
    fi
    if ! native_theme_command lipc-get-prop -s com.lab126.winmgr epdcMode; then
        launcher_log '%s\n' 'weread: Native dark mode read failed; native ownership is restored.'
        return 0
    fi
    case "$(cat "$LOCK_DIR/native-theme-output" 2>/dev/null || :)" in
        Y8|Y8INV) ;;
        *) launcher_log '%s\n' 'weread: Native dark mode returned an unknown mode; no theme was written.'; return 0 ;;
    esac
    # The mode query can itself be slow. Refresh the power-state evidence
    # immediately before releasing a theme setter into the native window.
    if ! native_theme_command lipc-get-prop -s com.lab126.powerd state ||
        [ "$(cat "$LOCK_DIR/native-theme-output" 2>/dev/null || :)" != active ]; then
        launcher_log '%s\n' 'weread: Native dark mode synchronization skipped because native power state changed.'
        return 0
    fi
    if ! native_theme_command lipc-set-prop -s com.lab126.winmgr epdcMode "$native_theme_target_mode" ||
        ! native_theme_command lipc-get-prop -s com.lab126.winmgr epdcMode ||
        [ "$(cat "$LOCK_DIR/native-theme-output" 2>/dev/null || :)" != "$native_theme_target_mode" ]; then
        launcher_log '%s\n' 'weread: Native dark mode write/readback was not confirmed; native ownership is restored.'
        return 0
    fi
    native_theme_persistence=not_checked
    if command -v get-dynconf-value >/dev/null 2>&1; then
        if ! native_theme_command get-dynconf-value winmgr.colorinverse.pref ||
            [ "$(cat "$LOCK_DIR/native-theme-output" 2>/dev/null || :)" != "$native_theme_target_mode" ]; then
            launcher_log '%s\n' 'weread: Native dark mode persistence was not confirmed; native ownership is restored.'
            return 0
        fi
        native_theme_persistence=confirmed
    fi
    launcher_log 'weread: Native dark mode synchronization confirmed mode=%s persistence=%s.\n' "$native_theme_target_mode" "$native_theme_persistence"
}

release_display() {
    display_release_status=0
    case "${1:-native}" in
        retain-frame) frame_guard_is_current || return 1 ;;
        native) stop_frame_guard || display_release_status=1 ;;
        *) return 1 ;;
    esac
    # A failed stop request may have reached Upstart before its caller timed out.
    # Restore its observed running baseline without claiming App ownership.
    [ "$DISPLAY_OWNED" -eq 1 ] || [ "$STATUSBAR_RESTORE_REQUIRED" -eq 1 ] ||
        [ "$STATUSBAR_STOPPED" -eq 1 ] || [ -n "$PAUSED_WM_IDENTITIES" ] ||
        [ -n "$WM_STOP_PENDING_IDENTITY" ] || return "$display_release_status"
    native_theme_wm_resumed=0
    native_theme_wm_failed=0
    restore_wm_identities=$PAUSED_WM_IDENTITIES
    if [ -n "$WM_STOP_PENDING_IDENTITY" ]; then
        case " $restore_wm_identities " in
            *" $WM_STOP_PENDING_IDENTITY "*) ;;
            *) restore_wm_identities="$restore_wm_identities $WM_STOP_PENDING_IDENTITY" ;;
        esac
    fi
    remaining_wm_identities=''
    remaining_pending_wm_identity=''
    for wm_identity in $restore_wm_identities; do
        wm_pid=${wm_identity%%:*}
        wm_birth=${wm_identity#*:}
        if [ "$(process_birth "$wm_pid" 2>/dev/null || :)" != "$wm_birth" ]; then
            launcher_log '%s\n' 'weread: Native step failed step=window-manager-identity; the captured identity is no longer current.'
            native_theme_wm_failed=1
            display_release_status=1
            case " $PAUSED_WM_IDENTITIES " in
                *" $wm_identity "*) remaining_wm_identities="$remaining_wm_identities $wm_identity" ;;
            esac
            if [ "$wm_identity" = "$WM_STOP_PENDING_IDENTITY" ]; then
                remaining_pending_wm_identity=$wm_identity
            fi
        elif kill -CONT "$wm_pid" 2>/dev/null; then
            native_theme_wm_resumed=1
        else
            wm_resume_exit=$?
            launcher_log 'weread: Native step failed step=window-manager-continue exit=%s.\n' "$wm_resume_exit"
            native_theme_wm_failed=1
            display_release_status=1
            case " $PAUSED_WM_IDENTITIES " in
                *" $wm_identity "*) remaining_wm_identities="$remaining_wm_identities $wm_identity" ;;
            esac
            if [ "$wm_identity" = "$WM_STOP_PENDING_IDENTITY" ]; then
                remaining_pending_wm_identity=$wm_identity
            fi
        fi
    done
    PAUSED_WM_IDENTITIES=$remaining_wm_identities
    WM_STOP_PENDING_IDENTITY=$remaining_pending_wm_identity
    if [ "$native_theme_wm_failed" -eq 1 ]; then native_theme_wm_resumed=0; fi
    synchronize_native_dark_mode "$native_theme_wm_resumed" || display_release_status=1
    if [ "$STATUSBAR_STOPPED" -eq 1 ] || [ "$STATUSBAR_RESTORE_REQUIRED" -eq 1 ]; then
        if owned_native_command "$NATIVE_QUERY_OUTPUT_LIMIT" start statusbar; then
            STATUSBAR_STOPPED=0
            STATUSBAR_RESTORE_REQUIRED=0
        else
            log_native_command_failure statusbar-start
            display_release_status=1
        fi
    fi
    if command -v xrefresh >/dev/null 2>&1; then
        if ! owned_native_command "$NATIVE_QUERY_OUTPUT_LIMIT" xrefresh; then
            log_native_command_failure native-redraw
            display_release_status=1
        fi
    fi
    if [ "$display_release_status" -eq 0 ]; then DISPLAY_OWNED=0; fi
    return "$display_release_status"
}

take_display() {
    [ -z "$WM_STOP_PENDING_IDENTITY" ] || return 1
    # winmgr is hosted by awesome, so query it only before this takeover.
    capture_native_epdc_mode
    # Each takeover records its own baseline. A sleep restores that baseline
    # before the stock screen saver receives display and input control.
    take_statusbar_running=0
    take_wm_pids=''
    if command -v status >/dev/null 2>&1 && command -v stop >/dev/null 2>&1 && command -v start >/dev/null 2>&1; then
        if ! owned_native_command "$NATIVE_QUERY_OUTPUT_LIMIT" status statusbar; then
            launcher_log '%s\n' 'weread: Display preparation failed because statusbar state is unavailable.'
            return 1
        fi
        take_statusbar_state=$(cat "$LOCK_DIR/native-theme-output" 3>&-) || return 1
        case "$take_statusbar_state" in
            *start/running*) take_statusbar_running=1 ;;
            *stop/waiting*) ;;
            *)
                launcher_log '%s\n' 'weread: Display preparation failed because statusbar state is unknown.'
                return 1
                ;;
        esac
    fi
    if command -v pidof >/dev/null 2>&1; then
        if ! query_native_processes awesome; then
            launcher_log '%s\n' 'weread: Display preparation failed because the process query is unavailable.'
            return 1
        fi
        take_wm_pids=$NATIVE_PROCESS_IDS
    fi
    # Finish all baseline queries before changing either native service.
    start_frame_guard || return 1
    if [ "$take_statusbar_running" -eq 1 ]; then
        STATUSBAR_RESTORE_REQUIRED=1
        if ! owned_native_command "$NATIVE_QUERY_OUTPUT_LIMIT" stop statusbar; then
            launcher_log '%s\n' 'weread: Display preparation failed because statusbar stop was not confirmed.'
            return 1
        fi
        STATUSBAR_STOPPED=1
    fi
    for wm_pid in $take_wm_pids; do
        case "$wm_pid" in
            *[!0-9]*|'') continue ;;
        esac
        # A previously stopped process must remain stopped after exit.
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$wm_pid/status" 2>/dev/null || :)" in
            T*|t*|'') continue ;;
        esac
        wm_birth=$(process_birth "$wm_pid" 2>/dev/null || :)
        [ -n "$wm_birth" ] || continue
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$wm_pid/status" 2>/dev/null || :)" in
            R*|S*|D*|I*) ;;
            *) continue ;;
        esac
        if [ "$(process_birth "$wm_pid" 2>/dev/null || :)" != "$wm_birth" ]; then
            launcher_log '%s\n' 'weread: Display preparation failed because the window manager identity changed.'
            return 1
        fi
        # Record the original running identity before STOP can make EXIT cleanup
        # responsible for continuing it, even before App ownership is acquired.
        WM_STOP_PENDING_IDENTITY="$wm_pid:$wm_birth"
        if [ "$(process_birth "$wm_pid" 2>/dev/null || :)" = "$wm_birth" ] &&
            kill -STOP "$wm_pid" 2>/dev/null; then
            case " $PAUSED_WM_IDENTITIES " in
                *" $WM_STOP_PENDING_IDENTITY "*) ;;
                *) PAUSED_WM_IDENTITIES="$PAUSED_WM_IDENTITIES $WM_STOP_PENDING_IDENTITY" ;;
            esac
            # The list owns the obligation before the pending slot is cleared.
            WM_STOP_PENDING_IDENTITY=''
        else
            WM_STOP_PENDING_IDENTITY=''
            launcher_log '%s\n' 'weread: Display preparation failed because window manager stop was not confirmed.'
            return 1
        fi
    done
    DISPLAY_OWNED=1
    return 0
}

native_theme_power_epoch_is_current() {
    owned_child_is_running "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" &&
        owned_child_is_running "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" "$EVENT_SOURCE_PID" || return 1
    [ -f "$LOCK_DIR/native-theme-power-epoch" ] && [ ! -L "$LOCK_DIR/native-theme-power-epoch" ] || return 1
    IFS=: read -r theme_epoch_token theme_epoch_count theme_epoch_extra <"$LOCK_DIR/native-theme-power-epoch" || return 1
    [ "$theme_epoch_token" = "$LAUNCH_TOKEN" ] && [ -z "$theme_epoch_extra" ] &&
        [ "$theme_epoch_count" = "$SLEEP_GENERATION" ]
}

latch_native_theme_interrupt() {
    printf '%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-theme-interrupt.pending" &&
        mv -f "$LOCK_DIR/native-theme-interrupt.pending" "$LOCK_DIR/native-theme-interrupt"
}

clear_native_theme_power_interrupt() {
    # The observer counts both SS and USB before relaying them. Main counts SS
    # only after consuming them. A queued/new event cannot become a wake base.
    if ! native_theme_power_epoch_is_current; then
        latch_native_theme_interrupt || :
        return 1
    fi
    if ! query_native_property com.lab126.powerd state; then
        log_native_command_failure wake-power-state
        return 1
    fi
    [ "$NATIVE_PROPERTY_VALUE" = active ] && native_theme_power_epoch_is_current || return 1
    rm -f "$LOCK_DIR/native-theme-interrupt" "$LOCK_DIR/native-theme-interrupt.pending" || return 1
    if ! native_theme_power_epoch_is_current; then
        latch_native_theme_interrupt || :
        return 1
    fi
}

publish_native_theme_available() {
    rm -f "$LOCK_DIR/native-theme-available" "$LOCK_DIR/native-theme-available.pending" || return 1
    [ "$NATIVE_THEME_CAPTURE_STATUS" = known ] && [ "$DISPLAY_OWNED" -eq 1 ] &&
        [ -n "$PAUSED_WM_IDENTITIES" ] && [ -z "$WM_STOP_PENDING_IDENTITY" ] &&
        native_theme_supported && command -v lipc-set-prop >/dev/null 2>&1 &&
        command -v get-dynconf-value >/dev/null 2>&1 || return 0
    printf '%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-theme-available.pending" &&
        mv -f "$LOCK_DIR/native-theme-available.pending" "$LOCK_DIR/native-theme-available"
}

native_theme_sequence_is_new() {
    case "$1" in *[!0-9]*|''|0*) return 1 ;; esac
    [ "${#1}" -le 20 ] || return 1
    # Prefix comparisons with a letter so awk never rounds a u64 as a number.
    awk -v candidate="$1" -v previous="$NATIVE_THEME_REQUEST_SEQUENCE" '
        BEGIN {
            maximum = "18446744073709551615"
            valid = length(candidate) < 20 || "x" candidate <= "x" maximum
            newer = length(candidate) > length(previous) ||
                (length(candidate) == length(previous) && "x" candidate > "x" previous)
            exit !(valid && newer)
        }
    '
}

read_native_theme_request() {
    [ -f "$LOCK_DIR/native-theme-request" ] && [ ! -L "$LOCK_DIR/native-theme-request" ] || return 1
    theme_request_bytes=$(wc -c <"$LOCK_DIR/native-theme-request" 2>/dev/null) || return 1
    theme_request_bytes=${theme_request_bytes#"${theme_request_bytes%%[![:space:]]*}"}
    theme_request_bytes=${theme_request_bytes%"${theme_request_bytes##*[![:space:]]}"}
    case "$theme_request_bytes" in *[!0-9]*|'') return 1 ;; esac
    [ "$theme_request_bytes" -gt 0 ] && [ "$theme_request_bytes" -le 128 ] || return 1
    IFS=: read -r theme_request_generation theme_request_token theme_request_sequence theme_request_target theme_request_extra <"$LOCK_DIR/native-theme-request" || return 1
    [ "$theme_request_generation:$theme_request_token" = "$APP_GENERATION:$LAUNCH_TOKEN" ] &&
        [ -z "$theme_request_extra" ] || return 1
    case "$theme_request_target" in Y8|Y8INV) ;; *) return 1 ;; esac
}

native_theme_request_is_current() {
    [ "$NATIVE_THEME_IN_PROGRESS" -eq 1 ] && [ "$APP_PHASE" = theme ] &&
        managed_application_is_current && marker_matches_application native-theme-available &&
        ! marker_matches_application ordinary-exit && native_theme_power_epoch_is_current &&
        ! marker_matches_application native-theme-interrupt || return 1
    read_native_theme_request || return 1
    [ "$theme_request_sequence:$theme_request_target" = "$NATIVE_THEME_REQUEST_SEQUENCE:$NATIVE_THEME_REQUEST_TARGET" ]
}

native_theme_transaction_power_is_active() {
    native_theme_request_is_current || return 1
    if ! query_native_property com.lab126.powerd state; then
        log_native_command_failure theme-power-state
        return 1
    fi
    [ "$NATIVE_PROPERTY_VALUE" = active ] && native_theme_request_is_current
}

continue_theme_window_managers() {
    [ -n "$PAUSED_WM_IDENTITIES" ] && [ -z "$WM_STOP_PENDING_IDENTITY" ] || return 1
    for theme_wm_identity in $PAUSED_WM_IDENTITIES; do
        theme_wm_pid=${theme_wm_identity%%:*}
        theme_wm_birth=${theme_wm_identity#*:}
        [ "$(process_birth "$theme_wm_pid" 2>/dev/null || :)" = "$theme_wm_birth" ] || return 1
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$theme_wm_pid/status" 2>/dev/null || :)" in T*|t*) ;; *) return 1 ;; esac
        kill -CONT "$theme_wm_pid" 2>/dev/null || return 1
    done
    # Retain every original restore obligation while the App is frozen.
    DISPLAY_OWNED=0
}

pause_theme_window_managers() {
    for theme_wm_identity in $PAUSED_WM_IDENTITIES; do
        theme_wm_pid=${theme_wm_identity%%:*}
        theme_wm_birth=${theme_wm_identity#*:}
        [ "$(process_birth "$theme_wm_pid" 2>/dev/null || :)" = "$theme_wm_birth" ] || return 1
        WM_STOP_PENDING_IDENTITY=$theme_wm_identity
        kill -STOP "$theme_wm_pid" 2>/dev/null || return 1
        theme_wm_checks=$(managed_wait_budget 1)
        theme_wm_stopped=0
        while [ "$theme_wm_checks" -gt 0 ]; do
            [ "$(process_birth "$theme_wm_pid" 2>/dev/null || :)" = "$theme_wm_birth" ] || return 1
            case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$theme_wm_pid/status" 2>/dev/null || :)" in
                T*|t*) theme_wm_stopped=1; break ;;
            esac
            managed_wait_tick || return 1
            theme_wm_checks=$((theme_wm_checks - 1))
        done
        [ "$theme_wm_stopped" -eq 1 ] || return 1
        WM_STOP_PENDING_IDENTITY=''
    done
    DISPLAY_OWNED=1
}

perform_native_theme_request() {
    NATIVE_THEME_STAGE=power-before-freeze
    native_theme_transaction_power_is_active || return 1
    NATIVE_THEME_STAGE=application-freeze
    signal_managed_application STOP || return 1
    theme_app_checks=$(managed_wait_budget 1)
    theme_app_stopped=0
    while [ "$theme_app_checks" -gt 0 ]; do
        native_theme_request_is_current || return 1
        case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$APP_CHILD_PID/status" 2>/dev/null || :)" in
            T*|t*) theme_app_stopped=1; break ;;
        esac
        managed_wait_tick || return 1
        theme_app_checks=$((theme_app_checks - 1))
    done
    [ "$theme_app_stopped" -eq 1 ] || return 1
    NATIVE_THEME_STAGE=frame-guard-capture
    request_frame_guard capture || return 1
    NATIVE_THEME_STAGE=power-before-native-window
    native_theme_transaction_power_is_active || return 1
    NATIVE_THEME_STAGE=window-manager-continue
    continue_theme_window_managers || return 1
    NATIVE_THEME_STAGE=power-before-set
    native_theme_transaction_power_is_active || return 1
    NATIVE_THEME_STAGE=system-set
    if ! native_theme_command lipc-set-prop -s com.lab126.winmgr epdcMode "$NATIVE_THEME_REQUEST_TARGET"; then
        log_native_command_failure theme-set
        [ "$NATIVE_COMMAND_FAILURE" = command-exit ] || return 1
    fi
    # A failed setter can still have changed the property. Only current native
    # readback determines the mode which the suspended renderer may compensate.
    NATIVE_THEME_STAGE=power-before-readback
    native_theme_transaction_power_is_active || return 1
    NATIVE_THEME_STAGE=system-readback
    if ! native_theme_command lipc-get-prop -s com.lab126.winmgr epdcMode; then
        log_native_command_failure theme-readback
        return 1
    fi
    theme_confirmed_mode=$(cat "$LOCK_DIR/native-theme-output") || return 1
    case "$theme_confirmed_mode" in Y8|Y8INV) ;; *) return 1 ;; esac
    theme_result_status=error
    NATIVE_THEME_STAGE=power-before-persistence
    native_theme_transaction_power_is_active || return 1
    NATIVE_THEME_STAGE=system-persistence
    if native_theme_command get-dynconf-value winmgr.colorinverse.pref; then
        theme_confirmed_preference=$(cat "$LOCK_DIR/native-theme-output") || return 1
        if [ "$theme_confirmed_mode" = "$NATIVE_THEME_REQUEST_TARGET" ] &&
            [ "$theme_confirmed_preference" = "$theme_confirmed_mode" ]; then
            theme_result_status=ok
        fi
    else
        log_native_command_failure theme-persistence
        [ "$NATIVE_COMMAND_FAILURE" = command-exit ] || return 1
    fi
    NATIVE_THEME_STAGE=portrait-calibration
    capture_native_portrait_calibration || return 1
    NATIVE_THEME_STAGE=power-before-display
    native_theme_transaction_power_is_active || return 1
    NATIVE_THEME_STAGE=window-manager-pause
    pause_theme_window_managers || return 1
    NATIVE_THEME_STAGE=frame-guard-alignment
    request_frame_guard align || return 1
    NATIVE_THEME_STAGE=power-before-result
    native_theme_transaction_power_is_active || return 1
    NATIVE_EPDC_MODE=$theme_confirmed_mode
    NATIVE_THEME_CAPTURE_STATUS=known
    NATIVE_THEME_STAGE=result-publication
    publish_native_epdc_mode native-epdc-mode || return 1
    publish_native_portrait_calibration native-theme-portrait-calibration || return 1
    printf '%s:%s:%s:%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" "$NATIVE_THEME_REQUEST_SEQUENCE" "$theme_result_status" "$theme_confirmed_mode" >"$LOCK_DIR/native-theme-result.pending" &&
        mv -f "$LOCK_DIR/native-theme-result.pending" "$LOCK_DIR/native-theme-result" || return 1
    NATIVE_THEME_STAGE=power-before-continue
    native_theme_transaction_power_is_active && [ "$DISPLAY_OWNED" -eq 1 ] || return 1
    NATIVE_THEME_STAGE=application-continue
    signal_managed_application CONT || return 1
    theme_ready_checks=$(managed_wait_budget 10)
    theme_ready_value="$APP_GENERATION:$LAUNCH_TOKEN:$NATIVE_THEME_REQUEST_SEQUENCE"
    NATIVE_THEME_STAGE=renderer-acknowledgment
    while [ "$theme_ready_checks" -gt 0 ]; do
        native_theme_request_is_current || return 1
        if [ -f "$LOCK_DIR/native-theme-ready" ] && [ ! -L "$LOCK_DIR/native-theme-ready" ] &&
            IFS= read -r theme_ready_record <"$LOCK_DIR/native-theme-ready" &&
            [ "$theme_ready_record" = "$theme_ready_value" ]; then
            case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$APP_CHILD_PID/status" 2>/dev/null || :)" in T*|t*|Z*|X*|'') return 1 ;; esac
            NATIVE_THEME_STAGE=power-after-renderer
            native_theme_transaction_power_is_active || return 1
            APP_PHASE=running
            NATIVE_THEME_IN_PROGRESS=0
            NATIVE_THEME_STAGE=commit-publication
            # Removing this exact request commits the server side. The client
            # retains its UI gate until that commit, so a second tap cannot
            # replace the request while its predecessor is still being checked.
            rm -f "$LOCK_DIR/native-theme-request" "$LOCK_DIR/native-theme-request.pending" || return 1
            NATIVE_THEME_STAGE=idle
            launcher_log 'weread: Native theme handoff confirmed result=%s mode=%s.\n' "$theme_result_status" "$theme_confirmed_mode"
            return 0
        fi
        managed_wait_tick || return 1
        theme_ready_checks=$((theme_ready_checks - 1))
    done
    launcher_log '%s\n' 'weread: Native theme renderer acknowledgment timed out.'
    return 1
}

begin_native_theme_request() {
    [ "$APP_PHASE" = running ] && [ "$NATIVE_THEME_IN_PROGRESS" -eq 0 ] || return 0
    marker_matches_application native-theme-available && marker_matches_application managed-sleep-available || return 0
    read_native_theme_request || return 0
    [ "$theme_request_generation:$theme_request_sequence" = "$1:$2" ] &&
        native_theme_sequence_is_new "$theme_request_sequence" || return 0
    managed_application_is_current || return 1
    NATIVE_THEME_REQUEST_SEQUENCE=$theme_request_sequence
    NATIVE_THEME_REQUEST_TARGET=$theme_request_target
    NATIVE_THEME_IN_PROGRESS=1
    NATIVE_THEME_STAGE=request-preparation
    APP_PHASE=theme
    rm -f "$LOCK_DIR/native-theme-result" "$LOCK_DIR/native-theme-result.pending" "$LOCK_DIR/native-theme-ready" "$LOCK_DIR/native-theme-ready.pending" "$LOCK_DIR/native-theme-portrait-calibration" "$LOCK_DIR/native-theme-portrait-calibration.pending" || return 1
    perform_native_theme_request
}

run_application() {
    trap - 0 HUP INT TERM
    WRAPPER_PID=''
    WRAPPER_BIRTH=''
    runner_child=''
    runner_child_birth=''
    runner_child_parent=''
    runner_child_termination_sent=0
    runner_stopping=0
    runner_interrupted=0
    runner_generation=$1
    runner_exit_reported=0
    finish_runner() {
        runner_exit_status=$1
        trap - 0
        trap '' HUP INT TERM
        if ! cleanup_wrapper_child "$runner_child" "$runner_child_birth" "$WRAPPER_PID"; then
            runner_exit_status=1
        fi
        if [ "$runner_exit_reported" -eq 0 ]; then
            printf '_weread_app_exit %s %s\n' "$runner_generation" "$runner_exit_status" >&3 || :
        fi
        exit "$runner_exit_status"
    }
    trap 'finish_runner "$?"' 0
    terminate_runner_child() {
        if [ -n "$runner_child" ] && [ "$runner_child_termination_sent" -eq 0 ]; then
            runner_child_termination_sent=1
            if signal_owned_process "$runner_child" "$runner_child_birth" "$runner_child_parent" TERM; then
                # TERM must also reach an ELF frozen for resident sleep.
                signal_owned_process "$runner_child" "$runner_child_birth" "$runner_child_parent" CONT || :
            else
                runner_child_termination_sent=0
            fi
        fi
    }
    stop_runner() {
        runner_stopping=1
        runner_interrupted=1
        terminate_runner_child
    }
    trap stop_runner HUP INT TERM
    capture_wrapper_identity || exit 1
    # Do not let an inherited resume variable turn an ordinary start into a
    # recovery. Only this supervisor's completed sleep cycle can request one.
    WEREAD_LAUNCH_CONTEXT_DIR=$LOCK_DIR WEREAD_LAUNCH_GENERATION=$1 WEREAD_LAUNCH_TOKEN=$LAUNCH_TOKEN WEREAD_RESUME_SESSION=$2 WEREAD_NATIVE_RETURN=$3 WEREAD_NATIVE_RETURN_RESULT=$4 "$APP_DIR/weread" --kindle --device "$PROFILE" --data-dir "$DATA_DIR" >"$DATA_DIR/logs/application-bootstrap.log" 2>&1 3>&- 6>&- &
    runner_child=$!
    runner_child_parent=$WRAPPER_PID
    if capture_wrapper_child_birth "$runner_child" 2>/dev/null; then
        runner_child_birth=$WRAPPER_CHILD_BIRTH
    elif ! owned_child_has_exited "$runner_child" '' "$WRAPPER_PID"; then
        launcher_log '%s\n' 'weread: Application identity could not be captured before publication.'
        exit 1
    fi
    if [ -n "$runner_child_birth" ]; then
        if ! printf '%s\n' "$runner_child" >"$LOCK_DIR/application-pid" ||
            ! printf '%s\n' "$runner_child_birth" >"$LOCK_DIR/application-birth"; then
            exit 1
        fi
        printf '_weread_app_started %s %s %s\n' "$1" "$runner_child" "$runner_child_birth" >&3
    fi
    if [ "$runner_stopping" -eq 1 ]; then
        terminate_runner_child
    fi
    while :; do
        runner_interrupted=0
        if wait "$runner_child"; then
            runner_status=0
        else
            runner_status=$?
        fi
        [ "$runner_interrupted" -eq 1 ] || break
    done
    runner_child=''
    runner_child_birth=''
    runner_child_parent=''
    printf '_weread_app_exit %s %s\n' "$1" "$runner_status" >&3
    runner_exit_reported=1
    exit "$runner_status"
}

start_application() {
    APP_GENERATION=$((APP_GENERATION + 1))
    APP_PHASE=running
    APP_RESUME_INTENT=$1
    APP_NATIVE_RETURN_INTENT=${2:-0}
    APP_NATIVE_RETURN_RESULT=${3:-closed}
    APP_CHILD_PID=''
    APP_CHILD_BIRTH=''
    MANAGED_FROZEN=0
    MANAGED_SLEEP_CYCLE=0
    NATIVE_DARK_EXIT_ALLOWED=0
    NATIVE_THEME_REQUEST_SEQUENCE=0
    NATIVE_THEME_REQUEST_TARGET=''
    NATIVE_THEME_IN_PROGRESS=0
    NATIVE_THEME_STAGE=idle
    rm -f "$LOCK_DIR/application-pid" "$LOCK_DIR/application-birth" "$LOCK_DIR/application-owned" "$LOCK_DIR/sleep-request" "$LOCK_DIR/resume-ready" "$LOCK_DIR/ordinary-exit" "$LOCK_DIR/state-open" "$LOCK_DIR/state-open.pending" "$LOCK_DIR/native-wifi-request" "$LOCK_DIR/native-quick-request" "$LOCK_DIR/native-return" "$LOCK_DIR/native-wifi-available" "$LOCK_DIR/native-quick-available" "$LOCK_DIR/managed-sleep-available" "$LOCK_DIR/managed-sleep-available.pending" "$LOCK_DIR/managed-sleep-request" "$LOCK_DIR/managed-sleep-request.pending" "$LOCK_DIR/managed-sleep-ready" "$LOCK_DIR/managed-sleep-ready.pending" "$LOCK_DIR/managed-wake-request" "$LOCK_DIR/managed-wake-request.pending" "$LOCK_DIR/managed-wake-ready" "$LOCK_DIR/managed-wake-ready.pending" "$LOCK_DIR/native-epdc-mode" "$LOCK_DIR/native-epdc-mode.pending" "$LOCK_DIR/native-portrait-calibration" "$LOCK_DIR/native-portrait-calibration.pending" "$LOCK_DIR/managed-native-epdc-mode" "$LOCK_DIR/managed-native-epdc-mode.pending" "$LOCK_DIR/managed-native-portrait-calibration" "$LOCK_DIR/managed-native-portrait-calibration.pending" "$LOCK_DIR/native-dark-target" "$LOCK_DIR/native-dark-target.pending"
    printf '%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" >"$LOCK_DIR/generation"
    publish_native_epdc_mode native-epdc-mode
    rm -f "$LOCK_DIR/native-theme-request" "$LOCK_DIR/native-theme-request.pending" "$LOCK_DIR/native-theme-result" "$LOCK_DIR/native-theme-result.pending" "$LOCK_DIR/native-theme-ready" "$LOCK_DIR/native-theme-ready.pending" "$LOCK_DIR/native-theme-interrupt" "$LOCK_DIR/native-theme-interrupt.pending" "$LOCK_DIR/native-theme-portrait-calibration" "$LOCK_DIR/native-theme-portrait-calibration.pending"
    publish_native_theme_available || fail 'Native theme capability publication failed.'
    if ! publish_native_portrait_calibration native-portrait-calibration; then
        launcher_log '%s\n' 'weread: Native portrait record publication failed; keeping fixed orientation.'
    fi
    if [ "$NATIVE_SETTINGS_SUPPORTED" -eq 1 ]; then
        printf '%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-wifi-available"
        printf '%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-quick-available"
    fi
    if [ "$APP_NATIVE_RETURN_INTENT" -eq 1 ]; then
        printf '%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-return"
    fi
    begin_owned_registration
    run_application "$APP_GENERATION" "$1" "$APP_NATIVE_RETURN_INTENT" "$APP_NATIVE_RETURN_RESULT" 6>&- &
    APP_RUNNER_PID=$!
    APP_RUNNER_BIRTH=$(process_birth "$APP_RUNNER_PID" 2>/dev/null || :)
    if [ -z "$APP_RUNNER_BIRTH" ]; then capture_runner_identity || :; fi
    finish_owned_registration
}

stop_application() {
    application_stop_status=0
    APP_STATUS=0
    if [ -n "$APP_RUNNER_PID" ]; then
        capture_runner_identity || :
        if [ -z "$APP_CHILD_PID" ]; then
            capture_application_identity \
                "$(cat "$LOCK_DIR/application-pid" 2>/dev/null || :)" \
                "$(cat "$LOCK_DIR/application-birth" 2>/dev/null || :)" || :
        fi
        # The runner queues one TERM and then CONT to its own child. Sending
        # a second immediate TERM here can interrupt a nonreentrant save.
        if ! signal_owned_process "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$" TERM; then
            signal_original_application TERM || :
            signal_original_application CONT || :
        fi
        remaining=10
        while owned_child_is_running "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$" &&
            [ "$remaining" -gt 0 ]; do
            sleep 1 || application_stop_status=1
            remaining=$((remaining - 1))
        done
        if ! owned_child_has_exited "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$"; then
            if [ -z "$APP_CHILD_PID" ]; then
                capture_application_identity \
                    "$(cat "$LOCK_DIR/application-pid" 2>/dev/null || :)" \
                    "$(cat "$LOCK_DIR/application-birth" 2>/dev/null || :)" || :
            fi
            signal_original_application KILL || :
            sleep 1 || application_stop_status=1
            signal_owned_process "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$" KILL || :
            runner_cleanup_checks=$(managed_wait_budget 1)
            while ! owned_child_has_exited "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$" &&
                [ "$runner_cleanup_checks" -gt 0 ]; do
                managed_wait_tick || :
                runner_cleanup_checks=$((runner_cleanup_checks - 1))
            done
        fi
        if owned_child_has_exited "$APP_RUNNER_PID" "$APP_RUNNER_BIRTH" "$$"; then
            if wait "$APP_RUNNER_PID"; then
                APP_STATUS=0
            else
                APP_STATUS=$?
            fi
        else
            APP_STATUS=1
            launcher_log '%s\n' 'weread: Application runner cleanup could not confirm exit; blocking wait was refused.'
        fi
        cleanup_captured_orphan || application_stop_status=1
        APP_RUNNER_PID=''
    fi
    APP_RUNNER_BIRTH=''
    APP_CHILD_PID=''
    APP_CHILD_BIRTH=''
    MANAGED_FROZEN=0
    rm -f "$LOCK_DIR/application-pid" "$LOCK_DIR/application-birth" "$LOCK_DIR/application-owned" || application_stop_status=1
    return "$application_stop_status"
}

power_subscriber() {
    trap - 0 HUP INT TERM
    trap 'exit 1' HUP INT TERM
    subscriber_owner=$1
    subscriber_owner_birth=$2
    subscriber_wrapper=$3
    subscriber_wrapper_birth=$4
    capture_wrapper_identity || exit 1
    subscriber_pid=$WRAPPER_PID
    subscriber_birth=$WRAPPER_BIRTH
    subscriber_permission="$LAUNCH_TOKEN:$subscriber_owner:$subscriber_owner_birth:$subscriber_wrapper:$subscriber_wrapper_birth:$subscriber_pid:$subscriber_birth"
    subscriber_checks=$(managed_wait_budget 5)
    while :; do
        # The wrapper must remain under the original supervisor until permission
        # is committed. An orphaned pre-exec helper never starts a subscription.
        [ "$(process_birth "$subscriber_owner" 2>/dev/null || :)" = "$subscriber_owner_birth" ] &&
            process_matches "$subscriber_wrapper" "$subscriber_wrapper_birth" "$subscriber_owner" &&
            process_matches "$subscriber_pid" "$subscriber_birth" "$subscriber_wrapper" || exit 1
        if [ -f "$LOCK_DIR/observer-go" ] && [ ! -L "$LOCK_DIR/observer-go" ]; then
            IFS= read -r subscriber_go <"$LOCK_DIR/observer-go" || exit 1
            [ "$subscriber_go" = "$subscriber_permission" ] || exit 1
            # Recheck after reading the mutable permission record. Identity was
            # already captured by Main and does not change when this shell execs.
            process_matches "$subscriber_wrapper" "$subscriber_wrapper_birth" "$subscriber_owner" &&
                process_matches "$subscriber_pid" "$subscriber_birth" "$subscriber_wrapper" || exit 1
            # The output FIFO was opened before this function started. Publish
            # that proof without changing the PID that will exec liblipc.
            printf '%s\n' "$subscriber_permission" >"$LOCK_DIR/observer-stream-ready.pending" &&
                mv -f "$LOCK_DIR/observer-stream-ready.pending" "$LOCK_DIR/observer-stream-ready" || exit 1
            exec lipc-wait-event -m -s 0 com.lab126.powerd goingToScreenSaver,outOfScreenSaver,exitingScreenSaver,usbConfigured
        fi
        [ "$subscriber_checks" -gt 0 ] || exit 1
        managed_wait_tick >/dev/null 2>&1 || exit 1
        subscriber_checks=$((subscriber_checks - 1))
    done
}

publish_native_theme_power_interrupt() {
    case "$1" in goingToScreenSaver*|usbConfigured*) ;; *) return 0 ;; esac
    observer_power_epoch=$((observer_power_epoch + 1))
    printf '%s:%s\n' "$LAUNCH_TOKEN" "$observer_power_epoch" >"$LOCK_DIR/native-theme-power-epoch.pending" &&
        mv -f "$LOCK_DIR/native-theme-power-epoch.pending" "$LOCK_DIR/native-theme-power-epoch" || return 1
    # This observer predates the first App generation. Read the current private
    # generation instead of publishing its inherited startup value.
    [ -f "$LOCK_DIR/generation" ] && [ ! -L "$LOCK_DIR/generation" ] || return 0
    if ! IFS=: read -r interrupt_generation interrupt_token interrupt_extra <"$LOCK_DIR/generation"; then
        return 1
    fi
    case "$interrupt_generation" in *[!0-9]*|''|0) return 0 ;; esac
    [ "$interrupt_token" = "$LAUNCH_TOKEN" ] && [ -z "$interrupt_extra" ] || return 0
    printf '%s:%s\n' "$interrupt_generation" "$interrupt_token" >"$LOCK_DIR/native-theme-interrupt.pending" &&
        mv -f "$LOCK_DIR/native-theme-interrupt.pending" "$LOCK_DIR/native-theme-interrupt"
}

observe_power() {
    trap - 0 HUP INT TERM
    WRAPPER_PID=''
    WRAPPER_BIRTH=''
    observer_child=''
    observer_child_birth=''
    observer_stopping=0
    observer_interrupted=0
    observer_read_open=0
    observer_anchor_open=0
    observer_relay_running=0
    observer_power_epoch=0
    observer_owner=$1
    observer_owner_birth=$2
    finish_power_observer() {
        observer_exit_status=$1
        trap - 0
        trap '' HUP INT TERM
        if ! cleanup_wrapper_child "$observer_child" "$observer_child_birth" "$WRAPPER_PID"; then
            observer_exit_status=1
        fi
        if [ "$observer_read_open" -eq 1 ]; then exec 4<&-; fi
        if [ "$observer_anchor_open" -eq 1 ]; then exec 5>&-; fi
        if [ "$observer_stopping" -eq 0 ]; then
            printf '_weread_observer_exit %s\n' "$observer_exit_status" >&3 || :
        fi
        exit "$observer_exit_status"
    }
    trap 'finish_power_observer "$?"' 0
    stop_observer_child() {
        observer_stopping=1
        observer_interrupted=1
        if [ -n "$observer_child" ]; then
            # liblipc handles INT; cleanup also bounds an unresponsive helper.
            signal_owned_process "$observer_child" "$observer_child_birth" "$WRAPPER_PID" INT || :
            signal_owned_process "$observer_child" "$observer_child_birth" "$WRAPPER_PID" CONT || :
        fi
        # The relay has already captured its original child and committed the
        # subscription proof. Enter bounded cleanup even if read is restarted
        # after the signal or the subscriber ignores both INT and TERM.
        if [ "$observer_relay_running" -eq 1 ]; then exit 0; fi
    }
    trap stop_observer_child HUP INT TERM
    capture_wrapper_identity || exit 1
    [ "$observer_stopping" -eq 0 ] || exit 0
    printf '%s:0\n' "$LAUNCH_TOKEN" >"$LOCK_DIR/native-theme-power-epoch.pending" &&
        mv -f "$LOCK_DIR/native-theme-power-epoch.pending" "$LOCK_DIR/native-theme-power-epoch" || exit 1
    # The temporary write anchor prevents an open from blocking if the child
    # cannot redirect its output. It is closed after that exact child's proof.
    exec 5<>"$LOCK_DIR/observer-events"
    observer_anchor_open=1
    exec 4<"$LOCK_DIR/observer-events"
    observer_read_open=1
    # These are actual powerd events used by the reviewed KOReader input code.
    # OutOfSS requests native screen-saver dismissal; ExitingSS completes it.
    # The child closes the duplicate FIFO descriptor and cannot exec liblipc
    # until Main has captured its original birth and live wrapper ancestry.
    power_subscriber "$observer_owner" "$observer_owner_birth" "$WRAPPER_PID" "$WRAPPER_BIRTH" >"$LOCK_DIR/observer-events" 2>/dev/null 3>&- 4<&- 5>&- &
    observer_child=$!
    if capture_wrapper_child_birth "$observer_child" 2>/dev/null; then
        observer_child_birth=$WRAPPER_CHILD_BIRTH
    elif ! owned_child_has_exited "$observer_child" '' "$WRAPPER_PID"; then
        launcher_log '%s\n' 'weread: Power observer identity could not be captured before publication.'
        exit 1
    fi
    [ "$observer_stopping" -eq 0 ] || exit 0
    if [ -n "$observer_child_birth" ]; then
        # The bare PID remains a diagnostic compatibility record only. Main
        # authorizes cleanup from the typed record captured before permission.
        if ! printf '%s\n' "$observer_child" >"$LOCK_DIR/observer-pid" ||
            ! printf '%s:%s:%s:%s:%s:%s:%s\n' "$LAUNCH_TOKEN" "$observer_owner" "$observer_owner_birth" "$WRAPPER_PID" "$WRAPPER_BIRTH" "$observer_child" "$observer_child_birth" >"$LOCK_DIR/observer-owned.pending" ||
            ! mv -f "$LOCK_DIR/observer-owned.pending" "$LOCK_DIR/observer-owned"; then
            exit 1
        fi
    fi
    if [ "$observer_stopping" -eq 1 ]; then
        signal_owned_process "$observer_child" "$observer_child_birth" "$WRAPPER_PID" INT || :
        signal_owned_process "$observer_child" "$observer_child_birth" "$WRAPPER_PID" CONT || :
    fi
    observer_stream_checks=$(managed_wait_budget 5)
    observer_stream_permission="$LAUNCH_TOKEN:$observer_owner:$observer_owner_birth:$WRAPPER_PID:$WRAPPER_BIRTH:$observer_child:$observer_child_birth"
    observer_stream_ready=0
    while [ "$observer_stream_checks" -gt 0 ] && [ "$observer_stopping" -eq 0 ]; do
        process_matches "$observer_child" "$observer_child_birth" "$WRAPPER_PID" || exit 1
        if [ -f "$LOCK_DIR/observer-stream-ready" ] && [ ! -L "$LOCK_DIR/observer-stream-ready" ] &&
            IFS= read -r observer_stream_record <"$LOCK_DIR/observer-stream-ready" &&
            [ "$observer_stream_record" = "$observer_stream_permission" ]; then
            observer_stream_ready=1
            break
        fi
        managed_wait_tick || exit 1
        observer_stream_checks=$((observer_stream_checks - 1))
    done
    [ "$observer_stopping" -eq 0 ] || exit 0
    [ "$observer_stream_ready" -eq 1 ] || exit 1
    exec 5>&-
    observer_anchor_open=0
    # Keep liblipc's original identity while the owning wrapper relays events.
    # Interruption reaches a blocking theme handoff before its queued FIFO hint.
    observer_relay_running=1
    while IFS= read -r observer_power_event <&4; do
        [ "$observer_stopping" -eq 0 ] || break
        [ "$(process_birth "$observer_owner" 2>/dev/null || :)" = "$observer_owner_birth" ] || exit 1
        publish_native_theme_power_interrupt "$observer_power_event" || exit 1
        printf '%s\n' "$observer_power_event" >&3 || exit 1
    done
    exec 4<&-
    observer_read_open=0
    observer_relay_running=0
    while :; do
        [ "$observer_stopping" -eq 0 ] || exit 0
        observer_interrupted=0
        if wait "$observer_child"; then
            observer_status=0
        else
            observer_status=$?
        fi
        [ "$observer_stopping" -eq 0 ] || exit 0
        [ "$observer_interrupted" -eq 1 ] || break
    done
    observer_child=''
    observer_child_birth=''
    exit "$observer_status"
}

capture_power_observer_identity() {
    [ -n "$EVENT_SOURCE_PID" ] || return 1
    if [ -n "$EVENT_SOURCE_BIRTH" ]; then
        process_matches "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$"
        return $?
    fi
    # Called only during startup, before Main can create any replacement work.
    # The fork's original shell job and live parent pin this first birth sample.
    [ -r "/proc/$EVENT_SOURCE_PID/cmdline" ] &&
        tr '\000' '\n' <"/proc/$EVENT_SOURCE_PID/cmdline" |
        grep -F -x -- "$0" >/dev/null || return 1
    capture_wrapper_child_birth "$EVENT_SOURCE_PID" "$$" 2>/dev/null || return 1
    EVENT_SOURCE_BIRTH=$WRAPPER_CHILD_BIRTH
}

capture_power_observer_child() {
    [ -n "$EVENT_SOURCE_BIRTH" ] || return 1
    process_matches "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" || return 1
    [ -f "$LOCK_DIR/observer-owned" ] && [ ! -L "$LOCK_DIR/observer-owned" ] || return 1
    if ! IFS=: read -r power_record_token power_record_owner power_record_owner_birth power_record_wrapper power_record_wrapper_birth power_record_child power_record_child_birth power_record_extra 2>/dev/null <"$LOCK_DIR/observer-owned"; then
        return 1
    fi
    [ -z "$power_record_extra" ] || return 1
    [ "$power_record_token:$power_record_owner:$power_record_owner_birth:$power_record_wrapper:$power_record_wrapper_birth" = "$LAUNCH_TOKEN:$$:$POWER_OWNER_BIRTH:$EVENT_SOURCE_PID:$EVENT_SOURCE_BIRTH" ] || return 1
    process_matches "$power_record_child" "$power_record_child_birth" "$EVENT_SOURCE_PID" || return 1
    if [ -n "$EVENT_SOURCE_CHILD_PID" ]; then
        [ "$EVENT_SOURCE_CHILD_PID:$EVENT_SOURCE_CHILD_BIRTH" = "$power_record_child:$power_record_child_birth" ] || return 1
    fi
    EVENT_SOURCE_CHILD_PID=$power_record_child
    EVENT_SOURCE_CHILD_BIRTH=$power_record_child_birth
}

start_power_observer() {
    begin_owned_registration
    power_start_status=1
    # Signal traps defer exit across fork/$! registration and finite ownership
    # capture. They preserve the first signal's original exit status.
    if capture_wrapper_identity && [ "$WRAPPER_PID" = "$$" ]; then
        POWER_OWNER_BIRTH=$WRAPPER_BIRTH
        observe_power "$$" "$POWER_OWNER_BIRTH" &
        EVENT_SOURCE_PID=$!
        if capture_power_observer_identity; then
            power_start_checks=$(managed_wait_budget 3)
            while [ "$power_start_checks" -gt 0 ] && [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; do
                if capture_power_observer_child; then
                    # Store the complete original identity before committing
                    # permission. A pending signal can now clean both processes.
                    if process_matches "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" &&
                        process_matches "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" "$EVENT_SOURCE_PID" &&
                        printf '%s:%s:%s:%s:%s:%s:%s\n' "$LAUNCH_TOKEN" "$$" "$POWER_OWNER_BIRTH" "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" >"$LOCK_DIR/observer-go.pending" &&
                        mv -f "$LOCK_DIR/observer-go.pending" "$LOCK_DIR/observer-go"; then
                        power_stream_checks=$(managed_wait_budget 5)
                        power_stream_permission="$LAUNCH_TOKEN:$$:$POWER_OWNER_BIRTH:$EVENT_SOURCE_PID:$EVENT_SOURCE_BIRTH:$EVENT_SOURCE_CHILD_PID:$EVENT_SOURCE_CHILD_BIRTH"
                        while [ "$power_stream_checks" -gt 0 ] && [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; do
                            process_matches "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" &&
                                process_matches "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" "$EVENT_SOURCE_PID" || break
                            if [ -f "$LOCK_DIR/observer-stream-ready" ] && [ ! -L "$LOCK_DIR/observer-stream-ready" ] &&
                                IFS= read -r power_stream_record <"$LOCK_DIR/observer-stream-ready" &&
                                [ "$power_stream_record" = "$power_stream_permission" ]; then
                                power_start_status=0
                                break
                            fi
                            managed_wait_tick || break
                            power_stream_checks=$((power_stream_checks - 1))
                        done
                    fi
                    break
                fi
                owned_child_has_exited "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" && break
                process_matches "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" || break
                managed_wait_tick || break
                power_start_checks=$((power_start_checks - 1))
            done
        fi
    fi
    finish_owned_registration
    return "$power_start_status"
}

power_observer_has_exited() {
    [ "$EVENT_SOURCE_REAPED" -eq 1 ] ||
        owned_child_has_exited "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$"
}

cleanup_power_observer_child() {
    [ -n "$EVENT_SOURCE_CHILD_PID" ] || return 0
    if process_matches "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" &&
        process_matches "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" "$EVENT_SOURCE_PID"; then
        signal_owned_process "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" "$EVENT_SOURCE_PID" INT || :
        signal_owned_process "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" "$EVENT_SOURCE_PID" CONT || :
        cleanup_wrapper_child "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" "$EVENT_SOURCE_PID"
        return $?
    fi
    # Only Main's already-proven original child can be retired under PID 1.
    # Mutable PID/owned/go records and a newly sampled wrapper birth are ignored.
    if ! owned_child_is_running "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" &&
        process_matches "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" 1; then
        signal_owned_process "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" 1 INT || :
        signal_owned_process "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" 1 CONT || :
        cleanup_wrapper_child "$EVENT_SOURCE_CHILD_PID" "$EVENT_SOURCE_CHILD_BIRTH" 1
        return $?
    fi
    [ ! -d "/proc/$EVENT_SOURCE_CHILD_PID" ]
}

stop_observer() {
    observer_stop_status=0
    if [ -n "$EVENT_SOURCE_PID" ]; then
        signal_owned_process "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" TERM || :
        signal_owned_process "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" CONT || :
        # Allow the wrapper's own bounded identity retry, TERM grace, and reap.
        # An unregistered pre-exec child cannot start liblipc without permission.
        observer_stop_checks=$(managed_wait_budget 6)
        while owned_child_is_running "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" &&
            [ "$observer_stop_checks" -gt 0 ]; do
            managed_wait_tick || observer_stop_status=1
            observer_stop_checks=$((observer_stop_checks - 1))
        done
        cleanup_power_observer_child || observer_stop_status=1
        # Main can signal a proven grandchild but cannot wait/reap it. Give its
        # original wrapper a bounded opportunity to reap before retiring that
        # wrapper; otherwise a killed child can be stranded under PID 1.
        observer_reap_checks=$(managed_wait_budget 2)
        while owned_child_is_running "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" &&
            [ "$observer_reap_checks" -gt 0 ]; do
            managed_wait_tick || observer_stop_status=1
            observer_reap_checks=$((observer_reap_checks - 1))
        done
        if owned_child_is_running "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$"; then
            signal_owned_process "$EVENT_SOURCE_PID" "$EVENT_SOURCE_BIRTH" "$$" KILL || observer_stop_status=1
        fi
        observer_stop_checks=$(managed_wait_budget 1)
        while ! power_observer_has_exited && [ "$observer_stop_checks" -gt 0 ]; do
            managed_wait_tick || observer_stop_status=1
            observer_stop_checks=$((observer_stop_checks - 1))
        done
        if power_observer_has_exited; then
            if [ "$EVENT_SOURCE_REAPED" -eq 0 ]; then
                if wait "$EVENT_SOURCE_PID" 2>/dev/null; then
                    observer_wait_status=0
                else
                    observer_wait_status=$?
                fi
                EVENT_SOURCE_REAPED=1
                # Startup failures without a captured descendant cannot prove
                # that an unknown child was retired by the wrapper itself.
                if [ -z "$EVENT_SOURCE_CHILD_PID" ] && [ "$observer_wait_status" -ne 0 ]; then
                    observer_stop_status=1
                fi
            fi
            if cleanup_power_observer_child; then
                EVENT_SOURCE_PID=''
                EVENT_SOURCE_BIRTH=''
                EVENT_SOURCE_REAPED=0
                EVENT_SOURCE_CHILD_PID=''
                EVENT_SOURCE_CHILD_BIRTH=''
            else
                observer_stop_status=1
            fi
        else
            observer_stop_status=1
            launcher_log '%s\n' 'weread: Power observer cleanup could not confirm the original identity exited; blocking wait was refused.'
        fi
    fi
    rm -f "$LOCK_DIR/observer-pid" "$LOCK_DIR/observer-owned" "$LOCK_DIR/observer-owned.pending" "$LOCK_DIR/observer-go" "$LOCK_DIR/observer-go.pending" "$LOCK_DIR/observer-stream-ready" "$LOCK_DIR/observer-stream-ready.pending" "$LOCK_DIR/observer-events" || observer_stop_status=1
    return "$observer_stop_status"
}

probe_wake_later() {
    trap - 0 HUP INT TERM
    WRAPPER_PID=''
    WRAPPER_BIRTH=''
    probe_child=''
    probe_child_birth=''
    probe_cycle=$1
    probe_stopping=0
    probe_interrupted=0
    probe_exit_reported=0
    finish_wake_probe() {
        probe_exit_status=$1
        trap - 0
        trap '' HUP INT TERM
        if ! cleanup_wrapper_child "$probe_child" "$probe_child_birth" "$WRAPPER_PID"; then
            probe_exit_status=1
        fi
        if [ "$probe_stopping" -eq 0 ] && [ "$probe_exit_reported" -eq 0 ]; then
            printf '_weread_wake_probe_error %s %s %s\n' "$probe_cycle" "$WRAPPER_PID" "$WRAPPER_BIRTH" >&3 || :
        fi
        exit "$probe_exit_status"
    }
    trap 'finish_wake_probe "$?"' 0
    stop_probe_child() {
        probe_stopping=1
        probe_interrupted=1
        if [ -n "$probe_child" ]; then
            signal_owned_process "$probe_child" "$probe_child_birth" "$WRAPPER_PID" TERM || :
            signal_owned_process "$probe_child" "$probe_child_birth" "$WRAPPER_PID" CONT || :
        fi
    }
    trap stop_probe_child HUP INT TERM
    capture_wrapper_identity || exit 1
    [ "$probe_stopping" -eq 0 ] || exit 0
    # Only the wrapper publishes events. A stopped or orphaned timer must not
    # keep the power FIFO's read/write descriptor alive after supervisor exit.
    # The signal handler latches stop until this fork's PID and birth are
    # captured, so the EXIT cleanup still owns an unpublished timer child.
    sleep "$2" 3>&- 6>&- &
    probe_child=$!
    if capture_wrapper_child_birth "$probe_child" 2>/dev/null; then
        probe_child_birth=$WRAPPER_CHILD_BIRTH
    elif ! owned_child_has_exited "$probe_child" '' "$WRAPPER_PID"; then
        launcher_log '%s\n' 'weread: Wake probe child identity could not be captured before publication.'
        exit 1
    fi
    [ "$probe_stopping" -eq 0 ] || exit 0
    if [ -n "$probe_child_birth" ]; then
        if ! printf '%s:%s:%s:%s:%s:%s\n' "$probe_cycle" "$LAUNCH_TOKEN" "$WRAPPER_PID" "$WRAPPER_BIRTH" "$probe_child" "$probe_child_birth" >"$LOCK_DIR/wake-probe-child.pending" ||
            ! mv -f "$LOCK_DIR/wake-probe-child.pending" "$LOCK_DIR/wake-probe-child"; then
            exit 1
        fi
    fi
    while :; do
        probe_interrupted=0
        if wait "$probe_child"; then
            probe_status=0
        else
            probe_status=$?
        fi
        # A trap-interrupted wait is not evidence that the child exited. Keep
        # its identity for the EXIT trap's bounded TERM/CONT/KILL cleanup.
        [ "$probe_stopping" -eq 0 ] || exit 0
        [ "$probe_interrupted" -eq 1 ] || break
    done
    probe_child=''
    probe_child_birth=''
    [ "$probe_status" -eq 0 ] || exit "$probe_status"
    printf '_weread_wake_check %s %s %s\n' "$probe_cycle" "$WRAPPER_PID" "$WRAPPER_BIRTH" >&3
    probe_exit_reported=1
    exit 0
}

capture_wake_probe_identity() {
    [ -n "$WAKE_PROBE_PID" ] || return 1
    [ -z "$WAKE_PROBE_BIRTH" ] || return 0
    [ "$(process_parent "$WAKE_PROBE_PID" 2>/dev/null || :)" = "$$" ] || return 1
    [ -r "/proc/$WAKE_PROBE_PID/cmdline" ] &&
        tr '\000' '\n' <"/proc/$WAKE_PROBE_PID/cmdline" |
        grep -F -x -- "$0" >/dev/null || return 1
    capture_wrapper_child_birth "$WAKE_PROBE_PID" "$$" 2>/dev/null || return 1
    WAKE_PROBE_BIRTH=$WRAPPER_CHILD_BIRTH
}

capture_wake_probe_child() {
    [ -n "$WAKE_PROBE_BIRTH" ] || return 1
    process_matches "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$" || return 1
    [ -r "$LOCK_DIR/wake-probe-child" ] || return 1
    if ! IFS=: read -r probe_record_cycle probe_record_token probe_record_wrapper probe_record_wrapper_birth probe_record_child probe_record_child_birth probe_record_extra 2>/dev/null <"$LOCK_DIR/wake-probe-child"; then
        return 1
    fi
    [ -z "$probe_record_extra" ] || return 1
    [ "$probe_record_cycle:$probe_record_token:$probe_record_wrapper:$probe_record_wrapper_birth" = "$WAKE_PROBE_CYCLE:$LAUNCH_TOKEN:$WAKE_PROBE_PID:$WAKE_PROBE_BIRTH" ] || return 1
    process_matches "$probe_record_child" "$probe_record_child_birth" "$WAKE_PROBE_PID" || return 1
    if [ -n "$WAKE_PROBE_CHILD_PID" ]; then
        [ "$WAKE_PROBE_CHILD_PID:$WAKE_PROBE_CHILD_BIRTH" = "$probe_record_child:$probe_record_child_birth" ] || return 1
    fi
    WAKE_PROBE_CHILD_PID=$probe_record_child
    WAKE_PROBE_CHILD_BIRTH=$probe_record_child_birth
}

wake_probe_has_exited() {
    [ "$WAKE_PROBE_REAPED" -eq 1 ] ||
        owned_child_has_exited "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$"
}

cleanup_wake_probe_child() {
    [ -n "$WAKE_PROBE_CHILD_PID" ] || return 0
    if process_matches "$WAKE_PROBE_CHILD_PID" "$WAKE_PROBE_CHILD_BIRTH" "$WAKE_PROBE_PID"; then
        cleanup_wrapper_child "$WAKE_PROBE_CHILD_PID" "$WAKE_PROBE_CHILD_BIRTH" "$WAKE_PROBE_PID"
        return $?
    fi
    # Only the identity captured with live ancestry above permits orphan
    # cleanup. Never extend this exception to a newly read mutable record.
    if wake_probe_has_exited &&
        process_matches "$WAKE_PROBE_CHILD_PID" "$WAKE_PROBE_CHILD_BIRTH" 1; then
        cleanup_wrapper_child "$WAKE_PROBE_CHILD_PID" "$WAKE_PROBE_CHILD_BIRTH" 1
        return $?
    fi
    [ ! -d "/proc/$WAKE_PROBE_CHILD_PID" ]
}

start_wake_probe() {
    stop_wake_probe || return 1
    # A terminal signal must not lose the fork's original shell job before its
    # PID, birth, and timer child have a bounded in-memory registration owner.
    begin_owned_registration
    wake_probe_registration_status=0
    if register_wake_probe "$1" "$2"; then :; else wake_probe_registration_status=$?; fi
    finish_owned_registration
    return "$wake_probe_registration_status"
}

register_wake_probe() {
    WAKE_PROBE_CYCLE=$1
    probe_wake_later "$1" "$2" 6>&- &
    WAKE_PROBE_PID=$!
    WAKE_PROBE_BIRTH=''
    WAKE_PROBE_REAPED=0
    WAKE_PROBE_CHILD_PID=''
    WAKE_PROBE_CHILD_BIRTH=''
    capture_wake_probe_identity || return 1
    # A wrapper that cannot identify itself cannot send a fenced error event.
    # Bind startup to its proven child record so that an early exit or failed
    # publication is visible before relying on this timer for a real wake.
    wake_probe_start_checks=2
    wake_probe_fast_start=0
    if command -v usleep >/dev/null 2>&1; then
        wake_probe_start_checks=200
        wake_probe_fast_start=1
    fi
    while [ "$wake_probe_start_checks" -gt 0 ]; do
        if capture_wake_probe_child; then
            return 0
        fi
        if wake_probe_has_exited ||
            ! process_matches "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$"; then
            return 1
        fi
        if [ "$wake_probe_fast_start" -eq 1 ]; then
            usleep 10000 || return 1
        else
            sleep 1 || return 1
        fi
        wake_probe_start_checks=$((wake_probe_start_checks - 1))
    done
    return 1
}

stop_wake_probe() {
    wake_probe_stop_status=0
    if [ -n "$WAKE_PROBE_PID" ]; then
        capture_wake_probe_identity || :
        capture_wake_probe_child || :
        signal_owned_process "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$" TERM || :
        signal_owned_process "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$" CONT || :
        # Healthy timers normally finish immediately after TERM/CONT. Observe
        # them at 10 ms before using the longer exceptional cleanup allowance.
        if command -v usleep >/dev/null 2>&1; then
            wake_probe_quick_checks=10
            while owned_child_is_running "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$" &&
                [ "$wake_probe_quick_checks" -gt 0 ]; do
                capture_wake_probe_child || :
                usleep 10000 || wake_probe_stop_status=1
                wake_probe_quick_checks=$((wake_probe_quick_checks - 1))
            done
        fi
        # Allow the original birth attempt, an EXIT retry, TERM grace, and
        # post-KILL reap, plus one second of margin. A cancelled wrapper can
        # still lack a published record, so it must finish that owned cleanup.
        wake_probe_checks=$(managed_wait_budget 6)
        while owned_child_is_running "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$" &&
            [ "$wake_probe_checks" -gt 0 ]; do
            capture_wake_probe_child || :
            managed_wait_tick || wake_probe_stop_status=1
            wake_probe_checks=$((wake_probe_checks - 1))
        done
        cleanup_wake_probe_child || wake_probe_stop_status=1
        if owned_child_is_running "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$"; then
            signal_owned_process "$WAKE_PROBE_PID" "$WAKE_PROBE_BIRTH" "$$" KILL || wake_probe_stop_status=1
        fi
        wake_probe_checks=$(managed_wait_budget 1)
        while ! wake_probe_has_exited &&
            [ "$wake_probe_checks" -gt 0 ]; do
            managed_wait_tick || wake_probe_stop_status=1
            wake_probe_checks=$((wake_probe_checks - 1))
        done
        if wake_probe_has_exited; then
            if [ "$WAKE_PROBE_REAPED" -eq 0 ]; then
                if wait "$WAKE_PROBE_PID" 2>/dev/null; then :; else wake_probe_stop_status=1; fi
                WAKE_PROBE_REAPED=1
            fi
            if cleanup_wake_probe_child; then
                WAKE_PROBE_PID=''
                WAKE_PROBE_BIRTH=''
                WAKE_PROBE_CYCLE=0
                WAKE_PROBE_REAPED=0
                WAKE_PROBE_CHILD_PID=''
                WAKE_PROBE_CHILD_BIRTH=''
            else
                # Preserve the already-proven descendant identity for a later
                # EXIT cleanup retry; a reaped wrapper must never be waited twice.
                wake_probe_stop_status=1
            fi
        else
            wake_probe_stop_status=1
            launcher_log '%s\n' 'weread: Wake probe cleanup could not confirm exit; blocking wait was refused.'
        fi
    fi
    rm -f "$LOCK_DIR/wake-probe-child" "$LOCK_DIR/wake-probe-child.pending" || wake_probe_stop_status=1
    return "$wake_probe_stop_status"
}

resume_application() {
    stop_wake_probe
    WAKE_PENDING=0
    if [ "$APP_PHASE" = native ]; then
        NATIVE_SLEEPING=0
        if [ -n "$NATIVE_RETURN_PENDING" ]; then
            resume_from_native "$NATIVE_RETURN_PENDING"
        fi
        return 0
    fi
    if [ "$APP_PHASE" = suspended ]; then
        if resume_managed_application; then
            return 0
        fi
        launcher_log '%s\n' 'weread: Resident wake was not acknowledged; restoring native ownership before cold recovery.'
        cold_sleep_application
    fi
    take_display || return 1
    start_application "$SLEEP_RESUME_INTENT" "$SLEEP_NATIVE_RETURN_INTENT" "$SLEEP_NATIVE_RETURN_RESULT"
}

marker_matches_application() {
    [ "$(sed -n '1p' "$LOCK_DIR/$1" 2>/dev/null || :)" = "$APP_GENERATION:$LAUNCH_TOKEN" ]
}

attempt_managed_sleep() {
    marker_matches_application managed-sleep-available || return 1
    if [ -z "$APP_CHILD_PID" ]; then
        # A very fast application can advertise support before its started
        # event has been read from the FIFO. Its recorded identity is usable
        # only after the same ancestry, birth and executable checks.
        capture_application_identity \
            "$(cat "$LOCK_DIR/application-pid" 2>/dev/null || :)" \
            "$(cat "$LOCK_DIR/application-birth" 2>/dev/null || :)" || return 1
    fi
    managed_application_is_current || return 1
    MANAGED_SLEEP_CYCLE=$SLEEP_GENERATION
    write_managed_request managed-sleep-request || return 1
    signal_managed_application USR1 || return 1
    managed_sleep_checks=$(managed_wait_budget 8)
    while [ "$managed_sleep_checks" -gt 0 ]; do
        marker_matches_application ordinary-exit && return 1
        managed_application_is_current || return 1
        if managed_marker_matches managed-sleep-ready; then
            # A resident acknowledgment is never a substitute for the
            # persistent checkpoint needed by a possible cold fallback.
            marker_matches_application resume-ready || return 1
            signal_managed_application STOP || return 1
            MANAGED_FROZEN=1
            managed_stop_checks=$(managed_wait_budget 1)
            while [ "$managed_stop_checks" -gt 0 ]; do
                managed_application_is_current || return 1
                case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$APP_CHILD_PID/status" 2>/dev/null || :)" in
                    T*|t*)
                        APP_PHASE=suspended
                        SLEEP_RESUME_INTENT=1
                        SLEEP_NATIVE_RETURN_INTENT=0
                        SLEEP_NATIVE_RETURN_RESULT=closed
                        request_frame_guard capture || return 1
                        return 0
                        ;;
                esac
                managed_stop_checks=$((managed_stop_checks - 1))
                managed_wait_tick
            done
            return 1
        fi
        managed_sleep_checks=$((managed_sleep_checks - 1))
        managed_wait_tick
    done
    launcher_log '%s\n' 'weread: Resident sleep preparation timed out; using saved-exit recovery.'
    return 1
}

resume_managed_application() {
    managed_application_is_current || return 1
    marker_matches_application ordinary-exit && return 1
    take_display || return 1
    clear_native_theme_power_interrupt || fail 'Resident wake was superseded by a power event or an unconfirmed active state.'
    publish_native_epdc_mode managed-native-epdc-mode
    if ! publish_native_portrait_calibration managed-native-portrait-calibration; then
        launcher_log '%s\n' 'weread: Native wake portrait record publication failed; keeping the previous calibration.'
    fi
    request_frame_guard align || return 1
    clear_native_theme_power_interrupt || fail 'Resident wake alignment was superseded by a power event or an unconfirmed active state.'
    write_managed_request managed-wake-request || return 1
    # SIGUSR2 is queued while the process is stopped and becomes visible
    # before its next application timer can perform ordinary work.
    signal_managed_application USR2 || return 1
    signal_managed_application CONT || return 1
    MANAGED_FROZEN=0
    managed_wake_checks=$(managed_wait_budget 5)
    while [ "$managed_wake_checks" -gt 0 ]; do
        marker_matches_application ordinary-exit && return 1
        managed_application_is_current || return 1
        if managed_marker_matches managed-wake-ready; then
            case "$(sed -n 's/^State:[[:space:]]*//p' "/proc/$APP_CHILD_PID/status" 2>/dev/null || :)" in
                T*|t*|'') return 1 ;;
            esac
            if marker_matches_application sleep-request; then
                rm -f "$LOCK_DIR/sleep-request" || return 1
            fi
            APP_PHASE=running
            return 0
        fi
        managed_wake_checks=$((managed_wake_checks - 1))
        managed_wait_tick
    done
    return 1
}

cold_sleep_application() {
    APP_PHASE=stopping
    stop_application
    release_display
    if marker_matches_application ordinary-exit; then
        exit "$APP_STATUS"
    fi
    if marker_matches_application state-open; then
        # A failed save or forced termination must not reopen an older
        # checkpoint as if this sleep cycle had saved it successfully.
        [ "$APP_STATUS" -eq 0 ] || exit "$APP_STATUS"
        marker_matches_application resume-ready ||
            fail 'The application did not acknowledge a saved sleep session.'
        SLEEP_RESUME_INTENT=1
        SLEEP_NATIVE_RETURN_INTENT=0
        SLEEP_NATIVE_RETURN_RESULT=closed
        if native_request_kind; then
            SLEEP_NATIVE_RETURN_INTENT=1
            SLEEP_NATIVE_RETURN_RESULT=failed
        fi
    else
        marker_matches_application sleep-request ||
            fail 'The initialization interruption has no matching sleep request.'
        case "$APP_STATUS" in
            0|143) ;;
            *) exit "$APP_STATUS" ;;
        esac
        SLEEP_RESUME_INTENT=$APP_RESUME_INTENT
        SLEEP_NATIVE_RETURN_INTENT=$APP_NATIVE_RETURN_INTENT
        SLEEP_NATIVE_RETURN_RESULT=$APP_NATIVE_RETURN_RESULT
        launcher_log '%s\n' 'weread: Initialization interrupted; the next wake will retry its original startup intent.'
    fi
    APP_PHASE=sleeping
}

native_request_kind() {
    native_requested_kind=''
    if marker_matches_application native-wifi-request; then
        native_requested_kind=wifi
    fi
    if marker_matches_application native-quick-request; then
        [ -z "$native_requested_kind" ] || return 1
        native_requested_kind=quick
    fi
    [ -n "$native_requested_kind" ]
}

native_window_tree() {
    owned_native_command "$NATIVE_WINDOW_OUTPUT_LIMIT" env DISPLAY=:0 xwininfo -root -tree || return 1
    cat "$LOCK_DIR/native-theme-output" 3>&-
}

configure_native_settings() {
    # These interfaces were reviewed in the Scribe 5.20.1 stock firmware.
    # A matching 5.19.5 exported interface is an experimental device gate,
    # not a claim that static firmware review verifies physical behavior.
    case "$PROFILE:$FIRMWARE" in
        scribe:5.19.5|scribe:5.20.1) ;;
        *) return 0 ;;
    esac
    command -v lipc-set-prop >/dev/null 2>&1 || return 0
    command -v lipc-get-prop >/dev/null 2>&1 || return 0
    command -v xwininfo >/dev/null 2>&1 || return 0
    command -v usleep >/dev/null 2>&1 || return 0
    if native_window_tree >"$LOCK_DIR/native-baseline" &&
        grep -q 'MapState=' "$LOCK_DIR/native-baseline"; then
        NATIVE_SETTINGS_SUPPORTED=1
    else
        launcher_log '%s\n' 'weread: Native settings are unavailable because native window visibility cannot be observed.'
    fi
}

observe_native_settings() {
    trap - 0 HUP INT TERM
    WRAPPER_PID=''
    WRAPPER_BIRTH=''
    native_kind=$1
    native_cycle=$2
    native_baseline_app=$3
    native_baseline_view=$4
    native_child=''
    native_child_birth=''
    native_stopping=0
    native_interrupted=0
    native_exit_reported=0
    finish_native_observer() {
        native_exit_status=$1
        trap - 0
        trap '' HUP INT TERM
        if ! cleanup_wrapper_child "$native_child" "$native_child_birth" "$WRAPPER_PID"; then
            native_exit_status=1
        fi
        if [ "$native_stopping" -eq 0 ] && [ "$native_exit_reported" -eq 0 ]; then
            printf '_weread_native_error %s\n' "$native_cycle" >&3 || :
        fi
        exit "$native_exit_status"
    }
    trap 'finish_native_observer "$?"' 0
    stop_native_child() {
        native_stopping=1
        native_interrupted=1
        if [ -n "$native_child" ]; then
            signal_owned_process "$native_child" "$native_child_birth" "$WRAPPER_PID" TERM || :
        fi
    }
    trap stop_native_child HUP INT TERM
    capture_wrapper_identity || exit 1
    native_command() {
        "$@" 3>&- 6>&- &
        native_child=$!
        if capture_wrapper_child_birth "$native_child" 2>/dev/null; then
            native_child_birth=$WRAPPER_CHILD_BIRTH
        elif ! owned_child_has_exited "$native_child" '' "$WRAPPER_PID"; then
            launcher_log '%s\n' 'weread: Native observer child identity could not be captured before publication.'
            report_native_error
            native_stopping=1
            return 1
        fi
        if [ -n "$native_child_birth" ] &&
            ! printf '%s\n' "$native_child" >"$LOCK_DIR/native-child-pid"; then
            if cleanup_wrapper_child "$native_child" "$native_child_birth" "$WRAPPER_PID"; then
                native_child=''
                native_child_birth=''
            fi
            report_native_error
            native_stopping=1
            return 1
        fi
        if [ "$native_stopping" -eq 1 ]; then
            signal_owned_process "$native_child" "$native_child_birth" "$WRAPPER_PID" TERM || :
        fi
        while :; do
            native_interrupted=0
            if wait "$native_child"; then
                native_command_status=0
            else
                native_command_status=$?
            fi
            [ "$native_interrupted" -eq 1 ] || break
        done
        native_child=''
        native_child_birth=''
        [ "$native_stopping" -eq 0 ] && [ "$native_command_status" -eq 0 ]
    }
    report_native_error() {
        [ "$native_exit_reported" -eq 0 ] || return 0
        printf '%s:%s\n' "$native_cycle" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-observer-finished" || :
        printf '_weread_native_error %s\n' "$native_cycle" >&3 || :
        native_exit_reported=1
    }
    native_result=closed
    native_seen=0
    if [ "${5:-open}" = monitor ]; then
        native_seen=1
        native_result=${6:-closed}
    fi
    native_open_checks=100
    native_quiet_checks=0
    native_query_errors=0
    native_navigation_errors=0
    # The stock search module opens this exact Wi-Fi dialog. The stock status
    # bar sends openedByTap; neither setter waits for the user's interaction.
    if [ "${5:-open}" = monitor ]; then
        :
    elif [ "$native_kind" = wifi ]; then
        if ! native_command lipc-set-prop -s com.lab126.pillow customDialog '{"name":"wifi_wizard_dialog","clientParams":{"show":true}}'; then
            report_native_error
            return 0
        fi
    elif ! native_command lipc-set-prop -s com.lab126.kppchromebar configurePillState openedByTap; then
        report_native_error
        return 0
    fi
    while [ "$native_stopping" -eq 0 ]; do
        if native_command env DISPLAY=:0 xwininfo -root -tree >"$LOCK_DIR/native-current" 2>/dev/null &&
            grep -q 'MapState=' "$LOCK_DIR/native-current"; then
            native_query_errors=0
        else
            native_query_errors=$((native_query_errors + 1))
            if [ "$native_query_errors" -ge 3 ]; then
                report_native_error
                return 0
            fi
            native_command usleep 100000 || :
            continue
        fi
        if [ "$native_kind" = wifi ]; then
            native_target='pillow-wifi-wizard-dialog'
        else
            native_target='com.lab126.kppQuickSettings'
        fi
        if awk -v target="$native_target" '
            /MapState=IsViewable/ && index($0, target) { found = 1 }
            END { exit !found }
        ' "$LOCK_DIR/native-current"; then
            native_seen=1
            native_quiet_checks=0
        fi
        if [ "$native_seen" -eq 0 ]; then
            native_open_checks=$((native_open_checks - 1))
            if [ "$native_open_checks" -eq 0 ]; then
                # An unobserved open may still be delayed. No supported cancel
                # interface is known, so leave native display and input alone.
                report_native_error
                return 0
            fi
        fi
        if [ "$native_seen" -eq 1 ] &&
            native_windows_are_closed "$LOCK_DIR/native-current"; then
            # Window visibility is sufficient while native UI owns the screen.
            # Query navigation only to validate a possible return to the App.
            native_active_app=''
            native_active_view=''
            if native_command lipc-get-prop -s com.lab126.appmgrd activeApp >"$LOCK_DIR/native-active-app" &&
                native_command lipc-get-prop -s com.lab126.appmgrd activeView >"$LOCK_DIR/native-active-view"; then
                native_navigation_errors=0
                native_active_app=$(cat "$LOCK_DIR/native-active-app")
                native_active_view=$(cat "$LOCK_DIR/native-active-view")
            else
                native_navigation_errors=$((native_navigation_errors + 1))
                if [ "$native_navigation_errors" -ge 3 ]; then
                    report_native_error
                    return 0
                fi
            fi
            if [ "$native_active_app" = "$native_baseline_app" ] &&
                [ "$native_active_view" = "$native_baseline_view" ]; then
                native_quiet_checks=$((native_quiet_checks + 1))
                if [ "$native_quiet_checks" -ge 5 ]; then
                    printf '%s:%s\n' "$native_cycle" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-observer-finished"
                    printf '_weread_native_closed %s %s\n' "$native_cycle" "$native_result" >&3
                    native_exit_reported=1
                    return 0
                fi
            else
                native_quiet_checks=0
            fi
        else
            native_quiet_checks=0
            native_navigation_errors=0
        fi
        native_command usleep 100000 || :
    done
    printf '%s:%s\n' "$native_cycle" "$LAUNCH_TOKEN" >"$LOCK_DIR/native-observer-finished"
}

native_windows_are_closed() {
    guard_owned_window=''
    if frame_guard_is_current; then guard_owned_window=$FRAME_GUARD_WINDOW; fi
    awk -v owned_guard="$guard_owned_window" '
        owned_guard != "" && $1 == owned_guard { next }
        NR == FNR { if (/MapState=IsViewable/) baseline[$1] = 1; next }
        /MapState=IsViewable/ {
            if (index($0, "pillow-wifi-wizard-dialog") || index($0, "com.lab126.kppQuickSettings")) busy = 1
            if (!baseline[$1] && ($0 ~ /"L:(D|AD|SD|KB)_/ || $0 ~ /_N:dialog_/)) busy = 1
        }
        END { exit busy }
    ' "$LOCK_DIR/native-baseline" "$1"
}

native_return_is_closed() {
    query_native_property com.lab126.appmgrd activeApp || return 1
    [ "$NATIVE_PROPERTY_VALUE" = "$NATIVE_BASELINE_APP" ] || return 1
    query_native_property com.lab126.appmgrd activeView || return 1
    [ "$NATIVE_PROPERTY_VALUE" = "$NATIVE_BASELINE_VIEW" ] &&
        native_window_tree >"$LOCK_DIR/native-return-check" &&
        grep -q 'MapState=' "$LOCK_DIR/native-return-check" &&
        native_windows_are_closed "$LOCK_DIR/native-return-check"
}

stop_native_observer() {
    native_observer_stop_status=0
    if [ -n "$NATIVE_OBSERVER_PID" ]; then
        native_wrapper_birth=$(process_birth "$NATIVE_OBSERVER_PID" 2>/dev/null || :)
        signal_owned_process "$NATIVE_OBSERVER_PID" "$native_wrapper_birth" "$$" TERM || :
        sleep 1 || native_observer_stop_status=1
        signal_recorded_child "$LOCK_DIR/native-child-pid" KILL "$NATIVE_OBSERVER_PID"
        cleanup_wrapper_child "$NATIVE_OBSERVER_PID" "$native_wrapper_birth" "$$" || native_observer_stop_status=1
        NATIVE_OBSERVER_PID=''
    fi
    rm -f "$LOCK_DIR/native-child-pid" "$LOCK_DIR/native-observer-finished" || native_observer_stop_status=1
    return "$native_observer_stop_status"
}

monitor_native_settings() {
    native_monitor_result=$1
    NATIVE_GENERATION=$((NATIVE_GENERATION + 1))
    NATIVE_RETURN_PENDING=''
    rm -f "$LOCK_DIR/native-observer-finished" "$LOCK_DIR/native-child-pid"
    observe_native_settings "$NATIVE_KIND" "$NATIVE_GENERATION" "$NATIVE_BASELINE_APP" "$NATIVE_BASELINE_VIEW" monitor "$native_monitor_result" 6>&- &
    NATIVE_OBSERVER_PID=$!
}

resume_from_native() {
    native_resume_result=$1
    stop_native_observer
    NATIVE_RETURN_PENDING=''
    NATIVE_SLEEPING=0
    if ! native_return_is_closed; then
        monitor_native_settings "$native_resume_result"
        return 0
    fi
    take_display || return 1
    # A pending close can predate a sleep or native navigation. Recheck after
    # suspending native input dispatch and before any App process can draw.
    if ! native_return_is_closed; then
        release_display
        monitor_native_settings "$native_resume_result"
        return 0
    fi
    start_application 1 1 "$native_resume_result"
}

begin_native_settings() {
    native_open_kind=$1
    [ "$NATIVE_SETTINGS_SUPPORTED" -eq 1 ] || fail 'Native settings have no supported window-observation interface.'
    release_display || return 1
    native_baseline_app=''
    native_baseline_view=''
    if query_native_property com.lab126.appmgrd activeApp; then native_baseline_app=$NATIVE_PROPERTY_VALUE; fi
    if query_native_property com.lab126.appmgrd activeView; then native_baseline_view=$NATIVE_PROPERTY_VALUE; fi
    if [ -z "$native_baseline_app" ] || [ -z "$native_baseline_view" ] ||
        ! native_window_tree >"$LOCK_DIR/native-baseline" ||
        ! grep -q 'MapState=' "$LOCK_DIR/native-baseline"; then
        # Native UI was never opened. Reacquire a fresh guard before starting
        # the saved App generation after this intentional native handback.
        take_display || return 1
        start_application 1 1 failed
        return 0
    fi
    APP_PHASE=native
    NATIVE_KIND=$native_open_kind
    NATIVE_BASELINE_APP=$native_baseline_app
    NATIVE_BASELINE_VIEW=$native_baseline_view
    NATIVE_GENERATION=$((NATIVE_GENERATION + 1))
    NATIVE_SLEEPING=0
    NATIVE_RETURN_PENDING=''
    NATIVE_POWER_FAILURES=0
    rm -f "$LOCK_DIR/native-observer-finished" "$LOCK_DIR/native-child-pid"
    observe_native_settings "$native_open_kind" "$NATIVE_GENERATION" "$native_baseline_app" "$native_baseline_view" 6>&- &
    NATIVE_OBSERVER_PID=$!
}

note_cleanup_failure() {
    CLEANUP_FAILED=1
    NATIVE_DARK_EXIT_ALLOWED=0
    # Each call names one fixed cleanup step. Diagnostics cannot grow with an
    # untrusted pathname or prevent the remaining resources from being released.
    launcher_log 'weread: Cleanup failed for %s; continuing remaining resources.\n' "$1"
}

cleanup() {
    trap '' HUP INT TERM
    trap - 0
    saved_status=$1
    if [ "$LAUNCH_STARTUP_SIGNAL" -ne 0 ]; then saved_status=$LAUNCH_STARTUP_SIGNAL; fi
    # Failure to acquire the lock owns no records, descriptors, or services.
    # Never use this cleanup path to repair another launcher's private state.
    if [ "$LOCK_OWNED" -ne 1 ]; then exit "$saved_status"; fi
    CLEANUP_RUNNING=1
    CLEANUP_FAILED=0
    if [ "$saved_status" -ne 0 ]; then NATIVE_DARK_EXIT_ALLOWED=0; fi
    # Conditional calls suppress POSIX errexit inside a function. These helpers
    # use explicit ownership/permission gates, and each reports its own failure
    # after attempting independent child and record cleanup.
    if ! stop_native_theme_command; then note_cleanup_failure 'native theme helper'; fi
    if ! stop_wake_probe; then note_cleanup_failure 'wake probe'; fi
    if ! stop_native_observer; then note_cleanup_failure 'native observer'; fi
    if ! stop_observer; then note_cleanup_failure 'power observer'; fi
    if ! stop_application; then note_cleanup_failure 'application'; fi
    if [ "$APP_STATUS" -ne 0 ] && [ "$saved_status" -eq 0 ]; then
        saved_status=$APP_STATUS
        NATIVE_DARK_EXIT_ALLOWED=0
    fi
    if ! release_display; then note_cleanup_failure 'native display'; fi
    if [ "$EVENT_FD_OPEN" -eq 1 ]; then
        if { exec 3>&-; }; then
            EVENT_FD_OPEN=0
        else
            note_cleanup_failure 'power event descriptor'
        fi
    fi
    if ! rm -f "$LOCK_DIR/pid" "$LOCK_DIR/power-events" "$LOCK_DIR/nonce" "$LOCK_DIR/generation" "$LOCK_DIR/sleep-request" "$LOCK_DIR/resume-ready" "$LOCK_DIR/ordinary-exit" "$LOCK_DIR/state-open" "$LOCK_DIR/resume-ready.pending" "$LOCK_DIR/ordinary-exit.pending" "$LOCK_DIR/state-open.pending" "$LOCK_DIR/state-open.pending" "$LOCK_DIR/native-wifi-request" "$LOCK_DIR/native-quick-request" "$LOCK_DIR/native-wifi-request.pending" "$LOCK_DIR/native-quick-request.pending" "$LOCK_DIR/native-return" "$LOCK_DIR/native-wifi-available" "$LOCK_DIR/native-quick-available" "$LOCK_DIR/native-baseline" "$LOCK_DIR/native-current" "$LOCK_DIR/native-active-app" "$LOCK_DIR/native-active-view" "$LOCK_DIR/native-return-check" "$LOCK_DIR/application-birth" "$LOCK_DIR/application-owned" "$LOCK_DIR/managed-sleep-available" "$LOCK_DIR/managed-sleep-available.pending" "$LOCK_DIR/managed-sleep-request" "$LOCK_DIR/managed-sleep-request.pending" "$LOCK_DIR/managed-sleep-ready" "$LOCK_DIR/managed-sleep-ready.pending" "$LOCK_DIR/managed-wake-request" "$LOCK_DIR/managed-wake-request.pending" "$LOCK_DIR/managed-wake-ready" "$LOCK_DIR/managed-wake-ready.pending" "$LOCK_DIR/native-epdc-mode" "$LOCK_DIR/native-epdc-mode.pending" "$LOCK_DIR/native-portrait-calibration" "$LOCK_DIR/native-portrait-calibration.pending" "$LOCK_DIR/managed-native-epdc-mode" "$LOCK_DIR/managed-native-epdc-mode.pending" "$LOCK_DIR/managed-native-portrait-calibration" "$LOCK_DIR/managed-native-portrait-calibration.pending" "$LOCK_DIR/native-dark-target" "$LOCK_DIR/native-dark-target.pending" "$LOCK_DIR/native-theme-output" "$LOCK_DIR/native-theme-done" "$LOCK_DIR/native-theme-done.pending"; then
        note_cleanup_failure 'launcher records'
    fi
    if ! rm -f "$LOCK_DIR/native-theme-go" "$LOCK_DIR/native-theme-go.pending" "$LOCK_DIR/native-portrait-step" "$LOCK_DIR/native-portrait-step.pending"; then note_cleanup_failure 'native theme permission records'; fi
    if ! rm -f "$LOCK_DIR/wake-probe-child" "$LOCK_DIR/wake-probe-child.pending"; then note_cleanup_failure 'wake probe records'; fi
    if ! rm -f "$LOCK_DIR/native-theme-available" "$LOCK_DIR/native-theme-available.pending" "$LOCK_DIR/native-theme-request" "$LOCK_DIR/native-theme-request.pending" "$LOCK_DIR/native-theme-result" "$LOCK_DIR/native-theme-result.pending" "$LOCK_DIR/native-theme-ready" "$LOCK_DIR/native-theme-ready.pending" "$LOCK_DIR/native-theme-interrupt" "$LOCK_DIR/native-theme-interrupt.pending" "$LOCK_DIR/native-theme-power-epoch" "$LOCK_DIR/native-theme-power-epoch.pending" "$LOCK_DIR/native-theme-portrait-calibration" "$LOCK_DIR/native-theme-portrait-calibration.pending"; then note_cleanup_failure 'native theme handshake records'; fi
    if rmdir "$LOCK_DIR" 2>/dev/null; then
        LOCK_OWNED=0
    else
        note_cleanup_failure 'launcher lock directory'
    fi
    if [ "$CLEANUP_FAILED" -ne 0 ] && [ "$saved_status" -eq 0 ]; then saved_status=1; fi
    exit "$saved_status"
}

begin_owned_registration() {
    # Clear before enabling deferral: a signal in between exits before any fork.
    LAUNCH_STARTUP_SIGNAL=0
    LAUNCH_REGISTRATION_ACTIVE=1
}

finish_owned_registration() {
    LAUNCH_REGISTRATION_ACTIVE=0
    # The handler also preserves this latch after deferral ends, so a second
    # terminal signal cannot override the first code at this command boundary.
    if [ "$LAUNCH_STARTUP_SIGNAL" -ne 0 ]; then exit "$LAUNCH_STARTUP_SIGNAL"; fi
}

launcher_signal_exit() {
    # Do not permit a different nested signal to replace the first terminal
    # intent while the shell is installing its in-memory latch.
    trap '' HUP INT TERM
    if [ "$LAUNCH_STARTUP_SIGNAL" -eq 0 ]; then LAUNCH_STARTUP_SIGNAL=$1; fi
    if [ "$LAUNCH_REGISTRATION_ACTIVE" -eq 1 ]; then
        trap 'launcher_signal_exit 129' HUP
        trap 'launcher_signal_exit 130' INT
        trap 'launcher_signal_exit 143' TERM
        return 0
    fi
    exit "$LAUNCH_STARTUP_SIGNAL"
}

trap 'cleanup "$?"' 0
trap 'launcher_signal_exit 129' HUP
trap 'launcher_signal_exit 130' INT
trap 'launcher_signal_exit 143' TERM

# Acquire ownership only after all state, helpers, and cleanup traps exist.
# A catchable signal between mkdir and registration retains its first status
# until the newly created directory has an explicit cleanup owner.
begin_owned_registration
if mkdir "$LOCK_DIR" 2>/dev/null; then
    LOCK_OWNED=1
else
    finish_owned_registration
    fail 'Another launcher owns the display lock.'
fi
finish_owned_registration

if [ "${2:-}" = --managed-logs ]; then
    # Only the normal detached entry redirects itself. Direct --detached callers
    # retain their capture descriptors, including the isolated lifecycle fixtures.
    exec >>"$DATA_DIR/logs/launcher.log" 2>&1
    MANAGED_LOGS_ACTIVE=1
    launcher_log '%s' ''
fi

# Even an initial state write can fail when temporary storage is exhausted.
# Install owned-resource cleanup before publishing the supervisor identity.
printf '%s\n' "$$" >"$LOCK_DIR/pid"
command -v lipc-wait-event >/dev/null 2>&1 || fail 'Power event observation is required.'
LAUNCH_TOKEN=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
[ "${#LAUNCH_TOKEN}" -eq 32 ] || fail 'The launcher context token could not be generated.'
printf '%s\n' "$LAUNCH_TOKEN" >"$LOCK_DIR/nonce"
mkfifo "$LOCK_DIR/power-events"
mkfifo "$LOCK_DIR/observer-events"
# Keep one descriptor open across cycles. Wake events remain queued even while
# the supervisor waits for a successful, persistent application shutdown.
exec 3<>"$LOCK_DIR/power-events"
EVENT_FD_OPEN=1
start_power_observer || fail 'Power observer startup failed before a proven subscription was permitted.'

sleep 2
export WEREAD_HOME
configure_native_settings
take_display
start_application 0

while IFS= read -r power_event <&3; do
    # A full event FIFO can reject the watcher's nonblocking notification;
    # any already queued event must still detect the retired original helper.
    if [ -n "$FRAME_GUARD_PID" ]; then
        frame_guard_is_current || fail 'The retained frame helper exited; restoring native ownership.'
    fi
    case "$power_event" in
        goingToScreenSaver*)
            SLEEP_GENERATION=$((SLEEP_GENERATION + 1))
            WAKE_PENDING=0
            stop_wake_probe
            if [ "$APP_PHASE" = running ]; then
                APP_PHASE=stopping
                # The application generation stays constant for resident
                # cycles, so every durable and volatile acknowledgment must
                # be refreshed rather than inherited from the last sleep.
                rm -f "$LOCK_DIR/resume-ready" "$LOCK_DIR/resume-ready.pending" "$LOCK_DIR/managed-sleep-request" "$LOCK_DIR/managed-sleep-request.pending" "$LOCK_DIR/managed-sleep-ready" "$LOCK_DIR/managed-sleep-ready.pending" "$LOCK_DIR/managed-wake-request" "$LOCK_DIR/managed-wake-request.pending" "$LOCK_DIR/managed-wake-ready" "$LOCK_DIR/managed-wake-ready.pending" "$LOCK_DIR/managed-native-epdc-mode" "$LOCK_DIR/managed-native-epdc-mode.pending" "$LOCK_DIR/managed-native-portrait-calibration" "$LOCK_DIR/managed-native-portrait-calibration.pending"
                printf '%s:%s\n' "$APP_GENERATION" "$LAUNCH_TOKEN" >"$LOCK_DIR/sleep-request"
                if attempt_managed_sleep; then
                    release_display retain-frame
                else
                    cold_sleep_application
                fi
            elif [ "$APP_PHASE" = native ]; then
                NATIVE_SLEEPING=1
            fi
            ;;
        outOfScreenSaver*)
            if { [ "$APP_PHASE" = sleeping ] || [ "$APP_PHASE" = suspended ] || { [ "$APP_PHASE" = native ] && [ "$NATIVE_SLEEPING" -eq 1 ]; }; } && [ "$WAKE_PENDING" -eq 0 ]; then
                WAKE_PENDING=1
                WAKE_PROBES_REMAINING=3
                start_wake_probe "$SLEEP_GENERATION" 2 || fail 'Wake probe startup failed; native ownership is preserved.'
            fi
            ;;
        exitingScreenSaver*)
            if { [ "$APP_PHASE" = sleeping ] || [ "$APP_PHASE" = suspended ] || { [ "$APP_PHASE" = native ] && [ "$NATIVE_SLEEPING" -eq 1 ]; }; } && [ "$WAKE_PENDING" -eq 1 ]; then
                resume_application
            fi
            ;;
        usbConfigured*)
            # USB ownership ends this launch, including a pending wake.
            exit 143
            ;;
        '_weread_frame_guard_exit '*)
            set -- $power_event
            if [ "$#" -eq 5 ] && [ "$2" = "$LAUNCH_TOKEN" ] &&
                [ "$3:$4" = "$FRAME_GUARD_PID:$FRAME_GUARD_BIRTH" ]; then
                fail 'The retained frame helper exited; restoring native ownership.'
            fi
            ;;
        '_weread_app_started '*)
            set -- $power_event
            if [ "$#" -eq 4 ] && [ "$2" = "$APP_GENERATION" ]; then
                capture_application_identity "$3" "$4" || :
            fi
            ;;
        '_weread_native_theme_request '*)
            set -- $power_event
            if [ "$#" -eq 3 ] && [ "$2" = "$APP_GENERATION" ]; then
                if ! begin_native_theme_request "$2" "$3"; then
                    launcher_log 'weread: Native theme handoff failed stage=%s; restoring native ownership.\n' "$NATIVE_THEME_STAGE"
                    fail 'Native theme handoff could not complete safely.'
                fi
            fi
            ;;
        '_weread_app_exit '*)
            set -- $power_event
            if [ "$2" = "$APP_GENERATION" ] && { [ "$APP_PHASE" = running ] || [ "$APP_PHASE" = suspended ]; }; then
                if wait "$APP_RUNNER_PID" 2>/dev/null; then
                    APP_STATUS=0
                else
                    APP_STATUS=$?
                fi
                APP_RUNNER_PID=''
                if marker_matches_application ordinary-exit; then
                    if [ "$APP_PHASE" = running ] && [ "$APP_STATUS" -eq 0 ] && [ "$3" = 0 ]; then
                        NATIVE_DARK_EXIT_ALLOWED=1
                    fi
                    exit "$APP_STATUS"
                fi
                if native_request_kind; then
                    [ "$APP_STATUS" -eq 0 ] || exit "$APP_STATUS"
                    marker_matches_application state-open || fail 'The native settings request has no initialized application state.'
                    marker_matches_application resume-ready || fail 'The application did not acknowledge a saved native return session.'
                    begin_native_settings "$native_requested_kind"
                else
                    exit "$APP_STATUS"
                fi
            fi
            ;;
        '_weread_native_closed '*)
            set -- $power_event
            if [ "$2" = "$NATIVE_GENERATION" ] && [ "$APP_PHASE" = native ]; then
                case "$3" in closed|failed) ;; *) fail 'The native return result is invalid.' ;; esac
                wait "$NATIVE_OBSERVER_PID" 2>/dev/null || :
                NATIVE_OBSERVER_PID=''
                rm -f "$LOCK_DIR/native-child-pid" "$LOCK_DIR/native-observer-finished"
                NATIVE_RETURN_PENDING=$3
                if [ "$NATIVE_SLEEPING" -eq 0 ]; then
                    native_return_power=''
                    if query_native_property com.lab126.powerd state; then native_return_power=$NATIVE_PROPERTY_VALUE; fi
                    case "$native_return_power" in
                        active)
                            NATIVE_POWER_FAILURES=0
                            resume_from_native "$NATIVE_RETURN_PENDING"
                            ;;
                        '')
                            NATIVE_POWER_FAILURES=$((NATIVE_POWER_FAILURES + 1))
                            [ "$NATIVE_POWER_FAILURES" -lt 3 ] ||
                                fail 'Native return power state is unavailable; native ownership is preserved.'
                            monitor_native_settings "$NATIVE_RETURN_PENDING"
                            ;;
                        *) NATIVE_SLEEPING=1 ;;
                    esac
                fi
            fi
            ;;
        '_weread_native_error '*)
            set -- $power_event
            if [ "$2" = "$NATIVE_GENERATION" ] && [ "$APP_PHASE" = native ]; then
                fail 'Native window observation failed; native display and input ownership are preserved.'
            fi
            ;;
        '_weread_wake_check '*)
            set -- $power_event
            if [ "$#" -eq 4 ] && [ "$2" = "$SLEEP_GENERATION" ] &&
                [ "$3:$4" = "$WAKE_PROBE_PID:$WAKE_PROBE_BIRTH" ] &&
                { [ "$APP_PHASE" = sleeping ] || [ "$APP_PHASE" = suspended ] || { [ "$APP_PHASE" = native ] && [ "$NATIVE_SLEEPING" -eq 1 ]; }; } && [ "$WAKE_PENDING" -eq 1 ]; then
                stop_wake_probe || fail 'Wake probe completion could not be cleaned up; native ownership is preserved.'
                # Some firmware paths omit ExitingSS. A timer or RTC wake is
                # never sufficient: require OutOfSS plus an explicit active
                # powerd state before reclaiming native display or input.
                power_state=''
                if command -v lipc-get-prop >/dev/null 2>&1; then
                    if query_native_property com.lab126.powerd state; then power_state=$NATIVE_PROPERTY_VALUE; fi
                fi
                if [ "$power_state" = active ]; then
                    resume_application
                else
                    probe_delay=2
                    if [ "$WAKE_PROBES_REMAINING" -gt 0 ]; then
                        WAKE_PROBES_REMAINING=$((WAKE_PROBES_REMAINING - 1))
                        if [ "$WAKE_PROBES_REMAINING" -eq 0 ]; then
                            launcher_log '%s\n' 'weread: Waiting for native unlock or screen-saver dismissal.'
                        fi
                    fi
                    if [ "$WAKE_PROBES_REMAINING" -eq 0 ]; then
                        probe_delay=10
                    fi
                    # Preserve a real wake intent even when native dismissal
                    # takes longer than the first few checks. A later sleep,
                    # USB takeover or launcher signal cancels this observer.
                    start_wake_probe "$SLEEP_GENERATION" "$probe_delay" || fail 'Wake probe replacement failed; native ownership is preserved.'
                fi
            fi
            ;;
        '_weread_wake_probe_error '*)
            set -- $power_event
            if [ "$#" -eq 4 ] && [ "$2" = "$SLEEP_GENERATION" ] &&
                [ "$3:$4" = "$WAKE_PROBE_PID:$WAKE_PROBE_BIRTH" ] &&
                { [ "$APP_PHASE" = sleeping ] || [ "$APP_PHASE" = suspended ] || { [ "$APP_PHASE" = native ] && [ "$NATIVE_SLEEPING" -eq 1 ]; }; } && [ "$WAKE_PENDING" -eq 1 ]; then
                fail 'Wake probe failed; native ownership is preserved.'
            fi
            ;;
        '_weread_observer_exit '*)
            fail 'Power event observation ended unexpectedly.'
            ;;
    esac
done

fail 'The power event channel closed unexpectedly.'
