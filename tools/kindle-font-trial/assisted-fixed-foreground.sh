fixed_foreground_check() {
    fixed_reason=gui; gui_args || return 1
    fixed_reason=identity; [ "$gui:$birth" = "$identity" ] || return 1
    fixed_reason=generation; plain "$lock/generation" && printf '%s\n' "$generation" | cmp -s - "$lock/generation" || return 1
    fixed_reason=owner; plain "$lock/application-owned" && printf '%s:%s\n' "$generation" "$identity" | cmp -s - "$lock/application-owned" || return 1
    fixed_reason=power; [ "$(power_state)" = active ] || return 1
    for fixed_peer in aidoku bilicomics; do
        fixed_reason=peer_lock; absent "/tmp/$fixed_peer-launcher.lock" || return 1
        fixed_reason=peer_process
        if pidof "$fixed_peer" >/dev/null 2>&1; then return 1; else [ "$?" = 1 ] || return 1; fi
    done
    fixed_reason=manager; fixed_managers=$(pidof awesome) && [ -n "$fixed_managers" ] || return 1
    for fixed_manager in $fixed_managers; do
        fixed_reason=manager_state; [ "$(awk '{print $3}' "/proc/$fixed_manager/stat" 2>/dev/null)" = T ] || return 1
    done
    fixed_display=0; fixed_touch=0
    for fixed_fd in "/proc/$gui/fd/"*; do
        case "$(readlink "$fixed_fd" 2>/dev/null || :)" in /dev/fb0) fixed_display=1;; /dev/input/event*) fixed_touch=1;; esac
    done
    fixed_reason=display; [ "$fixed_display" = 1 ] || return 1
    fixed_reason=touch; [ "$fixed_touch" = 1 ] || return 1
    case "$(task_state)" in S|R) return 0;; D) fixed_reason=task_busy;; *) fixed_reason=task;; esac
    return 1
}

fixed_uptime_cs() {
    awk 'NF==2 && $1 ~ /^[0-9]+\.[0-9][0-9]$/ {value=$1; sub(/\./,"",value); ok=1} END {if (!ok || NR!=1) exit 1; printf "%.0f\n", value+0}' /proc/uptime
}

fixed_capture_shelf() {
    fixed_waits=0
    while :; do
        if fixed_foreground_check; then fixed_reason=ready; fi
        case "$fixed_reason" in ready|task_busy) ;; *) fail "pre_$fixed_reason";; esac
        if [ "$fixed_reason" = task_busy ] || [ "$fixed_waits" -gt 0 ]; then
            fixed_now=$(fixed_uptime_cs) || fail pre_clock
            if [ "$fixed_waits" = 0 ]; then fixed_start=$fixed_now; fi
            [ "$fixed_now" -ge "$fixed_start" ] || fail pre_clock
            [ "$fixed_waits" -lt 10 ] || [ "$fixed_reason" = ready ] || fail pre_task_busy
            [ "$((fixed_now - fixed_start))" -lt 120 ] || fail pre_task_busy
        fi
        [ "$fixed_reason" != ready ] || break
        usleep 100000 || fail pre_wait
        fixed_waits=$((fixed_waits + 1))
    done
    absent "$out/shelf.next" && absent "$out/shelf" || fail scene_publish
    if ! (set -C; "$stage/shelf-scene" >"$out/shelf.next"); then
        rm -f "$out/shelf.next" && (set -C; printf 'not_observed\n' >"$out/shelf.next") || fail scene_publish
    fi
    if ! fixed_foreground_check; then
        rm -f "$out/shelf.next" || fail scene_cleanup
        fail "post_$fixed_reason"
    fi
    if ! mv "$out/shelf.next" "$out/shelf"; then
        rm -f "$out/shelf.next" || fail scene_cleanup
        fail scene_publish
    fi
}
