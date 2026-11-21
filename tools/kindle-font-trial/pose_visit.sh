cycle_marker() { plain "$lock/$1" && printf '%s:1\n' "$generation" | cmp -s - "$lock/$1"; }
same_gui() {
    gui_args && [ "$gui:$birth" = "$identity" ] && plain "$lock/generation" &&
        printf '%s\n' "$generation" | cmp -s - "$lock/generation" &&
        plain "$lock/application-owned" && printf '%s:%s\n' "$generation" "$identity" | cmp -s - "$lock/application-owned";
}
power_state() { lipc-get-prop -s com.lab126.powerd state 2>/dev/null; }
task_state() { awk '{print $3}' "/proc/$gui/stat" 2>/dev/null; }
public_foreground() {
    [ "$(power_state)" = active ] && same_gui || return 1
    case "$(task_state)" in S|R) ;; *) return 1;; esac
    for peer in aidoku bilicomics; do
        absent "/tmp/$peer-launcher.lock" && ! pidof "$peer" >/dev/null || return 1
    done
    managers=$(pidof awesome) && [ -n "$managers" ] || return 1
    for manager in $managers; do [ "$(awk '{print $3}' "/proc/$manager/stat")" = T ] || return 1; done
    display_open=0; input_open=0
    for descriptor in "/proc/$gui/fd/"*; do
        case "$(readlink "$descriptor" 2>/dev/null || :)" in /dev/fb0) display_open=1;; /dev/input/event*) input_open=1;; esac
    done
    [ "$display_open:$input_open" = 1:1 ]
}
pose_recover_wake() {
    out=$stage/out/B-reader
    if ! plain "$out/power-intent" || ! plain "$out/power-owner"; then return 0; fi
    printf 'power-cycle\n' | cmp -s - "$out/power-intent" || return 1
    [ "$(power_state)" != screenSaver ] && return 0
    name=B-reader; home=$stage/B
    gui_args && plain "$lock/generation" && plain "$lock/application-owned" || return 1
    generation=$(cat "$lock/generation"); identity=$gui:$birth
    cmp -s "$out/power-owner" "$lock/application-owned" && same_gui && cycle_marker managed-sleep-request && cycle_marker managed-sleep-ready || return 1
    for attempt in 1 2; do
        [ "$(power_state)" = screenSaver ] && same_gui && cycle_marker managed-sleep-ready || break
        lipc-set-prop -i com.lab126.powerd wakeUp 1 >/dev/null 2>&1 || :
        for tick in $(seq 1 10); do [ "$(power_state)" = active ] && return 0; usleep 200000; done
    done
    [ "$(power_state)" = active ]
}
pose_wake() {
    if [ "$(power_state)" != active ] || ! command -v powerd_test >/dev/null 2>&1 ||
       ! command -v lipc-set-prop >/dev/null 2>&1; then printf 'not_run power_state_unavailable\n'; return; fi
    same_gui && plain "$lock/managed-sleep-available" && cmp -s "$lock/generation" "$lock/managed-sleep-available" &&
        absent "$lock/managed-sleep-request" && absent "$lock/managed-wake-request" || { printf 'not_run guardian_unavailable\n'; return; }
    printf '%s:%s\n' "$generation" "$identity" >"$out/power-owner"
    "$stage/pose-wake" guarded-cycle "$out" "$lock" </dev/null >"$out/power-guard" 2>"$out/power-guard.log" &
    guardian=$!; observed=0
    for tick in $(seq 1 36); do
        [ ! -s "$out/power-guard" ] || break
        if [ "$(power_state)" = screenSaver ] && [ "$(task_state)" = T ] && same_gui &&
                cycle_marker managed-sleep-request && cycle_marker managed-sleep-ready; then observed=1; break; fi
        usleep 200000
    done
    guard_status=0; wait "$guardian" || guard_status=$?
    guard_out=$(cat "$out/power-guard")
    case "$guard_status:$guard_out" in
        '77:not_run alarm_unavailable'|'77:not_run wake_command_unavailable'|'77:not_run guardian_unavailable'|'77:not_run power_state_unavailable') printf '%s\n' "$guard_out"; return;;
        '0:requested') ;;
        *) printf 'not_qualified power_cycle_unconfirmed\n'; return;;
    esac
    if [ "$observed" != 1 ]; then printf 'not_qualified sleep_handshake_unobserved\n'; return; fi
    for tick in $(seq 1 48); do
        if [ "$(power_state)" = active ] && same_gui && cycle_marker managed-wake-request && cycle_marker managed-wake-ready; then
            case "$(task_state)" in S|R) break;; esac
        fi
        usleep 150000
    done
    if ! { [ "$(power_state)" = active ] && same_gui && cycle_marker managed-wake-request && cycle_marker managed-wake-ready; }; then
        printf 'not_qualified wake_handshake_unobserved\n'; return
    fi
    case "$(task_state)" in S|R) ;; *) printf 'not_qualified wake_handshake_unobserved\n'; return;; esac
    branch=native; wake_pose=unobserved
    if cycle_marker managed-cover-request && cycle_marker managed-cover-ready; then branch=cover; fi
    if plain "$lock/managed-native-portrait-calibration"; then
        record=$(cat "$lock/managed-native-portrait-calibration")
        case "$record" in "$generation:1:"[UD]:[0-3]) wake_pose=$(printf '%s\n' "$record" | cut -d: -f4-5);;
            *) printf 'not_qualified wake_handshake_unobserved\n'; return;; esac
    fi
    public_foreground && "$stage/measure" panel >"$out/resumed.pgm" && public_foreground || { rm -f "$out/resumed.pgm"; fail wake_picture; }
    printf 'observed screenSaver stopped same_process wake_ready active %s %s\n' "$branch" "$wake_pose"
}
pose_case() {
    [ "$name" = B-reader ] || exit 64
    prepare_case
    "$stage/pose-splash" "$stage" B-reader </dev/null >"$out/splash.reading" 2>"$out/splash.log" &
    watcher=$!
    "$stage/measure" request "$label" >"$out/request" || fail request
    log=$home/data/logs/application-bootstrap.log
    for tick in $(seq 1 200); do
        if plain "$log" && grep -q ' first_application_frame_send$' "$log"; then break; fi
        absent "$out/status" || fail no_send
        usleep 125000
    done
    plain "$log" && grep -q ' first_application_frame_send$' "$log" || fail no_send
    sleep 1
    absent "$stage/armed" && plain "$out/entered/arm" && printf '%s\n' "$name" | cmp -s - "$out/entered/arm" || fail entry_identity
    gui_args && live_pose || fail gui_identity_or_pose
    identity=$gui:$birth; generation=$(cat "$lock/generation")
    public_foreground && "$stage/measure" panel >"$out/panel.pgm" && "$stage/measure" thumb >"$out/thumb.pgm" && public_foreground || { rm -f "$out/panel.pgm" "$out/thumb.pgm"; fail pre_sleep_picture; }
    pose_wake >"$out/wake" || fail wake_observation
    pose_recover_wake || fail wake_recovery
    watcher_status=0; wait "$watcher" || watcher_status=$?
    if { [ "$watcher_status" = 0 ] || { [ "$watcher_status" = 20 ] && printf 'splash_refused rotated\n' | cmp -s - "$out/splash.log"; }; } &&
            awk 'NF!=6 || $1!="SPLASH" {bad=1} END{exit bad || NR!=1}' "$out/splash.reading"; then
        cp "$out/splash.reading" "$out/splash"; printf '%s\n' "$watcher_status" >"$out/splash-status"
    else printf 'not_observed\n' >"$out/splash"; printf 'not_observed\n' >"$out/splash-status"; fi
    for tick in $(seq 1 240); do plain "$out/status" && break; usleep 200000; done
    plain "$out/status" && [ "$(cat "$out/status")" = 0 ] && absent "$out/guard" || fail exit
    finish_case
    printf 'completed %s\n' "$name"
}
