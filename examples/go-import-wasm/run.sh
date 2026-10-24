#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cli="${WISP_CLI:-$repo_root/target/debug/wisp}"
example_root="$repo_root/examples/go-import-wasm"
if [[ "${1:-}" == "--native" ]]; then
  # A clean checkout keeps generated package sources, but downloads upstream
  # runtime sources through ordinary Go tooling on first use.
  if [[ ! -f "$example_root/go/vendor/github.com/wago-org/wago/wago.go" ]]; then
    "$example_root/install.sh"
  fi
  cd "$example_root/go"
  exec "${WISP_GO:-go}" run -mod=vendor .
fi

output="${1:-$repo_root/out/go-import-wasm}"
cache="${WISP_BUILD_CACHE:-$repo_root/.wisp-cache}"
: "${WISP_WASMTIME:?Set WISP_WASMTIME to the configured Wasmtime executable}"

"$cli" build "$repo_root/examples/go-import-wasm" --component-only \
  --out "$output" --cache "$cache" > /dev/null
result="$("$WISP_WASMTIME" run --invoke 'doubled-sum(20, 1)' "$output/component.wasm")"
if [[ "$result" != "42" ]]; then
  printf 'Expected 42 from the Go export, got: %s\n' "$result" >&2
  exit 1
fi
printf '%s\n' "$result"
