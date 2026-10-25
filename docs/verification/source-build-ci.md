# Source compiler CI

`.github/workflows/source-build.yml` tests the Wasm-hosted C/C++, Go and Rust
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
Wasmtime hosts. These tools provision dependencies and handle Serde host-package
admission.
The application source compilers run inside Wasm. Guest APIs are inferred from ordinary source functions or supplied as authored
WIT, then generated with pinned `wit-bindgen-cli 0.61.1`.
C/C++ packages must include the WASI SDK target sysroot, and Go packages must
include the pinned upstream WIT runtime sources.

Host setup also downloads the checksum-pinned Wasmtime 50.0.0 Preview 1 reactor
adapter used by Wisp's package runtime. The `wasm-tools` bundle's own adapter
does not replace that package compatibility pin.

Setup separately installs the checksum-pinned WAC 0.10.1 executable from the
existing Component-chain lock. `acceptance-env.sh` exports `WISP_WAC` to that
copy, and the setup receipt records its URL, hash and version. Full acceptance
checks WAC before building the mixed-language Component.

## Coverage and evidence

The full job runs three acceptance groups:

- `sources` runs `run.py` against the C, C++, Go, Rust and mixed
  examples. Each build uses `--component-only`, checks the inferred or authored contract mode
  and the Component hash, then transpiles with JCO and executes its exports.
  Calls cover scalar values, exact 64-bit integers, and the mixed Go consumer's
  scalar and rich calls into its C++ dependency. The Go check explicitly clears
  the shim's environment, arguments and preopens before testing runtime access.
  A deliberate compiler error must leave no published output.
- `values` runs `values.py` against the three `examples/build-values` projects.
  The shared `examples/build-values/go/verify.mjs` checks all three transpiled
  Components. It covers Unicode, byte buffers, exact 64-bit record fields,
  lists, optional enums, nested lists of options, records and typed results.
- `serde` compiles Rust `std`, vendored Serde 1.0.228 and serde_json 1.0.150,
  source-built `Serialize` and `Deserialize` derives, and Wasm dependency build
  scripts. Its authored WIT exports `roundtrip-score`; the implementation uses
  the upstream-generated `Guest` trait. This case retains package admission,
  compiler and macro traces, a native Rust query guard, and the invalid-union
  diagnostic check. A failed build must leave no application output.

The Serde fixture and its Cargo.lock live under
`tests/source-build/fixtures/serde`. Setup authenticates its crate downloads
with `cargo vendor --locked`. The verifier checks vendored files and crate
checksums, compiler receipts, successful macro interpreter traces, Wasm
build-script commands and the admitted Component result. It preserves the
fixture's source bytes and stages generated bindings separately.

Registry publication, package installation and cross-language library consumers
are covered by `tests/library-chain`. Source compiler acceptance concentrates on
WIT bindings, Wasm-hosted compilation and runtime behavior.

`setup-inputs.json` records the acquisition manifest digest, all five bundle
digests, the installed prefix and the CLI hash. `inputs.json` records compiler
and execution-host hashes plus the tested source files. `compiler-receipts.json`
collects each actual build receipt and rejects a compiler hash that differs
from preflight. JCO output and runtime results stay beside the source-build
evidence. Logs, receipts and exact application Components are saved
in the `source-build-linux-amd64-evidence` artifact, including failed runs.

A passing full run has `status: "passed"`, `complete: true` and
`cleanInstallVerified: true` in `out/source-build-ci/result.json`. Selecting
individual cases for diagnosis never reports complete acceptance.

## Run locally

Use a fresh prefix and output directory. The Linux bootstrap requires a
disposable Debian 13 container with root access. After installing the pinned
native prerequisites and building the CLI and its matching worker:

```sh
CARGO_BUILD_JOBS=1 cargo build --locked -p wisp -p wisp-worker
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
its authenticated `vendor` directory. Use `--case sources`, `--case values` or
`--case serde` to run one group.

The smaller runners can also run independently in a configured environment:

```sh
python3 tests/source-build/run.py --cli target/debug/wisp \
  --out out/source-apis --runtime-cache "$HOME/.cache/wisp/source-build"
python3 tests/source-build/values.py --cli target/debug/wisp \
  --out out/source-values --cache "$HOME/.cache/wisp/source-build"
```

Both need `WISP_NODE`, the installed JCO entrypoint in `WISP_JCO`, and the pinned
`WISP_WIT_BINDGEN`. JavaScript output resolves the existing JCO installation's
runtime shims; the runners do not install npm packages. Use disk-backed output
and cache directories and check host capacity before compiler acceptance.

`languages.py` covers the additional packaged scalar adapters using source
signature inference or adjacent WIT contracts. Python exercises the existing
fixed `wisp:python/api@0.1.0` scalar profile. Each resulting Component is invoked
with Wasmtime to verify `add(20, 22) == 42`.
