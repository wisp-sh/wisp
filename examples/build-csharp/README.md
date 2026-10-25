# C# scalar Component

This example compiles ordinary C# with Roslyn inside Mono WASI. The output embeds
Mono and managed IL. Native .NET and the WASI SDK are used only to bootstrap the
compiler package in the wisp-buildtools checkout.

After `python3 build.py build csharp --jobs 1` in wisp-buildtools, select that
adapter and its shared Wasm tools. From the Wisp checkout:

```sh
export WISP_CSHARP_TOOLCHAIN=/path/to/wisp-buildtools/csharp
export WISP_WASM_TOOLS=/path/to/wisp-buildtools/wasm-tools/dist/wasm-tools.wasm
export WISP_PREVIEW1_ADAPTER=/path/to/wisp-buildtools/wasm-tools/dist/wasi_snapshot_preview1.reactor.wasm
export WISP_WASMTIME=/absolute/path/to/wasmtime
wisp build examples/build-csharp --component-only --out out/csharp-math
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/csharp-math/component.wasm
# 42
```

An installed `wisp:csharp` package and its `wisp:wasm-tools` dependency can supply
the same paths through `wisp toolchain env`. No SDK is needed for source builds.
The public contract is authored WIT. There is no C# signature inference.

The experimental profile accepts one `.cs` file containing public static
`int Calculator.Add(int, int)`. Other public names, WIT imports, strings, objects,
resources, async contracts, project evaluation and NuGet restore are unsupported.
Private helpers, ordinary classes and portable managed collections may be used
within that file. API availability is limited by the WASI BCL. Each scalar
Component includes the interpreter and is about 22 MiB.
