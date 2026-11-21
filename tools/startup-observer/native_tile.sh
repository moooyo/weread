#!/bin/sh
set -efu
umask 077
export LC_ALL=C OMP_THREAD_LIMIT=1 DISPLAY=:0
[ "$#" = 2 ] || exit 64
operation=$1; label=$2
case "$label" in ''|*[!0-9a-f]*) exit 64;; esac
[ "${#label}" = 16 ] || exit 64
stage=/mnt/us/.weread-render-$label; kit=$stage/tile
memo=$stage/tile-original; prepared=$stage/tile-prepared; intent=$stage/tile-dispatch-intent; release=$stage/tile-release
fail() { printf 'tile_refused %s\n' "$1" >&2; exit 1; }
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
plain() { [ -f "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
native() {
    plain "$stage/owner" && printf '%s\n' "$label" | cmp -s - "$stage/owner" || return 1
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
    for app in weread aidoku bilicomics; do absent "/tmp/$app-launcher.lock" && ! pidof "$app" >/dev/null || return 1; done
}
probe() {
    tool=$1; shift
    plain "$kit/lib/ld-linux-armhf.so.3" && plain "$kit/$tool" && "$kit/lib/ld-linux-armhf.so.3" --inhibit-cache --library-path "$kit/lib" "$kit/$tool" "$@"
}
inspect() {
    result=$(probe native-target navigation "$kit/models") || return 1
    printf '%s\n' "$result" | awk '
    /^\{"screen":\[1860,2480\],"pointer":\[[0-9]+,[0-9]+\],"buttons_clear":true,"home":\{"count":1,"box":\[[0-9]+,[0-9]+,[0-9]+,[0-9]+\],"underline_pct":[0-9]+\},"library":\{"count":1,"box":\[[0-9]+,[0-9]+,[0-9]+,[0-9]+\],"underline_pct":[0-9]+\},"title":\{"count":0\}\}$/ {
        gsub(/[^0-9]+/," "); if(NF!=17 || $3>=1860 || $4>=2480 || $6>=$8 || $7>=$9 || $8>=$12 || $12>=$14 || $13>=$15 || $7<2108 || $13<2108 || $14>=1860 || $9>=2480 || $15>=2480) exit 1;
        if($10>=60 && $10<=100 && $16<=20){print "home",$3,$4;ok=1} else if($16>=60 && $16<=100 && $10<=20){print "library",$3,$4;ok=1}
    } END{if(!ok)exit 1}'
}
tab() {
    current=$(inspect) || return 1
    [ "${current%% *}" != "$1" ] || return 0
    native && probe card-input "nav-$1" "$kit/models" >/dev/null || return 1
    for attempt in 1 2 3 4 5; do
        native && current=$(inspect) && [ "${current%% *}" = "$1" ] && return 0
        sleep 1
    done
    return 1
}
target() {
    observation=$(probe native-target card "$kit/models") || return 1
    case "$observation" in *'"buttons_clear":true,'*'"title":{"count":1,"box":['*) return 0;; *) return 1;; esac
}
native || fail native
case "$operation" in
    inspect) inspect || fail stock_tabs;;
    prepare)
        absent "$intent" && absent "$memo" || fail prior_attempt
        original=$(inspect) && (set -C; printf '%s\n' "$original" >"$memo") || fail original
        tab library && native && target || fail library_or_artwork
        (set -C; printf '%s\n' "$label" >"$prepared") || fail prepared
        printf 'tile_prepared\n';;
    tap)
        plain "$memo" && plain "$prepared" && printf '%s\n' "$label" | cmp -s - "$prepared" || fail missing_preparation
        absent "$intent" && absent "$release" || fail prior_attempt
        (set -C; printf '%s\n' "$label" >"$stage/tile-gate") || fail prior_attempt
        export WEREAD_TILE_LABEL=$label
        if ! record=$(probe card-input open-card "$kit/models"); then
            absent "$intent" || fail uncertain_press
            fail no_card_press
        fi
        plain "$intent" && printf '%s\n' "$label" | cmp -s - "$intent" || fail uncertain_press
        case "$record" in '{"ok":true,"mode":"XTest-button-1","clock":"CLOCK_MONOTONIC",'*'"target":"open-card","down_before_ns":'*'"up_before_ns":'*'"up_after_ns":'*'}') ;; *) fail uncertain_receipt;; esac
        (set -C; printf '%s\n' "$record" >"$release") || fail uncertain_record
        printf '%s\n' "$record";;
    restore)
        if absent "$memo"; then absent "$intent" && absent "$release" || fail lost_original; printf 'tile_untouched\n'; exit 0; fi
        plain "$memo" && [ "$(wc -c <"$memo")" -le 24 ] || fail original
        IFS=' ' read -r original column row extra <"$memo" || fail original
        case "$original" in home|library) ;; *) fail original;; esac
        case "$column:$row" in *[!0-9:]*|:*|*:) fail original;; esac
        [ -z "$extra" ] && [ "$column" -lt 1860 ] && [ "$row" -lt 2480 ] || fail original
        tab "$original" && native && probe card-input restore-pointer "$column" "$row" >/dev/null &&
            [ "$(inspect)" = "$original $column $row" ] || fail restore
        printf 'tile_restored\n';;
    *) exit 64;;
esac
