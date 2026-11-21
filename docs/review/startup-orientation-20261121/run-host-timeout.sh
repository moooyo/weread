#!/bin/sh
set -eu
[ "$#" -eq 1 ] && [ "$(uname -s)" = Linux ] && [ -d /work ] || {
    printf '%s\n' 'Usage: run-host-timeout.sh /work/src (inside the designated Linux worker)' >&2
    exit 64
}
source_root=$(cd "$1" && pwd -P)
fixture=$(cd "$(dirname "$0")" && pwd -P)/host-timeout-bench.rs
test -r "$source_root/Cargo.lock" && test -r "$fixture"
stage=$(mktemp -d /work/.orientation-host-poll.XXXXXXXX)
trap 'rm -rf -- "$stage"' EXIT
trap 'exit 1' HUP INT TERM
cp "$source_root/Cargo.lock" "$stage/"
cp "$source_root/vendor/slint-backend-kindle/Cargo.toml" "$stage/"
cp -R "$source_root/vendor/slint-backend-kindle/src" "$stage/"
cp "$fixture" "$stage/src/host-timeout-bench.rs"
cat >>"$stage/src/platform.rs" <<'RUST'

#[cfg(all(test, feature = "startup-orientation-reuse"))]
mod host_timeout_bench {
    include!("host-timeout-bench.rs");
}
RUST
cat >>"$stage/Cargo.toml" <<EOF

[workspace]

[patch.crates-io]
i-slint-core = { path = "$source_root/vendor/i-slint-core" }
swash = { path = "$source_root/vendor/swash" }
zeno = { path = "$source_root/vendor/zeno" }
EOF
cd "$stage"
export CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0
unset WEREAD_STARTUP_TIMING
cargo metadata --offline --format-version 1 --filter-platform x86_64-unknown-linux-gnu >/dev/null
cargo test --locked --offline --no-default-features --features startup-orientation-reuse --lib \
    platform::host_timeout_bench::controlled_two_site_timeouts -- --ignored --exact --nocapture --test-threads=1
