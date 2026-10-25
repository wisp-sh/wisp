# Rust Component library chain

This harness composes two Rust Component libraries. Library A exports a
named arithmetic interface. Its
`total` function takes a `line-item` record and returns
`result<u64, calculation-error>`. Consumer B imports A and exports a checkout
interface that calls A. WAC embeds both Components in a new Component. An outer
Wasmtime process executes it.

Both Rust guest crates depend only on `wit-bindgen`. Neither links a host runtime.
The proof has no WASI imports. It exercises composition directly. The
[installed package suite](../../tests/library-chain/README.md) covers Wisp package generation and installation.

## Reproduce

The provisioner supports Linux x86_64 and macOS ARM. Install Python 3.9 or later,
Git and Rustup first. From the repository root:

```sh
rustup toolchain install 1.93.0 --profile minimal
export RUSTUP_TOOLCHAIN=1.93.0
python3 tests/component-chain/run.py --setup
bash verify-component-chain.sh
```

`--setup` downloads tools and Cargo dependencies and then exits. Every tool
download has a fixed SHA-256 in [tools.lock.json](tools.lock.json). The normal run
uses `cargo --locked --offline` and existing local Component paths. It performs no
registry resolution or code downloads.

The verification script runs the proof and its rejection controls. It saves
logs and reports under `evidence/component-library-chain`. Run `run.py` without
`--setup` to execute only the positive proof.

To keep evidence from another run, pass `--output-dir /absolute/path`. To use a
different provisioning directory, pass `--tools-dir /absolute/path` to both
commands. The default directories are `.tools/component-library-chain` and
`out/component-library-chain` in the repository.

The proof pins Rust 1.93.0, the compiler used for the first executed macOS ARM
result. The existing Linux language-package builder uses Rust 1.98.1 and a
separate toolchain lock. This guest-only proof does not establish native package
support on macOS.

## Checks and evidence

The run validates both input Components, checks their decoded imports, and
composes twice from the same bytes. It checks that both results have the same
hash and that the input files remain unchanged. The composed Component must have
no unresolved imports. The full Cargo dependency graph must contain no Wasmtime,
Wasmer or Wasmi packages.

The same calls execute directly against A and against the composed B:

| Input | Result |
| --- | --- |
| Quantity 3, unit price 7 | `ok(21)` |
| Quantity 2, unit price `18446744073709551615` | `err(overflow)` |
| Quantity 0, unit price `18446744073709551615` | `ok(0)` |

`report.json` records each command and its output, source hashes, Git revision
and worktree status, tool versions and binary hashes, the Cargo package graph,
decoded imports, Component hashes, and call results. `cargo-metadata.json`
records the complete Cargo graph. The `.wit` files show the contracts decoded
from the built bytes.

The rejection controls use real, validated Component fixtures. They check missing
dependencies, incorrect interface names, incompatible function results, invalid
Component bytes, and preservation of existing output after a failed composition.
They also check that B cannot execute before its dependency is composed, that
the recorded hashes match the artifacts, that a failed proof removes stale
success evidence, and that a modified tool cannot execute.

The [Component chain workflow](../../.github/workflows/component-library-chain.yml)
installs the pinned prerequisites on Linux x86_64 and macOS ARM, then disables
network access for the complete verification script. Linux uses a network
namespace; macOS uses `sandbox-exec`. CI preserves the Components and execution
reports for 90 days. A workflow definition alone is not evidence of a passing CI
run.

After setup, the same network restriction can be applied locally on macOS:

```sh
sandbox-exec -p '(version 1) (allow default) (deny network*)' bash verify-component-chain.sh
```

The report establishes what this run built and executed. It does not certify
uploaded Components as safe, or establish byte reproducibility between different
compilers, host platforms, source paths or operating systems.

## Equivalent build steps

The runner performs these steps with pinned tools:

```sh
cargo build --release --locked --offline --target wasm32-unknown-unknown
wasm-tools component new target/wasm32-unknown-unknown/release/chain_library.wasm -o library.wasm
wasm-tools component new target/wasm32-unknown-unknown/release/chain_consumer.wasm -o consumer.wasm
wac compose chain.wac --dep wisp:library=library.wasm --dep wisp:consumer=consumer.wasm -o composed.wasm
wasmtime run --invoke 'total({quantity: 3, unit-price: 7})' composed.wasm
wasmtime run --invoke 'total({quantity: 2, unit-price: 18446744073709551615})' composed.wasm
```

These commands assume this directory as the working directory and tools on
`PATH`. The Python runner uses absolute paths and preserves the results under its
output directory.

The [architecture decision](../../docs/verification/architecture.md) records the
selected tools, evidence and limits.
