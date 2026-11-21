#!/bin/sh
set -eu
umask 077
selector_kind=weread_stock_ab_one_shot
label=@LABEL@
stage=/mnt/us/.weread-render-@LABEL@
trial=$stage/arm
regular() { [ -f "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ] && [ "$(stat -c %h "$1")" = 1 ]; }
fixed() { regular "$stage/$1" && printf '%s\n' "$2" | cmp -s - "$stage/$1"; }
missing() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
@SELECTOR_GATE@
[ "$#" = 1 ] && [ "$1" = --library ] && [ "$(readlink -f "$trial")" = "$trial" ] || exit 1
fixed owner "$label" && fixed arm-name @LANE@ && fixed selector-live "$label" && fixed tile-dispatch-intent "$label" || exit 1
stock_selector_gate || exit 1
regular "$stage/selector-bind" && [ -x "$stage/selector-bind" ] && fixed bind-stat "$(stat -c '%d:%i:%s:%a:%u:%g:%Y:%h' "$stage/selector-bind")" &&
    [ "$("$stage/selector-bind" current stock "$label")" = bound ] || exit 1
(set -C; printf '%s\n' "$label" >"$stage/selector-service-view") && fixed selector-service-view "$label" || exit 1
regular "$stage/armed" && IFS= read -r stamp <"$stage/armed" && fence=${stamp#*:} && [ "$stamp" = "$label:$fence" ] || exit 1
case "$fence" in ''|0*|*[!0-9]*) exit 1;; esac
[ "${#fence}" -le 16 ] && fixed armed "$stamp" && missing "$stage/stopped" && missing "$stage/selector-return-intent" || exit 1
regular "$trial/launch.sh" && [ -x "$trial/launch.sh" ] && regular "$trial/data/library.sqlite" && missing "$stage/selector-entered" || exit 1
(set -C; printf '%s\n' "$label" >"$stage/selector-entered") && fixed selector-entered "$label" || exit 1
stock_selector_commit || exit 1
unset WEREAD_API_KEY WEREAD_STARTUP_TIMING
WEREAD_HOME=$trial
export WEREAD_HOME
exec /bin/sh "$WEREAD_HOME/launch.sh" --library
