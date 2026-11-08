#!/bin/sh
# Supervised, time-limited offline smoke test on an identified physical Kindle.
set -eu

run_root=${WEREAD_HOME:?}
binary=$(cat "$run_root/data/logs/test-binary" 2>/dev/null || printf '%s' weread-61fa9afd1b5b)
case "$binary" in
    weread-????????????)
        case "${binary#weread-}" in *[!0-9a-f]*) exit 2 ;; esac
        ;;
    *) exit 2 ;;
esac
test_case=$(cat "$run_root/data/logs/test-case" 2>/dev/null || printf '%s' smoke)
screen=library
case "$test_case" in
    smoke) duration=30000 ;;
    interactive|sleep) duration=300000 ;;
    experience) duration=600000 ;;
    *) exit 2 ;;
esac
printf '%s\n' "$$" > "$run_root/data/logs/native.pid"
exec "$run_root/app/$binary" "$@" \
    --demo --offline --screen "$screen" --exit-after-ms "$duration"
