# Source only in a diagnostic device program that already proved its stock
# GUI and owns the stage. Keep these records out of the v6 timing extractor.
collect_portrait_notification() (
    startup_trace_identity || return 1
    trace=$lock/portrait-notification.diag
    stock_private_record "$trace" && exec 5<"$trace" || return 1
    [ /proc/self/fd/5 -ef "$trace" ] &&
        [ "$(stat -Lc '%u:%a:%h' /proc/self/fd/5)" = '0:600:1' ] || return 1
    size=$(stat -Lc %s /proc/self/fd/5) && [ "$size" -gt 0 ] && [ "$size" -le 8192 ] || return 1
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
    [ "${#rows}" -eq "$((size - 1))" ] && [ /proc/self/fd/5 -ef "$trace" ] &&
        [ "$(stat -Lc %s /proc/self/fd/5)" = "$size" ] &&
        startup_trace_identity || return 1
    printf '%s\n' "$rows"
)
