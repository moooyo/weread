#!/bin/sh
# Prepare a fixed-entry client for the coordinated shared runtime extension.
set -eu
CATALOG_SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
CATALOG_SOURCE=$(CDPATH= cd "$CATALOG_SCRIPT_DIR/../vendor/kindle-catalog-client" && pwd)
CATALOG_OUTPUT=${1:?Pass the candidate output directory}
CATALOG_ZIG=${ZIG_BIN_DIR:+$ZIG_BIN_DIR/}zig
mkdir -p "$CATALOG_OUTPUT"
"$CATALOG_ZIG" cc -target arm-linux-gnueabihf.2.4 \
    -mcpu=cortex_a8 -mfpu=vfpv3-d16 -mfloat-abi=hard \
    -std=c11 -O2 -Wall -Wextra -Werror -fstack-protector-strong -fPIE -pie \
    -Wl,-z,relro,-z,now -s "$CATALOG_SOURCE/catalog-client.c" \
    -ldl -lrt -o "$CATALOG_OUTPUT/weread-catalog-client"
(
    cd "$CATALOG_OUTPUT"
    sha256sum weread-catalog-client >SHA256SUMS
)
