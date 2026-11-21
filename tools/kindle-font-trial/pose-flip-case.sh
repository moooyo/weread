flip_sensor_bound() {
    flip_reason=sensor_ended; [ -e "/proc/$flip_child/exe" ] || return 1
    flip_reason=sensor_executable; [ "$(readlink "/proc/$flip_child/exe" 2>/dev/null)" = "$stage/pose-snapshot" ] || return 1
    flip_reason=sensor_identity; [ "$(awk '{print $4 ":" $22}' "/proc/$flip_child/stat" 2>/dev/null)" = "$$:$flip_birth" ] || return 1
    flip_reason=sensor_output; [ "$out/flip-stream" -ef "/proc/$flip_child/fd/1" ] || return 1
    flip_reason=sensor_argv; tr '\000' '\n' <"/proc/$flip_child/cmdline" >"$out/flip-args" &&
        printf '%s\n' "$stage/pose-snapshot" --startup-flip | cmp -s - "$out/flip-args"
}
flip_reap() {
    if [ -n "${flip_child:-}" ]; then wait "$flip_child" 2>/dev/null || :; fi
}
flip_boot() {
    flip_boot_id=$(cat /proc/sys/kernel/random/boot_id) && [ "${#flip_boot_id}" = 36 ] &&
        printf '%s\n' "$flip_boot_id" | grep -Eq '^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$'
}
flip_binding() {
    flip_reason=gui_args; gui_args || return 1
    flip_reason=gui_identity; [ "$gui:$birth" = "$identity" ] || return 1
    flip_reason=generation; plain "$lock/generation" && printf '%s\n' "$generation" | cmp -s - "$lock/generation" || return 1
    flip_reason=owner; plain "$lock/application-owned" && printf '%s:%s\n' "$generation" "$identity" | cmp -s - "$lock/application-owned" || return 1
    flip_reason=log_file; plain "$log" || return 1
    flip_reason=log_writer; [ "$log" -ef "/proc/$gui/fd/2" ] || return 1
    flip_reason=log_inode; [ "$(stat -c '%d:%i' "$log")" = "$flip_log_id" ] || return 1
    flip_reason=boot; flip_boot && [ "$flip_boot_id" = "$flip_first_boot" ] || return 1
    flip_reason=record
    printf 'LAUNCH %s %s %s %s %s CLOCK_MONOTONIC\n' "$label" "$name" "$flip_boot_id" "$gui" "$birth"
}

flip_launch_anchor() {
    flip_reason=nonce; plain "$lock/nonce" && [ "$(stat -c %s "$lock/nonce")" = 33 ] || return 1
    flip_nonce=$(cat "$lock/nonce") || return 1
    case "$flip_nonce" in ''|*[!0-9a-f]*) return 1;; esac
    [ "${#flip_nonce}" = 32 ] && [ "$generation" = "1:$flip_nonce" ] && printf '%s\n' "$flip_nonce" | cmp -s - "$lock/nonce" || return 1
    flip_reason=state_open; plain "$lock/state-open" && cmp -s "$lock/generation" "$lock/state-open" || return 1
    flip_anchor_file=$lock/native-portrait-calibration; flip_anchor=malformed
    if absent "$flip_anchor_file"; then flip_anchor=missing
    elif plain "$flip_anchor_file" && [ "$(stat -c %s "$flip_anchor_file")" -le 64 ]; then
        flip_subject=$(awk -F: 'NR==1 && NF==4 && $1~/^[1-9][0-9]*$/ && length($2)==32 && $2~/^[0-9a-f]+$/ && $3~/^(U|D|FIXED)$/ && $4~/^[0-3]$/ {s=$0} END {if(NR!=1 || !s)exit 1; print s}' "$flip_anchor_file") &&
            printf '%s\n' "$flip_subject" | cmp -s - "$flip_anchor_file" && {
                case "$flip_subject" in "$generation":*) flip_anchor=${flip_subject#"$generation":};; *) flip_anchor=foreign_generation;; esac
            } || flip_anchor=malformed
    fi
    flip_reason=anchor_output; printf '%s\n' "$flip_anchor" >"$out/portrait-launch"
}

flip_checkpoint() {
    printf 'partial incomplete_after_pose\n' >"$out/observation.next" && mv "$out/observation.next" "$out/observation" &&
        printf 'bound pose capture\n' >"$out/checkpoint.next" && mv "$out/checkpoint.next" "$out/pose-checkpoint"
}
flip_partial() {
    printf 'partial %s\n' "$1" >"$out/observation.next" && mv "$out/observation.next" "$out/observation" || fail partial_publish
    fail "$1"
}

