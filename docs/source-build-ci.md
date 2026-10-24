# Source compiler CI

`.github/workflows/source-build.yml` tests the Wasm-hosted C++, Go and Rust
compilers on a fresh Linux x86_64 installation. It builds the CLI from the
locked repository, installs five schema 4 compiler bundles, builds Wisp's shared
Wasmtime hosts from pinned recipes, and runs `toolchain doctor` before and after
acceptance. Compiler caches and a sibling `wisp-buildtools` checkout are not
required.

The full compiler job currently runs through `workflow_dispatch`. Pushes and
pull requests run the acquisition contract tests. The full job needs published
compiler bundles, and no public compiler release URL is assumed. Dispatch it
with `bundle_manifest_url` and `bundle_manifest_sha256`, or configure these
repository variables and dispatch without input overrides:

- `WISP_SOURCE_TOOLCHAIN_MANIFEST_URL`: public HTTPS URL of the acquisition manifest.
- `WISP_SOURCE_TOOLCHAIN_MANIFEST_SHA256`: SHA-256 of its exact bytes.

Missing configuration fails an explicitly dispatched source-build job. The
acquisition tests do not claim compiler acceptance. After publishing the five
bundles and pinning the repository variables, remove the `workflow_dispatch`
condition on `jobs.source-build` to run the full job on pushes and pull
requests and configure it as a required check.

## Publish the inputs

Create and validate the bundles in the compiler repository first. The manifest
generator records all five bundle sizes, versions and complete ZIP hashes:

```sh
python3 tests/source-build/make-manifest.py \
  --bundle-dir /path/to/compiler-releases \
  --base-url https://YOUR-PUBLIC-HOST/wisp-toolchains/RELEASE \
  --out out/source-toolchains.json
```

Publish `source-toolchains.json` and the exact `clang.wisp.zip`, `go.wisp.zip`,
`rust.wisp.zip`, `llvm.wisp.zip` and `wasm-tools.wisp.zip`
files at that URL. The command only writes the manifest. It does not publish
artifacts or configure GitHub.

The acquisition manifest has exactly `schema`, `platform` and `bundles`.
`schema` is `1`, `platform` is `linux-x86_64`, and `bundles` contains those five
unique tools. Each entry has exactly `tool`, `version`, `url`, `size` and
`sha256`. Versions are stable canonical SemVer. URLs use public HTTPS without
credentials, query parameters or fragments. SHA-256 values are lowercase hex.
The manifest is limited to 64 KiB and each bundle to 2 GiB. Downloads must match
both their declared length and digest before `wisp toolchain install` admits
their contents. Redirects cannot downgrade HTTPS.

Node 24.20.0 and native Go 1.27.1 come from the repository's existing archive
locks. The npm dependency graph comes from `setup-tooling/package-lock.json`.
The native Rust builder uses Rust 1.98.1 in the same pinned Debian userspace as
the existing acceptance job. Rust 1.99.0 separately builds the two shared
Wasmtime hosts. These tools install dependencies and execute host packages.
The application source compilers run inside Wasm.

Host setup also downloads the checksum-pinned Wasmtime 50.0.0 Preview 1 reactor
adapter used by Wisp's package runtime. The `wasm-tools` bundle's own adapter
does not replace that package compatibility pin.

Setup separately installs the checksum-pinned WAC 0.10.1 executable from the
existing Component-chain lock. `acceptance-env.sh` exports `WISP_WAC` to that
copy, and the setup receipt records its URL, hash and version. Full acceptance
checks WAC and the configured native clangd before building any sources.

## Coverage and evidence

The full job runs these checks without optional skips:

- C++, Go and mixed source projects, local Cargo/npm/OCI publication, identical
  OCI fetches, and execution of installed npm packages after registry shutdown.
- Go importing a packaged C++ Component and C++ importing a packaged Go
  Component. Producer source and compiler paths are absent from consumer builds.
