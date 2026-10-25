# Target support

## Component library packages

The `wisp library pack` format uses structural WIT generation. Its release records the supported targets and runtime requirements. These rows describe executable acceptance cases, with narrower limits stated where a backend or ownership conversion remains incomplete.

| Contract or capability | Native Rust | Component guest | Node | Browser | Optional Go |
| --- | --- | --- | --- | --- | --- |
| Ordinary nested values, typed results, shared types and several interfaces | Executed | Executed through composition | Executed | Pure library covered by browser fixture | Executed |
| Shared instance state across interfaces | Executed through one `Library` | Executed | Executed in one ESM instance | Executed | Executed |
| Resource constructors, methods, static calls, owned and borrowed values, nested resource records/variants/lists/options | Executed | Executed through composition | Executed | Unverified | Executed through installed Go module |
| Direct future of a record and stream of strings | Executed with cancellation and cleanup | Executed through composition in outer Wasmtime | Direct library values and stream cancellation executed | Excluded | Executed through installed Go module |
| JCO execution of the composed async graph | N/A | Wasmtime execution passes | Tested graph passes with `yield_async()` before the future return; the earlier graph without that yield failed task context lookup | Excluded | N/A |
| Future cancellation in JCO | N/A | Rust guest close is exercised | No public cancel/dispose operation in pinned JCO | Excluded | N/A |
| Nested future/stream handles, async parameters and async resource methods | Rejected pending ownership coverage | Same rejection | No release advertised | Excluded | Rejected |
| Direct error-context values | Executed with 127 cumulative lifts per Store | Executed through WAT fixture composition | Executed | Excluded | Executed through installed Go module with the same Store limit |
| Standard WASI networking with exported resource values | HTTP, DNS and TCP fixture executed | Outer runtime supplies admitted host imports | HTTP, DNS and TCP fixture executed | Rejected with a stable reason | HTTP, DNS and TCP fixture executed |
| Go reactor libraries built by `wisp build` | Installed rich-value fixture executed; scalar and rich consumers are covered by [source compiler CI](source-build-ci.md) | Scalar graphs and rich Go imports into C++/Rust composed and executed through Node | Installed consumers execute, including clocks, randomness, heap growth and GC | Excluded with `unsupported-browser-wasi-reactor` | Native scalar execution passes with pinned upstream Wago main, including clocks, randomness, heap growth and GC on Linux x86_64. Rich APIs require native client bindings |
| Published scalar Go package imported by C++ | Not exercised for this consumer | Wisp registry fetch, generated C++ header, C++ compilation and composed Wasm execution verified | Installed composed consumer executed | Excluded by the Go reactor profile | C++ consumer needs no Go compiler; Go runtime remains inside the dependency |
| Published scalar C++ package imported as a Go module | Not exercised for this consumer | Go module proxy installation, Go compilation and composed Wasm execution verified | Installed composed consumer executed | Excluded by the Go reactor profile | One scalar module selects native Wago or typed `wasip1/wasm` imports automatically |

Import-free scalar Rust APIs use a bundled Wasmtime 50.0.0 worker. Scalar APIs
with WASI or other host imports retain their established native hosts, as do
richer Rust contracts. Worker admission checks fixed worker `0.2.0`, protocol
`1` and Wasmtime `50.0.0` versions and authenticates bundled hashes against an
independently trusted `WISP_WORKER_TRUST_MANIFEST`. With that manifest, validators
need no installed worker or matching local build. Without it, local development
uses hashes from the selected local worker bundle. Release-carried self-hashes
alone cannot authenticate executable bytes, and matching trusted bytes does not
prove safety. Never promote an untrusted uploaded manifest into the trust
store. See [package admission and provenance requirements](component-library-packages.md#admission-and-generator-authentication).

Rust native packages target macOS ARM64 and Linux x86_64. Installed ordinary,
resource, async, error-context and network chains passed on both hosts in
component-library CI
for commit `8da58a8`, including the optional Go installed consumers. Go's
richer client archive and admission remain tied to the declared native
platform. Scalar Go packages select native Wago at upstream commit
`dc13730184e8` or Wasm import
bindings from the same module. Their native build works with cgo disabled;
their Wasm build contains no host runtime. Native scalar execution is tested on Linux x86_64.
The Component target in the ordinary Cargo archive is
`wasm32-unknown-unknown`. Other Wasm targets fail explicitly for that archive.
The separate `wisp library rust-guest` projection used by source builds targets
`wasm32-wasip1` and supports the synchronous source values described below.
Both Rust guest dependency graphs contain no Wasmtime, Wasmer, or Wasmi dependency. Go module metadata declares Wago for native builds;
its Wasm implementation excludes that runtime.

The pure browser fixture covers application-local ESM loading. Browser and native host requirements differ. [Installed package tests](../tests/library-chain/README.md), [async tests](../tests/library-async/README.md), and [Go requirements](component-library-go.md) explain the tested paths. Application networking is independent of dependency fetching; library execution never downloads code.

## Source APIs

Wisp infers contracts from ordinary C++, Go and Rust source APIs. General
lists, optional values, enums, records and synchronous results map to WIT,
then wit-bindgen 0.61.1 generates the canonical ABI. Source adapters convert
ordinary language values to the generated types. Explicit WIT remains optional
and gives direct access to upstream generated APIs. Inferred scalar C source
uses the C++ compiler subset; authored C contracts use the upstream C generator.

C/C++ use a Wasm-hosted Clang driver with WASI SDK 34 target headers, libc,
libc++, libc++abi and compiler-rt. Exceptions and RTTI remain disabled.
Go uses the stock Wasm-hosted toolchain and upstream go-pkg v0.2.3 source.
Published Go guest imports support rich synchronous values; portable native
Wago bindings retain scalar signatures. Rich native Go clients retain their
existing typed host API.

Rust's dependency-free synchronous runtime projection currently rejects flags,
resources and async handles. Ordinary prebuilt-Component Rust packages retain
their broader existing guest/native support. Other source languages keep their
adapter-specific ABI limits.

See [project builds](project-builds.md) for generated package layouts and examples.
