# Target support

What you can consume a Wisp library from today, and which API features work
there. "Tested" means an installed consumer runs in Wisp's acceptance suites.
The exact cases, pinned versions and CI results are in
[target support evidence](verification/target-evidence.md).

## Consumers

| Consumer | Package | Platforms | Notes |
| --- | --- | --- | --- |
| Native Rust | Cargo crate | macOS ARM64, Linux x86_64 | Bundles Wasmtime; consumers install no separate runtime. |
| Node | npm package | Node 24 | ESM with top-level await. No npm dependencies or install scripts. |
| Browser | same npm package | Modern browsers | Pure libraries only. No WASI reactors, async values or networking. |
| Go | Go module (optional) | Native macOS ARM64 and Linux x86_64, or `wasip1/wasm` | Scalar APIs pick native or Wasm automatically; native scalar execution is tested on Linux x86_64. Richer APIs use platform-specific native clients. |
| Component guest | Cargo crate / Wasm import | Tested in Wasmtime and JCO | Wisp composes the consumer and its libraries into one Component. |

## API features

| Feature | Native Rust | Node | Browser | Go | Component guest |
| --- | --- | --- | --- | --- | --- |
| Scalars, strings, lists, options, enums, records, typed errors | Tested | Tested | Tested | Tested | Tested |
| Several interfaces sharing one instance | Tested | Tested | Tested | Tested | Tested |
| Resources (objects with methods) | Tested | Tested | Untested | Tested | Tested |
| `future` and `stream` results | Tested | Tested (needs JSPI) | No | Tested | Tested |
| Async parameters, nested async handles, async resource methods | No | No | No | No | No |
| WASI HTTP, DNS and TCP | Tested | Tested | No | Tested | Host must provide |

## Source languages

| Language | API definition |
| --- | --- |
| C++, Go, Rust | Inferred from ordinary public functions, including strings, lists, options, enums, records and results |
| C | Scalar functions inferred; richer APIs use authored WIT |
| Python | Guest inference across bundled modules for annotated functions, dataclasses, enums and typed results; authored WIT supported. Automatic stdlib-aware templates, offline hash-locked wheels with markers/extras, and read-only package resources. 16 MiB source/data and 64 MiB Components. Linux x86_64 provisioning is verified; native extensions remain unsupported. |
| AssemblyScript, Bend 2, Goose, Grain, Java, JavaScript, Kotlin, Nim, Swift, TypeScript, Zig | Builds supported; API limits vary by adapter |

See [project builds](project-builds.md#output-and-current-limits) for each
language's binding limits.
