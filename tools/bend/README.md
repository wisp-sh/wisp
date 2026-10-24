# Bend 2 compiler adapter

This adapter runs the pinned [Bend 2 checker and JavaScript compiler](https://github.com/bendlang/bend/tree/295e996574573e3a88827b1cf7045b437333c8f0)
inside a WebAssembly Component. Wisp's JavaScript toolchain then compiles the
generated JavaScript and selected WIT contract into the application Component.
Application builds require Python, Wasmtime Python Component bindings, the
Wasmtime command host, and the JavaScript and wasm-tools compiler packages.
They do not run native Bend, Bun, Node or C compilers.

## Bootstrap

Select the existing JavaScript adapter and its Wasm compiler dependencies:

```sh
export WISP_JAVASCRIPT_TOOLCHAIN=/path/to/javascript
export WISP_WASM_TOOLS_TOOLCHAIN=/path/to/wasm-tools
export WISP_WASM_TOOLS=/path/to/wasm-tools/dist/wasm-tools.wasm
export WISP_WASMTIME=/path/to/wasmtime
export WISP_WASMTIME_PYTHON=/directory/containing/wasmtime
python3 tools/bend/bootstrap.py
export WISP_BEND_TOOLCHAIN="$PWD/tools/bend"
```

`upstream.lock.json` pins the Bend revision and the length and SHA-256 of its
checker, compiler, Base library and Apache-2.0 license. Bootstrap downloads
only those files and verifies them before use. It builds `dist/compiler.wasm`
with the Wasm-hosted esbuild and QuickJS Component tools. `--offline` reuses the
verified `build/downloads` cache. Bootstrap requires an absent `dist` directory.
To rebuild, remove that generated directory explicitly.

The generated manifest records source pins, adapter recipe hashes and output
hashes. Application builds verify it before invoking the compiler. Bootstrap
replaces Node-specific module setup with an in-memory filesystem and adds a
public-export filter to the C/JS compiler's JS module entrypoint. Bend's parser,
type checker, proof checker and generated JavaScript runtime remain upstream.

## Build

```sh
wisp build examples/build-bend --component-only --out out/bend-math
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/bend-math/component.wasm
"$WISP_WASMTIME" run --invoke 'multiply(1.5,2.0)' out/bend-math/component.wasm
```

Select `compiler = "bend"` and one `.bend` source entrypoint. Use
`build.tools.bend`, `WISP_BEND_TOOLCHAIN`, or an installed `wisp:bend` toolchain
with a `build` entrypoint at `bend/compile.py`. The adapter directory includes
`bootstrap.py`, `upstream.lock.json`, `src/`, `compiler.wit` and its generated
`dist/` tree. `wisp toolchain env` exports a selected Bend package root.
Select the JavaScript package through `build.tools.javascript`,
`WISP_JAVASCRIPT_TOOLCHAIN`, or its installed toolchain selection.

The public API supports direct synchronous functions with `U32` and `F32`
parameters and return values. Names must start with a lowercase ASCII letter
and contain only lowercase ASCII letters and digits. Wisp infers their WIT
`u32` and `f32` signatures. Helpers may use other Bend types internally.
When an entrypoint contains proofs, nonnumeric helpers or other declarations
that prevent inference, supply an adjacent `.wit` contract or `build.wit`
with direct numeric function exports. This does not add conversions for
language-managed values or interface bindings.

`import Base` uses the pinned library. Local imports use the captured project
files, including subdirectories under `build.project`. Hub imports, foreign
effects, WIT imports, erased/template public parameters, heap-valued public
types and Bend 1/HVM programs are unsupported. The compiler runs Bend's
checking pass and rejects type errors and unproved laws. This does not add
proof guarantees beyond those provided by upstream Bend.

The JavaScript backend executes sequentially, including parallel call syntax.
It provides no CPU worker pool or GPU execution. Bend's C backend currently
emits native executables and uses pthreads, virtual memory reservations and
POSIX signal handling. Using it for Components requires a WASI runtime port
and a callable library ABI.

Build receipts retain the compiler revision and Component hash, source hashes,
export list, JavaScript stages and output Component hash. Source compilation
has a ten-minute process timeout and a 32 MiB source budget.

## Verify

After bootstrap, run the actual checker tests and the source-build check:

```sh
python3 tools/bend/test_compiler.py
python3 tests/source-build/languages.py --buildtools ../wisp-buildtools \
  --languages bend --out out/bend-check
```

The checker tests cover numeric exports, local imports, completed and invalid
proofs, missing proofs, type errors, import cycles and rejected import paths.
The source-build check blocks native compiler commands, builds a Component,
checks its receipt and invokes `add(20,22)`.