flip_case() {
    [ "$name" = B-library ] || exit 64
    prepare_case
    native && absent "$stage/armed" && flip_boot || fail flip_native
    flip_first_boot=$flip_boot_id
    trap flip_reap 0
    trap 'exit 1' HUP INT TERM
    "$stage/pose-snapshot" --startup-flip </dev/null >"$out/flip-stream" 2>/dev/null &
    flip_child=$!
    for tick in $(seq 1 60); do
        [ ! -s "$out/flip-stream" ] || break
        kill -0 "$flip_child" 2>/dev/null || fail flip_not_ready
        usleep 50000
    done
    plain "$out/flip-stream" && head -n 1 "$out/flip-stream" >"$out/sensor-ready" &&
        awk 'NF!=4 || $1!="READY" || $2!~/^[1-9][0-9]*$/ || $3!~/^[1-9][0-9]*$/ || $4!~/^(15|16|17|18)$/ {bad=1} END {exit bad || NR!=1}' "$out/sensor-ready" || fail flip_not_ready
    flip_birth=$(awk '{print $22}' "/proc/$flip_child/stat") || fail ready_sensor_identity
    flip_sensor_bound || fail "ready_$flip_reason"
    native && absent "$stage/armed" || fail launch_admission
    printf '%s\n' "$name" >"$stage/next" && mv "$stage/next" "$stage/armed" || fail arm
    "$stage/measure" request "$label" >"$out/request" || fail request
    log=$home/data/logs/application-bootstrap.log
    flip_published=0
    for tick in $(seq 1 180); do
        if plain "$log" && grep -q ' main_enter$' "$log" && plain "$lock/application-owned"; then flip_published=1; break; fi
        [ -e "/proc/$flip_child/exe" ] || fail early_sensor_ended
        absent "$out/status" || fail early_gui_ended
        usleep 100000
    done
    [ "$flip_published" = 1 ] || fail early_gui_unpublished
    absent "$stage/armed" && plain "$out/entered/arm" && printf '%s\n' "$name" | cmp -s - "$out/entered/arm" || fail entry_identity
    gui_args || fail early_gui_args
    identity=$gui:$birth; generation=$(cat "$lock/generation"); flip_log_id=$(stat -c '%d:%i' "$log")
    flip_sensor_bound || fail "early_$flip_reason"
    flip_binding >"$out/sensor-binding" || fail "early_$flip_reason"
    flip_sensor_bound || fail "early_after_$flip_reason"
    for tick in $(seq 1 270); do
        if plain "$log" && grep -q ' first_application_frame_send$' "$log"; then break; fi
        absent "$out/status" || fail no_send
        usleep 100000
    done
    plain "$log" && grep -q ' first_application_frame_send$' "$log" || fail no_send
    flip_binding >"$out/app-binding" || fail "send_$flip_reason"
    flip_launch_anchor || fail "send_$flip_reason"
    export_markers || fail send_markers
    flip_binding >"$out/log-check" || fail "after_send_$flip_reason"
    cmp -s "$out/app-binding" "$out/log-check" && cmp -s "$out/app-binding" "$out/sensor-binding" || fail send_binding_changed
    wait "$flip_child" || fail flip_incomplete
    flip_child=''; trap - 0 HUP INT TERM
    awk 'NR==1 {next} NF!=4 || $1!=NR-1 || $2!~/^[1-9][0-9]*$/ || $3!~/^[1-9][0-9]*$/ || $4!~/^(15|16|17|18)$/ {bad=1; exit} {print} END {exit bad || NR!=402}' "$out/flip-stream" >"$out/sensor" &&
        head -n 1 "$out/flip-stream" | cmp -s - "$out/sensor-ready" || fail flip_format
    flip_boot && [ "$flip_boot_id" = "$flip_first_boot" ] || fail closed_sensor_boot
    flip_checkpoint || fail checkpoint_publish
    flip_binding >"$out/log-check" || flip_partial "presentation_$flip_reason"
    cmp -s "$out/app-binding" "$out/log-check" || flip_partial presentation_binding
    if ! (fixed_capture_shelf) 2>"$out/scene-refusal"; then
        rm -f "$out/shelf.next" && absent "$out/shelf" && printf 'not_observed\n' >"$out/shelf" || flip_partial scene_cleanup
    fi
    for tick in $(seq 1 240); do plain "$out/status" && break; usleep 200000; done
    plain "$out/status" && [ "$(cat "$out/status")" = 0 ] && absent "$out/guard" || flip_partial exit
    plain "$out/final.metadata.json" && [ -s "$out/final.metadata.json" ] && [ "$(stat -c %s "$out/final.metadata.json")" -le 2048 ] &&
        plain "$out/final.png" && rm "$out/final.png" || flip_partial capture_metadata
    settled && same_state && portrait >"$out/portrait-return" || flip_partial native_return
    printf 'captured startup_turn\n' >"$out/observation.next" && mv "$out/observation.next" "$out/observation" &&
        printf '%s\n' "$name" >"$out/complete.next" && mv "$out/complete.next" "$out/complete" || flip_partial final_publish
    printf 'completed %s\n' "$name"
}
