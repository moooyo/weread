#!/bin/sh
# Run only on the Linux verification host with the product's actual Cargo outputs.
set -eu
test "$#" -eq 3 || {
    printf '%s\n' 'Usage: sh build.sh FEATURE_OUT_DIR ARM_RELEASE_DEPS NEW_OUTPUT_DIR' >&2
    exit 2
}
probe=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$probe/../../../.." && pwd)
out=$(CDPATH= cd -- "$1" && pwd)
deps=$(CDPATH= cd -- "$2" && pwd)
mkdir -- "$3"
work=$(CDPATH= cd -- "$3" && pwd)
export OUT_DIR=$out WEREAD_PROBE_PRODUCT_LOOKUP=$work/lookup.rs
grep -q 'static BUNDLED_CFF_RECEIPTS:' "$out/bundled_fonts.rs"
awk '/^fn bundled_cff_charge\(/, /^}/' "$repo/src/main.rs" > "$work/lookup.rs"
test -s "$work/lookup.rs"

unique_dep() {
    test "$#" -eq 1 && test -f "$1" || {
        printf '%s\n' 'Expected a single release rlib for each dependency.' >&2
        return 2
    }
    printf '%s\n' "$1"
}
anyhow=$(unique_dep "$deps"/libanyhow-*.rlib)
owned=$(unique_dep "$deps"/libowned_ttf_parser-*.rlib)
libc=$(unique_dep "$deps"/liblibc-*.rlib)
host=$(rustc -vV | sed -n 's/^host: //p')
linker=$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld
rustc --edition 2024 --target armv7-unknown-linux-musleabihf \
  --cfg 'feature="bundled-font-preflight"' -C opt-level=3 -C codegen-units=1 \
  -C linker-flavor=ld.lld -C linker="$linker" -C strip=symbols \
  -L "dependency=$deps" --extern "anyhow=$anyhow" --extern "owned_ttf_parser=$owned" --extern "libc=$libc" \
  "$probe/probe.rs" -o "$work/arm-font-admission"
printf 'Built ARM admission probe: %s\n' "$work/arm-font-admission"
