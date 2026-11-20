rollout_stage=/mnt/us/.weread-rollout-$label
stage=$rollout_stage
home=/mnt/us/weread
isdir() { [ -d "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
original_install() {
    plain "$daily" && isdir "$home" && isdir "$home/app" &&
        plain "$home/app/weread" && plain "$home/app/weread-frame-guard" && absent "$home/launch.sh" &&
        [ "$(sql "$db" "SELECT count(*) FROM Entries WHERE p_location COLLATE BINARY='$daily' AND p_mimeType='text/x-shellscript';")" = 1 ]
}
installed() {
    cmp -s "$daily" "$stage/ref/document" && cmp -s "$home/launch.sh" "$stage/ref/launch.sh" &&
        plain "$home/app/weread" && [ "$(stat -c '%d:%i:%s' "$home/app/weread")" = "$(cat "$stage/new-identity")" ] &&
        plain "$home/app/weread-frame-guard" && plain "$home/app/weread-native-query"
}
rollout_state() {
    cat /proc/sys/kernel/random/boot_id && pidof awesome && light && registration || return 1
    readlink -f /var/local/kmc/bin/sh_integration_launcher || return 1
    for node in /sys/class/input/event*; do [ -e "$node" ] || return 1; printf '%s\n' "${node##*/}"; done
}
rollout_admission() {
    native && absent "$stage" && original_install || fail normal_install_admission
    peer_proof
    df -Pk /mnt/us | awk 'END{exit !(NF>=6 && $4+0>=235520)}' || fail space
    [ "$(sed -n '1p' "$home/data/device-profile")" = scribe ] &&
        awk '$1=="scribe" && $2=="5.19.5" {ok=1} END{exit !ok}' "$home/data/device-allowlist" || fail existing_profile
}
stop_test() {
    plain "$stage/session" || return 1
    read -r supervisor birth token <"$stage/session" || return 1
    case "$supervisor:$birth" in *[!0-9:]*|:*|*:) return 1;; esac
    plain "$lock/pid" && [ "$(cat "$lock/pid")" = "$supervisor" ] && plain "$lock/nonce" &&
        [ "$(cat "$lock/nonce")" = "$token" ] && [ "$(awk '{print $22}' "/proc/$supervisor/stat" 2>/dev/null)" = "$birth" ] &&
        tr '\000' '\n' <"/proc/$supervisor/cmdline" | grep -Fxq "$home/launch.sh" || return 1
    [ "$(awk '{print $22}' "/proc/$supervisor/stat" 2>/dev/null)" = "$birth" ] && kill -TERM "$supervisor"
}
record_session() {
    plain "$stage/requested" && plain "$lock/pid" && plain "$lock/nonce" || return 1
    supervisor=$(cat "$lock/pid"); token=$(cat "$lock/nonce")
    case "$supervisor" in ''|*[!0-9]*) return 1;; esac
    case "$token" in ''|*[!0-9a-f]*) return 1;; esac
    [ "${#token}" = 32 ] && tr '\000' '\n' <"/proc/$supervisor/cmdline" | grep -Fxq "$home/launch.sh" || return 1
    birth=$(awk '{print $22}' "/proc/$supervisor/stat") || return 1
    [ "$supervisor" = "$(cat "$lock/pid")" ] && [ "$token" = "$(cat "$lock/nonce")" ] || return 1
    printf '%s %s %s\n' "$supervisor" "$birth" "$token" >"$stage/session.next" && mv "$stage/session.next" "$stage/session"
}
case "$mode" in
    admit) rollout_admission; printf 'admitted\n';;
    stage)
        rollout_admission
        mkdir "$stage"; printf '%s\n' "$label" >"$stage/owner"
        exec 8<"$stage"; flock -n 8 || fail live_stage
        rollout_state >"$stage/before" || fail native_projection
        mkdir "$stage/out"
        tar -xzf - -C "$stage" || fail partial_upload
        cmp -s "$stage/ref/old-document" "$daily" && original_install || fail changed_daily_document
        cmp -s "$stage/package/documents/weread.sh" "$stage/ref/document" &&
            cmp -s "$stage/package/weread/launch.sh" "$stage/ref/launch.sh" || fail rollout_document
        for file in weread weread-frame-guard weread-native-query; do
            plain "$stage/package/weread/app/$file" && [ -x "$stage/package/weread/app/$file" ] || fail rollout_binary
        done
        stat -c '%d:%i:%s' "$stage/package/weread/app/weread" >"$stage/new-identity"
        stat -c '%d:%i:%s' "$home/app/weread" >"$stage/old-identity"
        : >"$stage/installing"
        mv "$home/app" "$stage/previous-app"
        mv "$stage/package/weread/app" "$home/app"
        mv "$stage/package/weread/launch.sh" "$home/launch.sh"
        mv -f "$stage/package/documents/weread.sh" "$daily"
        installed || fail publish
        scan || fail rollout_scan
        printf 'indexed\n';;
    case)
        [ "$name" = normal ] && owned && native && installed || fail normal_smoke_admission
        exec 8<"$stage"; flock -n 8 || fail live_stage
        [ "$(sql "$db" "SELECT count(*) FROM Entries WHERE p_location COLLATE BINARY='$daily' AND p_mimeType='text/x-shellscript';")" = 1 ] || fail normal_index
        lipc-set-prop -i com.lab126.powerd touchScreenSaverTimeout 1 >/dev/null
        : >"$stage/requested"
        (
            for tick in $(seq 1 65); do
                absent "$stage/normal-exit" && owned || exit 0
                sleep 1
            done
            if ! plain "$stage/session"; then record_session || exit 0; fi
            stop_test || :
        ) </dev/null >/dev/null 2>&1 8>&- &
        watchdog=$!
        lipc-set-prop com.lab126.appmgrd start "file://$daily" >/dev/null || fail normal_request
        for tick in $(seq 1 200); do if record_session; then break; fi; usleep 100000; done
        plain "$stage/session" || fail no_session
        for tick in $(seq 1 180); do if gui_owned && plain "$lock/state-open"; then break; fi; usleep 150000; done
        gui_owned && plain "$lock/state-open" || fail no_state_open
        printf '%s\n' "$home/app/weread" --kindle --device scribe --data-dir "$home/data" >"$stage/expected-args"
        tr '\000' '\n' <"/proc/$gui/cmdline" | cmp -s - "$stage/expected-args" || fail normal_arguments
        sleep 4
        gui_owned && [ "$(cat "$lock/nonce")" = "$token" ] || fail normal_liveness
        display_open=0; input_open=0
        for descriptor in "/proc/$gui"/fd/*; do
            case "$(readlink "$descriptor" 2>/dev/null || :)" in /dev/fb0) display_open=1;; /dev/input/event*) input_open=1;; esac
        done
        [ "$display_open:$input_open" = 1:1 ] && gui_owned || fail normal_platform
        stop_test || fail stop_identity
        settled || fail normal_recovery
        : >"$stage/normal-exit"
        wait "$watchdog" || :
        rollout_state >"$stage/after" && cmp -s "$stage/before" "$stage/after" && installed || fail normal_restore
        printf 'production_uri one_catalog_row approved_profile normal_args state_open live_after_four_seconds display_and_input_open native_restored\n' >"$stage/out/normal.txt"
        : >"$stage/promoted"
        printf 'completed normal\n';;
    collect) owned && isdir "$stage/out" || fail no_results; archive_public;;
    restore)
        if absent "$stage"; then native && original_install || fail unknown_cleanup; printf 'restored\n'; exit 0; fi
        owned || fail unowned_stage
        exec 8<"$stage"; lock_stage || fail live_stage
        if ! native && plain "$stage/requested"; then
            if ! plain "$stage/session"; then record_session || :; fi
            stop_test || :
        fi
        settled || fail unrecovered
        : >"$stage/normal-exit"
        if plain "$stage/promoted"; then
            installed || fail changed_installation
        elif plain "$stage/installing"; then
            if ! absent "$home/launch.sh"; then cmp -s "$home/launch.sh" "$stage/ref/launch.sh" || fail ambiguous_launcher; fi
            if ! cmp -s "$daily" "$stage/ref/old-document"; then
                cmp -s "$daily" "$stage/ref/document" || fail ambiguous_document
                cp "$stage/ref/old-document" "$stage/return-document"; chmod 755 "$stage/return-document"
                mv -f "$stage/return-document" "$daily"; scan
            fi
            if isdir "$stage/previous-app"; then
                if ! absent "$home/app"; then
                    plain "$home/app/weread" && [ "$(stat -c '%d:%i:%s' "$home/app/weread")" = "$(cat "$stage/new-identity")" ] &&
                        absent "$stage/unaccepted-app" || fail ambiguous_app
                    mv "$home/app" "$stage/unaccepted-app"
                fi
                mv "$stage/previous-app" "$home/app"
            fi
            rm -f "$home/launch.sh"
            original_install && [ "$(stat -c '%d:%i:%s' "$home/app/weread")" = "$(cat "$stage/old-identity")" ] || fail rollback
        fi
        peer_proof
        if [ -s "$stage/before" ]; then rollout_state >"$stage/end" && cmp -s "$stage/before" "$stage/end" || fail native_projection; fi
        printf '%s\n' "$label" >"$stage/restored"; printf 'restored\n';;
    close)
        if ! absent "$stage"; then
            owned && plain "$stage/restored" && [ "$(cat "$stage/restored")" = "$label" ] && native || fail unowned_stage
            exec 8<"$stage"; flock -n 8 || fail live_stage
            rm -rf "$stage" || fail cleanup
        fi
        absent "$stage" && native || fail closeout
        printf 'closed\n';;
    *) exit 64;;
esac
