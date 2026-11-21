#!/bin/sh
set -eu
export LC_ALL=C
umask 077
[ "$#" = 3 ] && [ "$3" = none ] || exit 64
mode=$1; label=$2
case "$label" in ''|*[!0-9a-f]*) exit 64;; esac
[ "${#label}" = 16 ] || exit 64
tmp=$(readlink -f /tmp) || exit 1
case "$tmp" in /tmp|/var/tmp) ;; *) exit 1;; esac
stage=$tmp/weread-pose-probe-$label
proc=/proc
lock=$tmp/weread-launcher.lock
home=/mnt/us/weread
daily=/mnt/us/documents/weread.sh

@READERS@

fail() { printf 'refused %s\n' "$1" >&2; exit 1; }
owned() {
    directory "$stage" && matches "$stage/owner" "$label" &&
        [ "$(stat -c '%u:%a:%h' "$stage/owner")" = '0:600:1' ] || return 1
    case "$(stat -c '%u:%a' "$stage")" in 0:700|0:2700) ;; *) return 1;; esac
}
projection() {
    state || return "$?"
    view && peer_projection || return 14
}
refuse_projection() {
    case "$1" in 10) fail not_native_idle;; 11) fail model_or_firmware;; 12) fail daily_install_guard;;
        14) fail peer_or_registration_guard;; *) fail native_projection;; esac
}
clean_locked() {
    owned || return 1
    for item in "$stage"/* "$stage"/.[!.]* "$stage"/..?*; do
        absent "$item" && continue
        case "$item" in "$stage/owner"|"$stage/snapshot") plain "$item" || return 1;; *) return 1;; esac
    done
    rm -f "$stage/snapshot" && rm "$stage/owner" && rmdir "$stage" && absent "$stage"
}
stage_lock() { owned && exec 8<"$stage" && flock -n 8 && owned; }
cleanup_exit() {
    status=$?; trap - 0 HUP INT TERM
    clean_locked || fail cleanup
    exit "$status"
}
native_read() {
    if record=$("$home/app/weread" --kindle --device scribe --probe-native-portrait-calibration 2>/dev/null); then
        case "$record" in [UD]:[0-3]|FIXED:[0-3]) printf '%s\n' "$record";; *) return 1;; esac
    else printf 'unavailable\n'; fi
}
@TRACE_FUNCTIONS_BEGIN@
trace_timer_ready() {
    command -v timeout >/dev/null && timeout -s KILL 1 sh -c 'exit 0' 2>/dev/null || return 1
    if timeout -s KILL 1 sleep 2 </dev/null >/dev/null 2>&1; then return 1
    else case "$?" in 124|137) return 0;; *) return 1;; esac; fi
}
trace_sample() {
    if ! value=$(timeout -s KILL 1 "$stage/snapshot" --display-sample </dev/null 2>/dev/null); then return 1; fi
    printf '%s\n' "$value" | awk 'NR==1 && $1=="sample" && NF==5 && $2~/^[1-9][0-9]*$/ && length($2)<=19 && $3~/^[1-9][0-9]*$/ && length($3)<=19 && $4~/^(15|16|17|18)$/ && $5~/^[0-3]$/ {print $2,$3,$4,$5;ok=1} END{exit !(NR==1 && ok)}'
}
trace_winmgr() {
    if (ulimit -f 1; exec timeout -s KILL 1 lipc-get-prop com.lab126.winmgr accelerometer) \
        </dev/null >"$stage/winmgr.out" 2>/dev/null; then
        value=$(dd if="$stage/winmgr.out" bs=1 count=3 2>/dev/null | od -An -v -tx1 | tr -d ' \n')
        case "$value" in 4c|4c0a) echo L; return;; 52|520a) echo R; return;;
            55|550a) echo U; return;; 44|440a) echo D; return;; esac
    fi
    printf 'unknown\n'
}
trace_clock() {
    IFS=' ' read -r uptime ignored <"$proc/uptime" || return 1
    seconds=${uptime%%.*}; case "$seconds" in ''|0|*[!0-9]*) return 1;; esac
    [ "${#seconds}" -le 9 ] && printf '%s\n' "$seconds"
}
trace_pause() { sleep 1; }
display_rows() {
    origin=$(trace_clock) || return 1
    label_end=complete; sequence=1
    while [ "$sequence" -le @TRACE_COUNT@ ]; do
        current=$(trace_clock) || return 1
        [ "$current" -ge "$origin" ] || return 1
        if [ "$((current - origin))" -ge 18 ]; then label_end=budget; break; fi
        left=$(trace_sample) || return 1
        winmgr=$(trace_winmgr) || return 1
        right=$(trace_sample) || return 1
        printf 'read %s %s %s %s\n' "$sequence" "$left" "$winmgr" "$right"
        sequence=$((sequence + 1))
        [ "$sequence" -gt @TRACE_COUNT@ ] || trace_pause || return 1
    done
    [ "$sequence" -gt 1 ] && printf 'trace_end %s\n' "$label_end"
}
@TRACE_FUNCTIONS_END@

case "$mode" in
    admit) absent "$stage" || fail occupied; projection >/dev/null || refuse_projection "$?"; printf 'admitted\n';;
    probe)
        absent "$stage" || fail occupied
        initial=$(projection) || refuse_projection "$?"
@TRACE_TIMER@
        mkdir "$stage" || fail occupied
        (set -C; printf '%s\n' "$label" >"$stage/owner") && stage_lock || fail ownership
        trap cleanup_exit 0; trap 'exit 1' HUP INT TERM
        (set -C; ulimit -f 2048; cat >"$stage/snapshot") && plain "$stage/snapshot" &&
            [ "$(stat -c %s "$stage/snapshot")" = @BYTES@ ] && chmod 700 "$stage/snapshot" || fail upload
@ORDINARY_READS_BEGIN@
        before=$(native_read) || fail native_read
        raw=$("$stage/snapshot" @COUNT@ 2>/dev/null) || fail sensor_unavailable
        printf '%s\n' "$raw" | awk 'NF != 4 || $1 != NR || $2 !~ /^[1-9][0-9]*$/ || $3 !~ /^[1-9][0-9]*$/ || $4 !~ /^(15|16|17|18)$/ {bad=1} END{exit (bad || NR != @COUNT@)}' || fail sensor_format
        after=$(native_read) || fail native_read
@ORDINARY_READS_END@
@DISPLAY_READS_BEGIN@
        (set -C; : >"$stage/winmgr.out") && plain "$stage/winmgr.out" || fail trace_output
        raw=$(display_rows) || fail display_sampling
@DISPLAY_READS_END@
        final=$(projection) && [ "$final" = "$initial" ] || fail protected_drift
        clean_locked || fail cleanup
        trap - 0 HUP INT TERM
@ORDINARY_RESULT_BEGIN@
        printf 'pose_probe %s\nnative_before %s\n%s\nnative_after %s\nprotected_state unchanged\n' "$label" "$before" "$raw" "$after"
@ORDINARY_RESULT_END@
@DISPLAY_RESULT_BEGIN@
        printf 'pose_display_trace %s\n%s\nprotected_state unchanged\n' "$label" "$raw"
@DISPLAY_RESULT_END@
        ;;
    close)
        if ! absent "$stage"; then stage_lock && clean_locked || fail cleanup; fi
        printf 'closed\n';;
    absent) absent "$stage" || fail residue; printf 'absent\n';;
    *) exit 64;;
esac
