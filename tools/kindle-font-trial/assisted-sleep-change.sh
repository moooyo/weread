as_now() { awk 'NR==1 && $1~/^[0-9]+\.[0-9][0-9]$/ {v=$1;sub(/\./,"",v);printf "%.0f\n",v+0;ok=1} END{exit !(ok && NR==1)}' /proc/uptime; }
as_offer() {
    as_time=$(as_now) || return 1
    as_until=$((as_time + $2 * 100)); as_phase=$1
    printf '%s %s\n' "$as_phase" "$as_until" >"$out/offer.new" && mv "$out/offer.new" "$out/offer" || return 1
    printf 'PHASE %s %s\n' "$label" "$as_phase"
}
as_fresh() { as_check=$(as_now) && [ "$as_check" -lt "$as_until" ]; }
as_drop() { rm -f "$out/offer"; }
as_double() {
    as_a=$(wake_observe "$1") || return 1
    usleep 150000 || return 1
    as_b=$(wake_observe "$1") || return 1
    printf '%s\n%s\n' "$as_a" "$as_b" | awk 'NR==1 {end=$4} NR==2 {if($3<=end || $3-end<100000000) exit 1}' || return 1
    printf '%s\n%s\n' "$as_a" "$as_b" >"$out/$1.row"
}
as_wait() {
    while as_fresh; do
        if as_double "$1"; then return 0; fi
        if wake_guard post; then return 1; fi
        usleep 200000 || return 1
    done
    return 1
}
as_image() {
    as_attempt=1; as_site=observe_before; as_reason=observer_unknown
    as_start=$(as_now) || return 1
    while :; do
        if as_row=$(wake_observe "$1"); then break; else as_status=$?; fi
        as_reason=$(wake_reason "$as_status")
        case "$as_reason" in task_busy|sensor_busy|power_query|sensor_timeout|managed_available|input|display) ;;
            *) return 1;; esac
        as_elapsed=$(as_now) && [ "$as_elapsed" -ge "$as_start" ] && [ "$as_elapsed" -lt "$((as_start+120))" ] &&
            [ "$as_attempt" -lt 12 ] || return 1
        usleep 100000 || return 1; as_attempt=$((as_attempt+1))
    done
    set -- "$1" "$2" "$as_row"
    as_values=$(printf '%s\n' "$3" | awk 'NF==7 && $1=="WAKE_OBS"{print $5,$6}')
    as_site=image_state; as_reason=sensor_pair_changed; [ -n "$as_values" ] || return 1
    as_reason=occupied_output; absent "$out/$2.next" && absent "$out/$2" || return 1
    as_site=image_read; as_reason=raw_panel
    if ! (set -C; "$stage/measure" panel >"$out/$2.next"); then rm -f "$out/$2.next"; return 1; fi
    if as_after=$(wake_observe "$1"); then :; else as_status=$?; as_site=observe_after; as_reason=$(wake_reason "$as_status"); rm -f "$out/$2.next"; return 1; fi
    as_site=image_state; as_reason=sensor_pair_changed
    [ "$as_values" = "$(printf '%s\n' "$as_after" | awk 'NF==7 && $1=="WAKE_OBS"{print $5,$6}')" ] || { rm -f "$out/$2.next"; return 1; }
    as_reason=anchor_invalid
    case "$(cat "$out/portrait-launch")" in U:[0-3]) base_down=0;; D:[0-3]) base_down=1;; *) rm -f "$out/$2.next"; return 1;; esac
    base_rot=$(cut -d: -f2 "$out/portrait-launch")
    as_raw=${as_values% *}; as_rot=${as_values#* }
    as_reason=mapping_unsupported
    case "$as_raw" in 15|17) as_down=0;; 16|18) as_down=1;; *) rm -f "$out/$2.next"; return 1;; esac
    as_delta=$(((as_rot + 4 - base_rot) % 4)); case "$as_delta" in 0|2) ;; *) rm -f "$out/$2.next"; return 1;; esac
    if [ "$(((as_down + base_down + as_delta / 2) % 2))" = 0 ]; then as_map=keep; else as_map=turn; fi
    as_site=publish; as_reason=publish_failed
    printf '%s\n' "$as_row" >"$out/$1.row" && printf 'IMAGE %s %s %s %s\n' "$1" "$as_map" "$as_raw" "$as_rot" >>"$out/image-mapping" && mv "$out/$2.next" "$out/$2" || return 1
    as_site=ready; as_reason=ready
}
as_observer() {
    owned && [ "$name" = B-reader ] && plain "$stage/wake-observe" || return 1
    . "$stage/wake-observe"
}
as_hydrate() {
    requested_phase=$name; name=B-reader; home=$stage/B; out=$stage/out/B-reader
    as_observer && plain "$out/wake-owner" && plain "$out/portrait-launch" && absent "$out/status" && absent "$out/complete" || return 1
    gui_args && plain "$lock/generation" || return 1
    generation=$(cat "$lock/generation"); identity=$gui:$birth
    printf '%s:%s\n' "$generation" "$identity" | cmp -s - "$out/wake-owner"
}
as_permit() {
    as_hydrate && plain "$out/offer" || return 1
    read -r as_phase as_until as_extra <"$out/offer" || return 1
    case "$as_until" in ''|*[!0-9]*) return 1;; esac
    [ -z "$as_extra" ] && [ "$as_phase" = "$requested_phase" ] || return 1
    case "$as_phase" in
        sleep) plain "$out/pre.row" && plain "$out/panel.pgm" && wake_observe pre >/dev/null;;
        turn) plain "$out/sleep-pre.row" && wake_observe sleep-pre >/dev/null;;
        wake) plain "$out/sleep-post.row" && wake_observe sleep-post >/dev/null;;
        recover-wake) wake_guard sleep-pre;;
        *) return 1;;
    esac || return 1
    printf '%s %s\n' "$as_phase" "$as_until" | cmp -s - "$out/offer" && as_check=$(as_now) &&
        [ "$as_until" -ge "$((as_check+1200))" ] || return 1
    printf 'permitted %s %s\n' "$label" "$as_phase"
}
as_recover() {
    as_drop
    if ! wake_guard sleep-pre; then return 0; fi
    as_offer recover-wake 23 || return 1
    while as_fresh; do wake_guard sleep-pre || break; usleep 250000; done
    as_drop
}
as_run_cycle() {
    as_result='not_qualified initial_reader'
    if as_image pre panel.pgm; then as_initial=ready; else as_initial=refused; fi
    (set -C; printf 'INITIAL %s %s %s\n' "$as_site" "$as_reason" "$as_attempt" >"$out/initial-diagnostic") || fail initial_record
    [ "$as_initial" = ready ] || return 0
    as_offer sleep 22 || fail phase
    if ! as_wait sleep-pre; then as_result='not_qualified sleep_unconfirmed'; as_recover; return; fi
    as_drop; as_offer turn 21 || fail phase
    if ! as_wait sleep-post; then as_result='not_qualified turn_unconfirmed'; as_recover; return; fi
    as_drop
    if ! wake_observe sleep-post >/dev/null; then as_result='not_qualified wake_unconfirmed'; as_recover; return; fi
    as_offer wake 22 || fail phase
    while as_fresh; do
        if as_image post resumed.pgm; then as_result='observed changed_sleep_cycle'; as_drop; return; fi
        usleep 250000
    done
    as_result='not_qualified resume_unconfirmed'; as_recover
}
as_case() {
    [ "$name" = B-reader ] || exit 64
    prepare_case; as_observer || fail observer
    "$stage/measure" request "$label" >"$out/request" || fail request
    log=$home/data/logs/application-bootstrap.log
    for tick in $(seq 1 200); do
        if plain "$log" && grep -q ' first_application_frame_send$' "$log"; then break; fi
        absent "$out/status" || fail no_send; usleep 150000
    done
    plain "$log" && grep -q ' first_application_frame_send$' "$log" || fail no_send
    sleep 1
    absent "$stage/armed" && plain "$out/entered/arm" && printf '%s\n' "$name" | cmp -s - "$out/entered/arm" && gui_args && live_pose || fail gui_identity_or_pose
    identity=$gui:$birth; generation=$(cat "$lock/generation")
    absent "$out/wake-owner" && (set -C; printf '%s:%s\n' "$generation" "$identity" >"$out/wake-owner") || fail owner
    as_run_cycle; as_drop
    printf '%s\n' "$as_result" >"$out/observation"
    for tick in $(seq 1 455); do plain "$out/status" && break; usleep 250000; done
    plain "$out/status" && [ "$(cat "$out/status")" = 0 ] && absent "$out/guard" || fail exit
    finish_case; printf 'completed %s\n' "$name"
}
