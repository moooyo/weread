#!/bin/sh
set -eu
export LC_ALL=C
umask 077
[ "$#" = 3 ] && [ "$3" = none ] || exit 64
mode=$1; label=$2
case "$label" in ''|*[!0-9a-f]*) exit 64;; esac
[ "${#label}" = 16 ] || exit 64
volume=/mnt/us
tmp=/var/tmp
stage=$tmp/weread-exchange-diagnostic-$label
scratch=$volume/.weread-exchange-diagnostic-$label
fail() { printf 'refused %s\n' "$1" >&2; exit 1; }
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
directory() { [ -d "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
regular() { [ -f "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
owned() {
    directory "$1" && regular "$1/owner" && [ "$(stat -c '%h:%s' "$1/owner")" = 1:17 ] &&
        printf '%s\n' "$label" | cmp -s - "$1/owner"
}
native() {
    [ "$(uname -m)" = armv7l ] || return 1
    case "$(dd if=/proc/usid bs=6 count=1 2>/dev/null)" in G??27J|G??2BL|G??263|G??227|G??2BM|G??23L|G??23M|G??270) ;; *) return 1;; esac
    [ "$(sed -n 's/^Kindle[[:space:]]*\([0-9][0-9.]*\).*$/\1/p' /etc/prettyversion.txt)" = 5.19.5 ] &&
        [ "$(lipc-get-prop -s com.lab126.powerd state)" = active ] &&
        [ "$(lipc-get-prop -s com.lab126.appmgrd activeApp)" = com.lab126.KPPMainApp ] || return 1
    case "$(status statusbar)" in *start/running*) ;; *) return 1;; esac
    for app in weread aidoku bilicomics; do
        absent "/tmp/$app-launcher.lock" || return 1
        if pidof "$app" >/dev/null 2>&1; then return 1; else [ "$?" = 1 ] || return 1; fi
    done
    managers=$(pidof awesome) && [ -n "$managers" ] || return 1
    for pid in $managers; do case "$(awk '{print $3}' "/proc/$pid/stat")" in R|S) ;; *) return 1;; esac; done
}
admission() {
    directory "$volume" && directory "$tmp" && absent "$scratch" && absent "$stage" && native &&
        awk -v volume="$volume" '$2==volume && $4 ~ /(^|,)rw(,|$)/ {found=1} END{exit !found}' /proc/mounts
}
owner_file() { (set -C; printf '%s\n' "$label" >"$1/owner") && owned "$1"; }
lock_stage() { owned "$stage" && exec 8<"$stage" && flock -n 8 && [ "$stage" -ef /proc/self/fd/8 ] && owned "$stage"; }
marker() {
    regular "$1" && [ "$(stat -c %s "$1")" = 19 ] || return 1
    links=$(stat -c %h "$1") || return 1
    case "$links" in 1) ;; 2)
        shared=0
        for name in "$scratch/left" "$scratch/sub/right" "$scratch/sub/held"; do
            if regular "$name" && [ "$name" -ef "$1" ]; then shared=$((shared+1)); fi
        done
        [ "$shared" = 2 ] || return 1;; *) return 1;; esac
    printf '%s:A\n' "$label" | cmp -s - "$1" || printf '%s:B\n' "$label" | cmp -s - "$1"
}
close_stages() {
    if absent "$stage"; then absent "$scratch"; return; fi
    lock_stage || return 1
    for item in "$stage"/* "$stage"/.[!.]* "$stage"/..?*; do
        absent "$item" && continue
        case "$item" in "$stage/owner") ;; "$stage/helper") regular "$item" && [ "$(stat -c %h "$item")" = 1 ] || return 1;; *) return 1;; esac
    done
    if ! absent "$scratch"; then
        owned "$scratch" || return 1
        for item in "$scratch"/* "$scratch"/.[!.]* "$scratch"/..?*; do
            absent "$item" && continue
            case "$item" in "$scratch/owner") ;; "$scratch/left") marker "$item" || return 1;; "$scratch/sub") directory "$item" || return 1;; *) return 1;; esac
        done
        if ! absent "$scratch/sub"; then
            for item in "$scratch/sub"/* "$scratch/sub"/.[!.]* "$scratch/sub"/..?*; do
                absent "$item" && continue
                case "$item" in "$scratch/sub/right"|"$scratch/sub/held") marker "$item" || return 1;; *) return 1;; esac
            done
        fi
        for item in "$scratch/left" "$scratch/sub/right" "$scratch/sub/held"; do
            if ! absent "$item"; then marker "$item" && rm "$item" || return 1; fi
        done
        if ! absent "$scratch/sub"; then directory "$scratch/sub" && rmdir "$scratch/sub" || return 1; fi
        owned "$scratch" && rm "$scratch/owner" && rmdir "$scratch" || return 1
    fi
    if ! absent "$stage/helper"; then rm "$stage/helper" || return 1; fi
    owned "$stage" && rm "$stage/owner" && rmdir "$stage" && absent "$stage" && absent "$scratch"
}

case "$mode" in
    admit) admission || fail admission; printf 'admitted\n';;
    probe)
        admission || fail admission
        mkdir "$stage" && owner_file "$stage" && lock_stage || fail private_stage
        (set -C; ulimit -f 512; cat >"$stage/helper") && regular "$stage/helper" &&
            [ "$(stat -c '%h:%s' "$stage/helper")" = '1:@HELPER_BYTES@' ] && chmod 700 "$stage/helper" || fail upload
        [ "$("$stage/helper" --policy 2>/dev/null)" = selector-diagnostic-v1 ] || fail helper_policy
        mkdir "$scratch" && owner_file "$scratch" || fail scratch_stage
        diagnostic_rc=0; "$stage/helper" diagnose "$label" 2>/dev/null || diagnostic_rc=$?
        case "$diagnostic_rc" in 0|1) ;; *) fail diagnostic_process;; esac
        ;;
    close) close_stages || fail cleanup_unconfirmed; printf 'closed\n';;
    absent) absent "$stage" && absent "$scratch" || fail residual; printf 'absent\n';;
    *) exit 64;;
esac
