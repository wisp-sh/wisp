#!/usr/bin/env bash
set -euo pipefail

example_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
go="${WISP_GO:-go}"
export GOPROXY="file://$example_root/proxy,${GOPROXY:-https://proxy.golang.org,direct}"
export GONOSUMDB="${GONOSUMDB:+$GONOSUMDB,}example.org/rust-arithmetic"
export GOWORK=off

cd "$example_root/go"
"$go" mod download
"$go" mod vendor
