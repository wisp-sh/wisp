#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
: "${WISP_WASMTIME:?Configure the Wisp Wasmtime executable}"
cli="${WISP_CLI:-$repo_root/target/debug/wisp}"
output="${1:-$repo_root/out/cpp-import-rust}"
"$cli" build "$repo_root/examples/cpp-import-rust" --component-only --out "$output" \
  --cache "${WISP_BUILD_CACHE:-$repo_root/.tools/build/runtime-cache}" > /dev/null
result="$("$WISP_WASMTIME" run --invoke 'doubled-sum(20,1)' "$output/component.wasm")"
[[ "$result" == "42" ]] || { printf 'Expected 42, got %s\n' "$result" >&2; exit 1; }
printf '%s\n' "$result"
