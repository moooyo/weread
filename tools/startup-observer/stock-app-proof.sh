stock_trial_nodes() (
    directory "$stage" && directory "$trial" && directory "$trial/app" && directory "$trial/data" && single "$trial/data/library.sqlite" &&
        stamp "$trial/app/weread" && stamp "$trial/launch.sh" &&
        stat -c '%d:%i:%f:%u:%g' "$stage" "$trial" "$trial/app" "$trial/data" "$trial/data/library.sqlite"
)

stock_private_record() { single "$1" && private_records 0 "$1"; }

stock_request_anchor() (
    tag request-live && tag request-holder && single "$stage/stock-confirmed-release" && single "$stage/tile-release" &&
        [ -s "$stage/tile-release" ] && [ "$(stat -c %s "$stage/tile-release")" -le 480 ] &&
        cmp -s "$stage/tile-release" "$stage/stock-confirmed-release" || return 1
    if [ "$1" = close ]; then stock_request_absent && channel_idle; return; fi
    stock_request_paths && exec 5<"$stage/request-holder" || return 1
    if flock -n 5; then return 1; else [ "$?" = 1 ] || return 1; fi
    dir_id=$(stock_request_dir "$stock_fixed") && file_id=$(stock_request_file "$stock_fixed/request") &&
        matches "$stage/request-dir" "$dir_id" && matches "$stage/request-file" "$file_id"
)

stock_exact_gui_set() (
    set +f
    found=''
    for image in "$proc"/[0-9]*/exe; do
        [ "$image" -ef /proc/self/fd/4 ] || continue
        candidate=${image%/exe}; candidate=${candidate##*/}
        [ -z "$found" ] && number "$candidate" || return 1
        found=$candidate
    done
    [ "$found" = "$1" ] || return 1
    if [ -n "$found" ]; then [ "$(pidof weread)" = "$found" ];
    else if pidof weread >/dev/null 2>&1; then return 1; else [ "$?" = 1 ]; fi; fi
)

stock_no_trial_gui() ( exec 4<"$trial/app/weread" && stock_exact_gui_set ''; )

stock_gui_snapshot() (
    purpose=$1
    group=$(supervision) && matches "$stage/group" "$group" && matches "$stage/launch-owner" 0 && [ "$(lock_owner)" = 0 ] || return 1
    set -- $group; [ "$#" = 5 ] || return 1
    entry=$1; master=$3; master_birth=$4; nonce=$5
    for file in pid nonce application-pid application-birth generation; do stock_private_record "$lock/$file" || return 1; done
    gui_pid=$(line "$lock/application-pid") && gui_birth=$(line "$lock/application-birth") && identity "$gui_pid" && [ "$birth" = "$gui_birth" ] || return 1
    runner=$parent; identity "$runner" && [ "$parent" = "$master" ] && [ "$birth" -ge "$master_birth" ] && [ "$gui_birth" -ge "$birth" ] || return 1
    runner_birth=$birth
    [ "$gui_pid" != "$runner" ] && [ "$gui_pid" != "$master" ] && [ "$gui_pid" != "$entry" ] &&
        [ "$runner" != "$master" ] && [ "$runner" != "$entry" ] && [ "$entry" != "$master" ] &&
        worker_args "$runner" && process_owner "$runner" 0 && process_owner "$gui_pid" 0 &&
        [ "$(readlink "$proc/$gui_pid/exe")" = "$trial/app/weread" ] && [ "$proc/$gui_pid/exe" -ef /proc/self/fd/4 ] &&
        [ "$trial/app/weread" -ef /proc/self/fd/4 ] && argv "$gui_pid" "$trial/app/weread" --kindle --device scribe --data-dir "$trial/data" &&
        matches "$lock/generation" "1:$nonce" && stock_exact_gui_set "$gui_pid" || return 1
    for tag_name in owner selector-callback selector-service-view selector-entered tile-dispatch-intent arm-name; do single "$stage/$tag_name" || return 1; done
    owned_ack=0
    set -- "$lock" "$lock/pid" "$lock/nonce" "$lock/application-pid" "$lock/application-birth" "$lock/generation" \
        "$stage/owner" "$stage/selector-callback" "$stage/selector-service-view" "$stage/selector-entered" "$stage/tile-dispatch-intent" "$stage/arm-name"
    if ! absent "$lock/application-owned"; then
        stock_private_record "$lock/application-owned" && matches "$lock/application-owned" "1:$nonce:$gui_pid:$gui_birth" || return 1
        owned_ack=1; set -- "$@" "$lock/application-owned"
    fi
    if ! absent "$lock/state-open"; then
        stock_private_record "$lock/state-open" && matches "$lock/state-open" "1:$nonce" || return 1
        set -- "$@" "$lock/state-open"
    else [ "$purpose" = close ] && absent "$stage/session" || return 1; fi
    printf '%s %s %s %s %s\n' "$group" "$runner" "$runner_birth" "$gui_pid" "$gui_birth"
    printf 'owned_ack=%s\n' "$owned_ack"
    stat -c '%n:%d:%i:%f:%u:%g' "$@"
)

stock_prove_gui() (
    owned && selector_active && stock_request_anchor "$1" &&
        exec 4<"$trial/app/weread" && nodes=$(stock_trial_nodes) && matches "$stage/stock-trial-nodes" "$nodes" || return 1
    first=$(stock_gui_snapshot "$1") && second=$(stock_gui_snapshot "$1") && [ "$first" = "$second" ] &&
        [ "$trial/app/weread" -ef /proc/self/fd/4 ] && [ "$(stock_trial_nodes)" = "$nodes" ] && stock_request_anchor "$1" || return 1
    case "$first" in *'
owned_ack=1
'*) write_tag stock-ack-seen || return 1;; *) absent "$stage/stock-ack-seen" || return 1;; esac
    printf '%s\n' "${first%%'
'*}"
)

stock_close_session() (
    proven=$(stock_prove_gui close) && { absent "$stage/session" || matches "$stage/session" "$proven"; } &&
        record_once stock-stop-session "$proven"
)

stock_preterm_identity() (
    if ! absent "$stage/stock-stop-session"; then
        current=$(stock_prove_gui close) && matches "$stage/stock-stop-session" "$current"; return
    fi
    absent "$stage/session" && absent "$lock/application-pid" && absent "$lock/application-birth" &&
        absent "$lock/application-owned" && absent "$lock/state-open" && absent "$lock/generation" && stock_no_trial_gui
)

stock_close_chain_retired() (
    if absent "$stage/stock-stop-session"; then stock_no_trial_gui; return; fi
    proof=$(line "$stage/stock-stop-session") || return 1
    set -- $proof
    [ "$#" = 9 ] && matches "$stage/group" "$1 $2 $3 $4 $5" && retired "$1" "$2" && retired "$3" "$4" &&
        retired "$6" "$7" && retired "$8" "$9" && stock_no_trial_gui
)
