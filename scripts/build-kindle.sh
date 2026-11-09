#!/bin/sh
# Run on the designated Linux verification/build environment.

set -eu

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd "$SCRIPT_DIR/.." && pwd)
TARGET=armv7-unknown-linux-musleabihf
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$REPO_ROOT/target}
CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-12}
X11_HEADER_ROOT=${X11_HEADER_ROOT:-/usr/include}
STAGING_ROOT=$REPO_ROOT/dist/kindle
STAGED_APP=$STAGING_ROOT/weread/app

command -v cargo >/dev/null 2>&1 || { printf '%s\n' 'Rust Cargo is required.' >&2; exit 1; }
command -v zig >/dev/null 2>&1 || { printf '%s\n' 'Zig is required by cargo-zigbuild.' >&2; exit 1; }
[ -r "$X11_HEADER_ROOT/X11/Xlib.h" ] && [ -r "$X11_HEADER_ROOT/X11/Xutil.h" ] || {
    printf '%s\n' 'X11 development headers are required for the native frame guard.' >&2
    exit 1
}

export CARGO_TARGET_DIR CARGO_BUILD_JOBS
cd "$REPO_ROOT"
cargo zigbuild --locked --release --no-default-features --features kindle --target "$TARGET"
# The existing Kindle X11 runtime stays in an independent glibc process.
# The main application remains a static musl executable.
zig cc -target arm-linux-gnueabihf.2.17 -std=c11 -Wall -Wextra -Werror -O2 -s \
    -idirafter "$X11_HEADER_ROOT" "$REPO_ROOT/packaging/kindle/frame_guard.c" -ldl \
    -o "$CARGO_TARGET_DIR/$TARGET/release/weread-frame-guard"

mkdir -p "$STAGED_APP/config" "$STAGED_APP/licenses/fonts" "$STAGED_APP/licenses/pinyin" "$STAGED_APP/licenses/backend" "$STAGED_APP/licenses/zip" "$STAGED_APP/licenses/core" "$STAGED_APP/licenses/swash" "$STAGED_APP/licenses/zeno" "$STAGING_ROOT/documents"
# Remove the retired helper if this build reuses an older staging directory.
rm -f -- "$STAGED_APP/weread-wifi-helper"
install -m 755 "$CARGO_TARGET_DIR/$TARGET/release/weread" "$STAGED_APP/weread"
install -m 755 "$CARGO_TARGET_DIR/$TARGET/release/weread-frame-guard" "$STAGED_APP/weread-frame-guard"
install -m 755 "$REPO_ROOT/packaging/kindle/launch.sh" "$STAGING_ROOT/documents/weread.sh"
install -m 644 "$REPO_ROOT/packaging/kindle/device-profile.example" "$STAGED_APP/config/device-profile.example"
install -m 644 "$REPO_ROOT/packaging/kindle/device-allowlist.example" "$STAGED_APP/config/device-allowlist.example"
install -m 644 "$REPO_ROOT/packaging/kindle/manifest.draft.json" "$STAGED_APP/manifest.draft.json"

install -m 644 "$REPO_ROOT/THIRD_PARTY_NOTICES.md" "$STAGED_APP/THIRD_PARTY_NOTICES.md"
install -m 644 "$REPO_ROOT/packaging/kindle/README.md" "$STAGED_APP/README.md"
for notice in OFL.txt NotoSansCJK-LICENSE.txt NotoSerifCJK-LICENSE.txt NOTICES.txt manifest.json; do
    install -m 644 "$REPO_ROOT/assets/fonts/$notice" "$STAGED_APP/licenses/fonts/$notice"
done
for notice in "$REPO_ROOT/assets/pinyin/"*; do
    case "$notice" in
        *.txt|*.md|*/NOTICE|*/LICENSE|*/COPYING)
            [ -f "$notice" ] && install -m 644 "$notice" "$STAGED_APP/licenses/pinyin/$(basename "$notice")"
            ;;
    esac
done
for notice in LICENSE-MIT LICENSE-APACHE UPSTREAM_REVISION PATCHES.md; do
    install -m 644 "$REPO_ROOT/vendor/slint-backend-kindle/$notice" "$STAGED_APP/licenses/backend/$notice"
done
for notice in LICENSE UPSTREAM_REVISION PATCHES.md; do
    install -m 644 "$REPO_ROOT/vendor/zip/$notice" "$STAGED_APP/licenses/zip/$notice"
done
for notice in NOTICE.md UPSTREAM_REVISION PATCHES.md SOURCE_PROVENANCE.json; do
    install -m 644 "$REPO_ROOT/vendor/i-slint-core/$notice" "$STAGED_APP/licenses/core/$notice"
done
for notice in LICENSE-MIT LICENSE-APACHE UPSTREAM_REVISION PATCHES.md SOURCE_PROVENANCE.json; do
    install -m 644 "$REPO_ROOT/vendor/swash/$notice" "$STAGED_APP/licenses/swash/$notice"
done
for notice in LICENSE-MIT LICENSE-APACHE UPSTREAM_REVISION PATCHES.md SOURCE_PROVENANCE.json; do
    install -m 644 "$REPO_ROOT/vendor/zeno/$notice" "$STAGED_APP/licenses/zeno/$notice"
done
# Fonts, icons, dictionary and interface strings are embedded in the binary.
# Runtime account files and content directories are never package inputs.

printf '%s\n' "Staged development bundle: $STAGING_ROOT"
printf '%s\n' 'No KPM release archive was created. Device and license acceptance remain required.'
