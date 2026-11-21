#!/bin/sh
set -eu
export LC_ALL=C
umask 077
[ "$#" = 2 ] || exit 64
mode=$1; label=$2
case "$label" in ''|*[!0-9a-f]*) exit 64;; esac
[ "${#label}" = 16 ] || exit 64
stage=/mnt/us/.weread-render-$label
home=/mnt/us/weread
daily=/mnt/us/documents/weread.sh
proc=/proc
tmp=$(readlink -f /tmp) || exit 1
lock=$tmp/weread-launcher.lock
stock_shell=/bin/sh
shell_trusted=0
admit_reason=stage
prepare_reason=preparation

fail() { printf 'refused %s\n' "$1" >&2; exit 1; }
reply() { printf '{"state":"%s"}\n' "$1"; }
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
plain() { [ -f "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
directory() { [ -d "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
matches() { plain "$1" && printf '%s\n' "$2" | cmp -s - "$1"; }
tag() { matches "$stage/$1" "$label"; }
write_tag() { record_once "$1" "$label"; }
record_once() (
    dest=$stage/$1; value=$2
    if absent "$dest"; then
        if ! absent "$dest.next"; then plain "$dest.next" && rm "$dest.next" || return 1; fi
        (set -C; printf '%s\n' "$value" >"$dest.next") && mv "$dest.next" "$dest" || return 1
    fi
    matches "$dest" "$value"
)
owned() { directory "$stage" && tag owner; }
stage_lock() { owned || return 1; exec 8<"$stage"; flock -n 8 && owned; }
number() { case "$1" in ''|0|0*|*[!0-9]*) return 1;; esac; [ "${#1}" -le 16 ]; }
token() { case "$1" in ''|*[!0-9a-f]*) return 1;; esac; [ "${#1}" = 32 ]; }
seconds() {
    IFS=' ' read -r uptime ignored <"$proc/uptime" || return 1
    whole=${uptime%%.*}; number "$whole" && printf '%s\n' "$whole"
}
line() {
    plain "$1" && [ "$(stat -c %s "$1")" -le 256 ] || return 1
    IFS= read -r value <"$1" && matches "$1" "$value" && printf '%s\n' "$value"
}
identity() {
    number "$1" && IFS= read -r record <"$proc/$1/stat" && [ "${record%% *}" = "$1" ] || return 1
    tail=${record##*) }; [ "$tail" != "$record" ] || return 1
    set -- $tail
    [ "$#" -ge 20 ] || return 1
    process_state=$1; case "$process_state" in R|S|D|I) ;; *) return 1;; esac
    parent=$2; shift 19; birth=$1
    number "$parent" && number "$birth"
}
argv() { pid=$1; shift; printf '%s\000' "$@" | cmp -s - "$proc/$pid/cmdline"; }
system_shell() (
    actual=$(readlink -f "$stock_shell") && [ -f "$actual" ] && [ -x "$actual" ] || return 1
    case "$actual" in /bin/*|/usr/bin/*) ;; *) return 1;; esac
    for path in "${stock_shell%/*}" "$actual"; do
        while [ -n "$path" ]; do
            trust=$(stat -Lc '%u:%a' "$path") || return 1
            case "$trust" in 0:[0-7][0145][0145]|0:[0-7][0-7][0145][0145]) ;; *) return 1;; esac
            path=${path%/*}
        done
    done
)
shell_args() {
    shell_pid=$1; shift
    [ "$shell_trusted" = 1 ] && [ "$proc/$shell_pid/exe" -ef "$stock_shell" ] &&
        { argv "$shell_pid" sh "$home/launch.sh" "$@" || argv "$shell_pid" "$stock_shell" "$home/launch.sh" "$@"; }
}
worker_args() { shell_args "$1" --detached --managed-logs || shell_args "$1" --library-catalog-worker; }
lock_owner() (
    directory "$lock" && metadata=$(stat -c '%u:%a' "$lock") || return 1
    case "$metadata" in *:700|*:2700) ;; *) return 1;; esac
    account=${metadata%:*}; { [ "$account" = 0 ] || number "$account"; } && printf '%s\n' "$account"
)
private_records() (
    account=$1; shift
    values=$(stat -c '%u:%a' "$@") || return 1
    set -- $values; [ "$#" -gt 0 ] || return 1
    for metadata; do [ "$metadata" = "$account:600" ] || return 1; done
)
process_owner() {
    awk -v owner="$2" '$1=="Uid:" {seen++; if(NF!=5)bad=1; for(i=2;i<=5;i++)if(("u" $i)!=("u" owner))bad=1} END{exit !(seen==1 && !bad)}' "$proc/$1/status"
}
no_peer() {
    for app in aidoku bilicomics; do
        absent "$tmp/$app-launcher.lock" || return 1
        if pidof "$app" >/dev/null 2>&1; then return 1; else [ "$?" = 1 ] || return 1; fi
    done
}
native_idle() {
    no_peer && absent "$lock" || return 1
    if pidof weread >/dev/null 2>&1; then return 1; else [ "$?" = 1 ] || return 1; fi
    [ "$(lipc-get-prop -s com.lab126.powerd state)" = active ] &&
        [ "$(lipc-get-prop -s com.lab126.appmgrd activeApp)" = com.lab126.KPPMainApp ] || return 1
    case "$(status statusbar)" in *start/running*) ;; *) return 1;; esac
    managers=$(pidof awesome) && [ -n "$managers" ] || return 1
    for manager in $managers; do identity "$manager" || return 1; case "$process_state" in R|S) ;; *) return 1;; esac; done
}
document_identity() {
    plain "$daily" && size=$(stat -c %s "$daily") && number "$size" && [ "$size" -le 131072 ] || return 1
    { IFS= read -r header && [ "$header" = '#!/bin/sh' ] && IFS= read -r title && [ "$title" = '# Name: WeRead for Kindle' ]; } <"$daily" || return 1
    [ "$(tail -n 1 "$daily")" = 'exec sh "$WEREAD_HOME/launch.sh" "$@"' ] && stat -c '%d:%i:%s:%f:%u:%g:%h' "$daily"
}
save_document() {
    absent "$stage/document-original" && absent "$stage/document-original.next" &&
        (set -C; dd if="$daily" bs=1024 count=129 >"$stage/document-original.next" 2>/dev/null) &&
        [ "$(stat -c %s "$stage/document-original.next")" -le 131072 ] && cmp -s "$daily" "$stage/document-original.next" &&
        mv "$stage/document-original.next" "$stage/document-original"
}
state() {
    native_idle || return 10
    [ "$(uname -m)" = armv7l ] || return 11
    case "$(dd if=/proc/usid bs=6 count=1 2>/dev/null)" in G??27J|G??2BL|G??263|G??227|G??2BM|G??23L|G??23M|G??270) ;; *) return 11;; esac
    [ "$(sed -n 's/^Kindle[[:space:]]*\([0-9][0-9.]*\).*$/\1/p' /etc/prettyversion.txt)" = 5.19.5 ] || return 11
    document_identity || return 12
    for file in "$home/launch.sh" "$home/app/weread" "$home/app/weread-frame-guard" "$home/app/weread-native-query"; do
        plain "$file" && stat -c '%d:%i:%s:%Y:%Z:%f' "$file" || return 12
    done
    [ "$(stat -c %s "$home/app/weread")" = 95137176 ] && [ -x "$home/app/weread" ] || return 12
    plain /var/local/cc.db && [ "$(sqlite3 -readonly -batch -noheader -cmd '.timeout 1500' /var/local/cc.db "SELECT count(*)||':'||coalesce(sum(p_location COLLATE BINARY='$daily' AND p_mimeType='text/x-shellscript'),0) FROM Entries WHERE p_location COLLATE NOCASE='$daily';")" = 1:1 ] || return 12
    for manager in $managers; do identity "$manager" && printf 'manager %s:%s\n' "$manager" "$birth" || return 1; done
    for node in /sys/class/input/event*; do [ -e "$node" ] && printf 'input %s\n' "${node##*/}" || return 1; done
    for prop in flIntensity currentAmberLevel; do
        value=$(lipc-get-prop -i com.lab126.powerd "$prop") || return 1
        case "$value" in ''|*[!0-9]*) return 1;; esac
        [ "$value" -le 24 ] && printf 'light %s %s\n' "$prop" "$value" || return 1
    done
    awk '$1=="btime"&&$2~/^[0-9]+$/ {print "boot",$2; ok=1} END{exit !ok}' "$proc/stat"
}
view() {
    value=$(lipc-get-prop -s com.lab126.appmgrd activeView 2>/dev/null) || value=
    case "$value" in KPP_HOME) printf 'kpp_home\n';; KPP_LIBRARY) printf 'kpp_library\n';; *) printf 'unavailable\n';; esac
}
ready_volume() {
    directory /mnt/us && awk '$2=="/mnt/us" && $4 ~ /(^|,)rw(,|$)/ && $4 !~ /(^|,)noexec(,|$)/ {ok=1} END{exit !ok}' "$proc/mounts" &&
        df -Pk /mnt/us | awk 'END{exit !(NF>=6 && $4+0>=245760)}'
}
state_refusal() {
    case "$1" in 10) admit_reason=not_native_idle;; 11) admit_reason=model_or_firmware;; 12) admit_reason=daily_install_guard;; *) admit_reason=native_projection;; esac
}
admit() {
    absent "$stage" || { admit_reason=stage_occupied; return 1; }
    ready_volume || { admit_reason=volume; return 1; }
    state >/dev/null || { state_refusal "$?"; return 1; }
}
same() {
    snapshot=$(state) && matches "$stage/before" "$snapshot" && plain "$stage/document-original" && cmp -s "$daily" "$stage/document-original"
}
new_stage() {
    admit || return 1
    mkdir "$stage" && write_tag owner && stage_lock || return 1
    initial=$(state) || { state_refusal "$?"; return 1; }
    initial_view=$(view) || return 1
    save_document && record_once before "$initial" && record_once view "$initial_view" && write_tag navigation-untouched && write_tag staged
}
install() {
    tag staged && absent "$stage/installed" && absent "$stage/armed" && absent "$stage/input.tgz" && same || return 1
    for leaf in members types unpack tile render-observer; do absent "$stage/$leaf" || return 1; done
    (set -C; ulimit -f 131072; cat >"$stage/input.tgz") || return 1
    [ "$(stat -c %s "$stage/input.tgz")" -le 67108864 ] || return 1
    (set -C; ulimit -f 64; tar -tzf "$stage/input.tgz" >"$stage/members" && tar -tvzf "$stage/input.tgz" >"$stage/types") || return 1
    awk '/\.\./ || seen[$0]++ || length($0)>90 {exit 1}
        !/^(render-observer|tile\/(native_tile\.sh|native-target|card-input|lib\/[a-zA-Z0-9][a-zA-Z0-9._+-]*|models\/(eng|chi_sim)\.traineddata))$/ {exit 1}
        END{if(NR<7 || NR>96 || !("render-observer" in seen) || !("tile/native_tile.sh" in seen) || !("tile/native-target" in seen) || !("tile/card-input" in seen) || !("tile/lib/ld-linux-armhf.so.3" in seen) || !("tile/models/eng.traineddata" in seen) || !("tile/models/chi_sim.traineddata" in seen)) exit 1}' "$stage/members" || return 1
    awk '$1!~/^-(rwxr-xr-x|rw-r--r--)$/ || $3!~/^[0-9]+$/ || $3<1 || $3+0>50331648 {bad=1} {size+=$3} END{exit (bad || NR<7 || NR>96 || size>134217728)}' "$stage/types" || return 1
    mkdir "$stage/unpack" && tar -xzf "$stage/input.tgz" -C "$stage/unpack" || return 1
    while IFS= read -r member; do plain "$stage/unpack/$member" || return 1; done <"$stage/members"
    for member in render-observer tile/native_tile.sh tile/card-input tile/native-target tile/lib/ld-linux-armhf.so.3; do [ -x "$stage/unpack/$member" ] || return 1; done
    absent "$stage/tile" && absent "$stage/render-observer" && mv "$stage/unpack/tile" "$stage/tile" &&
        mv "$stage/unpack/render-observer" "$stage/render-observer" && rmdir "$stage/unpack" &&
        rm "$stage/input.tgz" && write_tag installed
}
tile() { tag installed && plain "$stage/tile/native_tile.sh" && sh "$stage/tile/native_tile.sh" "$1" "$label"; }
stock_inspect() {
    position=$(tile inspect) && [ "${#position}" -le 24 ] || return 1
    printf '%s\n' "$position" | awk 'NR==1 && /^(home|library) (0|[1-9][0-9]*) (0|[1-9][0-9]*)$/ && $2<1860 && $3<2480 {valid=1; record=$0} END{if(NR!=1 || !valid)exit 1; print record}'
}
origin_agrees() {
    absent "$stage/navigation-untouched" && tag prepare-started && original=$(line "$stage/stock-original") && matches "$stage/tile-original" "$original"
}
prepare() {
    for leaf in stopped armed stock-original prepare-started prepare-no-input tile-original tile-prepared tile-gate tile-dispatch-intent tile-release; do absent "$stage/$leaf" || return 1; done
    tag navigation-untouched && same || return 1
    matches "$stage/view" "$(view)" || { prepare_reason=native_view_changed; return 1; }
    original=$(stock_inspect) || { prepare_reason=stock_tabs_unconfirmed; return 1; }
    record_once stock-original "$original" && mv "$stage/navigation-untouched" "$stage/prepare-started" && tag prepare-started || return 1
    if report=$(tile prepare 2>&1); then
        [ "${report##*'
'}" = tile_prepared ] && tag tile-prepared || return 1
        origin_agrees || { prepare_reason=stock_origin_changed; return 1; }
    else
        case "${report##*'
'}" in
            'tile_refused original'|'tile_refused native')
                absent "$stage/tile-original" && write_tag prepare-no-input || return 1
                prepare_reason=stock_tabs_unconfirmed;;
            'tile_refused library_or_artwork') prepare_reason=library_or_artwork;;
        esac
        return 1
    fi
}
arm() {
    tag installed && tag tile-prepared && origin_agrees && absent "$stage/armed" && absent "$stage/tile-dispatch-intent" && same && identity "$$" || return 1
    record_once armed "$label:$birth"
}
armed() {
    arm_value=$(line "$stage/armed") || return 1
    fence=${arm_value#*:}; [ "$arm_value" = "$label:$fence" ] && number "$fence"
}
tap() {
    armed && tag tile-prepared && absent "$stage/stopped" && absent "$stage/render-cancel" && absent "$stage/tile-dispatch-intent" &&
        native_idle && tag observer-active && exec 9<"$stage/observer-active" || return 1
    if flock -n 9; then return 1; else [ "$?" = 1 ] || return 1; fi
    receipt=$(tile tap) && tag tile-dispatch-intent && matches "$stage/tile-release" "$receipt" && [ "${#receipt}" -le 480 ] || return 1
    printf '%s\n' "$receipt" | awk 'NR==1 && /^\{"ok":true,"mode":"XTest-button-1","clock":"CLOCK_MONOTONIC","hold_requested_ms":50,"down_confirmed":true,"up_sent":true,"buttons_clear_after":true,"target":"open-card","down_before_ns":[0-9]+,"down_after_ns":[0-9]+,"up_before_ns":[0-9]+,"up_after_ns":[0-9]+\}$/ {valid=1} END{exit !(NR==1 && valid)}' || return 1
    printf '%s\n' "$receipt"
}
observation_output() {
    IFS= read -r banner && [ "$banner" = ARMED ] || return 1
    printf 'ARMED\n'
    IFS= read -r candidate && [ "${#candidate}" -le 360 ] && IFS= read -r final && [ "$final" = __OK__ ] || return 1
    extra=; if IFS= read -r extra || [ -n "$extra" ]; then return 1; fi
    printf '%s\n' "$candidate" | awk '/^\{"clock":"CLOCK_MONOTONIC","scene":"(grid|list|empty)","evidence":"(submit_upper_bound|memory_only)","last_negative_ns":[0-9]+,"first_complete_scene_ns":[0-9]+,"confirmed_scene_ns":[0-9]+,"submit_upper_bound_ns":[0-9]+\}$/ {valid=1} END{exit !(NR==1 && valid)}' || return 1
    current=$(gui) && owned && absent "$stage/render-cancel" && matches "$stage/session" "$current" && record_once observed-session "$current" || return 1
    printf '%s\n' "$candidate"
}
observe() {
    armed && plain "$stage/render-observer" && absent "$stage/observer-active" && absent "$stage/render-cancel" || return 1
    write_tag observer-active && exec 9<"$stage/observer-active" && flock -n 9 || return 1
    exec 8<&-
    { if "$stage/render-observer" "$label" 2>/dev/null; then printf '__OK__\n'; else printf '__ERROR__\n'; fi; } | observation_output
}
observer_idle() {
    if ! absent "$stage/observer-active"; then tag observer-active && exec 9<"$stage/observer-active" && flock -n 9 || return 1; fi
    helpers=$(pidof render-observer 2>/dev/null) || { [ "$?" = 1 ] || return 1; helpers=; }
    for pid in $helpers; do
        number "$pid" || return 1
        path=$(readlink "$proc/$pid/exe") || { absent "$proc/$pid" && continue; return 1; }
        [ "$path" != "$stage/render-observer" ] || return 1
    done
}
observer_stopped() {
    observer_idle && return 0
    tag render-cancel || return 1
    for tick in $(seq 1 16); do usleep 125000; observer_idle && return 0; done
    return 1
}
supervision() {
    armed && tag tile-dispatch-intent && no_peer && account=$(lock_owner) && private_records "$account" "$lock/pid" "$lock/nonce" || return 1
    if ! absent "$stage/launch-owner" || ! absent "$stage/group"; then matches "$stage/launch-owner" "$account" || return 1; fi
    supervisor=$(line "$lock/pid") && nonce=$(line "$lock/nonce") && token "$nonce" && identity "$supervisor" || return 1
    super_birth=$birth; entry=$parent
    [ "$super_birth" -ge "$fence" ] && worker_args "$supervisor" && identity "$entry" || return 1
    entry_birth=$birth
    [ "$entry_birth" -ge "$fence" ] && [ "$super_birth" -ge "$entry_birth" ] && shell_args "$entry" --library &&
        process_owner "$entry" "$account" && process_owner "$supervisor" "$account" && identity "$supervisor" &&
        [ "$parent:$birth" = "$entry:$super_birth" ] && matches "$lock/pid" "$supervisor" && matches "$lock/nonce" "$nonce" || return 1
    printf '%s %s %s %s %s\n' "$entry" "$entry_birth" "$supervisor" "$super_birth" "$nonce"
}
gui() {
    group=$(supervision) && account=$(lock_owner) && matches "$stage/launch-owner" "$account" &&
        private_records "$account" "$lock/application-pid" "$lock/application-birth" "$lock/generation" "$lock/state-open" "$lock/application-owned" || return 1
    set -- $group; supervisor=$3; supervisor_birth=$4; nonce=$5
    gui_pid=$(line "$lock/application-pid") && gui_birth=$(line "$lock/application-birth") && identity "$gui_pid" || return 1
    [ "$birth" = "$gui_birth" ] && [ "$(readlink "$proc/$gui_pid/exe")" = "$home/app/weread" ] &&
        argv "$gui_pid" "$home/app/weread" --kindle --device scribe --data-dir "$home/data" || return 1
    runner=$parent; identity "$runner" && [ "$parent" = "$supervisor" ] && worker_args "$runner" || return 1
    runner_birth=$birth
    [ "$runner_birth" -ge "$supervisor_birth" ] && [ "$gui_birth" -ge "$runner_birth" ] &&
        process_owner "$runner" "$account" && process_owner "$gui_pid" "$account" &&
        matches "$lock/generation" "1:$nonce" && matches "$lock/state-open" "1:$nonce" &&
        matches "$lock/application-owned" "1:$nonce:$gui_pid:$gui_birth" && [ "$(supervision)" = "$group" ] &&
        identity "$gui_pid" && [ "$parent:$birth" = "$runner:$gui_birth" ] || return 1
    printf '%s %s %s %s %s\n' "$group" "$runner" "$runner_birth" "$gui_pid" "$gui_birth"
}
bind_group() {
    observed=$(supervision) && account=$(lock_owner) && record_once launch-owner "$account" && record_once group "$observed"
}
bind_gui() {
    bind_group && observed=$(gui) || return 1
    set -- $observed; [ "$#" = 9 ] && matches "$stage/group" "$1 $2 $3 $4 $5" && record_once session "$observed"
}
claim() {
    armed && tag tile-dispatch-intent && start=$(seconds) || return 1
    for tick in $(seq 1 80); do
        bind_gui && return 0
        at=$(seconds) && [ "$at" -lt "$((start+20))" ] || return 1
        usleep 150000
    done
    return 1
}
witness() {
    observer_idle && tag observer-active && absent "$stage/render-cancel" && current=$(gui) &&
        matches "$stage/session" "$current" && matches "$stage/observed-session" "$current"
}
retired() {
    number "$1" && number "$2" || return 1
    if absent "$proc/$1"; then return 0; fi
    IFS= read -r record <"$proc/$1/stat" || return 1
    [ "${record%% *}" = "$1" ] || return 1
    tail=${record##*) }; [ "$tail" != "$record" ] || return 1
    set -- "$2" $tail; [ "$#" -ge 21 ] || return 1
    known=$1; state_field=$2; shift 20; number "$1" || return 1
    [ "$1" != "$known" ] || [ "$state_field" = Z ] || [ "$state_field" = X ]
}
drained() {
    saved=$(line "$stage/group") || return 1
    set -- $saved; [ "$#" = 5 ] && retired "$1" "$2" && retired "$3" "$4" || return 1
    if ! absent "$stage/session"; then
        saved=$(line "$stage/session") || return 1
        set -- $saved; [ "$#" = 9 ] && matches "$stage/group" "$1 $2 $3 $4 $5" && retired "$6" "$7" && retired "$8" "$9" || return 1
    fi
}
stop_session() {
    observer_stopped || return 1
    if absent "$stage/tile-dispatch-intent"; then
        absent "$stage/session" && absent "$stage/group" && absent "$stage/launch-owner" && absent "$stage/tile-release" && absent "$stage/observed-session" &&
            absent "$stage/term-intent" && absent "$stage/term-sent" && native_idle && write_tag stopped
        return
    fi
    armed && tag tile-dispatch-intent && start=$(seconds) || return 1
    if ! absent "$stage/term-sent"; then tag term-sent && tag term-intent || return 1; fi
    for tick in $(seq 1 240); do
        if ! absent "$stage/group" && drained && native_idle; then write_tag stopped; return; fi
        if ! tag term-sent && bind_group; then
            ready=0
            if bind_gui; then current=$(gui) && matches "$stage/session" "$current" && ready=1
            elif absent "$stage/session" && absent "$lock/application-pid" && absent "$lock/application-birth" &&
                 absent "$lock/application-owned" && absent "$lock/generation"; then ready=1; fi
            if [ "$ready" = 1 ]; then
                write_tag term-intent && current=$(supervision) && matches "$stage/group" "$current" || return 1
                set -- $current; target=$3; original=$4; owner_pid=$1
                identity "$target" && [ "$birth:$parent" = "$original:$owner_pid" ] && kill -TERM "$target" && write_tag term-sent || return 1
            fi
        fi
        at=$(seconds) && [ "$at" -lt "$((start+52))" ] || return 1
        usleep 150000
    done
    return 1
}
no_input_evidence() {
    for leaf in tile-original tile-prepared armed tile-gate tile-dispatch-intent tile-release group launch-owner session observed-session term-intent term-sent; do absent "$stage/$leaf" || return 1; done
}
no_navigation_handoff() {
    absent "$stage/prepare-started" && absent "$stage/prepare-no-input" && no_input_evidence || return 1
    if tag staged; then tag navigation-untouched; else absent "$stage/staged" && absent "$stage/installed"; fi
}
stock_return() {
    if absent "$stage/stock-original"; then
        no_navigation_handoff
        return
    fi
    tag staged && original=$(line "$stage/stock-original") || return 1
    if absent "$stage/prepare-started"; then
        no_navigation_handoff || return 1
    elif ! tag prepare-started || ! absent "$stage/navigation-untouched"; then return 1
    elif absent "$stage/tile-original"; then
        tag prepare-no-input && no_input_evidence || return 1
    else
        absent "$stage/prepare-no-input" && matches "$stage/tile-original" "$original" || return 1
        if [ "$1" = restore ]; then [ "$(tile restore)" = tile_restored ] || return 1; fi
    fi
    observed_stock=$(stock_inspect) && [ "$observed_stock" = "$original" ]
}
restore() {
    tag stopped && observer_idle && native_idle && stock_return restore || return 1
    if tag staged; then same && matches "$stage/view" "$(view)" || return 1
    else absent "$stage/tile-original" && absent "$stage/armed" && state >/dev/null || return 1; fi
    write_tag restored
}
close_stage() {
    tag restored && tag stopped && observer_idle && native_idle && stock_return verify || return 1
    if tag staged; then same && matches "$stage/view" "$(view)" || return 1; else state >/dev/null || return 1; fi
    if ! absent "$stage/tile-dispatch-intent"; then drained || return 1; fi
    awk -v stage="$stage" '$2==stage || index($2,stage"/")==1 {bad=1} END{exit bad}' "$proc/mounts" &&
        owned && rm -rf "$stage" && absent "$stage" && native_idle
}

case "$mode" in
    admit) admit 2>/dev/null || fail "$admit_reason"; reply admitted;;
    absent) absent "$stage" && state >/dev/null 2>&1 || fail absent; reply absent;;
    stage) new_stage 2>/dev/null || fail "$admit_reason"; reply staged;;
    install|prepare|arm|tap|claim|observe|witness|cancel|stop|restore|close)
        if absent "$stage"; then
            case "$mode" in stop|restore|close) state >/dev/null 2>&1 || fail unsafe_absence;; *) fail absent_stage;; esac
            reply already_absent
            exit 0
        fi
        stage_lock 2>/dev/null || fail stage_owner_or_busy
        case "$mode" in claim|stop|witness|observe) if system_shell 2>/dev/null; then shell_trusted=1; fi;; esac
        case "$mode" in
            install) install 2>/dev/null || fail install; reply installed;;
            prepare) prepare 2>/dev/null || fail "$prepare_reason"; reply prepared;;
            arm) arm 2>/dev/null || fail arm; reply armed;;
            tap) tap 2>/dev/null || fail uncertain_or_refused_tap;;
            observe) observe 2>/dev/null || fail observer;;
            witness) witness 2>/dev/null || fail scene_identity; reply witnessed;;
            cancel) write_tag render-cancel 2>/dev/null || fail cancel; reply cancelled;;
            claim) claim 2>/dev/null || fail launch_identity; reply claimed;;
            stop) stop_session 2>/dev/null || fail unproven_close; reply stopped;;
            restore) restore 2>/dev/null || fail native_restore; reply restored;;
            close) close_stage 2>/dev/null || fail closeout; reply closed;;
        esac;;
    *) exit 64;;
esac
