set -eu
umask 077
export LC_ALL=C
mode=$1 label=$2 bytes=$3
fail() { printf 'refused %s\n' "$1"; exit 1; }
case "$label" in ''|*[!0-9a-f]*) fail label;; esac
[ "${#label}" = 16 ] || fail label
case "$bytes" in ''|*[!0-9]*) fail size;; esac
[ "${#bytes}" -le 9 ] && [ "$bytes" -gt 0 ] && [ "$bytes" -le 201326592 ] || fail size
volume=/mnt/us
stage=$volume/.weread-turn-cpu-$label
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
plain() { [ -f "$1" ] && [ ! -L "$1" ]; }
owned() { [ -d "$stage" ] && [ "$(readlink -f "$stage")" = "$stage" ] && plain "$stage/owner" && [ "$(cat "$stage/owner")" = "$label" ]; }
identity() {
    IFS= read -r rec <"/proc/$1/stat" || return 1; rest=${rec##*) }
    [ "${rec%% *}" = "$1" ] && [ "$rest" != "$rec" ] || return 1
    set -- $rest; [ "$#" -ge 20 ] || return 1; proc_state=$1; shift 19; birth=$1
    case "$birth" in ''|0|*[!0-9]*) return 1;; esac
}
registered() {
    plain "$stage/process" && IFS=' ' read -r child original <"$stage/process" || return 1
    case "$child:$original" in *[!0-9:]*|:*|*:) return 1;; esac
    identity "$child" 2>/dev/null && [ "$birth" = "$original" ] && [ "$proc_state" != Z ]
}
stop() {
    if absent "$stage/stopping"; then (set -C; : >"$stage/stopping") || return 1; fi
    if ! absent "$stage/go"; then plain "$stage/process" && plain "$stage/go" && [ "$(cat "$stage/go")" = "$label:$(cat "$stage/process")" ] || return 1; fi
    for tick in $(seq 1 15); do
        if ! registered; then return 0; fi
        if [ "$(readlink "/proc/$child/exe" 2>/dev/null)" = "$stage/probe" ]; then
            if [ "$tick" = 1 ]; then kill -TERM "$child" || :; elif [ "$tick" = 6 ]; then kill -KILL "$child" || :; fi
        fi
        sleep 1
    done
    ! registered
}
snapshot() {
    why=architecture; [ "$(uname -m)" = armv7l ] || return 1
    why=model; case "$(dd if=/proc/usid bs=6 count=1 2>/dev/null)" in G??27J|G??2BL|G??263|G??227|G??2BM|G??23L|G??23M|G??270) ;; *) return 1;; esac
    why=firmware; [ "$(sed -n 's/^Kindle[[:space:]]*\([0-9][0-9.]*\).*$/\1/p' /etc/prettyversion.txt)" = 5.19.5 ] || return 1
    why=peer_active; for app in weread aidoku bilicomics; do
        absent "/tmp/$app-launcher.lock" || return 1
        if pidof "$app" >/dev/null 2>&1; then return 1; else [ "$?" = 1 ] || return 1; fi
    done
    why=power; power=$(lipc-get-prop -s com.lab126.powerd state) || return 1
    case "$power" in active|screenSaver) ;; *) return 1;; esac
    why=foreground; app=$(lipc-get-prop -s com.lab126.appmgrd activeApp) || return 1
    if [ "$power" = active ]; then [ "$app" = com.lab126.KPPMainApp ] || return 1; fi
    case "$app" in *[!a-zA-Z0-9._-]*) return 1;; esac
    printf 'power=%s\napp=%s\n' "$power" "$app"
    why=services; status statusbar || return 1; managers=$(pidof awesome) || return 1
    for manager in $managers; do
        identity "$manager" || return 1
        case "$proc_state" in R|S) state=running;; T|t) state=stopped;; *) return 1;; esac
        printf 'wm=%s:%s:%s\n' "$manager" "$birth" "$state"
    done
    why=input; for item in /sys/class/input/event*; do [ -e "$item" ] || return 1; printf '%s\n' "${item##*/}"; done
    why=light; for prop in flIntensity currentAmberLevel; do light=$(lipc-get-prop -i com.lab126.powerd "$prop") || return 1; case "$light" in ''|*[!0-9]*) return 1;; esac; printf '%s=%s\n' "$prop" "$light"; done
    why=boot; awk '$1=="btime" && $2~/^[0-9]+$/ {print "boot",$2;ok=1} END {exit !ok}' /proc/stat
}
admit() {
    why=stage_occupied; absent "$stage" || return 1
    for item in "$volume"/.weread-turn-*; do absent "$item" || return 1; done
    snapshot >/dev/null || return 1
    why=mount; [ "$(readlink -f "$volume")" = "$volume" ] && awk '$2=="/mnt/us" && $4~/(^|,)rw(,|$)/ && $4!~/(^|,)noexec(,|$)/{ok=1} END {exit !ok}' /proc/mounts || return 1
    why=space; df -Pk "$volume" | awk -v need="$((bytes + 100663296))" 'END{exit !(NF>=6 && $4*1024>=need)}' || return 1
    why=memory; awk '$1=="MemAvailable:" && $2>=147456 {ok=1} END{exit !ok}' /proc/meminfo
}
case "$mode" in
    admit) admit || fail "$why"; printf 'admitted %s\n' "$power";;
    stage)
        admit || fail "$why"; mkdir "$stage" || fail creation
        (set -C; printf '%s\n' "$label" >"$stage/owner") || { rmdir "$stage" 2>/dev/null || :; fail owner; }
        owned && exec 9<"$stage" && flock -n 9 && snapshot >"$stage/before.next" && mv "$stage/before.next" "$stage/before" || fail staging_state
        (ulimit -f 393216; gzip -dc >"$stage/probe.next") && [ "$(stat -c %s "$stage/probe.next")" = "$bytes" ] && chmod 700 "$stage/probe.next" && mv "$stage/probe.next" "$stage/probe" || fail upload
        mkdir "$stage/tmp" "$stage/home" && printf 'staged\n';;
    run)
        owned && exec 9<"$stage" && flock -n 9 && plain "$stage/probe" && plain "$stage/before" && absent "$stage/starting" &&
            snapshot >"$stage/check" && cmp -s "$stage/before" "$stage/check" || fail admission_changed
        (set -C; : >"$stage/starting") || fail starting
        identity "$$" || fail parent; parent=$$; original_parent=$birth
        trap 'trap "" HUP INT TERM; stop || :; exit 1' HUP INT TERM
        (
            exec 9<&-; ulimit -c 0; ulimit -t 160; ulimit -f 65536
            export HOME="$stage/home" TMPDIR="$stage/tmp"
            unset DISPLAY WEREAD_HOME WEREAD_API_KEY WEREAD_READER_TURN_FRAME_DIR
            for tick in $(seq 1 8); do
                owned && absent "$stage/stopping" && identity "$parent" 2>/dev/null && [ "$birth" = "$original_parent" ] && [ "$proc_state" != Z ] || exit 1
                if plain "$stage/go" && registered && [ "$(cat "$stage/go")" = "$label:$child $original" ]; then
                    exec "$stage/probe" --ignored --exact app::reading::render_tests::measure_ordinary_pointer_turns --nocapture --test-threads=1
                fi
                sleep 1
            done
            exit 1
        ) </dev/null >"$stage/measurement" 2>"$stage/stderr" & child=$!
        identity "$child" && (set -C; printf '%s %s\n' "$child" "$birth" >"$stage/process.next") && mv "$stage/process.next" "$stage/process" &&
            (set -C; printf '%s:%s %s\n' "$label" "$child" "$birth" >"$stage/go.next") && mv "$stage/go.next" "$stage/go" || fail registration
        for tick in $(seq 1 200); do registered || break; sleep 1; done
        if registered; then stop || fail cleanup; fail deadline; fi
        wait "$child" || fail benchmark_failed
        [ "$(wc -c <"$stage/measurement")" -le 262144 ] && grep -F 'test result: ok. 1 passed; 0 failed; 0 ignored;' "$stage/measurement" >/dev/null || fail missing_test
        snapshot >"$stage/check" && cmp -s "$stage/before" "$stage/check" || fail changed_during_measurement
        sed -n 's/^.*READER_TURN_JSON /READER_TURN_JSON /p' "$stage/measurement";;
    close)
        if absent "$stage"; then printf 'absent\n'; exit 0; fi
        owned && exec 9<"$stage" || fail ownership
        for tick in $(seq 1 15); do flock -n 9 && break; [ "$tick" != 15 ] || fail busy; sleep 1; done
        owned && stop && awk -v p="$stage" '$5==p || index($5,p"/")==1 {bad=1} END{exit bad}' /proc/self/mountinfo || fail ownership
        comparable=1; if plain "$stage/before"; then
            if ! snapshot >"$stage/check" || ! cmp -s "$stage/before" "$stage/check"; then comparable=0; fi
        elif ! absent "$stage/starting"; then fail missing_baseline; fi
        rm -rf "$stage" && absent "$stage" || fail cleanup
        if [ "$comparable" = 1 ]; then printf 'closed\n'; else printf 'closed_state_changed\n'; fi;;
    absent) absent "$stage" || fail stage_present; printf 'absent\n';;
    *) fail mode;;
esac
