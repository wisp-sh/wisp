# Test suites

The executable acceptance suites behind each user guide. Run targeted suites
before broad ones; most need the pinned toolchains from
[toolchain packages](../toolchain-packages.md).

## Source builds

Run the CLI regression tests and upstream guest package checks:

```sh
cargo test --locked -p wisp --test build --test library_go_upstream
```

The source runners build component-only C/C++/Go/Rust implementations and a
mixed Go-to-C++ graph, then execute the Components through JCO. They do not
install npm packages or rebuild native library hosts.

```sh
python3 tests/source-build/run.py --cli target/debug/wisp \
  --out out/source-apis --runtime-cache .wisp-cache
python3 tests/source-build/values.py --cli target/debug/wisp --out out/source-values
```

The shared rich verifier checks general string lists, enum options, nested lists
of options, records, typed results and repeated ownership transfers. The Serde
runner separately checks procedural macros inside the Wasm-hosted Rust compiler.
[Source compiler CI](source-build-ci.md) describes provisioning and receipts.
Existing library-chain suites cover registry publication and native consumers.

## Go modules

Run the executable checks after provisioning the pinned toolchains and Cargo
sources:

```sh
cargo build --locked -p wisp -p wisp-worker
cargo test --locked -p wisp --test library_go_guest --test library_go --test library_go_resources \
  --test library_go_network --test library_go_async \
  --test library_go_error_context
```

`library_go_guest` checks the pinned Wago module and the absence of worker and
native-archive directories. It executes installed native modules with cgo
disabled, builds the same API for Wasm, tests scalar state, concurrency and traps,
and runs retained C++, Rust and Go compiler output. The other fixtures exercise
nested values, typed errors, shared state, resources,
async readers, error contexts, and networking through installed modules. The
ordinary-value test also checks source substitution, a forged native report,
and archive path controls. Evidence is written under
`evidence/library-go-*/` for the exercised native platform.

## Publication

After creating a schema-3 release with the pinned generator, run:

```bash
python3 tests/library-publication/run.py --cli /absolute/path/to/wisp \
  --release out/library-release --evidence evidence/publication-new-run
```

The fixture starts empty loopback registries and publishes through their standard
Cargo/npm APIs and OCI upload endpoints. It injects an npm failure after OCI and
Cargo succeed, verifies that retry preserves those successes, and checks a
completed retry makes no writes. Negative controls alter remote tarball bytes,
registry dependency metadata, and local release bytes, and send an OCI upload
location to another origin.

Fresh Cargo, npm, and optional Go consumers live in a temporary directory outside the checkout.
They install from the published registries, then reuse their caches after those
registries shut down. Node imports the installed public package. Cargo verifies
package installation and offline fetching here; native execution and the typed
success/error comparison belong to the separate library-chain acceptance.
No public registry is contacted for publication. Initial Cargo dependency
resolution may fetch the generated package's pinned runtime dependencies from
crates.io when they are absent from the local cache.

The all-channel run at `evidence/publication-all-v2/result.json` passed package
installation, offline cache reuse, partial-success retry, immutable Go version
checks, OCI fetch, and rejection of a forged OCI blob. These are local registry
results, not evidence of publication to public accounts.

## Component dependencies

After provisioning the Component proof tools:

```sh
cargo test --locked --test dependencies -- --test-threads=1
```

These tests build and execute C → B → A, share A between consumers, remove the
registry before cache-only composition, and compare successful output bytes
across cache reinstallation. They exercise targeted updates to another A release,
same-version substitution, incompatible WIT signatures, requirement conflicts,
cycles, missing packages, cache tampering, two explicit interface versions, and
outer host imports. They do not establish support for a public Component
registry, authenticated publisher ownership, or runtime safety of arbitrary
Components. Hashes identify bytes and detect changes.
