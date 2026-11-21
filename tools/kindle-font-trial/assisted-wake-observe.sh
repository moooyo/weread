wake_record() { plain "$1" && printf '%s\n' "$2" | cmp -s - "$1"; }

wake_reason() {
    case "${1:-}" in
        20) echo stage_scope;; 21) echo entry_state;; 22) echo owner_record;; 23) echo generation;;
        24) echo application_identity;; 25) echo state_open;; 26) echo managed_available;; 27) echo executable;;
        28) echo process_birth;; 29) echo arguments;; 30) echo peer;; 31) echo launch_calibration;;
        32) echo task_busy;; 33) echo task_state;; 34) echo manager;; 35) echo display;; 36) echo input;;
        37) echo phase_markers;; 38) echo sleep_cycle;; 39) echo wake_cycle;; 40) echo wake_calibration;;
        41) echo power_query;; 42) echo power_state;; 43) echo sensor_helper;; 44) echo sensor_busy;;
        45) echo sensor_query;; 46) echo sensor_format;; 47) echo pose_disagrees;; 48) echo unstable_scope;;
        49) echo sensor_timeout;; *) echo observer_unknown;;
    esac
}

wake_identity() {
    case "$label" in ''|*[!0-9a-f]*) return 20;; esac
    [ "${#label}" = 16 ] && [ "$name" = B-reader ] && [ "$stage" = "/mnt/us/.weread-orientation-$label" ] &&
        [ "$out" = "$stage/out/B-reader" ] && [ "$home" = "$stage/B" ] &&
        wake_record "$stage/owner" "$label" && wake_record "$out/entered/arm" B-reader || return 20
    absent "$stage/armed" && absent "$out/status" && absent "$out/guard" && absent "$lock/ordinary-exit" || return 21
    plain "$out/wake-owner" && IFS=: read -r wake_version wake_nonce wake_pid wake_birth wake_extra <"$out/wake-owner" || return 22
    case "$wake_nonce" in ''|*[!0-9a-f]*) return 22;; esac
    case "$wake_pid" in ''|0*|*[!0-9]*) return 22;; esac
    case "$wake_birth" in ''|0*|*[!0-9]*) return 22;; esac
    [ "$wake_version" = 1 ] && [ "${#wake_nonce}" = 32 ] && [ -z "$wake_extra" ] || return 22
    wake_generation=1:$wake_nonce
    wake_record "$out/wake-owner" "$wake_generation:$wake_pid:$wake_birth" || return 22
    wake_record "$lock/nonce" "$wake_nonce" && wake_record "$lock/generation" "$wake_generation" || return 23
    wake_record "$lock/application-pid" "$wake_pid" && wake_record "$lock/application-birth" "$wake_birth" &&
        wake_record "$lock/application-owned" "$wake_generation:$wake_pid:$wake_birth" || return 24
    wake_record "$lock/state-open" "$wake_generation" || return 25
    wake_record "$lock/managed-sleep-available" "$wake_generation" || return 26
    [ "$(readlink "/proc/$wake_pid/exe" 2>/dev/null)" = "$home/app/weread" ] || return 27
    wake_task=$(awk -v birth="$wake_birth" '{sub(/^.*\) /, ""); if (NF>=20 && "b"$20=="b"birth) {print $1; ok=1}} END {exit !(ok && NR==1)}' "/proc/$wake_pid/stat" 2>/dev/null) || return 28
    printf '%s\000' "$home/app/weread" --kindle --device scribe --data-dir "$home/data" --demo --offline --screen reader \
        --exit-after-ms 105000 --capture "$out/final.png" | cmp -s - "/proc/$wake_pid/cmdline" || return 29
    for wake_peer in aidoku bilicomics; do
        absent "/tmp/$wake_peer-launcher.lock" || return 30
        if pidof "$wake_peer" >/dev/null 2>&1; then return 30; else [ "$?" = 1 ] || return 30; fi
    done
    plain "$out/portrait-launch" && IFS= read -r wake_anchor <"$out/portrait-launch" || return 31
    case "$wake_anchor" in U:3|D:1) ;; *) return 31;; esac
    wake_record "$out/portrait-launch" "$wake_anchor" && wake_record "$out/portrait" "$wake_anchor" &&
        wake_record "$lock/native-portrait-calibration" "$wake_generation:$wake_anchor" || return 31
}

