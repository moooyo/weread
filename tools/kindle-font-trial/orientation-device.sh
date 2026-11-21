#!/bin/sh
set -eu
export LC_ALL=C
umask 077
mode=$1
label=$2
name=${3:-none}
case "$label" in ''|*[!0-9a-f]*) exit 64;; esac
[ "${#label}" -eq 16 ] || exit 64
stage=/mnt/us/.weread-orientation-$label
doc=/mnt/us/documents/weread-orientation-trial-$label.sh
daily=/mnt/us/documents/weread.sh
lock=$(readlink -f /tmp)/weread-launcher.lock
db=/var/local/cc.db
reg=/var/local/appreg.db
handler=tech.hackerdude.shell_integration.launcher
runtime=/var/local/kindle-catalog-entry/v1-2db100d0bcb60a76571cd8de96bed7ea6c016e6ab73e388a885f692955cacd86

@FONT_COMMON@
@FONT_GUI@

sidecars_absent() { absent "${doc%.sh}.sdr" && absent "$doc.sdr"; }
catalog_one() {
    [ "$(sql "$db" "SELECT count(*)||':'||coalesce(sum(p_location COLLATE BINARY='$1' AND p_mimeType='text/x-shellscript' $2),0) FROM Entries WHERE p_location COLLATE NOCASE='$1';")" = 1:1 ] &&
        sql "$db" "SELECT quote(p_location)||','||quote(p_titles_0_nominal)||','||quote(p_mimeType) FROM Entries WHERE p_location COLLATE BINARY='$1';"
}
file_identity() {
    if [ "$1" = /var/local/kmc/bin/sh_integration_launcher ]; then
        [ -f "$1" ] && [ ! -L "$1" ] || return 1
    else plain "$1" || return 1; fi
    [ "${2:-any}" = any ] || [ "$(stat -c %s "$1")" = "$2" ] || return 1
    resolved=$(readlink -f "$1") && identity=$(stat -c '%d:%i:%s:%Y:%Z:%f:%u:%g:%h' "$1") || return 1
    printf '%s %s %s\n' "$1" "$resolved" "$identity"
}
state() {
    plain "$db" && plain "$reg" && registration || return 1
    catalog_one "$daily" '' && catalog_one /mnt/us/documents/Aidoku.sh '' &&
        catalog_one /mnt/us/documents/bilicomics.sh "AND p_titles_0_nominal='BiliComics for Kindle'" || return 1
    cat /proc/sys/kernel/random/boot_id && light || return 1
    managers=$(pidof awesome) && [ -n "$managers" ] || return 1
    for pid in $managers; do
        identity=$(awk '{print $22}' "/proc/$pid/stat") && [ -n "$identity" ] || return 1
        printf 'manager %s:%s\n' "$pid" "$identity"
    done
    for node in /sys/class/input/event*; do [ -e "$node" ] || return 1; printf '%s\n' "${node##*/}"; done
    for file in "$daily" /mnt/us/weread/launch.sh /mnt/us/weread/app/weread-frame-guard /mnt/us/weread/app/weread-native-query \
        /mnt/us/documents/bilicomics.sh /mnt/us/documents/Aidoku.sh /mnt/us/aidoku/launch.sh \
        /mnt/us/aidoku/app/aidoku /mnt/us/aidoku/app/aidoku-startup-portrait /mnt/us/aidoku/app/aidoku-native-query \
        /mnt/us/aidoku/app/kindle-jpeg-worker /mnt/us/aidoku/assets/aidoku-cover-v2.png /mnt/us/aidoku/assets/aidoku-cover.png \
        "$runtime/aidoku-library-dispatch" "$runtime/aidoku-entry-hooks.so" "$runtime/aidoku-catalog-client" /var/local/kmc/bin/sh_integration_launcher; do
        file_identity "$file" || return 1
    done
    file_identity /mnt/us/weread/app/weread 95137176 && file_identity /mnt/us/bilicomics/app/bilicomics 74340832 &&
        file_identity /mnt/us/bilicomics/app/kindle-jpeg-worker 426672 && file_identity /mnt/us/bilicomics/launch.sh 80185
}
new_stage() {
    native || fail native_not_idle
    state >/dev/null || fail protected_state
    portrait >/dev/null || fail portrait_unavailable
    [ "$(readlink -f /mnt/us/documents)" = /mnt/us/documents ] || fail documents
    absent "$stage" && absent "$doc" && sidecars_absent && [ "$(count_doc)" = 0 ] || fail occupied
    awk '$2=="/mnt/us" && $4 ~ /(^|,)rw(,|$)/ && $4 !~ /(^|,)noexec(,|$)/ {ok=1} END{exit !ok}' /proc/mounts || fail volume
    df -Pk /mnt/us | awk 'END{exit !(NF>=6 && $4+0>=655360)}' || fail space
}
same_state() { owned && plain "$stage/before" && state >"$stage/observed" && cmp -s "$stage/before" "$stage/observed"; }
stage_lock() { owned || fail unowned_stage; exec 8<"$stage"; flock -n 8 && owned || fail live_stage; }
payload() {
    archive=$stage/input.tgz
    (ulimit -f 450560; cat >"$archive") || return 1
    (ulimit -f 32; tar -tzf "$archive" >"$stage/members" && tar -tvzf "$archive" >"$stage/member-types") || return 1
    awk 'BEGIN{n=split("document measure seed.tar.gz A/launch.sh B/launch.sh A/app/weread B/app/weread A/app/weread-frame-guard B/app/weread-frame-guard A/app/weread-native-query B/app/weread-native-query",paths," ");for(i=1;i<=n;i++)ok[paths[i]]=1} !($0 in ok) || seen[$0]++ {bad=1} END{exit (bad || NR!=n)}' "$stage/members" || return 1
    awk 'substr($1,1,1)!="-" || $3!~/^[0-9]+$/ {bad=1} {bytes+=$3} END{exit (bad || NR!=11 || bytes>230686720)}' "$stage/member-types" || return 1
    tar -xzf "$archive" -C "$stage" && rm "$archive" || return 1
    plain "$stage/document" && plain "$stage/measure" && [ -x "$stage/measure" ] && plain "$stage/seed.tar.gz" || return 1
    (ulimit -f 512; tar -tzf "$stage/seed.tar.gz" >"$stage/members" && tar -tvzf "$stage/seed.tar.gz" >"$stage/member-types") || return 1
    awk '{sub(/\/$/,"")} !/^(library-covers\.json|demo(\/[A-Za-z0-9._+-]+)*)$/ || /(^|\/)\.\.?(\/|$)/ || seen[$0]++ {bad=1} END{exit (bad || NR>2048 || !("library-covers.json" in seen) || !("demo/library.sqlite" in seen))}' "$stage/members" || return 1
    awk '$1 !~ /^[-d]/ || $3!~/^[0-9]+$/ {bad=1} {bytes+=$3} END{exit (bad || bytes>33554432 || !NR)}' "$stage/member-types" || return 1
    grep -Fxq "stage=$stage" "$stage/document" || return 1
    for arm in A B; do
        for file in launch.sh app/weread app/weread-frame-guard app/weread-native-query; do
            plain "$stage/$arm/$file" && [ -x "$stage/$arm/$file" ] || return 1
        done
    done
    [ "$(stat -c %s "$stage/A/app/weread")" = @ARM_A_BYTES@ ] && [ "$(stat -c %s "$stage/B/app/weread")" = @ARM_B_BYTES@ ] || return 1
    for file in launch.sh app/weread-frame-guard app/weread-native-query; do cmp -s "$stage/A/$file" "$stage/B/$file" || return 1; done
}
publish() {
    cp "$stage/document" "$stage/document.ready" && chmod 755 "$stage/document.ready" &&
        printf '%s\n' "$label" >"$stage/publish-intent" && absent "$doc" && mv -n "$stage/document.ready" "$doc" &&
        absent "$stage/document.ready" && published
}
published() {
    plain "$stage/publish-intent" && [ "$(cat "$stage/publish-intent")" = "$label" ] && absent "$stage/document.ready" &&
        plain "$stage/document" && plain "$doc" && cmp -s "$stage/document" "$doc"
}
recover_state() {
    if plain "$stage/before"; then same_state
    else absent "$stage/before" && absent "$stage/publish-intent" && absent "$doc" && sidecars_absent && state >/dev/null; fi
}
entries_drained() {
    absent "$stage/armed" || return 1
    absent "$stage/out" && return 0
    [ -d "$stage/out" ] && [ "$(readlink -f "$stage/out")" = "$stage/out" ] || return 1
    for entry in "$stage"/out/*; do
        [ -d "$entry" ] || continue
        [ "$(readlink -f "$entry")" = "$entry" ] || return 1
        absent "$entry/entered/arm" && continue
        plain "$entry/entered/arm" && printf '%s\n' "${entry##*/}" | cmp -s - "$entry/entered/arm" || return 1
        plain "$entry/status" || return 1
        result=$(cat "$entry/status") || return 1
        case "$result" in [0-9]|[1-9][0-9]|[12][0-9][0-9]) [ "$result" -le 255 ] || return 1;; *) return 1;; esac
    done
}
portrait() {
    observed=$(/mnt/us/weread/app/weread --kindle --device scribe --probe-native-portrait-calibration) || return 1
    case "$observed" in [UD]:[0-3]) printf '%s\n' "$observed";; *) return 1;; esac
}
begin_pose() {
    native && portrait >"$out/portrait" || fail missing_native_portrait
    if absent "$stage/pose"; then cp "$out/portrait" "$stage/pose" || fail pose_record; fi
    plain "$stage/pose" && cmp -s "$stage/pose" "$out/portrait" || fail pose_changed
    printf 'native_idle_calibration\n' >"$out/portrait-source"
}
live_pose() {
    gui_owned && plain "$lock/nonce" && plain "$lock/generation" && plain "$lock/state-open" && plain "$lock/native-portrait-calibration" || return 1
    token=$(cat "$lock/nonce") || return 1
    case "$token" in ''|*[!0-9a-f]*) return 1;; esac
    [ "${#token}" -eq 32 ] && [ "$(cat "$lock/generation")" = "1:$token" ] && cmp -s "$lock/generation" "$lock/state-open" || return 1
    pose=$(cat "$out/portrait") || return 1
    printf '1:%s:%s\n' "$token" "$pose" | cmp -s - "$lock/native-portrait-calibration" || return 1
    printf '%s\n' "$pose" >"$out/portrait-launch"
}
gui_args() {
    gui_owned || return 1
    route=library; dwell=6000
    case "$name" in *-library|*-reader) route=${name#*-}; dwell=12000;; esac
    set -- "$home/app/weread" --kindle --device scribe --data-dir "$home/data" --demo --offline --screen "$route" --exit-after-ms "$dwell"
    case "$name" in *-library|*-reader) set -- "$@" --capture "$out/final.png";; esac
    printf '%s\n' "$@" >"$stage/expected-args" && tr '\000' '\n' <"/proc/$gui/cmdline" | cmp -s - "$stage/expected-args"
}
case_admission() {
    stage_lock
    absent "$stage/restored" && entries_drained && plain "$stage/document" && plain "$doc" && native && same_state &&
        cmp -s "$stage/document" "$doc" && wait_catalog 1 || fail case_admission
}
export_markers() {
    plain "$log" || return 1
    awk @EXPERIMENT_ARM@'$1=="WEREAD_STARTUP" {if(NF!=3 || $2 !~ /^[0-9]+\.[0-9]+$/ || @EXPERIMENT_OUTCOME@$3 !~ /^(main_enter|platform_ready|startup_feedback_(ready|paused|resumed|finished)|fonts_loaded|backend_ready|window_created|fonts_registered|application_attach_begin|application_attached|software_ready|backend_event_loop_enter|touch_opened|portrait_opened|first_application_frame_send|(splash|event_loop)_orientation_(begin|timeout|success|unavailable|failed|reused))$/ || ++count>40)exit 1;print $1,$2,$3} END{if(!count)exit 1}' "$log" >"$out/markers"
}
finish_case() {
    settled && same_state && portrait >"$out/portrait-return" && cmp -s "$out/portrait" "$out/portrait-return" || fail native_return
    export_markers || fail markers
    printf '%s\n' "$name" >"$out/complete"
}
archive_public() {
    stage_lock
    entries_drained && native && same_state && plain "$stage/out/orientation-batch" && [ "$(cat "$stage/out/orientation-batch")" = "$label" ] || fail collect_state
    cd "$stage"; set -- out/orientation-batch
    for entry in out/*; do
        [ -d "$entry" ] || continue
        trial=${entry#out/}; case "$trial" in [AB][0-6]|[AB]-library|[AB]-reader) ;; *) fail unexpected_case;; esac
        [ "$(readlink -f "$entry")" = "$stage/$entry" ] || fail linked_case
        if absent "$entry/complete"; then continue; fi
        plain "$stage/$entry/complete" && [ "$(cat "$entry/complete")" = "$trial" ] || fail case_completion
        leaves='request markers portrait portrait-source portrait-launch portrait-return thumb.pgm'
        case "$trial" in *-library|*-reader) leaves="$leaves panel.pgm final.png final.metadata.json";; esac
        for leaf in $leaves; do
            plain "$stage/$entry/$leaf" && [ "$(stat -c %s "$entry/$leaf")" -gt 0 ] && [ "$(stat -c %s "$entry/$leaf")" -le 12582912 ] || fail public_output
            set -- "$@" "$entry/$leaf"
        done
    done
    tar -czf - "$@"
}

stage_trial() {
    new_stage
    mkdir "$stage" && printf '%s\n' "$label" >"$stage/owner" || fail stage_owner
    stage_lock
    state >"$stage/before.next" && mv "$stage/before.next" "$stage/before" || fail native_projection
    payload || fail invalid_payload
    mkdir "$stage/out" && printf '%s\n' "$label" >"$stage/out/orientation-batch" || fail output_directory
    native && same_state && absent "$doc" && sidecars_absent && [ "$(count_doc)" = 0 ] && publish || fail publish
    scan && wait_catalog 1 || fail indexing
    printf 'indexed\n'
}
prepare_case() {
    case "$name" in [AB][0-6]|[AB]-library|[AB]-reader) ;; *) exit 64;; esac
    case_admission
    arm=${name%"${name#?}"}; home=$stage/$arm; out=$stage/out/$name
    mkdir "$out" || fail repeated_case
    [ "$(readlink -f "$home")" = "$home" ] || fail lane
    if ! absent "$home/data"; then
        [ -d "$home/data" ] && [ "$(readlink -f "$home/data")" = "$home/data" ] && rm -rf "$home/data" || fail data_reset
    fi
    mkdir "$home/data" && tar -xzf "$stage/seed.tar.gz" -C "$home/data" || fail public_seed
    printf 'scribe\n' >"$home/data/device-profile"; printf 'scribe 5.19.5\n' >"$home/data/device-allowlist"
    lipc-set-prop -i com.lab126.powerd touchScreenSaverTimeout 1 >/dev/null || fail active_timer
    sleep 8
    begin_pose
    native && absent "$stage/armed" || fail launch_admission
    printf '%s\n' "$name" >"$stage/next" && mv "$stage/next" "$stage/armed" || fail arm
}
capture_case() {
    prepare_case
    "$stage/measure" request "$label" >"$out/request" || fail request
    log=$home/data/logs/application-bootstrap.log
    for tick in $(seq 1 270); do
        if plain "$log" && grep -q ' first_application_frame_send$' "$log"; then break; fi
        absent "$out/status" || fail no_send
        usleep 150000
    done
    plain "$log" && grep -q ' first_application_frame_send$' "$log" || fail no_send
    sleep 1
    absent "$stage/armed" && plain "$out/entered/arm" && printf '%s\n' "$name" | cmp -s - "$out/entered/arm" || fail entry_identity
    gui_args && live_pose || fail gui_identity_or_pose
    "$stage/measure" thumb >"$out/thumb.pgm" || fail thumb
    case "$name" in *-library|*-reader) "$stage/measure" panel >"$out/panel.pgm" || fail panel;; esac
    if ! gui_args || ! live_pose; then rm -f "$out/thumb.pgm" "$out/panel.pgm"; fail gui_changed; fi
    for tick in $(seq 1 200); do plain "$out/status" && break; usleep 200000; done
    plain "$out/status" && [ "$(cat "$out/status")" = 0 ] && absent "$out/guard" || fail exit
    finish_case
    printf 'completed %s\n' "$name"
}
no_trial() { absent "$stage" && absent "$doc" && sidecars_absent && [ "$(count_doc)" = 0 ] && native && state >/dev/null; }
restore_trial() {
    if ! absent "$stage"; then
        owned || fail unowned_stage
        exec 8<"$stage"; lock_stage && owned || fail live_stage
        rm -f "$stage/armed"
        for tick in $(seq 1 75); do entries_drained && native && break; sleep 1; done
        entries_drained || fail pending_entry
        native && recover_state || fail protected_drift
        if ! absent "$doc"; then published && rm "$doc" && scan || fail unowned_document; fi
        wait_catalog 0 || fail residual_row
        for sidecar in "${doc%.sh}.sdr" "$doc.sdr"; do
            absent "$sidecar" && continue
            plain "$stage/publish-intent" && [ -d "$sidecar" ] && [ "$(readlink -f "$sidecar")" = "$sidecar" ] && rmdir "$sidecar" || fail unknown_sidecar
        done
        settled && recover_state || fail native_restore
        printf '%s\n' "$label" >"$stage/restored"
    else no_trial || fail unowned_document; fi
    printf 'restored\n'
}
close_trial() {
    if ! absent "$stage"; then
        stage_lock
        plain "$stage/restored" && [ "$(cat "$stage/restored")" = "$label" ] && absent "$doc" && sidecars_absent &&
            [ "$(count_doc)" = 0 ] && entries_drained && native && recover_state || fail unclosed_trial
        rm -rf "$stage" || fail cleanup
    fi
    no_trial || fail closeout
    printf 'closed\n'
}

case "$mode" in
    reference) native && state >/dev/null && portrait || fail native_reference;;
    admit) new_stage; printf 'admitted\n';;
    stage) stage_trial;;
    case) capture_case;;
    collect) archive_public;;
    restore) restore_trial;;
    close) close_trial;;
    *) exit 64;;
esac
