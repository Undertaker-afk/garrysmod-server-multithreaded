#!/bin/sh
# No Cargo/registry access required. Build products stay outside the repository.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
RUSTC=${RUSTC:-rustc}
PYTHON=${PYTHON:-python3}
TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/scprust-standalone.XXXXXXXX")
trap 'rm -rf "${TEMP_DIR:?}"' EXIT HUP INT TERM
# Keep the direct-rustc build's wire version aligned with the crate manifest.
CARGO_PKG_VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' crates/scprust-server/Cargo.toml)
export CARGO_PKG_VERSION

"$RUSTC" --edition 2021 --test crates/scprust-core/src/lib.rs -D warnings -o "$TEMP_DIR/core-tests"
"$TEMP_DIR/core-tests"
"$RUSTC" --edition 2021 --test crates/scprust-server/src/lib.rs -D warnings -o "$TEMP_DIR/query-tests"
"$TEMP_DIR/query-tests"
"$RUSTC" --edition 2021 --crate-name scprust_server --crate-type lib \
    crates/scprust-server/src/lib.rs -D warnings --out-dir "$TEMP_DIR"
"$RUSTC" --edition 2021 --test crates/scprust-server/src/main.rs -D warnings \
    --extern "scprust_server=$TEMP_DIR/libscprust_server.rlib" -o "$TEMP_DIR/cli-tests"
"$TEMP_DIR/cli-tests"
"$RUSTC" --edition 2021 crates/scprust-server/src/main.rs -D warnings \
    --extern "scprust_server=$TEMP_DIR/libscprust_server.rlib" -o "$TEMP_DIR/scprust-server"
"$PYTHON" -m unittest discover -s tests -p test_query_probe.py
"$PYTHON" tests/query_smoke.py "$TEMP_DIR/scprust-server"
"$RUSTC" --edition 2021 --test crates/scprust-server/src/bin/scprust-map.rs -D warnings \
    --extern "scprust_server=$TEMP_DIR/libscprust_server.rlib" -o "$TEMP_DIR/map-cli-tests"
"$TEMP_DIR/map-cli-tests"
"$RUSTC" --edition 2021 crates/scprust-server/src/bin/scprust-map.rs -D warnings \
    --extern "scprust_server=$TEMP_DIR/libscprust_server.rlib" -o "$TEMP_DIR/scprust-map"
"$PYTHON" tests/map_smoke.py "$TEMP_DIR/scprust-map"
