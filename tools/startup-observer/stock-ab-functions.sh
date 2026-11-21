production=$home/launch.sh
slot=$stage/selector-original-slot
install_reason=install
activate_reason=selector_activation

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
stock_clear() (
    directory "$home" && directory "${daily%/*}" || return 1
    for other in /mnt/us/.weread-render-*; do absent "$other" || [ "$other" = "$stage" ] || return 1; done
    awk -v stage="$stage" -v home="$home" -v daily="$daily" '
        $2==stage || index($2,stage"/")==1 || $2==home || index($2,home"/")==1 || $2==daily || $2=="/mnt/us/documents" {bad=1}
        END{exit bad}' "$proc/mounts"
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
    else absent "$stage/installed" && absent "$stage/swap-intent" && absent "$stage/selector-live" && fresh_production &&
        { absent "$stage/launch-stat" || matches "$stage/launch-stat" "$(stamp "$production")"; }; fi
}
stock_empty() { absent "$stage" && stock_clear && fresh_production && state >/dev/null && stock_request_absent; }
bounded_audit() (
    stock_clear && fresh_production && stock_request_absent && state >/dev/null || return 1
    if ! absent "$stage"; then stage_lock && tag installed && absent "$stage/swap-intent" && original_matches && same || return 1; fi
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
install() {
    tag staged && same && stock_clear && stock_request_absent && fresh_production && matches "$stage/launch-stat" "$(stamp "$production")" || return 1
    for leaf in installed swap-intent armed input.tgz names types outer-manifest seed-manifest unpack arm tile render-observer selector-exchange launch-reference document-reference arm-seed.tar.gz selector-original-slot selector-stat exchange-stat arm-name; do
        absent "$stage/$leaf" || return 1
    done
    (set -C; head -c $((@ARCHIVE_BYTES@+1)) >"$stage/input.tgz" && outer_manifest >"$stage/outer-manifest" && seed_manifest >"$stage/seed-manifest") &&
        [ "$(stat -c %s "$stage/input.tgz")" = @ARCHIVE_BYTES@ ] && mkdir "$stage/unpack" &&
        check_archive "$stage/input.tgz" "$stage/outer-manifest" "$stage/unpack" || return 1
    for leaf in arm tile render-observer selector-exchange launch-reference document-reference arm-seed.tar.gz; do mv "$stage/unpack/$leaf" "$stage/$leaf" || return 1; done
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
        record_once exchange-stat "$(stamp "$stage/selector-exchange")" || return 1
    install_reason=selector_exchange_probe
    [ "$("$stage/selector-exchange" probe "$label")" = exchange_supported ] || return 1
    install_reason=selector_original_pair
    pair_original && write_tag installed
}
known_exchange() {
    single "$stage/selector-exchange" && [ -x "$stage/selector-exchange" ] && matches "$stage/exchange-stat" "$(stamp "$stage/selector-exchange")"
}
selector_at() {
    id=$(stamp "$1") && matches "$stage/selector-stat" "$id" && selector_text | cmp -s - "$1"
}
saved_at() {
    id=$(stamp "$1") && matches "$stage/launch-stat" "$id" && single "$stage/launch-reference" && single "$trial/launch.sh" &&
        cmp -s "$stage/launch-reference" "$1" && cmp -s "$trial/launch.sh" "$1"
}
pair_original() { stock_clear && known_exchange && selector_at "$slot" && saved_at "$production" && original_matches; }
pair_swapped() { stock_clear && known_exchange && selector_at "$production" && saved_at "$slot"; }
selector_active() {
    tag installed && tag swap-intent && tag selector-live && absent "$stage/selector-return-intent" && absent "$stage/stopped" &&
        matches "$stage/arm-name" @LANE@ && pair_swapped
}
activate() {
    tag installed && tag tile-prepared && origin_agrees && stock_request_absent &&
        absent "$stage/stopped" && absent "$stage/swap-intent" && absent "$stage/selector-live" && absent "$stage/selector-entered" &&
        absent "$stage/selector-return-intent" && absent "$stage/tile-dispatch-intent" && absent "$stage/armed" &&
        same || return 1
    activate_reason=selector_pair_before
    pair_original && write_tag swap-intent && pair_original || return 1
    activate_reason=selector_exchange_unconfirmed
    [ "$("$stage/selector-exchange" swap "$label")" = exchanged ] || return 1
    activate_reason=selector_pair_after
    pair_swapped && write_tag selector-live
}
selector_return() {
    if absent "$stage/swap-intent"; then absent "$stage/selector-live" && absent "$stage/selector-entered" && original_or_pristine; return; fi
    tag swap-intent && tag stopped && write_tag selector-return-intent || return 1
    if pair_original; then return 0; fi
    pair_swapped && [ "$("$stage/selector-exchange" swap "$label")" = exchanged ] && pair_original
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
    selector_active && tag selector-entered && tag tile-dispatch-intent && absent "$stage/render-cancel" && current=$(gui) &&
        matches "$stage/session" "$current"
}
restore() {
    tag stopped && channel_idle && stock_request_cleanup && native_idle && stock_clear && selector_return && stock_return restore || return 1
    if tag staged; then same && matches "$stage/view" "$(view)" || return 1
    else absent "$stage/swap-intent" && absent "$stage/tile-original" && absent "$stage/armed" && state >/dev/null || return 1; fi
    write_tag restored
}
close_stage() {
    tag stopped && tag restored && channel_idle && stock_request_cleanup && stock_clear && original_or_pristine && native_idle && stock_return verify || return 1
    if ! absent "$stage/swap-intent"; then tag selector-return-intent && pair_original || return 1; fi
    if tag staged; then same && matches "$stage/view" "$(view)" || return 1; else state >/dev/null || return 1; fi
    if ! absent "$stage/tile-dispatch-intent"; then drained || return 1; fi
    unknown=$(find "$stage" ! -type d ! -type f -print) && [ -z "$unknown" ] && owned && stock_clear && rm -rf "$stage" && stock_empty
}

case "$mode" in
    admit) admit 2>/dev/null || fail "$admit_reason"; reply admitted;;
    audit) bounded_audit 2>/dev/null || fail production_byte_audit; reply audited;;
    absent) stock_empty 2>/dev/null || fail absent; original_report absent || fail absent;;
    stage) new_stage 2>/dev/null || fail "$admit_reason"; reply staged;;
    install|original|prepare|activate|arm|request|tap|claim|witness|stop|restore|close)
        if absent "$stage"; then
            case "$mode" in stop|restore|close) stock_empty 2>/dev/null || fail unsafe_absence;; *) fail absent_stage;; esac
            reply already_absent; exit 0
        fi
        stock_clear && stage_lock 2>/dev/null || fail stage_owner_or_busy
        case "$mode" in request|claim|stop|witness) if system_shell 2>/dev/null; then shell_trusted=1; else fail untrusted_shell; fi;; esac
        case "$mode" in
            install) install 2>/dev/null || fail "$install_reason"; reply installed;;
            original) tag installed && pair_original && same >/dev/null 2>&1 || fail original; original_report original || fail original;;
            prepare) pair_original && stock_request_absent && prepare 2>/dev/null || fail "$prepare_reason"; reply prepared;;
            activate) activate 2>/dev/null || fail "$activate_reason"; reply activated;;
            arm) arm 2>/dev/null || fail arm; reply armed;;
            request) request 2>/dev/null || fail frame_request;;
            tap) tap 2>/dev/null || fail uncertain_or_refused_tap;;
            claim) selector_active && claim 2>/dev/null || fail launch_identity; reply claimed;;
            witness) witness 2>/dev/null || fail scene_identity; reply witnessed;;
            stop) stop_session 2>/dev/null || fail unproven_close; reply stopped;;
            restore) restore 2>/dev/null || fail native_restore; reply restored;;
            close) close_stage 2>/dev/null || fail closeout; reply closed;;
        esac;;
    *) exit 64;;
esac
