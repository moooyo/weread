witness_permit() {
    as_hydrate && plain "$out/offer" || return 1
    read -r as_phase as_until as_extra <"$out/offer" || return 1
    case "$as_until" in ''|*[!0-9]*) return 1;; esac
    [ -z "$as_extra" ] && [ "$as_phase" = "$requested_phase" ] || return 1
    as_minimum=1200
    case "$as_phase" in
        sleep) plain "$out/pre.row" && plain "$out/panel.pgm" && wake_observe_any pre >/dev/null;;
        turn) as_minimum=2600; plain "$out/sleep-pre.row" && wake_guard sleep-pre;;
        wake) plain "$out/turn-window" && awk 'NR==1 && NF==6 && $1=="WINDOW" && $6=="elapsed" && $3-$2>=2200 {ok=1} END{exit !(NR==1 && ok)}' "$out/turn-window" && wake_guard sleep-post;;
        recover-wake) wake_guard sleep-pre;; *) return 1;;
    esac || return 1
    printf '%s %s\n' "$as_phase" "$as_until" | cmp -s - "$out/offer" && as_check=$(as_now) &&
        [ "$as_until" -ge "$((as_check+as_minimum))" ] || return 1
    printf 'permitted %s %s\n' "$label" "$as_phase"
}

witness_window() {
    as_offer turn 32 || fail phase
    witness_start=$as_time; witness_end=$((witness_start+2200)); witness_count=0; witness_skipped=0; witness_stop=elapsed
    while :; do
        witness_now=$(as_now) || fail observation_clock
        [ "$witness_now" -ge "$witness_start" ] || fail observation_clock
        [ "$witness_now" -lt "$witness_end" ] || break
        if [ "$witness_count" -lt 24 ]; then
            witness_count=$((witness_count+1))
            if witness_sample=$(wake_observe_any sleep-post); then
                printf '%s\n' "$witness_sample" >>"$out/sleep-trace" || fail trace_write
            else
                witness_skipped=$((witness_skipped+1))
                if wake_guard post; then witness_stop=early_wake; break; fi
            fi
        elif wake_guard post; then witness_stop=early_wake; break; fi
        usleep 650000 || fail sample_wait
    done
    if [ "$witness_stop" = elapsed ] && ! wake_guard sleep-post; then
        if wake_guard post; then witness_stop=early_wake; else witness_stop=lost_state; fi
    fi
    as_drop
    printf 'WINDOW %s %s %s %s %s\n' "$witness_start" "$witness_now" "$witness_count" "$witness_skipped" "$witness_stop" >"$out/turn-window" || fail window_record
}

witness_post() {
    until_post=$(as_now) || return 1
    until_post=$((until_post+3100)); captured_post=0
    while :; do
        post_now=$(as_now) || return 1
        [ "$post_now" -lt "$until_post" ] || break
        if wake_guard post; then
            if [ "$captured_post" = 0 ] && post_raw=$(wake_observe_any post); then
                (set -C; printf '%s\n' "$post_raw" >"$out/post-status.row") || fail post_record
                captured_post=1
            fi
            if [ "$captured_post" = 1 ] && as_image post resumed.pgm; then break; fi
            [ "$captured_post" = 1 ] || { usleep 350000; continue; }
            case "$as_site" in observe_before|observe_after) ;;
                *) break;; esac
        fi
        usleep 350000 || return 1
    done
    [ "$captured_post" = 1 ]
}

witness_cycle() {
    as_result='not_qualified initial_reader'
    if as_image pre panel.pgm; then as_initial=ready; else as_initial=refused; fi
    (set -C; printf 'INITIAL %s %s %s\n' "$as_site" "$as_reason" "$as_attempt" >"$out/initial-diagnostic") || fail initial_record
    [ "$as_initial" = ready ] || return 0
    as_offer sleep 30 || fail phase
    if ! as_wait sleep-pre; then
        as_result='not_qualified sleep_unconfirmed'; as_recover
        if wake_guard post; then witness_post || :; fi
        return 0
    fi
    as_drop; witness_window
    if [ "$witness_stop" = elapsed ] && wake_guard sleep-post; then
        as_double sleep-post || :
        if wake_guard sleep-post; then as_offer wake 30 || fail phase; fi
    elif [ "$witness_stop" = lost_state ]; then as_recover; fi
    if witness_post; then as_result='observed resident_wake'; else as_result='not_qualified resume_unconfirmed'; as_recover; fi
    as_drop
}

witness_finish() {
    wake_guard post || return 0
    gui_args && plain "$out/wake-owner" && plain "$lock/generation" &&
        printf '%s:%s:%s\n' "$(cat "$lock/generation")" "$gui" "$birth" | cmp -s - "$out/wake-owner" || return 0
    plain "$stage/pose-finish" && [ -x "$stage/pose-finish" ] && "$stage/pose-finish" "$label" "$gui" "$birth" || :
}
