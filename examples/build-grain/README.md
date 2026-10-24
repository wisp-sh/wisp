# Build Grain with a compiler running in Wasm

[math.gr](math.gr) exports addition, 64-bit increment and floating-point scaling.
The Grain compiler runs inside the shared Wasmtime host. Component assembly uses
the shared Wasm tools package. Neither stage needs Node.

From the Wisp checkout, build the compiler package and run this example:

```sh
source .tools/build/env.sh
python3 ../wisp-buildtools/build.py build grain
python3 examples/build-grain/build.py --out out/grain-math
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/grain-math/component.wasm
# 42
"$WISP_WASMTIME" run --invoke 'widen(9007199254740993)' out/grain-math/component.wasm
# 9007199254740994
"$WISP_WASMTIME" run --invoke 'scale(4)' out/grain-math/component.wasm
# 6
```

`WISP_GRAIN_TOOLCHAIN` selects the Grain package directory. Its default is the
sibling `wisp-buildtools/grain` checkout. `WISP_WASMTIME` selects Wisp's execution
host. `WISP_PREVIEW1_ADAPTER` selects the reactor adapter for any WASI Preview 1
imports retained by the Grain program.

For repeated builds, set `WISP_GRAIN_CACHE` to a writable directory such as
`$PWD/.tools/grain-cache`. It stores optional compiler bytecode and compiled
standard-library objects outside the compiler package. The first build populates
that cache. See the [Grain toolchain documentation](https://github.com/wisp-sh/wisp-buildtools/blob/main/grain/README.md)
for its size and cache checks.

The [WIT world](wit/world.wit) declares the public scalar contract. Grain's
`WasmI32`, `WasmI64` and `WasmF64` types give these functions the raw values
expected by that contract. Ordinary Grain `Number`, strings and collections have
their own runtime representation and require conversion before crossing this
boundary. The `@unsafe` annotations allow Grain's low-level Wasm primitives.
Unsigned addition and increment wrap at 32 and 64 bits.

The build script compiles the Grain source, converts its entry point to reactor
initialization after WASI adapter setup, wraps the result as a Component, and
generates a Wisp project containing its exact SHA-256. Wisp validates the Component
against the selected world through its existing prebuilt unit support. The script
extracts the full Component contract so any WASI host imports remain explicit.
Its build receipt records a `component` unit and preserves the admitted bytes.
The compiler assembly receipt and source are retained under `build/grain` in the
output. Wisp carries Grain's standard-library MIT notice into its packages.

Add `--package` to produce the normal Cargo and npm packages as well. That step
uses Wisp's existing packaging tools, as described in
[project builds](../../docs/project-builds.md#configure-the-compilers).

Grain source inference is not part of `wisp build`. This example gives it an
explicit contract and then uses the same Component admission and packaging flow
as a prebuilt Component from any other language.
