#!/bin/sh
set -eu
exec "$@" \
  "--remap-path-prefix=$WISP_GO_BUILD_TARGET=/wisp/target" \
  "--remap-path-prefix=$WISP_GO_CARGO_HOME=/wisp/cargo" \
  "--remap-path-prefix=$WISP_GO_BUILD_SHIM=/wisp/go-shim"
