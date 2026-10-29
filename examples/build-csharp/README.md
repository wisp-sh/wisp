# C# library Component

This example builds two `net8.0` projects using Roslyn inside Mono WASI. The API
project references a shared model/helper project and has two source files. The
output embeds Mono and managed IL with typed WIT bindings. Native .NET and the
WASI SDK are used only to bootstrap the compiler package.

After building or installing `wisp:csharp` and its `wisp:wasm-tools` dependency,
select their paths through `wisp toolchain env`, or use a local buildtools checkout:

```sh
export WISP_CSHARP_TOOLCHAIN=/path/to/wisp-buildtools/csharp
export WISP_WASM_TOOLS=/path/to/wisp-buildtools/wasm-tools/dist/wasm-tools.wasm
export WISP_PREVIEW1_ADAPTER=/path/to/wisp-buildtools/wasm-tools/dist/wasi_snapshot_preview1.reactor.wasm
export WISP_WASMTIME=/absolute/path/to/wasmtime
wisp build examples/build-csharp --component-only --out out/csharp-library
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/csharp-library/component.wasm
# 42
"$WISP_WASMTIME" run --invoke 'greet("世界")' out/csharp-library/component.wasm
# "Hello, 世界!"
"$WISP_WASMTIME" run --invoke 'advance({name: "Ana", age: 41})' out/csharp-library/component.wasm
# ok({name: "Ana", age: 42})
```

`entry` names the public type whose static methods form the public API. Roslyn
discovers their signatures in Wasm; no WIT is needed for this project. The
inferred contract covers strings, nested records/lists, options,
results, enums/flags, tuples, integer widths and floating point. `add.cs` and
`library.wit` at this directory's root retain the original one-file scalar example
for direct adapter use.

For offline dependencies, put exact `.nupkg` files in `build.project/packages`
and declare exact `PackageReference` versions, or use a local assembly
`Reference/HintPath` inside the project. Wisp snapshots these inputs and passes
the package directory to the compiler. No SDK or network restore runs during
source builds. Project references must remain inside `build.project`.

WIT imports, resources, general variants and async are not implemented. Project metadata supports a static subset of MSBuild; custom tasks,
conditions, generators and native packages are rejected. Runtime APIs depend on
the WASI BCL. Each Component is roughly 21 MiB and runs interpreted C#.

Omit `--component-only` to emit normal Cargo and npm packages as well. The emitted
`api` interface uses the package identity `example:csharp-library@1.0.0`.
Generated bindings and receipts remain under the output's `build/` directory.
For an authored contract, add these settings to `[build]`:

```toml
wit = "library/library.wit"
world = "library"
```

The [C# acceptance workflow](../../.github/workflows/csharp-acceptance.yml)
bootstraps a pinned compiler commit on GitHub, installs its ZIP and wasm-tools
into a fresh prefix, hides the compiler checkout, then disables network access.
It executes authored and inferred rich APIs, installs the generated npm archive,
runs a Cargo consumer of the emitted crate, changes a referenced source file and
checks that the changed result is rebuilt. Compiler-package CI also checks
multiple loose files, offline DLL/NuGet dependencies, unsupported public APIs,
managed exception traps and more than 300 calls in each rich Component instance.
Neither workflow publishes a release or a registry package.
