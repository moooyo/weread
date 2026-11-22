set -eu
umask 077
export LC_ALL=C
mode=$1 label=$2 bytes=$3 expanded=$4 lane=$5
fail() { printf 'refused %s\n' "$1"; exit 1; }
case "$label" in ''|*[!0-9a-f]*) fail label;; esac
[ "${#label}" = 16 ] || fail label
case "$lane" in A|B) ;; *) fail lane;; esac
for number in "$bytes" "$expanded"; do
    case "$number" in ''|*[!0-9]*) fail size;; esac
    [ "${#number}" -le 9 ] && [ "$number" -le 209715200 ] || fail size
done
volume=/mnt/us
stage=$volume/.weread-turn-$label
lock=/tmp/weread-launcher.lock
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
plain() { [ -f "$1" ] && [ ! -L "$1" ]; }
owned() { [ -d "$stage" ] && [ "$(readlink -f "$stage")" = "$stage" ] && plain "$stage/owner" && [ "$(cat "$stage/owner")" = "$label" ]; }
stage_lock() { owned && exec 9<"$stage" && flock -n 9 && owned; }
close_lock() {
    owned && exec 9<"$stage" || return 1
    for tick in $(seq 1 16); do
        if flock -n 9; then owned; return; fi
        sleep 1
    done
    return 1
}
identity() {
    IFS= read -r rec <"/proc/$1/stat" || return 1
    [ "${rec%% *}" = "$1" ] || return 1
    rest=${rec##*) }; [ "$rest" != "$rec" ] || return 1
    set -- $rest; [ "$#" -ge 20 ] || return 1
    proc_state=$1; shift 19; birth=$1
    case "$birth" in ''|*[!0-9]*|0) return 1;; esac
}
unoccupied() {
    for app in weread aidoku bilicomics; do
        absent "/tmp/$app-launcher.lock" || return 1
        if pidof "$app" >/dev/null 2>&1; then return 1; else [ "$?" = 1 ] || return 1; fi
    done
}
state() {
    state_error=app_or_lease_active; unoccupied || return 1
    state_error=architecture; [ "$(uname -m)" = armv7l ] || return 1
    state_error=model; case "$(dd if=/proc/usid bs=6 count=1 2>/dev/null)" in G??27J|G??2BL|G??263|G??227|G??2BM|G??23L|G??23M|G??270) ;; *) return 1;; esac
    state_error=firmware; [ "$(sed -n 's/^Kindle[[:space:]]*\([0-9][0-9.]*\).*$/\1/p' /etc/prettyversion.txt)" = 5.19.5 ] || return 1
    state_error=power_state; [ "$(lipc-get-prop -s com.lab126.powerd state)" = active ] || return 1
    state_error=foreground; [ "$(lipc-get-prop -s com.lab126.appmgrd activeApp)" = com.lab126.KPPMainApp ] || return 1
    state_error=statusbar; case "$(status statusbar)" in *start/running*) ;; *) return 1;; esac
    state_error=window_manager; managers=$(pidof awesome) && [ -n "$managers" ] || return 1
    for manager in $managers; do identity "$manager" || return 1; case "$proc_state" in R|S) ;; *) return 1;; esac; printf 'manager %s %s\n' "$manager" "$birth"; done
    state_error=input_roster; for node in /sys/class/input/event*; do [ -e "$node" ] || return 1; printf 'input %s\n' "${node##*/}"; done
    state_error=lighting_state
    for prop in flIntensity currentAmberLevel; do
        value=$(lipc-get-prop -i com.lab126.powerd "$prop") || return 1
        case "$value" in ''|*[!0-9]*) return 1;; esac; printf 'light %s %s\n' "$prop" "$value"
    done
    state_error=public_installation; for path in /mnt/us/documents/weread.sh /mnt/us/weread/launch.sh /mnt/us/weread/app/weread /mnt/us/weread/app/weread-frame-guard /mnt/us/weread/app/weread-native-query /mnt/us/aidoku/launch.sh /mnt/us/aidoku/app/aidoku /mnt/us/bilicomics/launch.sh /mnt/us/bilicomics/app/bilicomics; do
        if absent "$path"; then printf 'absent %s\n' "$path"; else plain "$path" && stat -c '%d:%i:%s:%Y:%f' "$path" || return 1; fi
    done
    view=$(lipc-get-prop -s com.lab126.appmgrd activeView 2>/dev/null) || view=
    case "$view" in KPP_HOME|KPP_LIBRARY) printf 'view %s\n' "$view";; *) printf 'view unavailable\n';; esac
    state_error=boot_state; awk '$1=="btime" && $2~/^[0-9]+$/ {print "boot",$2;seen=1} END {exit !seen}' /proc/stat
}
matches() { plain "$stage/before" && state >"$stage/after" && cmp -s "$stage/before" "$stage/after"; }
no_other_stage() {
    for other in "$volume"/.weread-turn-*; do [ "$other" = "$stage" ] || absent "$other" || return 1; done
}
admit() {
    state_error=stage_occupied; absent "$stage" && no_other_stage || return 1
    state_error=volume_alias; [ "$(readlink -f "$volume")" = "$volume" ] || return 1
    state >/dev/null || return 1
    state_error=volume_mount; awk '$2=="/mnt/us" && $4 ~ /(^|,)rw(,|$)/ && $4 !~ /(^|,)noexec(,|$)/ {ok=1} END {exit !ok}' /proc/mounts || return 1
    state_error=volume_space; df -Pk "$volume" | awk -v need="$((bytes + expanded + 67108864))" 'END{exit !(NF>=6 && $4*1024>=need)}' || return 1
    state_error=available_memory; awk '$1=="MemAvailable:" && $2>=131072 {ok=1} END {exit !ok}' /proc/meminfo
}
diagnosis() {
    for file in "$stage/data/logs/application-bootstrap.log" "$stage/data/logs/application.log" "$stage/launcher.out"; do
        plain "$file" && [ "$(stat -c %s "$file")" -le 4194304 ] || continue
        for reason in no_release_send:target-did-not-submit-in-release-iteration draw_target:unexpected-target-at-draw \
            ordinary_write:target-not-written-in-ordinary-frame send_target:invalid-target-submission \
            changed_target:target-changed wait_marker:wait-marker-mismatch no_target_send:no-target-send \
            wait_worker:wait-worker-lost witness_disabled:reader-witness-not-enabled witness_unarmed:reader-witness-not-armed \
            lane:scheduled_arm lane:invalid_build_arm stage_marker:Reader_trial_marker stage_path:Reader_trial_stage \
            stage_path:Reader_trial_data stage_output:Reader_trial_output activation:Reader_trial_activation \
            environment:Reader_trial_environment route:Reader_trial_arguments_rejected fixture:public_fixture \
            begin:start_at_the_beginning source:changed_between_turns press:press_changed short:receive_a_short_press \
            release:release_did_not_publish target:skipped_the_expected_target arm:arm_the_target_frame \
            frame_timeout:target_frame_timed_out identity:frame_identity_changed position:durable_position order:timestamp_order \
            deadline:exceeded_its_deadline guard:frame_guard portrait:portrait power:Power_observer; do
            pattern=$(printf '%s' "${reason#*:}" | tr '_' ' ')
            if grep -iF "$pattern" "$file" >/dev/null 2>&1; then printf '%s\n' "${reason%%:*}"; return; fi
        done
    done
    printf 'unclassified\n'
}
registered_alive() {
    plain "$stage/supervisor" && IFS=' ' read -r sup original <"$stage/supervisor" || return 1
    case "$sup:$original" in *[!0-9:]*|:*|*:) return 1;; esac
    identity "$sup" 2>/dev/null && [ "$birth" = "$original" ] && [ "$proc_state" != Z ]
}
supervisor_owned() {
    registered_alive && tr '\000' '\n' <"/proc/$sup/cmdline" | grep -Fx -- "$stage/launch.sh" >/dev/null
}
retire() {
    if ! absent "$stage/permission"; then
        plain "$stage/permission" && plain "$stage/supervisor" &&
            [ "$(cat "$stage/permission")" = "$label:$(cat "$stage/supervisor")" ] || return 1
    fi
    if absent "$stage/stopping"; then (set -C; : >"$stage/stopping") || return 1; fi
    for tick in $(seq 1 60); do
        if ! registered_alive; then return 0; fi
        if supervisor_owned && absent "$stage/term-sent"; then
            (set -C; : >"$stage/term-sent") && kill -TERM "$sup" || return 1
        fi
        sleep 1
    done
    ! registered_alive
}
safe_remove() {
    owned && awk -v p="$stage" '$5==p || index($5,p"/")==1 {bad=1} END{exit bad}' /proc/self/mountinfo || return 1
    rm -rf "$stage" && absent "$stage"
}
case "$mode" in
    admit) admit || fail "$state_error"; printf 'admitted\n';;
    stage)
        admit || fail "$state_error"
        mkdir "$stage" || fail stage
        (set -C; printf '%s\n' "$label" >"$stage/owner") || { rmdir "$stage" 2>/dev/null || :; fail owner; }
        stage_lock || fail stage_ownership
        state >"$stage/before.next" || fail "$state_error"
        mv "$stage/before.next" "$stage/before" || fail initial_state
        (ulimit -f 409600; cat >"$stage/payload.tgz") && [ "$(wc -c <"$stage/payload.tgz")" -eq "$bytes" ] || fail upload
        tar -xzf "$stage/payload.tgz" -C "$stage" && rm "$stage/payload.tgz" || fail extract
        for member in launch.sh app/weread app/weread-frame-guard app/weread-native-query; do plain "$stage/$member" && [ -x "$stage/$member" ] || fail executable; done
        plain "$stage/data/library.sqlite" && [ "$(cat "$stage/data/device-profile")" = scribe ] && [ "$(cat "$stage/data/device-allowlist")" = 'scribe 5.19.5' ] || fail fixture
        (set -C; printf 'pointer-turn-fixture\n' >"$stage/reader-turn-public") && matches || fail staged_state
        printf 'staged\n';;
    launch-gate)
        [ "${WEREAD_HOME:-}" = "$stage" ] && [ "${WEREAD_READER_TURN_TRIAL:-}" = 1 ] &&
            [ "$(readlink -f "$0")" = "$stage/launch-gate.sh" ] && owned && plain "$stage/started" || fail child_scope
        identity "$$" || fail child_identity; own_birth=$birth
        for tick in $(seq 1 8); do
            owned && absent "$stage/stopping" && plain "$stage/run-parent" &&
                IFS=' ' read -r parent parent_birth <"$stage/run-parent" && [ "$parent" = "$PPID" ] &&
                identity "$parent" 2>/dev/null && [ "$birth" = "$parent_birth" ] && [ "$proc_state" != Z ] || fail parent_gone
            if plain "$stage/permission" && plain "$stage/supervisor" &&
                [ "$(cat "$stage/supervisor")" = "$$ $own_birth" ] &&
                [ "$(cat "$stage/permission")" = "$label:$$ $own_birth" ]; then
                exec sh "$stage/launch.sh" --detached
            fi
            sleep 1
        done
        fail permission_timeout;;
    run)
        stage_lock && matches && absent "$stage/supervisor" && absent "$stage/started" && absent "$stage/turn-result.jsonl" && absent "$stage/turn-complete" || fail run_admission
        plain "$stage/launch-gate.sh" && identity "$$" &&
            (set -C; printf '%s %s\n' "$$" "$birth" >"$stage/run-parent") || fail parent_identity
        (set -C; : >"$stage/started") || fail started
        unset WEREAD_API_KEY WEREAD_STARTUP_TIMING
        export WEREAD_HOME="$stage" DISPLAY=:0.0 WEREAD_READER_TURN_TRIAL=1 WEREAD_READER_TURN_EXPECTED_ARM="$lane"
        trap 'trap "" HUP INT TERM; retire || :; exit 1' HUP INT TERM
        sh "$stage/launch-gate.sh" launch-gate "$label" "$bytes" "$expanded" "$lane" 9<&- </dev/null >"$stage/launcher.out" 2>&1 & child=$!
        identity "$child" && (set -C; printf '%s %s\n' "$child" "$birth" >"$stage/supervisor.next") &&
            absent "$stage/supervisor" && mv "$stage/supervisor.next" "$stage/supervisor" || fail supervisor_identity
        (set -C; printf '%s:%s %s\n' "$label" "$child" "$birth" >"$stage/permission.next") &&
            absent "$stage/permission" && mv "$stage/permission.next" "$stage/permission" || fail permission
        for tick in $(seq 1 170); do identity "$child" 2>/dev/null && [ "$proc_state" != Z ] || break; sleep 1; done
        if supervisor_owned; then retire || fail supervisor_timeout; fail deadline; fi
        wait "$child" || fail "gui_exit_$(diagnosis)"
        : >"$stage/exited"; printf 'exited\n';;
    collect)
        stage_lock || fail stage_ownership
        plain "$stage/exited" && plain "$stage/turn-result.jsonl" && plain "$stage/turn-complete" && [ "$(cat "$stage/turn-complete")" = 24 ] &&
            [ "$(wc -l <"$stage/turn-result.jsonl")" -eq 24 ] && [ "$(wc -c <"$stage/turn-result.jsonl")" -le 131072 ] || fail "no_result_$(diagnosis)"
        cat "$stage/turn-result.jsonl";;
    close)
        if absent "$stage"; then printf 'absent\n'; exit 0; fi
        close_lock && retire || fail ownership
        if plain "$stage/before"; then
            restored=0; for tick in $(seq 1 20); do if matches; then restored=1; break; fi; sleep 1; done
            [ "$restored" = 1 ] || fail restoration
        else absent "$stage/started" && state >/dev/null || fail unknown_state; fi
        safe_remove || fail cleanup; printf 'closed\n';;
    absent) absent "$stage" && state >/dev/null || fail not_returned; printf 'returned\n';;
    *) fail mode;;
esac
