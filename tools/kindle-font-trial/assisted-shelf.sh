assisted_case() {
    [ "$name" = B-library ] || exit 64
    prepare_case
    "$stage/pose-snapshot" 10 >"$out/sensor" || fail sensor_unavailable
    awk 'NF != 4 || $1 != NR || $2 !~ /^[1-9][0-9]*$/ || $3 !~ /^[1-9][0-9]*$/ || $4 !~ /^(15|16|17|18)$/ {bad=1} END{exit (bad || NR != 10)}' "$out/sensor" || fail sensor_format
    if ! { grep -Fxq '@PORTRAIT@' "$out/portrait" && awk '$4!=@RAW_FIRST@ && $4!=@RAW_SECOND@ {bad=1} END{exit bad}' "$out/sensor" && [ "$(portrait)" = '@PORTRAIT@' ]; }; then
        settled && same_state && portrait >"$out/portrait-return" || fail native_return
        printf 'not_started pose_gate\n' >"$out/observation"
        printf '%s\n' "$name" >"$out/complete"
        printf 'completed %s\n' "$name"
        return
    fi
    native && absent "$stage/armed" || fail launch_admission
    printf '%s\n' "$name" >"$stage/next" && mv "$stage/next" "$stage/armed" || fail arm
    "$stage/measure" request "$label" >"$out/request" || fail request
    log=$home/data/logs/application-bootstrap.log
    for tick in $(seq 1 270); do
        if plain "$log" && grep -q ' first_application_frame_send$' "$log"; then break; fi
        absent "$out/status" || fail no_send
        usleep 150000
    done
    plain "$log" && grep -q ' first_application_frame_send$' "$log" || fail no_send
    absent "$stage/armed" && plain "$out/entered/arm" && printf '%s\n' "$name" | cmp -s - "$out/entered/arm" || fail entry_identity
    gui_args && live_pose || fail gui_identity_or_pose
    identity=$gui:$birth; generation=$(cat "$lock/generation")
    public_foreground || fail public_owner
    if "$stage/shelf-scene" >"$out/shelf.next"; then
        public_foreground && mv "$out/shelf.next" "$out/shelf" || fail public_owner
    else
        rm -f "$out/shelf.next"; printf 'not_observed\n' >"$out/shelf"
    fi
    for tick in $(seq 1 240); do plain "$out/status" && break; usleep 200000; done
    plain "$out/status" && [ "$(cat "$out/status")" = 0 ] && absent "$out/guard" || fail exit
    plain "$out/final.metadata.json" && [ "$(stat -c %s "$out/final.metadata.json")" -le 4096 ] &&
        plain "$out/final.png" && rm "$out/final.png" || fail capture_metadata
    printf 'captured candidate_library_exit\n' >"$out/observation"
    finish_case
    printf 'completed %s\n' "$name"
}
