stock_selector_declined() {
    fixed owner "$label" && fixed selector-callback "$stock_selector_identity" && missing "$stage/selector-entered" &&
        (set -C; printf '%s\n' "$stock_selector_identity" >"$stage/selector-declined")
}

stock_selector_gate() {
    regular "$stage/owner" && exec 4<"$stage/owner" && flock -n 4 &&
        fixed owner "$label" && fixed selector-live "$label" && fixed tile-dispatch-intent "$label" || return 1
    if ! missing "$stage/selector-callback"; then
        (set -C; printf '%s\n' "$label" >"$stage/selector-duplicate") || :
        return 1
    fi
    IFS= read -r stock_selector_stat <"/proc/$$/stat" && [ "${stock_selector_stat%% *}" = "$$" ] || return 1
    stock_selector_tail=${stock_selector_stat##*) }
    [ "$stock_selector_stat" != "$stock_selector_tail" ] || return 1
    set -- $stock_selector_tail; [ "$#" -ge 20 ] || return 1
    shift 19
    case "$1" in ''|0*|*[!0-9]*) return 1;; esac
    [ "${#1}" -le 16 ] || return 1
    stock_selector_identity=$label:$$:$1
    (set -C; printf '%s\n' "$stock_selector_identity" >"$stage/selector-callback") && fixed selector-callback "$stock_selector_identity" || return 1
    trap 'stock_selector_declined || :; exit 1' EXIT
    trap 'exit 1' HUP INT TERM
    missing "$stage/selector-disarmed" && missing "$stage/selector-return-intent" && missing "$stage/stopped"
}

stock_selector_commit() {
    fixed selector-callback "$stock_selector_identity" && fixed selector-entered "$label" &&
        missing "$stage/selector-disarmed" && missing "$stage/selector-duplicate" || return 1
    trap - EXIT HUP INT TERM
    exec 4<&-
}