- Rust `std`, source-built `Serialize` and `Deserialize` derives, vendored
  Serde 1.0.228 and serde_json 1.0.150, and Wasm dependency build scripts.
  A Serde derive on an invalid union must fail with the expected diagnostic and
  leave no application output. A native Rust query guard rejects application
  source compilation.
- All four Rust import flows: Go imports Rust, C++ imports Rust, Rust imports Go,
  and Rust imports C++. Local registries stop before consumer builds; the Go
  proxy and module cache are removed. Scalar calls include an exact 64-bit value.
- Generated Rust packages installed through a local sparse Cargo registry and
  executed with `cargo run --locked --offline` after that registry stops. This
  includes the C++, Go, Rust and Serde producers and the composed import flows.
- Strings, byte buffers, records and typed results compiled from C++, Go and
  Rust sources. Both npm and native Rust consumers check Unicode, embedded NULs,
  100 KB strings, exact 64-bit record fields, error values and repeated buffer
  ownership transfers. Nine-string records exercise indirect parameters, and
  mixed float/integer results check canonical variant representations.
- C++ and Rust importing the installed Go rich-value provider. String copies
  survive later calls, and returned buffers, records and errors survive 1,000
  import cycles in npm and native Rust consumers.
- Nested results, unit payloads and records containing results in C++ and Rust
  source projects.

The repository contains the ordinary Serde fixture and its Cargo.lock under
`tests/source-build/fixtures/serde`. Setup authenticates its crate downloads
with `cargo vendor --locked`. The verifier checks every vendored file and crate
checksum, source-build and split-LLVM receipts, successful macro interpreter
traces, Wasm build-script commands and the admitted Component result.

`setup-inputs.json` records the acquisition manifest digest, all five bundle
digests, the installed prefix and the CLI hash. `inputs.json` records compiler
and execution-host hashes plus the tested source files. `compiler-receipts.json`
collects each actual build receipt and rejects a compiler hash that differs
from preflight. Native consumers retain their Cargo.lock, publication requests
and runtime results. Logs, receipts and exact application Components are saved
in the `source-build-linux-amd64-evidence` artifact, including failed runs.

A passing full run has `status: "passed"`, `complete: true` and
`cleanInstallVerified: true` in `out/source-build-ci/result.json`. Selecting
individual cases for diagnosis never reports complete acceptance.

## Run locally

Use a fresh prefix and output directory. The Linux bootstrap requires a
disposable Debian 13 container with root access. After installing the pinned
native prerequisites and building the CLI and its matching worker:

```sh
cargo build --locked -p wisp -p wisp-worker
```

Verifier snapshots must retain both binaries. The worker runs compiler jobs and
is bundled into import-free scalar Rust consumer packages. Scalar Go consumers
use pinned upstream Wago instead.

```sh
python3 tests/source-build/setup.py \
  --manifest out/source-toolchains.json --manifest-sha256 MANIFEST_SHA256 \
  --cli target/debug/wisp --prefix "$PWD/.tools/source-build" \
  --cache "$PWD/.downloads/source-build" --evidence "$PWD/evidence/source-build"
source .tools/source-build/acceptance-env.sh
export CARGO_TARGET_DIR="$PWD/target"
python3 tests/source-build/ci.py --cli target/debug/wisp \
  --prefix .tools/source-build --setup-evidence evidence/source-build \
  --out out/source-build-ci
```

Pass `--bundle-dir` to setup to use saved local bundles with the same strict
manifest checks. Shared execution hosts still use the pinned Wisp bootstrap.
An existing configured environment can run `ci.py` without `--prefix` and
`--setup-evidence`; its result explicitly leaves `cleanInstallVerified` false.
Set `WISP_SOURCE_SERDE` or pass `--serde-project` to a copy of the fixture with
its authenticated `vendor` directory. `WISP_CLANGD` selects the native clangd
used for C++ editor checks.
