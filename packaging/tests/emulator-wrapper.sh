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

printf '%s\n' "$$" > "$run_root/data/logs/native.pid"
test_case=$(cat "$run_root/data/logs/test-case" 2>/dev/null || printf '%s' interactive)
duration=900000
screen=library
case "$test_case" in
    startup-1|startup-2|startup-3) duration=2500 ;;
    interactive|reader|recovery|review) ;;
    ux-reader) screen=reader ;;
    *) exit 2 ;;
esac

if [ "$test_case" = review ]; then
    exec "$run_root/app/$binary" "$@" --demo --offline --screen library
fi
if [ "$test_case" = recovery ]; then
    exec "$run_root/app/$binary" "$@" \
        --demo --offline --screen library --exit-after-ms "$duration"
fi
exec "$run_root/app/$binary" "$@" \
    --demo --offline --screen "$screen" \
    --capture "$run_root/data/logs/final-native.png" \
    --exit-after-ms "$duration"
