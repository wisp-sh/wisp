#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cli="${WISP_CLI:-$repo_root/target/debug/wisp}"
output="${1:-$repo_root/out/cpp-import-go}"
cache="${WISP_BUILD_CACHE:-$repo_root/.wisp-cache}"
: "${WISP_WASMTIME:?Set WISP_WASMTIME to the configured Wasmtime executable}"

"$cli" build "$repo_root/examples/cpp-import-go" --component-only \
  --out "$output" --cache "$cache" > /dev/null
result="$("$WISP_WASMTIME" run --invoke 'doubled-sum(20, 1)' "$output/component.wasm")"
if [[ "$result" != "42" ]]; then
  printf 'Expected 42 from the C++ export, got: %s\n' "$result" >&2
  exit 1
fi
printf '%s\n' "$result"
