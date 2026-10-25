# Wisp Serde candidate check

This verifier requires a source-built Rust compiler with macro support. The
downloaded bootstrap compiler has no macro loader and is rejected. Supply an
explicit bundle and compiler hash from your installed compiler package.

From the Wisp repository root, with the package tools installed:

```sh
source .tools/build/env.sh
rust_candidate_sha="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["compiler"]["moduleSha256"])' "$WISP_RUST/toolchain.json")"
python3 -I tests/source-build/rust-serde.py \
  --candidate "$WISP_RUST" \
  --compiler-sha256 "$rust_candidate_sha" \
  --compiler-wasmtime "$WISP_RUST_WASMTIME" \
  --fixture tests/source-build/fixtures/serde \
  --vendor-dir ../wisp-buildtools/rust/build/vendor \
  --cli target/debug/wisp \
  --wasmtime "$WISP_WASMTIME" \
  --preview1-adapter "$WISP_PREVIEW1_ADAPTER" \
  --node "$WISP_NODE" \
  --jco "$WISP_JCO" \
  --out out/rust-serde-candidate
```

Use a fresh output directory. `--wasmtime` selects Wasmtime 50 for the final
Component. Compilation uses the separately pinned Wisp Wasmtime 38 host selected
by `--compiler-wasmtime`. Split compiler packages bind their compiler and LLVM
provider hashes to the original source-build and post-link receipts.
`--preflight` verifies input files and hashes
without running compilers or writing a passing `result.json`.

The script copies the ordinary Serde fixture and its locked vendored crates,
adds a normal `wisp.toml`, and retains dependency notices. Wisp builds and
admits the release, then Wasmtime calls `roundtrip-score(20,22)` and must return
`42`. The fixture source and published Serde crates remain unchanged.

Compiler evidence checks the exact source-build SHA, Wasm commands, dependency
build scripts, and successful `Serialize`/`Deserialize` interpreter traces.
Wisp atomically renames its staging directory. The verifier records that path
mapping while preserving the original report and trace bytes.

A second source copy adds `#[derive(Serialize)]` to a union. Its build must fail
with `Serde does not support derive for unions` and leave no application output.
A query-only native Rust guard forbids native source compilation. Cargo metadata
and normal host package/admission tooling have separate command phases and do
not count as application compiler evidence.

`commands.json`, `logs/`, `inputs.json`, `macro-evidence.json` and `result.json`
retain the actual evidence. Only a complete positive build, execution and
negative diagnostic check produce `status: "passed"`. This check does not
replace the standalone six-case compiler acceptance or the registry verifier.

[Source compiler CI](../../docs/verification/source-build-ci.md) installs versioned bundles
into a fresh prefix, provisions the shared hosts and vendors this committed
fixture without a sibling compiler checkout. It also executes the generated
Serde Rust package through an offline native Cargo consumer.
