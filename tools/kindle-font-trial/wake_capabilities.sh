#!/bin/sh
set -eu
export LC_ALL=C
[ "$#" = 1 ] && [ "$1" = 'powerd-metadata' ] || exit 64
unknown() { printf 'WAKE_CAP unknown unknown\n'; exit 2; }
command -v timeout >/dev/null && command -v lipc-probe >/dev/null || unknown
if command -v lipc-set-prop >/dev/null; then setter=present; else setter=absent; fi
if ! rows=$(timeout -s KILL 2 lipc-probe com.lab126.powerd </dev/null 2>/dev/null); then unknown; fi
if ! property=$(printf '%s\n' "$rows" | awk '
    NF==0 {next}
    NR>512 || length($0)>1024 {bad=1;exit}
    ($0 ~ /^[[:space:]]*com\.lab126\.powerd:[[:space:]]*$/ || $0 ~ /^[[:space:]]*com\.lab126\.powerd[[:space:]]*$/) {head++;next}
    NF==3 && $1~/^(r|w|rw|wr)$/ && $2~/^(Int|Str|Has)$/ {list++; if($3=="wakeUp"){found++; if ($1=="w" && $2=="Int") prop="write_only_int"; else prop="other"}; next}
    $0 ~ /(^|[[:space:]])wakeUp([[:space:]]|$)/ {bad=1}
    END{if(bad || head!=1 || list<1 || found>1) exit 1; print found?prop:"not_listed"}'); then unknown; fi
printf 'WAKE_CAP %s %s\n' "$setter" "$property"
