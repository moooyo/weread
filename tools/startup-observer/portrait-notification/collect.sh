# Source only in a diagnostic device program, never in the v6 timing extractor.
portrait_diagnostic_identity() {
    [ "$(stat -c '%d:%i' "$stage")" = "$portrait_stage" ] && [ "$lock" -ef /proc/self/fd/6 ] &&
        absent "$stage/term-intent" && current=$(stock_prove_gui close read_only) &&
        matches "$stage/session" "$current" &&
        matches "$stage/portrait-diagnostic-submit" "$current $portrait_submitted $portrait_observed"
}

portrait_diagnostic_receipt() (
    # Recheck the stage identity without retaining a stage fd or flock. An
    # outstanding read must not carry a stage resource into native closeout.
    owned && portrait_stage=$(stat -c '%d:%i' "$stage") && entry_guard && system_shell || return 1
    shell_trusted=1
    single "$stage/portrait-diagnostic-submit" && anchor=$(line "$stage/portrait-diagnostic-submit") || return 1
    case "$anchor" in ''|*[!0-9a-f\ ]*) return 1;; esac
    set -- $anchor
    [ "$#" = 11 ] || return 1
    portrait_submitted=${10}; portrait_observed=${11}
    printf '%s %s\n' "$portrait_submitted" "$portrait_observed" | grep -Eq '^[1-9][0-9]{0,18} [1-9][0-9]{0,18}$' || return 1
    rows=$(collect_portrait_notification) || return 1
    printf 'WEREAD_PORTRAIT_SNAPSHOT %s %s %s\n%s\n' "$label" "$portrait_submitted" "$portrait_observed" "$rows"
)

collect_portrait_notification() (
    [ -d "$lock" ] && [ ! -L "$lock" ] && exec 6<"$lock" &&
        [ "$lock" -ef /proc/self/fd/6 ] && portrait_diagnostic_identity || return 1
    trace=$lock/portrait-notification.diag
    stock_private_record "$trace" && exec 5<"$trace" || return 1
    [ /proc/self/fd/5 -ef "$trace" ] &&
        stamp=$(stat -Lc '%u:%a:%h:%s' /proc/self/fd/5) || return 1
    case "$stamp" in 0:600:1:*) size=${stamp##*:};; *) return 1;; esac
    [ "$size" -gt 0 ] && [ "$size" -le 8192 ] || return 1
    rows=$(dd bs=8193 count=1 <&5 2>/dev/null | LC_ALL=C awk '
        {bytes+=length($0)+1}
        bytes>8192 || NR>30 || length($0)>240 || $1!="WEREAD_PORTRAIT_DIAG" {bad=1; exit}
        $2=="publish_begin" && NF==3 && $3~/^[0-9]+$/ {out[NR]=$0; next}
        $2=="publish_end" && NF==4 && $3~/^[0-9]+$/ && $4~/^(ok|rename_error)$/ {out[NR]=$0; next}
        $2=="wait" && NF==11 && $11~/^(hint|deadline|setup_error|poll_error|read_error|watch_lost|lookup_error|clock_error)$/ {
            for(i=3;i<=10;i++) if($i!~/^[0-9]+$/) {bad=1; exit}
            out[NR]=$0; next
        }
        {bad=1; exit}
        END {if(bad || !NR) exit 1; for(i=1;i<=NR;i++) print out[i]}
    ') || return 1
    [ "${#rows}" -eq "$((size - 1))" ] && portrait_diagnostic_identity &&
        stock_private_record "$trace" && [ /proc/self/fd/5 -ef "$trace" ] &&
        [ "$(stat -Lc '%u:%a:%h:%s' /proc/self/fd/5)" = "$stamp" ] || return 1
    printf '%s\n' "$rows"
)
