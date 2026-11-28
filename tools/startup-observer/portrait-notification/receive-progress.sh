# Only the separately sealed diagnostic device program embeds this reader.
stage_receive_reader() (
    base64 -d <<'WEREAD_RECEIVE_ELF'
@RECEIVER_BASE64@
WEREAD_RECEIVE_ELF
)

prepare_receive_reader() {
    [ "$stage" -ef /proc/self/fd/8 ] && owned || return 1
    receive_reader=$stage/load-reader
    receive_records=$stage/load-receive-progress
    receive_records_id=
    absent /proc/self/fd/8/load-reader && absent /proc/self/fd/8/load-receive-progress || return 1
    (set -C; stage_receive_reader >/proc/self/fd/8/load-reader) &&
        single "$receive_reader" && command exec 3</proc/self/fd/8/load-reader &&
        [ "$receive_reader" -ef /proc/self/fd/3 ] && [ "$(stat -Lc %s /proc/self/fd/3)" = @RECEIVER_BYTES@ ] &&
        chmod 700 /proc/self/fd/3 && [ -x "$receive_reader" ] &&
        receive_reader_id=$(stat -Lc '%d:%i' /proc/self/fd/3) || return 1
    # An unavailable sidecar is not an install failure. Give the reader an
    # empty identity rather than let it create or write an unproved file.
    if (set -C; : >/proc/self/fd/8/load-receive-progress) && single "$receive_records" &&
        command exec 4</proc/self/fd/8/load-receive-progress &&
        [ "$receive_records" -ef /proc/self/fd/4 ]; then
        receive_records_id=$(stat -Lc '%d:%i' /proc/self/fd/4) || receive_records_id=
    fi
    return 0
}

receive_progress_snapshot() (
    [ -n "$receive_records_id" ] && single "$receive_records" &&
        [ "$receive_records" -ef /proc/self/fd/4 ] &&
        identity=$(stat -Lc '%d:%i:%s' /proc/self/fd/4) || return 1
    size=${identity##*:}
    [ "${identity%:*}" = "$receive_records_id" ] && [ "$size" -gt 0 ] && [ "$size" -le 25600 ] || return 1
    latest=$(dd bs=25601 count=1 <&4 2>/dev/null | LC_ALL=C awk -v label="$label" -v size="$size" '
        {bytes+=length($0)+1}
        NR>160 || length($0)>159 || NF!=8 || $1!="WEREAD_STOCK_RECEIVE_PROGRESS" || "x"$2!="x"label ||
            $8!~/^(receiving|eof|overrun|read_error|write_error)$/ {bad=1; exit}
        {
            for (i=3;i<=7;i++) if (length($i)>19 || $i!~/^(0|[1-9][0-9]*)$/) {bad=1; exit}
            if ($0!=($1" "$2" "$3" "$4" "$5" "$6" "$7" "$8)) {bad=1; exit}
            last=$0
        }
        END {if(bad || !NR || bytes!=size) exit 1; print last}
    ') || return 1
    single "$receive_records" && [ "$receive_records" -ef /proc/self/fd/4 ] &&
        [ "$(stat -Lc '%d:%i:%s' /proc/self/fd/4)" = "$identity" ] && printf '%s\n' "$latest"
)

report_receive_progress() {
    # The archive writer lock has been reacquired. An unknown writer or a
    # malformed sidecar must not produce an install observation.
    if [ "${receive_joined:-0}:$load_child_unconfirmed" = 1:0 ] &&
        sample=$(receive_progress_snapshot 2>/dev/null); then
        printf '%s\n' "$sample" >&2 || :
    fi
}
