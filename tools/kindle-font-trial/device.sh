#!/bin/sh
set -eu
export LC_ALL=C
umask 077
mode=$1
label=$2
name=${3:-none}
case "$label" in *[!0-9a-f]*|'') exit 64;; esac
[ "${#label}" -eq 16 ] || exit 64
stage=/mnt/us/.weread-indexed-$label
doc=/mnt/us/documents/weread-font-trial-$label.sh
daily=/mnt/us/documents/weread.sh
lock=$(readlink -f /tmp)/weread-launcher.lock
db=/var/local/cc.db
reg=/var/local/appreg.db
handler=tech.hackerdude.shell_integration.launcher
runtime=/var/local/kindle-catalog-entry/v1-2db100d0bcb60a76571cd8de96bed7ea6c016e6ab73e388a885f692955cacd86
fail() { printf 'refused %s\n' "$1" >&2; exit 1; }
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
plain() { [ -f "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
owned() { [ -d "$stage" ] && [ "$(readlink -f "$stage")" = "$stage" ] && plain "$stage/owner" && [ "$(cat "$stage/owner")" = "$label" ]; }
lock_stage() { for check in $(seq 1 80); do if flock -n 8; then return 0; fi; usleep 250000; done; return 1; }
sql() { sqlite3 -readonly -batch -noheader -cmd '.timeout 1500' "$1" "$2"; }
count_doc() { sql "$db" "SELECT count(*) FROM Entries WHERE p_location COLLATE NOCASE='$doc';"; }
wait_catalog() {
    for tick in $(seq 1 36); do
        if [ "$(count_doc)" = "$1" ]; then
            [ "$1" = 0 ] || [ "$(sql "$db" "SELECT count(*) FROM Entries WHERE p_location COLLATE BINARY='$doc' AND p_mimeType='text/x-shellscript';")" = 1 ] || return 1
            return 0
        fi
        sleep 1
    done
    return 1
}
scan() { lipc-set-prop -i com.lab126.scanner doFullScan 1 >/dev/null; }
native() {
    [ "$(uname -m)" = armv7l ] || return 1
    case "$(dd if=/proc/usid bs=6 count=1 2>/dev/null)" in G??27J|G??2BL|G??263|G??227|G??2BM|G??23L|G??23M|G??270) ;; *) return 1;; esac
    [ "$(sed -n 's/^Kindle[[:space:]]*\([0-9][0-9.]*\).*$/\1/p' /etc/prettyversion.txt)" = 5.19.5 ] &&
        [ "$(lipc-get-prop -s com.lab126.powerd state)" = active ] &&
        [ "$(lipc-get-prop -s com.lab126.appmgrd activeApp)" = com.lab126.KPPMainApp ] || return 1
    case "$(status statusbar)" in *start/running*) ;; *) return 1;; esac
    managers=$(pidof awesome) && [ -n "$managers" ] || return 1
    for pid in $managers; do
        case "$(sed -n 's/^State:[[:space:]]*\([A-Z]\).*/\1/p' "/proc/$pid/status")" in R|S) ;; *) return 1;; esac
    done
    for app in weread aidoku bilicomics; do
        absent "/tmp/$app-launcher.lock" || return 1
        if pidof "$app" >/dev/null; then return 1; fi
    done
}
settled() { for tick in $(seq 1 45); do if native; then return 0; fi; usleep 250000; done; return 1; }
light() {
    for prop in flIntensity currentAmberLevel; do
        value=$(lipc-get-prop -i com.lab126.powerd "$prop") || return 1
        case "$value" in ''|*[!0-9]*) return 1;; esac
        [ "$value" -le 24 ] || return 1
        printf '%s %s\n' "$prop" "$value"
    done
}
registration() {
    plain "$reg" &&
        [ "$(sql "$reg" "SELECT value FROM properties WHERE handlerId='$handler' AND name='command';")" = "$runtime/aidoku-library-dispatch" ] &&
        [ "$(sql "$reg" "SELECT count(*) FROM associations WHERE contentId='MT:text/x-shellscript' AND handlerId='$handler';")" = 1 ] || return 1
    sql "$reg" "BEGIN; SELECT * FROM properties WHERE handlerId='$handler' ORDER BY name,value; SELECT * FROM associations WHERE handlerId='$handler' OR contentId='MT:text/x-shellscript' ORDER BY contentId,handlerId; COMMIT;"
}
peer_proof() {
    registration >/dev/null || fail peer_registration
    drift=0
    while read -r expected file; do
        if [ "$file" = /var/local/kmc/bin/sh_integration_launcher ] && [ -f "$file" ] && [ ! -L "$file" ]; then :
        elif ! plain "$file"; then
            if [ -L "$file" ]; then kind=link
            elif absent "$file"; then kind=absent
            elif [ -f "$file" ]; then kind=linked_parent
            else kind=nonregular; fi
            printf 'peer_type %s %s\n' "$kind" "$file" >&2
            drift=1; continue
        fi
        actual=$(sha256sum "$file") || fail peer_file_read
        if [ "${actual%% *}" != "$expected" ]; then printf 'peer_content differs %s\n' "$file" >&2; drift=1; fi
    done <<'FROZEN_PEERS'
@PEER_FILES@
FROZEN_PEERS
    [ "$drift" = 0 ] || fail peer_files
    for file in /mnt/us/bilicomics /mnt/us/documents/bilicomics.sh /mnt/us/documents/BiliComics.sh \
        /mnt/us/documents/bilicomics.sdr /mnt/us/documents/BiliComics.sdr /mnt/us/documents/bilicomics.sh.sdr /mnt/us/documents/BiliComics.sh.sdr; do
        absent "$file" || fail peer_presence
    done
    [ "$(sql "$db" "SELECT count(*) FROM Entries WHERE p_location COLLATE NOCASE='/mnt/us/documents/bilicomics.sh';")" = 0 ] || fail peer_catalog
}
state() {
    cat /proc/sys/kernel/random/boot_id && pidof awesome && light && registration || return 1
    readlink -f /var/local/kmc/bin/sh_integration_launcher || return 1
    for node in /sys/class/input/event*; do [ -e "$node" ] || return 1; printf '%s\n' "${node##*/}"; done
    for file in "$daily" /mnt/us/weread/app/weread /mnt/us/weread/app/weread-frame-guard; do
        plain "$file" && stat -c '%d:%i:%s:%Y:%a' "$file" || return 1
    done
    absent /mnt/us/weread/launch.sh || return 1
    [ "$(sql "$db" "SELECT count(*) FROM Entries WHERE p_location COLLATE BINARY='$daily' AND p_mimeType='text/x-shellscript';")" = 1 ] || return 1
}
sidecars_absent() { for path in "${doc%.sh}.sdr" "$doc.sdr"; do absent "$path" || return 1; done; }
new_stage() {
    native || fail native_admission
    [ "$(readlink -f /mnt/us/documents)" = /mnt/us/documents ] || fail documents
    absent "$stage" && absent "$doc" && sidecars_absent && [ "$(count_doc)" = 0 ] || fail occupied
    peer_proof
    state >/dev/null || fail native_projection
    awk '$2=="/mnt/us" && $4 ~ /(^|,)rw(,|$)/ && $4 !~ /(^|,)noexec(,|$)/ {ok=1} END{exit !ok}' /proc/mounts || fail volume
    df -Pk /mnt/us | awk 'END{exit !(NF>=6 && $4+0>=393216)}' || fail space
}
same_state() { state >"$stage/observed" && cmp -s "$stage/before" "$stage/observed"; }
gui_owned() {
    plain "$lock/application-pid" && plain "$lock/application-birth" || return 1
    gui=$(cat "$lock/application-pid"); birth=$(cat "$lock/application-birth")
    case "$gui:$birth" in *[!0-9:]*|:*|*:) return 1;; esac
    [ "$(readlink "/proc/$gui/exe")" = "$home/app/weread" ] &&
        [ "$(awk '{print $22}' "/proc/$gui/stat" 2>/dev/null)" = "$birth" ]
}
archive_public() {
    cd "$stage"
    set -- out
    tar -czf - "$@"
}
case "$mode" in
    admit) new_stage; printf 'admitted\n';;
    stage)
        new_stage
        mkdir "$stage"; printf '%s\n' "$label" >"$stage/owner"
        exec 8<"$stage"; flock -n 8 || fail live_stage
        state >"$stage/before" || fail native_projection
        tar -xzf - -C "$stage" || fail partial_upload
        for arm in A B; do
            for file in launch.sh app/weread app/weread-frame-guard app/weread-native-query; do
                plain "$stage/$arm/$file" || fail missing_input
            done
        done
        plain "$stage/document" && plain "$stage/seed.tar.gz" && plain "$stage/measure" || fail missing_input
        mkdir "$stage/out"
        cmp -s "$stage/A/launch.sh" "$stage/B/launch.sh" || fail mismatched_launcher
        absent "$doc" && sidecars_absent && [ "$(count_doc)" = 0 ] || fail occupied
        (set -C; cat "$stage/document" >"$doc") || fail publish
        cmp -s "$doc" "$stage/document" || fail publish
        chmod 755 "$doc"; scan && wait_catalog 1 || fail indexing
        printf 'indexed\n';;
    case)
        owned && native && same_state && cmp -s "$stage/document" "$doc" && wait_catalog 1 || fail case_admission
        exec 8<"$stage"; flock -n 8 || fail live_stage
        case "$name" in [AB][0-6]|[AB]-library|[AB]-reader) ;; *) exit 64;; esac
        home=$stage/${name%"${name#?}"}; out=$stage/out/$name
        mkdir "$out" || fail repeated_case
        if [ -e "$home/data" ]; then [ "$(readlink -f "$home/data")" = "$home/data" ] && rm -rf "$home/data" || fail data_reset; fi
        mkdir "$home/data"; tar -xzf "$stage/seed.tar.gz" -C "$home/data" || fail data_restore
        printf 'scribe\n' >"$home/data/device-profile"; printf 'scribe 5.19.5\n' >"$home/data/device-allowlist"
        lipc-set-prop -i com.lab126.powerd touchScreenSaverTimeout 1 >/dev/null || fail active_timer
        sleep 8
        native || fail lost_native
        absent "$stage/armed" || fail stale_arm
        printf '%s\n' "$name" >"$stage/next"; mv "$stage/next" "$stage/armed"
        "$stage/measure" request "$label" >"$out/request" || fail request
        log=$home/data/logs/application-bootstrap.log
        ready=0
        for tick in $(seq 1 270); do
            if [ -f "$log" ] && grep -q ' first_application_frame_send$' "$log"; then ready=1; break; fi
            [ ! -f "$out/status" ] || break
            usleep 150000
        done
        [ "$ready" = 1 ] || fail no_send
        sleep 1
        gui_owned || fail gui_identity
        "$stage/measure" thumb >"$out/thumb.pgm" || fail thumb
        case "$name" in *-library|*-reader)
            "$stage/measure" panel >"$out/panel.pgm" || fail panel;;
        esac
        if ! gui_owned; then rm -f "$out/thumb.pgm" "$out/panel.pgm"; fail gui_changed; fi
        for tick in $(seq 1 200); do [ ! -f "$out/status" ] || break; usleep 200000; done
        [ "$(cat "$out/status")" = 0 ] && absent "$out/guard" || fail exit
        grep '^WEREAD_STARTUP ' "$log" >"$out/markers" || fail markers
        settled && same_state || fail native_return
        printf 'completed %s\n' "$name";;
    collect) owned && [ -d "$stage/out" ] || fail no_results; archive_public;;
    restore)
        if absent "$stage"; then absent "$doc" && sidecars_absent && [ "$(count_doc)" = 0 ] || fail unowned_document; printf 'restored\n'; exit 0; fi
        owned || fail unowned_stage
        exec 8<"$stage"; lock_stage || fail live_stage
        rm -f "$stage/armed"
        for tick in $(seq 1 75); do if native; then break; fi; sleep 1; done
        native || fail unrecovered
        if [ -d "$stage/out" ]; then
            for arm in A B; do
                for log in launcher.log application-bootstrap.log application.log frame-guard-bootstrap.log; do
                    file=$stage/$arm/data/logs/$log
                    if plain "$file"; then tail -c 16384 "$file" >"$stage/out/last-$arm-$log"; fi
                done
            done
        fi
        if ! absent "$doc"; then plain "$doc" && plain "$stage/document" && cmp -s "$doc" "$stage/document" || fail unowned_document; rm "$doc"; scan || fail removal_scan; fi
        wait_catalog 0 || fail residual_row
        for path in "${doc%.sh}.sdr" "$doc.sdr"; do
            absent "$path" && continue
            [ -d "$path" ] && [ "$(readlink -f "$path")" = "$path" ] && plain "$stage/before" || fail unknown_sidecar
            rmdir "$path" || fail nonempty_sidecar
        done
        settled && { [ ! -s "$stage/before" ] || same_state; } || fail native_restore
        peer_proof
        printf '%s\n' "$label" >"$stage/restored"; printf 'restored\n';;
    close)
        if ! absent "$stage"; then
            owned && plain "$stage/restored" && [ "$(cat "$stage/restored")" = "$label" ] && absent "$doc" && sidecars_absent || fail unowned_stage
            exec 8<"$stage"; flock -n 8 || fail live_stage
            rm -rf "$stage" || fail cleanup
        fi
        absent "$stage" && absent "$doc" && sidecars_absent && [ "$(count_doc)" = 0 ] && native || fail closeout
        printf 'closed\n';;
    *) exit 64;;
esac
