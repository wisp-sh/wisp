# Component library builds and composition

Rust Component consumers use generated Component import bindings. Wisp
composes the consumer and its resolved libraries before execution. The composed
artifact contains both guest Components and runs in an outer runtime. A guest
package must not depend on Wasmtime to call another Component.

The executable proof is
[component-library-chain](../../tests/component-chain/README.md).
Library A accepts a record and returns either a `u64` total or a named overflow
error. B calls A through a Component import. The harness requires the outer
Wasmtime process to return `ok(21)` and `err(overflow)` from B, matching direct
calls to A.

## Selected tools

| Step | Selected tool | Reason |
| --- | --- | --- |
| Guest compilation | Rust 1.93.0, `wasm32-unknown-unknown` | Guest compiler pin for this library without WASI. |
| Guest bindings | `wit-bindgen =0.61.1` | Reuses the version already locked by Wisp's Rust guest fixtures. Generates imports, records, enums and typed results from WIT. |
| Component encoding and validation | `wasm-tools 1.245.0` | Reuses the existing Wisp provisioner version and accepts these guest bindings. |
| Composition | `wac-cli 0.10.1`, explicit `.wac` file | Connects the imported interface and shared types; embeds both inputs in a fresh artifact. |
| Outer execution | Wasmtime CLI 50.0.0 | Reuses Wisp's runtime version; typed WAVE invocations exercise the actual Component export. |

The guest fixture's Cargo lock and the proof's tool-download lock pin separate
build graphs. Rust 1.93.0 is a proof compiler, not a change to the Linux native
package builder's existing Rust 1.98.1 pin.

`wit-bindgen` generates guest bindings from a WIT world and provides the ABI
support needed by those bindings. See the
[versioned crate documentation](https://docs.rs/wit-bindgen/0.61.1/wit_bindgen/).
Rust's `wasm32-wasip2` target directly emits Components and supports WASI, but this
proof uses `wasm32-unknown-unknown` and a separate encoding step. `wasm32-wasip2`
remains a target to verify before Wisp advertises it. See the
[Rust target documentation](https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip2.html).

The [WAC documentation](https://github.com/bytecodealliance/wac/tree/v0.10.1)
describes explicit composition and embedding dependencies. Version 0.10.1 also
[fixes aliases for imported types](https://github.com/bytecodealliance/wac/releases/tag/v0.10.1).
The proof uses `wac compose` with explicit file dependencies. Upstream
[issue 211](https://github.com/bytecodealliance/wac/issues/211) reports variable
ordering in `wac plug` with multiple dependencies. The runner compares repeated
explicit compositions byte for byte. That check covers these inputs and tool
versions; it is not a general reproducibility guarantee for WAC.

## Binding selection and build graph

The canonical input remains `wit_parser::Resolve` plus the selected world.
Code generation must use that model. Expanded inspection JSON is not a new
contract source.

A generated Rust library package needs target-specific dependencies and APIs:

| Consumer target | Binding behavior |
| --- | --- |
| Supported native Rust host | Calls the Component through Wisp's packaged host runtime. |
| `wasm32-unknown-unknown` Component guest | Calls imports generated from the dependency WIT; excludes host-runtime crates and native archives. |
| Other Rust Wasm targets | Reject with an explicit unsupported-target explanation until a real consumer passes. |

The architecture proof implements the guest path directly. The [installed package acceptance fixture](../../tests/library-chain/README.md) also verifies one generated Cargo package on native Rust and inside a Component guest, together with Cargo/npm registry installation, typed errors, and offline replay. Its general fixture covers nested values, shared types, multiple interfaces, and shared instance state.

The Component build sequence is:

1. Resolve and verify each dependency's original Component bytes and contract.
2. Build the consumer with guest import bindings from the same contract.
3. Encode the consumer core module as a Component.
4. Use explicit dependencies to compose the consumer and the original libraries.
5. Validate the new Component, inventory remaining host imports and execute the
   intended acceptance calls.

The proof's consumer imports `wisp:library-chain/arithmetic@0.1.0`. Composition
satisfies that import with A. The resulting Component only exports checkout and
has no host imports. Future WASI libraries must leave their host imports at the
outer boundary and declare which execution targets support those imports.

## Artifact identity and offline execution

Composition produces a new artifact. Original A and B files remain unchanged.
The report hashes both originals, the WIT, Rust sources, locks, composition file,
tools and composed output. This identifies the inputs used for the local run.
The full guest Cargo graph contains `wit-bindgen` and its build dependencies;
it contains no Wasmtime, Wasmer or Wasmi host runtime.

Tool installation and Cargo fetching happen in an explicit online setup phase.
The proof then builds with `--locked --offline` and resolves WAC dependencies
through local file paths. Executing the composed artifact requires the outer
runtime and those final bytes. It does not fetch code or instantiate an embedded
runtime. Application networking through a future WASI import is separate from
dependency installation.

[Component dependency locks](../component-dependencies.md) and [library
packages](../component-library-packages.md) have separate formats and admission
rules.
