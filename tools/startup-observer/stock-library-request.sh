#!/bin/sh

stock_request_paths() {
    case "$label" in ''|*[!0-9a-f]*) return 1;; esac
    [ "${#label}" = 16 ] || return 1
    stock_fixed=/var/tmp/weread-library-frame
    stock_draft=/var/tmp/.weread-library-frame-$label
}

stock_request_absent() (
    stock_request_paths && absent "$stock_fixed" && absent "$stock_draft"
)

stock_request_dir() {
    [ -d "$1" ] && [ ! -L "$1" ] && [ "$(stat -c '%u:%a' "$1")" = 0:700 ] && stat -c '%d:%i' "$1"
}

stock_request_file() {
    [ -f "$1" ] && [ ! -L "$1" ] && [ "$(stat -c '%u:%a:%h' "$1")" = 0:600:1 ] && stat -c '%d:%i' "$1"
}

stock_request_purge() (
    owned && stock_request_paths || return 1
    if ! absent "$stock_draft"; then
        stock_target=$stock_draft
    elif ! absent "$stock_fixed"; then
        stock_target=$stock_fixed
    else return 0; fi
    dir_id=$(stock_request_dir "$stock_target") && [ -n "$dir_id" ] && matches "$stage/request-dir" "$dir_id" || return 1
    (
        set +f
        for leaf in "$stock_target"/* "$stock_target"/.[!.]* "$stock_target"/..?*; do
            absent "$leaf" || [ "$leaf" = "$stock_target/request" ] || exit 1
        done
    ) || return 1
    if ! absent "$stock_target/request"; then
        file_id=$(stock_request_file "$stock_target/request") && [ -n "$file_id" ] && matches "$stage/request-file" "$file_id" || return 1
        [ "$(stat -c %s "$stock_target/request")" -le 80 ] || return 1
        if [ -s "$stock_target/request" ]; then
            deadline=$(line "$stage/request-deadline") && body=$(line "$stock_target/request") || return 1
            if [ "$body" != "WEREAD_LIBRARY_REQUEST $label $deadline" ]; then
                printf '%s\n' "$body" | grep -Eq "^WEREAD_LIBRARY_SUBMITTED $label [1-9][0-9]{0,18} (entries|empty)$" || return 1
            fi
        fi
        rm "$stock_target/request" || return 1
    fi
    matches "$stage/request-dir" "$(stock_request_dir "$stock_target")" && rmdir "$stock_target" && stock_request_absent
)

stock_request_cleanup() (
    owned || return 1
    if absent "$stage/request-holder"; then stock_request_absent; return; fi
    tag request-holder && exec 5<"$stage/request-holder" && flock -n 5 && tag request-holder && stock_request_purge
)

stock_request_active() (
    stock_request_paths && owned && armed && tag request-holder && tag request-live &&
        exec 5<"$stage/request-holder" || return 1
    if flock -n 5; then return 1; else [ "$?" = 1 ] || return 1; fi
    deadline=$(line "$stage/request-deadline") && [ -n "$(line "$stage/request-dir")" ] && [ -n "$(line "$stage/request-file")" ] &&
        matches "$stage/request-dir" "$(stock_request_dir "$stock_fixed")" &&
        matches "$stage/request-file" "$(stock_request_file "$stock_fixed/request")" &&
        matches "$stock_fixed/request" "WEREAD_LIBRARY_REQUEST $label $deadline" &&
        "$stage/render-observer" --monotonic-ns "$deadline" >/dev/null
)

stock_request_hold() {
    umask 077
    tag request-holder && owned && armed && native_idle && stock_request_absent || return 1
    trap 'result=$?; trap - EXIT HUP INT TERM; stock_request_purge || result=1; exit "$result"' EXIT
    trap 'exit 1' HUP INT TERM
    sampled=$("$stage/render-observer" --monotonic-ns) && deadline=$((sampled + 60000000000)) &&
        record_once request-deadline "$deadline" && mkdir -m 700 "$stock_draft" && exec 6<"$stock_draft" &&
        [ "$stock_draft" -ef /proc/self/fd/6 ] && chmod g-s /proc/self/fd/6 &&
        dir_id=$(stock_request_dir "$stock_draft") && record_once request-dir "$dir_id" || return 1
    (set -C; printf 'WEREAD_LIBRARY_REQUEST %s %s\n' "$label" "$deadline" >"/proc/self/fd/6/request") &&
        exec 7</proc/self/fd/6/request && file_id=$(stock_request_file "$stock_draft/request") &&
        [ "$stock_draft/request" -ef /proc/self/fd/7 ] && record_once request-file "$file_id" &&
        "$stage/render-observer" --publish-frame-request "$label" &&
        [ "$(library_submit_pinned)" = "$(printf '%s\n%s' "$dir_id" "$file_id")" ] && write_tag request-live &&
        library_submit_arm "$deadline" || return 1
    command=$("$stage/render-observer" --request-stdin "$deadline" 5<&- 6<&- 7<&-) || return 1
    case "$command" in cancel) return 0;; collect) ;; *) return 1;; esac
    while matches "$stock_fixed/request" "WEREAD_LIBRARY_REQUEST $label $deadline" || [ ! -s "$stock_fixed/request" ]; do
        owned && absent "$stage/stopped" && [ "$(library_submit_pinned)" = "$library_submit_identity" ] &&
            "$stage/render-observer" --monotonic-ns "$deadline" >/dev/null || return 1
        usleep 100000 || return 1
    done
    library_submit_collect
}

stock_request_serve() {
    owned && armed && native_idle && tag tile-prepared && stock_request_paths && stock_request_absent &&
        absent "$stage/tile-dispatch-intent" && absent "$stage/tile-release" && absent "$stage/stopped" &&
        plain "$stage/render-observer" && [ -x "$stage/render-observer" ] || return 1
    for name in request-holder request-live request-dir request-file request-deadline; do absent "$stage/$name" || return 1; done
    write_tag request-holder && exec 5<"$stage/request-holder" && flock -n 5 && tag request-holder || return 1
    exec 8<&-
    stock_request_hold
}
