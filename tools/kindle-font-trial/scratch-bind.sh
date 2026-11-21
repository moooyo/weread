fixed() { regular "$1" && [ "$(stat -c %h "$1")" = 1 ] && printf '%s\n' "$2" | cmp -s - "$1"; }
once() { (set -C; printf '%s\n' "$2" >"$1") && fixed "$1" "$2"; }
lock_owner() {
    owned "$stage" && [ "$(stat -c '%u:%a' "$stage")" = 0:700 ] && exec 8<"$stage/owner" && flock -n 8 &&
        [ "$stage/owner" -ef /proc/self/fd/8 ] && owned "$stage";
}
same_scratch() {
    directory "$scratch" && scratch_identity=$(stat -c '%d:%i' "$scratch") && fixed "$stage/scratch-dir-id" "$scratch_identity";
}
checked_helper() {
    regular "$stage/helper" && [ "$(stat -c '%h:%s' "$stage/helper")" = '1:@HELPER_BYTES@' ] &&
        [ "$("$stage/helper" --policy 2>/dev/null)" = selector-bind-v1 ];
}
stamp() { regular "$1" && stat -c '%d:%i:%f:%u:%g:%h:%s:%Y' "$1"; }
originals() {
    owned "$scratch" && fixed "$scratch/source" "$label:S" && fixed "$scratch/target" "$label:T" &&
        ! [ "$scratch/source" -ef "$scratch/target" ] && stamp "$scratch/source" && stamp "$scratch/target";
}
clear_views() {
    for table in /proc/self/mountinfo /proc/1/mountinfo; do
        awk -v root="$scratch" '$5==root || index($5,root"/")==1 {bad=1} END{exit !(NR && !bad)}' "$table" || return 1
    done
}
no_helper_state() {
    absent "$stage/bind-journal" && absent "$stage/bind-active" && absent "$stage/bind-journal.next" && absent "$stage/bind-active.next";
}
helper_step() {
    bind_reply=$("$stage/helper" "$1" scratch "$label" 2>&1) && [ "$bind_reply" = "$2" ] && return 0
    case "$bind_reply" in
        'refused bind_stage'|'refused bind_files'|'refused bind_native_service'|'refused bind_native_view'|'refused bind_native_proc_permission'|'refused bind_native_proc_read'|'refused bind_native_proc_limit'|'refused bind_namespace'|'refused bind_mount_table'|'refused bind_propagation'|'refused bind_foreign_mount'|'refused bind_journal'|'refused bind_provenance'|'refused bind_unknown_state'|'refused bind_contended'|'refused bind_mount_unconfirmed'|'refused bind_unmount_unconfirmed') reason=${bind_reply#refused };;
        'refused bind_mount_call_'*|'refused bind_unmount_call_'*)
            reason=${bind_reply#refused }; number=${reason##*_}; operation=${reason%_*}
            case "$operation:$number" in bind_mount_call:*|bind_unmount_call:*)
                case "$number" in ''|0*|*[!0-9]*|?????*) reason=helper_response;; *) [ "$number" -le 4095 ] || reason=helper_response;; esac;;
                *) reason=helper_response;;
            esac;;
        *) reason=helper_response;;
    esac
    return 1
}
ordinary_restore() {
    if ! no_helper_state || { ! absent "$stage/bind-intent" && ! fixed "$stage/before-mount" "$label"; }; then
        fixed "$stage/prepare-intent" "$label" && checked_helper && helper_step restore bind_restored &&
            helper_step status unbound && helper_step absent bind_absent || return 1
    fi
    clear_views || return 1
    if ! absent "$stage/original"; then
        regular "$stage/original" && records=$(originals) && printf '%s\n' "$records" | cmp -s - "$stage/original" || return 1
    fi
}
checked_names() {
    for item in "$1"/* "$1"/.[!.]* "$1"/..?*; do
        absent "$item" && continue
        case "$2:${item##*/}" in
            tmp:bind-journal.next|tmp:bind-active.next)
                regular "$item" && [ "$(stat -c '%u:%a:%h' "$item")" = 0:600:1 ] || return 1;;
            tmp:owner|tmp:helper|tmp:original|tmp:scratch-dir-id|tmp:prepare-intent|tmp:before-mount|tmp:bound-evidence|tmp:bind-intent|tmp:bind-journal|tmp:bind-active|tmp:removing|usb:owner|usb:source|usb:target)
                regular "$item" && [ "$(stat -c %h "$item")" = 1 ] || return 1;;
            *) return 1;;
        esac
    done
}
clear_bind() {
    if absent "$stage"; then absent "$scratch" && clear_views; return; fi
    lock_owner && checked_names "$stage" tmp || return 1
    if absent "$stage/removing"; then
        if ! absent "$scratch"; then
            same_scratch && checked_names "$scratch" usb || return 1
            if ! owned "$scratch"; then
                absent "$scratch/owner" && absent "$scratch/source" && absent "$scratch/target" && absent "$stage/original" &&
                    absent "$stage/prepare-intent" && absent "$stage/bind-intent" && no_helper_state || return 1
            fi
        fi
        ordinary_restore && once "$stage/removing" "$label" || return 1
    fi
    fixed "$stage/removing" "$label" && clear_views || return 1
    if ! absent "$scratch"; then
        same_scratch && checked_names "$scratch" usb || return 1
        if ! absent "$scratch/owner"; then
            owned "$scratch" || return 1
            for marker in source target; do
                if ! absent "$scratch/$marker"; then
                    case "$marker" in source) value=S; row=1;; target) value=T; row=2;; esac
                    fixed "$scratch/$marker" "$label:$value" || return 1
                    if ! absent "$stage/original"; then
                        [ "$(stamp "$scratch/$marker")" = "$(sed -n "${row}p" "$stage/original")" ] || return 1
                    fi
                    rm "$scratch/$marker" || return 1
                fi
            done
            rm "$scratch/owner" || return 1
        fi
        same_scratch && rmdir "$scratch" || return 1
    fi
    absent "$scratch" && clear_views && owned "$stage" || return 1
    for leaf in helper original scratch-dir-id prepare-intent before-mount bound-evidence bind-intent bind-journal bind-active bind-journal.next bind-active.next; do
        if ! absent "$stage/$leaf"; then rm "$stage/$leaf" || return 1; fi
    done
    rm "$stage/removing" "$stage/owner" && rmdir "$stage" && absent "$stage"
}
finish_refusal() {
    hint=''
    case "$reason" in bind_mount_call_*|bind_unmount_call_*|bind_mount_unconfirmed|bind_unmount_unconfirmed|helper_response) ;;
        *) if [ "$1" != status ] && no_helper_state && clear_views; then
               once "$stage/before-mount" "$label" || fail phase_evidence
               hint=' before_mount'
           fi;;
    esac
    printf 'scratch_bind refused %s %s%s\n' "$1" "$reason" "$hint"; exit 0;
}

