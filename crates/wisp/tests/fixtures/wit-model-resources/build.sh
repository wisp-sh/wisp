#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
OUTPUT="${1:?usage: build.sh OUTPUT.wasm}"
: "${WISP_WASM_TOOLS:?set WISP_WASM_TOOLS to a wasm-tools executable before building the resource fixture}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
"$WISP_WASM_TOOLS" component embed "$ROOT" --world fixture "$ROOT/component.wat" -o "$TMP/embedded.wasm"
"$WISP_WASM_TOOLS" component new "$TMP/embedded.wasm" -o "$OUTPUT"
"$WISP_WASM_TOOLS" validate "$OUTPUT"
