stock_note_release() {
    matches "$stage/tile-release" "$1" && record_once stock-confirmed-release "$1"
}

stock_no_trial_identity() {
    for name in selector-entered selector-duplicate launch-owner group session observed-session term-intent term-sent; do
        absent "$stage/$name" || return 1
    done
}

stock_declined_callback() (
    callback=$(line "$stage/selector-callback") && matches "$stage/selector-declined" "$callback" || return 1
    token_rest=${callback#*:}; callback_pid=${token_rest%%:*}; callback_birth=${token_rest#*:}
    [ "$callback" = "$label:$callback_pid:$callback_birth" ] && number "$callback_pid" && number "$callback_birth" &&
        armed && [ "$callback_birth" -ge "$fence" ] && retired "$callback_pid" "$callback_birth"
)

stock_declined_quiet() (
    owned && tag selector-disarmed && tag tile-dispatch-intent && stock_no_trial_identity && stock_declined_callback &&
        single "$stage/tile-release" && single "$stage/stock-confirmed-release" &&
        [ "$(stat -c %s "$stage/tile-release")" -le 480 ] && [ -s "$stage/tile-release" ] &&
        cmp -s "$stage/tile-release" "$stage/stock-confirmed-release" && stock_request_absent && channel_idle &&
        native_idle && state >/dev/null
)

stock_quarantine_settled() (
    tag quarantined && tag stopped && exec 4<"$stage/owner" && flock -n 4 && stock_declined_quiet
)

stock_quarantine_no_group() (
    owned && armed && tag tile-dispatch-intent && tag selector-live && stock_no_trial_identity &&
        stock_request_absent && channel_idle && exec 4<"$stage/owner" || return 1
    if ! flock -n 4; then sleep 1 && flock -n 4 || return 1; fi
    owned && tag tile-dispatch-intent && tag selector-live &&
        stock_no_trial_identity && write_tag selector-disarmed || return 1
    exec 4<&-
    for tick in 1 2 3 4 5 6; do
        stock_declined_callback && break
        sleep 1 || return 1
    done
    exec 4<"$stage/owner" && flock -n 4 && stock_declined_quiet && sleep 1 && stock_declined_quiet && write_tag quarantined && write_tag stopped
)
