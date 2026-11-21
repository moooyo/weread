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

case "$mode" in
    admit) absent "$stage" || fail occupied; projection >/dev/null || refuse_projection "$?"; printf 'admitted\n';;
    probe)
        absent "$stage" || fail occupied
        initial=$(projection) || refuse_projection "$?"
        mkdir "$stage" || fail occupied
        (set -C; printf '%s\n' "$label" >"$stage/owner") && stage_lock || fail ownership
        trap cleanup_exit 0; trap 'exit 1' HUP INT TERM
        (set -C; ulimit -f 2048; cat >"$stage/snapshot") && plain "$stage/snapshot" &&
            [ "$(stat -c %s "$stage/snapshot")" = @BYTES@ ] && chmod 700 "$stage/snapshot" || fail upload
        before=$(native_read) || fail native_read
        raw=$("$stage/snapshot" @COUNT@ 2>/dev/null) || fail sensor_unavailable
        printf '%s\n' "$raw" | awk 'NF != 4 || $1 != NR || $2 !~ /^[1-9][0-9]*$/ || $3 !~ /^[1-9][0-9]*$/ || $4 !~ /^(15|16|17|18)$/ {bad=1} END{exit (bad || NR != @COUNT@)}' || fail sensor_format
        after=$(native_read) || fail native_read
        final=$(projection) && [ "$final" = "$initial" ] || fail protected_drift
        clean_locked || fail cleanup
        trap - 0 HUP INT TERM
        printf 'pose_probe %s\nnative_before %s\n%s\nnative_after %s\nprotected_state unchanged\n' "$label" "$before" "$raw" "$after"
        ;;
    close)
        if ! absent "$stage"; then stage_lock && clean_locked || fail cleanup; fi
        printf 'closed\n';;
    absent) absent "$stage" || fail residue; printf 'absent\n';;
    *) exit 64;;
esac
