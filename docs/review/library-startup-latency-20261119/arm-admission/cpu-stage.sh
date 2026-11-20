#!/bin/sh
set -eu
export LC_ALL=C
umask 077

refuse() { printf 'refused %s\n' "$1"; exit 1; }
[ "$#" -eq 3 ] || refuse arguments
mode=$1
label=$2
bytes=$3
case "$mode" in check|sample|close) ;; *) refuse mode ;; esac
case "$label" in *[!0-9a-f]*|'') refuse label ;; esac
[ "${#label}" -eq 16 ] || refuse label
case "$bytes" in *[!0-9]*|'') refuse size ;; esac
[ "${#bytes}" -le 8 ] && [ "$bytes" -gt 0 ] && [ "$bytes" -le 67108864 ] || refuse size
volume=/mnt/us
stage=$volume/.weread-font-$label

absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
idle() {
    [ "$(uname -m)" = armv7l ] || refuse architecture
    case "$(dd if=/proc/usid bs=6 count=1 2>/dev/null)" in
        G??27J|G??2BL|G??263|G??227|G??2BM|G??23L|G??23M|G??270) ;;
        *) refuse model ;;
    esac
    firmware=$(sed -n 's/^Kindle[[:space:]]*\([0-9][0-9.]*\).*$/\1/p' /etc/prettyversion.txt)
    [ "$firmware" = 5.19.5 ] || refuse firmware
    [ "$(lipc-get-prop -s com.lab126.powerd state)" = active ] || refuse not_active
    [ "$(lipc-get-prop -s com.lab126.appmgrd activeApp)" = com.lab126.KPPMainApp ] || refuse foreground
    case "$(status statusbar)" in *start/running*) ;; *) refuse statusbar ;; esac
    managers=$(pidof awesome) || refuse window_manager
    for pid in $managers; do
        case "$(sed -n 's/^State:[[:space:]]*\([A-Z]\).*/\1/p' "/proc/$pid/status")" in
            S|R) ;; *) refuse window_manager ;;
        esac
    done
    for app in weread aidoku bilicomics; do
        if pidof "$app" >/dev/null; then refuse reader_active; fi
        absent "/tmp/$app-launcher.lock" || refuse launcher_active
    done
    if pidof font-cpu >/dev/null; then refuse operation_active; fi
    for path in "$volume"/.weread-font-* "$volume"/.aidoku-hw-*; do
        [ "$path" = "$stage" ] || absent "$path" || refuse existing_operation
    done
}
clear_stage() {
    [ -d "$stage" ] && [ ! -L "$stage" ] && [ -f "$stage/owner" ] &&
        [ ! -L "$stage/owner" ] && [ "$(cat "$stage/owner")" = "$label" ] || return 1
    for entry in "$stage"/* "$stage"/.[!.]* "$stage"/..?*; do
        absent "$entry" && continue
        case "${entry##*/}" in owner|font-cpu|probe.part) ;; *) return 1 ;; esac
        [ -f "$entry" ] && [ ! -L "$entry" ] || return 1
    done
    rm -f "$stage/font-cpu" "$stage/probe.part" && rm "$stage/owner" && rmdir "$stage"
}

if [ "$mode" = close ]; then
    if ! absent "$stage"; then
        [ -d "$stage" ] && [ ! -L "$stage" ] || refuse unowned_stage
        exec 9<"$stage"
        flock -n 9 || refuse operation_active
        clear_stage || refuse cleanup
    fi
    printf 'stage_absent\n'
    exit 0
fi

idle
absent "$stage" || refuse existing_stage
if [ "$mode" = check ]; then printf 'scribe_2022 fw_5.19.5 native_idle stage_absent\n'; exit 0; fi
awk -v path="$volume" '$2 == path && $4 ~ /(^|,)rw(,|$)/ && $4 !~ /(^|,)noexec(,|$)/ {ok=1} END {exit !ok}' /proc/mounts || refuse user_volume
df -Pk "$volume" | awk -v bytes="$bytes" 'END {exit !(NF>=6 && $4*1024 >= bytes+67108864)}' || refuse space
awk '$1 == "MemAvailable:" && $2 >= 131072 {ok=1} END {exit !ok}' /proc/meminfo || refuse memory
command -v flock >/dev/null && command -v gzip >/dev/null || refuse utilities
mkdir "$stage" || refuse stage_creation
exec 9<"$stage"
flock -n 9 || { rmdir "$stage"; refuse stage_lock; }
printf '%s\n' "$label" >"$stage/owner" || { rm -f "$stage/owner"; rmdir "$stage"; refuse owner_record; }
finish() {
    result=$?
    trap - 0 HUP INT TERM
    clear_stage || refuse cleanup
    printf 'stage_removed\n'
    exit "$result"
}
trap finish 0
trap 'exit 1' HUP INT TERM
gzip -dc >"$stage/probe.part" || refuse transfer
[ "$(wc -c <"$stage/probe.part")" -eq "$bytes" ] || refuse short_copy
chmod 700 "$stage/probe.part" && mv "$stage/probe.part" "$stage/font-cpu" || refuse publish
idle
ulimit -c 0
"$stage/font-cpu" --bench-guarded "$$" </dev/null || refuse benchmark
