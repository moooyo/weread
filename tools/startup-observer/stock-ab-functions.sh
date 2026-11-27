production=$home/launch.sh
slot=$stage/selector-original-slot
authority=/var/tmp/weread-stock-selector-bind
install_reason=install
activate_reason=selector_activation
bind_reason=
load_child_unconfirmed=0

selector_text() {
    cat <<'WEREAD_ONE_SHOT'
@SELECTOR@
WEREAD_ONE_SHOT
}

outer_manifest() {
    cat <<'WEREAD_OUTER'
@OUTER_MANIFEST@
WEREAD_OUTER
}

seed_manifest() {
    cat <<'WEREAD_SEED'
@SEED_MANIFEST@
WEREAD_SEED
}

single() { plain "$1" && [ "$(stat -c %h "$1")" = 1 ]; }
stamp() { single "$1" && stat -c '%d:%i:%s:%a:%u:%g:%Y:%h' "$1"; }
stock_mount_scope() (
    directory "$home" && directory "${daily%/*}" || return 1
    for other in /mnt/us/.weread-render-*; do absent "$other" || [ "$other" = "$stage" ] || return 1; done
    awk -v stage="$stage" -v home="$home" -v daily="$daily" -v target="$production" -v allow="$1" '
        $2==target && allow==1 {if(++exact>1)bad=1; next}
        $2==stage || index($2,stage"/")==1 || $2==home || index($2,home"/")==1 || $2==daily || $2=="/mnt/us/documents" {bad=1}
        END{exit (!NR || bad)}' "$proc/mounts" || return 1
    [ /proc/self/ns/mnt -ef /proc/1/ns/mnt ] && [ / -ef /proc/1/root ] || return 1
    for table in /proc/self/mountinfo /proc/1/mountinfo; do
        awk -v target="$production" -v count="$1" 'NF<10 || $1!~/^[0-9]+$/ {bad=1} $5=="/" {root++} $5==target {exact++}
            END{exit (bad || root!=1 || exact+0!=count)}' "$table" || return 1
    done
)
stock_clear() (
    stock_mount_scope 0 || return 1
    for file in "$production" "$daily" "$home/app/weread"; do
        single "$file" && [ "$file" -ef "/proc/1/root$file" ] &&
            [ "$(stat -c '%d:%i:%f:%u:%g:%s:%h' "$file")" = "$(stat -Lc '%d:%i:%f:%u:%g:%s:%h' "/proc/1/root$file")" ] || return 1
    done
    cmp -s "$daily" "/proc/1/root$daily" && cmp -s "$production" "/proc/1/root$production"
)
fresh_production() (
    single "$production" && [ -x "$production" ] && [ "$(stat -c %s "$production")" = @LAUNCH_BYTES@ ] &&
        single "$daily" && [ "$(stat -c %s "$daily")" = @DOCUMENT_BYTES@ ] || return 1
    if grep -Fqx 'selector_kind=weread_stock_ab_one_shot' "$production"; then return 1; else [ "$?" = 1 ]; fi
)
original_report() (
    launch_id=$(stamp "$production") && single "$daily" && document_id=$(stat -c '%d:%i:%s:%a:%u:%g:%h' "$daily") && app_id=$(stamp "$home/app/weread") || return 1
    printf '{"state":"%s","launch":"%s","document":"%s","application":"%s"}\n' "$1" "$launch_id" "$document_id" "$app_id"
)
original_matches() {
    stock_clear && fresh_production && matches "$stage/launch-stat" "$(stamp "$production")" &&
        single "$stage/launch-reference" && cmp -s "$stage/launch-reference" "$production" &&
        single "$stage/document-reference" && cmp -s "$stage/document-reference" "$daily"
}
original_or_pristine() {
    if tag installed; then original_matches
    else absent "$stage/installed" && absent "$stage/bind-intent" && absent "$stage/selector-live" && stock_clear && fresh_production &&
        { absent "$stage/launch-stat" || matches "$stage/launch-stat" "$(stamp "$production")"; }; fi
}
stock_empty() { absent "$stage" && absent "$authority" && stock_clear && fresh_production && state >/dev/null && stock_request_absent; }
bounded_audit() (
    absent "$authority" && stock_clear && fresh_production && stock_request_absent && state >/dev/null || return 1
    if ! absent "$stage"; then stage_lock && tag installed && absent "$stage/bind-intent" && original_matches && same || return 1; fi
    exec 4<&0
    cat "$daily" "$production" | cmp -s - /proc/self/fd/4 || return 1
    fresh_production
)
new_stage() {
    admit && mkdir "$stage" && write_tag owner && stage_lock || return 1
    initial=$(state) || { state_refusal "$?"; return 1; }
    initial_view=$(view) && id=$(stamp "$production") && fresh_production && save_document &&
        record_once launch-stat "$id" && record_once before "$initial" && record_once view "$initial_view" &&
        write_tag navigation-untouched && write_tag staged
}
check_archive() (
    file=$1; expected=$2; dest=$3
    single "$file" && single "$expected" && directory "$dest" || return 1
    (ulimit -f 96; tar -tzf "$file" >"$stage/names" && tar -tvzf "$file" >"$stage/types") || return 1
    awk 'NR==FNR{if(NF!=3||$1 in allowed)bad=1;allowed[$1]=$2":"$3;count++;next}
        !($0 in allowed) || seen[$0]++ {bad=1} END{exit(bad || NR!=count*2)}' "$expected" "$stage/names" || return 1
    awk 'NR==FNR{allowed[$1]=$2":"$3; count++; next}
        NF!=6 || !($6 in allowed) || seen[$6]++ || $3!~/^[0-9]+$/ || ($3":"$1)!=allowed[$6] {bad=1}
        END{exit(bad || NR!=count*2)}' "$expected" "$stage/types" || return 1
    tar -xzf "$file" -C "$dest" || return 1
    while read -r name size permission; do
        single "$dest/$name" && [ "$(stat -c %s "$dest/$name")" = "$size" ] || return 1
        [ "$permission" != -rwxr-xr-x ] || [ -x "$dest/$name" ] || return 1
    done <"$expected"
)
helpers_match() {
    for leaf in weread-frame-guard weread-native-query; do
        single "$trial/app/$leaf" && single "$home/app/$leaf" && cmp -s "$trial/app/$leaf" "$home/app/$leaf" || return 1
    done
}
receive_archive() {
    # A lingering BusyBox timer must not inherit the stage lock or an
    # archive descriptor. Pin the empty file first; the supervised reader
    # opens and locks that inode itself, without truncating it.
    (set -C; : >"$stage/input.tgz") || return 1
    single "$stage/input.tgz" && exec 7<>"$stage/input.tgz" && [ /proc/$$/fd/7 -ef "$stage/input.tgz" ] || return 1
    receive_id=$(stat -Lc '%d:%i' "/proc/$$/fd/7") || return 1
    if (exec 7<&- 8<&-; exec timeout -k 1 150 sh -c '
        exec 6<>"$1" && [ "$(stat -Lc "%d:%i" /proc/self/fd/6)" = "$2" ] && flock -nx 6 || exit 76
        exec head -c "$3" >&6
    ' weread-load-read "$stage/input.tgz" "$receive_id" $((@ARCHIVE_BYTES@+1))); then
        receive_status=0
    else
        receive_status=$?
    fi
    # A supervisor exit or signal status is not a join on its child. Take
    # the writer's file lock before processing or confirming retirement.
    if ! single "$stage/input.tgz" || ! { exec 6<"$stage/input.tgz" && [ /proc/$$/fd/6 -ef /proc/$$/fd/7 ] && flock -nx 6; }; then
        load_child_unconfirmed=1
        return 1
    fi
    exec 7<&- || { load_child_unconfirmed=1; return 1; }
    case "$receive_status" in
        0) return 0;;
        1) return 1;;
        124|143) return 124;;
        *) load_child_unconfirmed=1; return 1;;
    esac
}
install() {
    tag staged && same && stock_clear && absent "$authority" && stock_request_absent && fresh_production && matches "$stage/launch-stat" "$(stamp "$production")" || return 1
    for leaf in installed bind-intent armed input.tgz names types outer-manifest seed-manifest unpack arm tile render-observer selector-bind launch-reference document-reference arm-seed.tar.gz selector-original-slot selector-stat bind-stat bind-directory bind-owner arm-name; do
        absent "$stage/$leaf" || return 1
    done
    install_reason=install_receive_unavailable
    command -v timeout >/dev/null && (exec 8<&-; exec timeout -k 1 1 head -c 1) </dev/null >/dev/null || return 1
    if (exec 8<&-; exec timeout -k 1 1 sleep 3) </dev/null >/dev/null; then
        return 1
    else
        case "$?" in 124|143) ;; *) return 1;; esac
    fi
    install_reason=install_receive_failed
    if receive_archive; then :; else
        case "$?" in 124) install_reason=install_receive_timeout;; esac
        if [ "$load_child_unconfirmed" = 1 ]; then install_reason=install_receiver_unconfirmed; fi
        return 1
    fi
    install_reason=install_archive_stat
    received=$(stat -c %s "$stage/input.tgz") || return 1
    if [ "$received" != @ARCHIVE_BYTES@ ]; then
        install_reason=install_archive_size
        if number "$received" || [ "$received" = 0 ]; then
            if [ "$received" -lt @ARCHIVE_BYTES@ ]; then install_reason=install_archive_short
            else install_reason=install_archive_long; fi
        fi
        return 1
    fi
    install_reason=install_outer_archive
    (set -C; outer_manifest >"$stage/outer-manifest" && seed_manifest >"$stage/seed-manifest") &&
        mkdir "$stage/unpack" && check_archive "$stage/input.tgz" "$stage/outer-manifest" "$stage/unpack" || return 1
    for leaf in arm tile render-observer selector-bind launch-reference document-reference arm-seed.tar.gz; do mv "$stage/unpack/$leaf" "$stage/$leaf" || return 1; done
    rmdir "$stage/unpack" && rm "$stage/input.tgz" || return 1
    install_reason=production_reference
    original_matches && helpers_match && cmp -s "$trial/launch.sh" "$stage/launch-reference" || return 1
    install_reason=public_seed
    mkdir "$trial/data" &&
        check_archive "$stage/arm-seed.tar.gz" "$stage/seed-manifest" "$trial/data" &&
        matches "$trial/data/device-profile" scribe && matches "$trial/data/device-allowlist" 'scribe 5.19.5' || return 1
    install_reason=selector_stage
    rm "$stage/arm-seed.tar.gz" && record_once arm-name @LANE@ &&
        (set -C; selector_text >"$slot") && chmod 755 "$slot" && record_once selector-stat "$(stamp "$slot")" &&
        record_once bind-stat "$(stamp "$stage/selector-bind")" || return 1
    install_reason=selector_bind_prepare
    known_bind && checked_bind prepare bind_prepared || return 1
    install_reason=selector_original_pair
    pair_original && write_tag installed
}
known_bind() {
    single "$stage/selector-bind" && [ -x "$stage/selector-bind" ] && matches "$stage/bind-stat" "$(stamp "$stage/selector-bind")"
}
checked_bind() {
    if bind_message=$("$stage/selector-bind" "$1" stock "$label" 2>&1); then [ "$bind_message" = "$2" ]; return; fi
    case "$bind_message" in
        'refused bind_'*)
            category=${bind_message#refused }
            if printf '%s\n' "$category" | awk 'NR==1 && (/^bind_(stage|files|contended|journal|provenance|foreign_mount|mount_table|parent_(value|scope)|propagation|namespace|native_view|native_service|native_proc_(read|permission|limit)|unknown_state|(mount|unmount)_unconfirmed)$/ || (/^bind_(mount|unmount)_call_[1-9][0-9]*$/ && (substr($0,match($0,/[0-9]+$/))+0)<=4095)) {ok=1} END{exit !(ok && NR==1)}'; then bind_reason=$category; fi;;
    esac
    return 1
}
selector_at() {
    id=$(stamp "$1") && matches "$stage/selector-stat" "$id" && selector_text | cmp -s - "$1"
}
saved_at() {
    id=$(stamp "$1") && matches "$stage/launch-stat" "$id" && single "$stage/launch-reference" && single "$trial/launch.sh" &&
        cmp -s "$stage/launch-reference" "$1" && cmp -s "$trial/launch.sh" "$1"
}
authority_pin() {
    directory "$authority" && exec 3<"$authority" && [ "$authority" -ef /proc/self/fd/3 ] &&
        [ "$(stat -Lc '%u:%a' /proc/self/fd/3)" = 0:700 ] && matches "$stage/bind-directory" "$(stat -Lc '%d:%i' /proc/self/fd/3)"
}
authority_file() {
    [ -f "$1" ] && [ ! -L "$1" ] && [ "$(stat -c '%u:%a:%h' "$1")" = 0:600:1 ] && [ "$(stat -c %s "$1")" -le 5200 ]
}
authority_owned() (
    authority_pin && authority_file /proc/self/fd/3/owner && printf '%s\n' "$label" | cmp -s - /proc/self/fd/3/owner &&
        matches "$stage/bind-owner" "$(stat -c '%d:%i' /proc/self/fd/3/owner)" && [ "$authority" -ef /proc/self/fd/3 ]
)
authority_create() (
    owned && tag bind-intent && absent "$authority" && mkdir -m 700 "$authority" && directory "$authority" && exec 3<"$authority" &&
        [ "$authority" -ef /proc/self/fd/3 ] && [ "$(stat -Lc %u /proc/self/fd/3)" = 0 ] && chmod g-s /proc/self/fd/3 &&
        [ "$(stat -Lc '%u:%a' /proc/self/fd/3)" = 0:700 ] && record_once bind-directory "$(stat -Lc '%d:%i' /proc/self/fd/3)" &&
        (set -C; printf '%s\n' "$label" >/proc/self/fd/3/owner) && authority_file /proc/self/fd/3/owner &&
        record_once bind-owner "$(stat -c '%d:%i' /proc/self/fd/3/owner)" && authority_owned
)
authority_record() (
    leaf=/proc/self/fd/3/$1
    authority_file "$leaf" && IFS= read -r first <"$leaf" || return 1
    case "$1" in
        owner) authority_owned;;
        bind-journal|bind-journal.next)
            awk -v owner="$label" 'NR==1 {if(NF!=11 || $1!="bind-v1" || $2!="stock" || "l"$3!="l"owner || length($4)!=36 || $4~/[^0-9a-f-]/ || $11!~/^[0-9]+:[0-9]+$/)bad=1;
                    for(i=5;i<=10;i++)if($i!~/^[0-9]+$/)bad=1}
                NR==2 {suffix="/.weread-render-"owner"/selector-original-slot"; if($0!~/^\// || substr($0,length($0)-length(suffix)+1)!=suffix)bad=1}
                NR==3 || NR==4 {if(split($0,field,":")!=9 || field[6]!="1")bad=1; for(i=1;i<=9;i++)if(field[i]!~/^[0-9]+$/)bad=1}
                END{exit (NR!=4 || bad)}' "$leaf";;
        bind-active|bind-active.next)
            printf '%s\n' "$first" | awk -v owner="$label" 'NF==3 && "l"$1=="l"owner && $2~/^[1-9][0-9]*$/ && length($2)<=10 && $3~/^(0|[1-9][0-9]*)$/ && length($3)<=20 && $0==($1" "$2" "$3) {ok=1} END{exit !(NR==1 && ok)}' &&
                printf '%s\n' "$first" | cmp -s - "$leaf";;
        *) return 1;;
    esac
)
authority_contents() (
    for file in /proc/self/fd/3/* /proc/self/fd/3/.[!.]* /proc/self/fd/3/..?*; do
        absent "$file" || authority_record "${file##*/}" || return 1
    done
)
authority_empty_pinned() (
    [ "$authority" -ef /proc/self/fd/3 ] || return 1
    for file in /proc/self/fd/3/* /proc/self/fd/3/.[!.]* /proc/self/fd/3/..?*; do absent "$file" || return 1; done
)
finish_empty_authority() (
    tag stopped && tag selector-return-verified && tag bind-cleanup-intent && pair_original && same && matches "$stage/view" "$(view)" &&
        stock_return verify && authority_pin && flock -n 3 && authority_empty_pinned &&
        [ "$authority" -ef /proc/self/fd/3 ] && rmdir "$authority"
)
release_authority() (
    tag selector-return-verified && native_idle && stock_clear && original_matches && owned || return 1
    if absent "$authority"; then absent "$stage/bind-directory" || tag bind-cleanup-intent; return; fi
    authority_pin && flock -n 3 && authority_owned && authority_contents && write_tag bind-cleanup-intent || return 1
    for name in bind-active.next bind-active bind-journal.next bind-journal; do
        file=/proc/self/fd/3/$name
        if ! absent "$file"; then authority_owned && authority_record "$name" && [ "$authority" -ef /proc/self/fd/3 ] && rm "$file" || return 1; fi
    done
    authority_owned && authority_contents && [ "$authority" -ef /proc/self/fd/3 ] && rm /proc/self/fd/3/owner &&
        authority_empty_pinned && [ "$authority" -ef /proc/self/fd/3 ] && rmdir "$authority"
)
pair_original() { known_bind && selector_at "$slot" && saved_at "$production" && original_matches; }
pair_bound() (
    tag bind-intent && stock_mount_scope 1 && known_bind && authority_owned && selector_at "$slot" &&
        [ "$slot" -ef "$production" ] && [ "$slot" -ef "/proc/1/root$production" ] && matches "$stage/selector-stat" "$(stamp "$production")"
)
entry_guard() {
    if [ "$slot" -ef "$production" ]; then
        pair_bound || return 1
        case "$mode" in stop|restore|close) checked_bind status bound || return 1;; esac
    else stock_clear || return 1; fi
    if ! absent "$authority"; then
        tag bind-intent && { authority_owned || { tag bind-cleanup-intent && authority_pin && authority_empty_pinned; }; } || return 1
    fi
}
selector_active() {
    tag installed && tag bind-intent && tag selector-live && absent "$stage/selector-return-intent" && absent "$stage/stopped" &&
        absent "$stage/selector-disarmed" && absent "$stage/selector-duplicate" && absent "$stage/quarantined" && matches "$stage/arm-name" @LANE@ && pair_bound
}
activate() {
    tag installed && tag tile-prepared && origin_agrees && stock_request_absent &&
        absent "$authority" && absent "$stage/stopped" && absent "$stage/bind-intent" && absent "$stage/selector-live" && absent "$stage/selector-entered" &&
        absent "$stage/selector-callback" && absent "$stage/selector-service-view" && absent "$stage/selector-disarmed" && absent "$stage/quarantined" &&
        absent "$stage/selector-return-intent" && absent "$stage/tile-dispatch-intent" && absent "$stage/armed" &&
        same || return 1
    activate_reason=selector_pair_before
    pair_original && write_tag bind-intent && authority_create && pair_original || return 1
    activate_reason=selector_bind_unconfirmed
    checked_bind bind bound || return 1
    activate_reason=selector_pair_after
    checked_bind status bound && pair_bound && write_tag selector-live
}
selector_return() {
    if absent "$stage/bind-intent"; then
        absent "$authority" && absent "$stage/selector-live" && absent "$stage/selector-entered" && original_or_pristine || return 1
        if tag installed; then known_bind && checked_bind restore bind_restored && checked_bind absent bind_absent || return 1; fi
        return 0
    fi
    tag bind-intent && tag stopped && known_bind && selector_at "$slot" && write_tag selector-return-intent || return 1
    if ! absent "$authority" && absent "$authority/owner"; then finish_empty_authority || return 1; fi
    if absent "$authority"; then
        { absent "$stage/bind-directory" && absent "$stage/selector-live"; } || { tag selector-return-verified && tag bind-cleanup-intent; } || return 1
    else authority_owned || return 1; fi
    checked_bind restore bind_restored && checked_bind absent bind_absent && pair_original
}
channel_idle() (
    if absent "$stage/observer-active"; then return 0; fi
    tag observer-active && exec 9<"$stage/observer-active" && flock -n 9
)
request() {
    selector_active && absent "$stage/observer-active" && absent "$stage/render-cancel" &&
        write_tag observer-active && exec 9<"$stage/observer-active" && flock -n 9 && tag observer-active && stock_request_serve
}
witness() {
    selector_active && tag selector-service-view && tag selector-entered && tag tile-dispatch-intent && absent "$stage/render-cancel" && current=$(gui) &&
        matches "$stage/session" "$current"
}
restore() {
    tag stopped && channel_idle && stock_request_cleanup && native_idle && selector_return && stock_return restore || return 1
    if tag staged; then same && matches "$stage/view" "$(view)" || return 1
    else absent "$stage/bind-intent" && absent "$stage/tile-original" && absent "$stage/armed" && state >/dev/null || return 1; fi
    if tag bind-intent; then write_tag selector-return-verified && release_authority || return 1; fi
    absent "$authority" && write_tag restored
}
close_stage() {
    tag stopped && tag restored && channel_idle && stock_request_cleanup && absent "$authority" && stock_clear && original_or_pristine && native_idle && stock_return verify || return 1
    if ! absent "$stage/bind-intent"; then tag selector-return-intent && tag selector-return-verified && pair_original || return 1; fi
    if tag installed; then known_bind && checked_bind absent bind_absent || return 1; fi
    if tag staged; then same && matches "$stage/view" "$(view)" || return 1; else state >/dev/null || return 1; fi
    if ! absent "$stage/tile-dispatch-intent"; then
        if ! absent "$stage/quarantined"; then stock_quarantine_settled || return 1; else drained || return 1; fi
    fi
    unknown=$(find "$stage" ! -type d ! -type f -print) && [ -z "$unknown" ] && owned && stock_clear && rm -rf "$stage" && stock_empty
}

# A normally returned foreground load has no further stage writes to do.
# Emit the visit-bound terminal record only after the load function returns;
# never produce it from a signal trap or an interrupted child wait.
load_terminal() {
    case "$mode" in
        stage) printf 'WEREAD_STOCK_STAGE_RETIRED %s\n' "$label" >&2;;
        install) printf 'WEREAD_STOCK_INSTALL_RETIRED %s\n' "$label" >&2;;
    esac
}
load_refused() {
    printf 'refused %s\n' "$1" >&2
    if [ "$load_child_unconfirmed" = 0 ]; then load_terminal; fi
    exit 1
}

case "$mode" in
    admit) admit 2>/dev/null || fail "$admit_reason"; reply admitted;;
    audit) bounded_audit 2>/dev/null || fail production_byte_audit; reply audited;;
    absent) stock_empty 2>/dev/null || fail absent; original_report absent || fail absent;;
    stage) new_stage 2>/dev/null || load_refused "$admit_reason"; reply staged; load_terminal;;
    install)
        absent "$stage" && load_refused absent_stage
        stage_lock 2>/dev/null && entry_guard 2>/dev/null || load_refused "${bind_reason:-stage_owner_or_busy}"
        install 2>/dev/null || load_refused "${bind_reason:-$install_reason}"
        reply installed; load_terminal;;
    original|prepare|activate|arm|request|tap|claim|witness|stop|restore|close)
        if absent "$stage"; then
            case "$mode" in stop|restore|close) stock_empty 2>/dev/null || fail unsafe_absence;; *) fail absent_stage;; esac
            reply already_absent; exit 0
        fi
        stage_lock 2>/dev/null && entry_guard 2>/dev/null || fail "${bind_reason:-stage_owner_or_busy}"
        case "$mode" in request|claim|stop|witness) if system_shell 2>/dev/null; then shell_trusted=1; else fail untrusted_shell; fi;; esac
        case "$mode" in
            original) tag installed && pair_original && same >/dev/null 2>&1 || fail original; original_report original || fail original;;
            prepare) pair_original && stock_request_absent && prepare 2>/dev/null || fail "$prepare_reason"; reply prepared;;
            activate) activate 2>/dev/null || fail "${bind_reason:-$activate_reason}"; reply activated;;
            arm) arm 2>/dev/null || fail arm; reply armed;;
            request) request 2>/dev/null || fail frame_request;;
            tap) tap 2>/dev/null || fail uncertain_or_refused_tap;;
            claim) selector_active && claim 2>/dev/null || fail launch_identity; reply claimed;;
            witness) witness 2>/dev/null || fail scene_identity; reply witnessed;;
            stop) stop_session 2>/dev/null || fail unproven_close; reply stopped;;
            restore) restore 2>/dev/null || fail "${bind_reason:-native_restore}"; reply restored;;
            close) close_stage 2>/dev/null || fail "${bind_reason:-closeout}"; reply closed;;
        esac;;
    *) exit 64;;
esac
