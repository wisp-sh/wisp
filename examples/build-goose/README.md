# Build Goose with a Wasm-hosted compiler

The Goose frontend emits C inside WASI. The shared LLVM guest compiles and
links it into a reactor. Wisp initializes Goose, derives the scalar contract
and packages the resulting WebAssembly Component.

From the Wisp checkout:

```sh
python3 ../wisp-buildtools/build.py build goose
source ../wisp-buildtools/env.sh
cargo build --locked -p wisp -p wisp-worker
# The frontend needs Wasm exceptions; LLVM uses the shared host.
export WISP_GOOSE_FRONTEND_HOST="$WISP_BOOTSTRAP_WASMTIME"
./target/debug/wisp build examples/build-goose --component-only --out out/goose-math
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/goose-math/component.wasm
# 42
"$WISP_WASMTIME" run --invoke 'widen(9007199254740993)' out/goose-math/component.wasm
# 9007199254740994
"$WISP_WASMTIME" run --invoke 'scale(4)' out/goose-math/component.wasm
# 6
```

Install `wisp:goose`, `wisp:llvm` and `wisp:clang` toolchain bundles to build
without a buildtools checkout. `WISP_GOOSE_TOOLCHAIN` overrides the Goose
package selection. The reactor adapter comes from `wisp toolchain setup-hosts`;
`WISP_PREVIEW1_ADAPTER` can select that pinned adapter explicitly.

The initial `scalar-v1` profile supports scalar APIs and implementations that
need no Goose dynamic data-stack regions. Dynamic arrays, strings, pools,
threads and native library calls are rejected. Goose's native virtual-memory
runtime has not been ported by this integration. See the
[compiler package documentation](https://github.com/wisp-sh/wisp-buildtools/blob/main/goose/README.md)
for the exact limits and bootstrap pins.
