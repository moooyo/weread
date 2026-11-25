#!/bin/sh
# Install only in the disposable firmware-emulator guest, never in a product bundle.
set -eu

run_root=${WEREAD_HOME:?}
binary=$(cat "$run_root/data/logs/test-binary")
case "$binary" in
    weread-????????????)
        case "${binary#weread-}" in *[!0-9a-f]*) exit 2 ;; esac
        ;;
    *) exit 2 ;;
esac

# Launcher calibration calls must run the actual probe, not the configured UI case.
for argument do
    case "$argument" in
        --probe-native-portrait-framebuffer|--probe-native-portrait-calibration)
            exec "$run_root/app/$binary" "$@"
            ;;
    esac
done

test_case=$(cat "$run_root/data/logs/test-case" 2>/dev/null || printf '%s' interactive)
case "$test_case" in
    walkthrough-library|walkthrough-search|walkthrough-detail|walkthrough-reader|walkthrough-mp|walkthrough-article|walkthrough-settings|walkthrough-login)
        workspace=$(cat "$run_root/data/logs/test-walkthrough")
        case "$workspace" in
            ui-walkthrough-????????????)
                case "${workspace#ui-walkthrough-}" in *[!0-9a-f]*) exit 2 ;; esac
                ;;
            *) exit 2 ;;
        esac
        workspace=$run_root/data/logs/$workspace
        [ -d "$workspace" ] || exit 2
        printf '%s\n' "$$" > "$workspace/native.pid"
        screen=${test_case#walkthrough-}
        WEREAD_STARTUP_TIMING=1
        export WEREAD_STARTUP_TIMING
        set -- --kindle --device "$WEREAD_APPROVED_PROFILE" --data-dir "$workspace" \
            --offline --screen "$screen" --capture "$workspace/final-native.png" --exit-after-ms 180000
        unset WEREAD_API_KEY
        if [ "$screen" != login ]; then
            set -- "$@" --demo
        fi
        if [ -f "$workspace/actions.json" ]; then
            set -- "$@" --actions "$workspace/actions.json"
        fi
        # The current launcher verifies the expected application command line.
        exec -a "$run_root/app/weread" "$run_root/app/$binary" "$@"
        ;;
esac

printf '%s\n' "$$" > "$run_root/data/logs/native.pid"
duration=900000
screen=library
case "$test_case" in
    startup-1|startup-2|startup-3) duration=2500 ;;
    interactive|reader|recovery|review) ;;
    ux-reader) screen=reader ;;
    *) exit 2 ;;
esac

if [ "$test_case" = review ]; then
    exec -a "$run_root/app/weread" "$run_root/app/$binary" "$@" --demo --offline --screen library
fi
if [ "$test_case" = recovery ]; then
    exec -a "$run_root/app/weread" "$run_root/app/$binary" "$@" \
        --demo --offline --screen library --exit-after-ms "$duration"
fi
exec -a "$run_root/app/weread" "$run_root/app/$binary" "$@" \
    --demo --offline --screen "$screen" \
    --capture "$run_root/data/logs/final-native.png" \
    --exit-after-ms "$duration"