wake_foreground() {
    case "$wake_task" in R|S) ;; D) return 32;; *) return 33;; esac
    wake_managers=$(pidof awesome) && [ -n "$wake_managers" ] || return 34
    for wake_manager in $wake_managers; do
        case "$wake_manager" in ''|0*|*[!0-9]*) return 34;; esac
        [ "$(awk '{sub(/^.*\) /, ""); print $1}' "/proc/$wake_manager/stat" 2>/dev/null)" = T ] || return 34
    done
    wake_display=0; wake_input=0
    for wake_fd in "/proc/$wake_pid/fd/"*; do
        case "$(readlink "$wake_fd" 2>/dev/null || :)" in /dev/fb0) wake_display=1;; /dev/input/event[0-9]*) wake_input=1;; esac
    done
    [ "$wake_display" = 1 ] || return 35
    [ "$wake_input" = 1 ] || return 36
}

wake_guard() {
    [ "$#" = 1 ] || return 1
    wake_identity || return "$?"
    wake_cal=none
    case "$1" in
        pre)
            for wake_marker in sleep-request resume-ready managed-sleep-request managed-sleep-ready managed-wake-request managed-wake-ready; do
                absent "$lock/$wake_marker" && absent "$lock/$wake_marker.pending" || return 37
            done
            wake_foreground || return "$?"
            wake_cal=$wake_anchor; wake_power=active;;
        sleep-pre|sleep-post|post)
            wake_cycle=$wake_generation:1
            wake_record "$lock/managed-sleep-request" "$wake_cycle" && wake_record "$lock/managed-sleep-ready" "$wake_cycle" || return 38
            if [ "$1" != post ]; then
                [ "$wake_task" = T ] || return 33
                wake_record "$lock/sleep-request" "$wake_generation" && wake_record "$lock/resume-ready" "$wake_generation" || return 38
                for wake_marker in managed-wake-request managed-wake-ready; do
                    absent "$lock/$wake_marker" && absent "$lock/$wake_marker.pending" || return 39
                done
                wake_power=screenSaver
            else
                wake_record "$lock/managed-wake-request" "$wake_cycle" && wake_record "$lock/managed-wake-ready" "$wake_cycle" || return 39
                wake_foreground || return "$?"
                if ! absent "$lock/managed-native-portrait-calibration"; then
                    plain "$lock/managed-native-portrait-calibration" && IFS= read -r wake_recorded <"$lock/managed-native-portrait-calibration" || return 40
                    wake_cal=${wake_recorded#"$wake_cycle:"}
                    case "$wake_cal" in U:3|D:1) ;; *) return 40;; esac
                    wake_record "$lock/managed-native-portrait-calibration" "$wake_cycle:$wake_cal" || return 40
                fi
                wake_power=active
            fi;;
        *) return 1;;
    esac
    wake_actual=$(timeout -s KILL 1 lipc-get-prop -s com.lab126.powerd state 2>/dev/null) || return 41
    [ "$wake_actual" = "$wake_power" ] || return 42
}

wake_observe() (
    [ "$#" = 1 ] || exit 1
    wake_guard "$1" || exit "$?"
    wake_phase=$1; wake_before_identity=$wake_generation:$wake_pid:$wake_birth:$wake_anchor:$wake_cal
    plain "$stage/pose-snapshot" && [ -x "$stage/pose-snapshot" ] || exit 43
    wake_sample=$(timeout -s KILL 1 "$stage/pose-snapshot" --display-sample 2>/dev/null) || {
        case "$?" in 124|137) exit 49;; 126|127) exit 43;; *) exit 45;; esac
    }
    printf '%s\n' "$wake_sample" | awk '
        function ns(value) {return value ~ /^[1-9][0-9]*$/ && (length(value)<19 || (length(value)==19 && "t"value<="t9223372036854775807"))}
        $1=="sample" && NF==5 && ns($2) && ns($3) && (length($2)<length($3) || (length($2)==length($3) && "t"$2<="t"$3)) && $4~/^1[5-8]$/ && $5~/^[0-3]$/ {ok=1}
        END {exit !(NR==1 && ok)}' || exit 46
    set -- $wake_sample
    case "$wake_phase:$wake_anchor:$4" in
        pre:U:3:1[57]|sleep-pre:U:3:1[57]|pre:D:1:1[68]|sleep-pre:D:1:1[68]|\
        sleep-post:U:3:1[68]|post:U:3:1[68]|sleep-post:D:1:1[57]|post:D:1:1[57]) ;;
        *) exit 47;;
    esac
    wake_guard "$wake_phase" || exit "$?"
    [ "$wake_generation:$wake_pid:$wake_birth:$wake_anchor:$wake_cal" = "$wake_before_identity" ] || exit 48
    printf 'WAKE_OBS %s %s %s %s %s %s\n' "$wake_phase" "$2" "$3" "$4" "$5" "$wake_cal"
)