case "$mode" in
    admit) admission && clear_views || fail admission; printf 'admitted\n';;
    probe)
        admission && clear_views || fail admission
        mkdir "$stage" && chmod g-s "$stage" && chmod 700 "$stage" && owner_file "$stage" && lock_owner || fail private_stage
        (set -C; ulimit -f 512; cat >"$stage/helper") && chmod 700 "$stage/helper" && checked_helper || fail helper_policy
        mkdir "$scratch" && identity=$(stat -c '%d:%i' "$scratch") && once "$stage/scratch-dir-id" "$identity" &&
            same_scratch && owner_file "$scratch" || fail scratch_stage
        once "$scratch/source" "$label:S" && once "$scratch/target" "$label:T" && (set -C; originals >"$stage/original") || fail original_record
        once "$stage/prepare-intent" "$label" || fail prepare_intent
        helper_step prepare bind_prepared || finish_refusal prepare
        once "$stage/bind-intent" "$label" || fail bind_intent
        helper_step bind bound || finish_refusal bind
        helper_step status bound || finish_refusal status
        once "$stage/bound-evidence" "$label" || fail bind_evidence
        printf 'scratch_bind bound\n'
        ;;
    witness)
        lock_owner && same_scratch && owned "$scratch" || fail witness_ownership
        if absent "$stage/bound-evidence"; then printf 'independent_view not_attempted\n'; else
            fixed "$stage/bound-evidence" "$label" && checked_helper || fail witness_ownership
            if helper_step status bound; then printf 'independent_view bound\n'; else
                if [ "$bind_reply" = unbound ]; then printf 'independent_view unbound\n'; else printf 'independent_view refused %s\n' "$reason"; fi
            fi
        fi
        ;;
    close) trap '' HUP INT TERM; reason=cleanup_unconfirmed; clear_bind || fail "$reason"; printf 'closed\n';;
    absent) absent "$stage" && absent "$scratch" && clear_views || fail residual; printf 'absent\n';;
    *) exit 64;;
esac
