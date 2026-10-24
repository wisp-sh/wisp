#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
example_root="$repo_root/examples/go-import-rust"
if [[ "${1:-}" == "--native" ]]; then
  if [[ ! -f "$example_root/go/vendor/github.com/wago-org/wago/wago.go" ]]; then
    "$example_root/install.sh"
  fi
  cd "$example_root/go"
  exec "${WISP_GO:-go}" run -mod=vendor .
fi

: "${WISP_WASMTIME:?Configure the Wisp Wasmtime executable}"
cli="${WISP_CLI:-$repo_root/target/debug/wisp}"
output="${1:-$repo_root/out/go-import-rust}"
"$cli" build "$example_root" --component-only --out "$output" \
  --cache "${WISP_BUILD_CACHE:-$repo_root/.tools/build/runtime-cache}" > /dev/null
result="$("$WISP_WASMTIME" run --invoke 'doubled-sum(20,1)' "$output/component.wasm")"
[[ "$result" == "42" ]] || { printf 'Expected 42, got %s\n' "$result" >&2; exit 1; }
printf '%s\n' "$result"
