#!/bin/sh

library_submit_pinned() (
    root=/var/tmp/weread-library-frame
    [ -d "$root" ] && [ ! -L "$root" ] && [ "$root" -ef /proc/self/fd/6 ] &&
        [ -f "$root/request" ] && [ ! -L "$root/request" ] && [ "$root/request" -ef /proc/self/fd/7 ] &&
        [ "$(stat -Lc '%u:%a' /proc/self/fd/6)" = 0:700 ] && [ "$(stat -Lc '%u:%a:%h' /proc/self/fd/7)" = 0:600:1 ] &&
        stat -Lc '%d:%i' /proc/self/fd/6 /proc/self/fd/7
)

library_submit_arm() {
    [ "$#" = 1 ] && [ -z "${library_submit_batch-}" ] && owned && armed && native_idle && tag tile-prepared &&
        absent "$stage/tile-dispatch-intent" && absent "$stage/tile-release" && absent "$stage/stopped" && plain "$stage/render-observer" || return 1
    library_submit_identity=$(library_submit_pinned) &&
        printf 'WEREAD_LIBRARY_REQUEST %s %s\n' "$label" "$1" | cmp -s - /proc/self/fd/7 || return 1
    library_submit_at=$("$stage/render-observer" --monotonic-ns "$1") &&
        [ "$(library_submit_pinned)" = "$library_submit_identity" ] || return 1
    library_submit_batch=$label
    printf '{"state":"armed","label":"%s","request_clock":"CLOCK_MONOTONIC","request_armed_ns":%s,"request_deadline_ns":%s}\n' \
        "$label" "$library_submit_at" "$1"
}

library_submit_collect() (
    [ "${library_submit_batch-}" = "$label" ] && owned && armed && absent "$stage/stopped" && tag tile-dispatch-intent && witness &&
        session=$(line "$stage/session") && [ "$(library_submit_pinned)" = "$library_submit_identity" ] || return 1
    [ "$(stat -Lc %s /proc/self/fd/7)" -le 80 ] && IFS= read -r marker </proc/self/fd/7 &&
        printf '%s\n' "$marker" | grep -Eq "^WEREAD_LIBRARY_SUBMITTED $label [1-9][0-9]{0,18} (entries|empty)$" || return 1
    leaf=$stage/tile-release
    plain "$leaf" && [ "$(stat -c %s "$leaf")" -le 480 ] && IFS= read -r tile_receipt <"$leaf" || return 1
    prefix='{"ok":true,"mode":"XTest-button-1","clock":"CLOCK_MONOTONIC","hold_requested_ms":50,"down_confirmed":true,"up_sent":true,"buttons_clear_after":true,"target":"open-card","down_before_ns":'
    case "$tile_receipt" in "$prefix"*) ;; *) return 1;; esac
    printf '%s\n' "${tile_receipt#"$prefix"}" | grep -Eq '^[1-9][0-9]{0,18},"down_after_ns":[1-9][0-9]{0,18},"up_before_ns":[1-9][0-9]{0,18},"up_after_ns":[1-9][0-9]{0,18}\}$' || return 1
    observed=$("$stage/render-observer" --monotonic-ns) && owned && armed && absent "$stage/stopped" && tag tile-dispatch-intent &&
        witness && matches "$stage/session" "$session" &&
        [ "$(library_submit_pinned)" = "$library_submit_identity" ] && matches "$leaf" "$tile_receipt" &&
        printf '%s\n' "$marker" | cmp -s - /proc/self/fd/7 || return 1
    printf '{"label":"%s","marker":"%s\\n","tile_receipt":%s,"observed_ns":%s}\n' "$label" "$marker" "$tile_receipt" "$observed"
)
