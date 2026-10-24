# Installed Rust, Node and Component consumers

This acceptance test builds library A, packages it with `wisp library pack`, and
installs the generated packages through isolated loopback Cargo and npm
registries. A native Rust program, a Node module and a Rust Component consumer B
call the same contract. All return a total of 21, the named overflow error, and a
zero total in the same cases.

The Cargo and npm installations use package names and exact versions. They do
not use path dependencies or tarball installation shortcuts. The local servers
stop before the first build or execution of a consumer. The test then rebuilds
with Cargo's offline mode and reinstalls Node dependencies from npm's cache.

## Run

Install Rustup and Python 3.12 or later. The setup command installs Rust 1.98.1,
Node 24.20.0 with npm 11.19.0, locked JCO 1.33.0 dependencies, and the
[Component architecture proof](../component-chain/README.md)'s
pinned `wasm-tools`, WAC and Wasmtime binaries. It also fetches the locked native
Wasmtime dependency graph. Node's archive checksums come from the
[official release](https://nodejs.org/en/blog/release/v24.20.0).

```sh
python3 tests/library-chain/setup.py
cargo +1.98.1 build --locked --offline -p wisp
python3 tests/library-chain/run.py
python3 tests/library-chain/run.py --replay
```

The first run uses the locked JCO 1.33.0 installation at
`.tools/library-js/node_modules/.bin/jco`. Pass `--jco` to use another provisioned
path. It may fetch missing Cargo dependencies during installation. The replay
performs no installation from a registry and requires no running server.

`setup.py` populates the native backend's cache from
[`native-seed/Cargo.lock`](native-seed/Cargo.lock). The Component and native
compiler dependency graphs remain separate. Setup does not change the system's
default Rust or Node installation.

Use `--output-dir /absolute/path` for an independent installation. Each fresh
run chooses a loopback port and records that registry URL in the Cargo and npm
locks. Keep this directory and the package-manager caches for replay. A prepared
directory must be replayed or replaced with a new output path; the runner does
not silently overwrite its release.

Pass `--build-dir /absolute/path` to share compiled Cargo dependencies across
separate installation runs. This changes the compilation cache, not the package
source. `--wisp` and `--tools-dir` select already provisioned executables.

On macOS, replay can run with the operating system denying network operations:

```sh
sandbox-exec -p '(version 1)(allow default)(deny network*)' \
  python3 tests/library-chain/run.py --replay
```

## What the consumers import

The native Rust program imports `wisp_wisp__library_chain::arithmetic`. It creates
an arithmetic client and uses its generated `LineItem` and `CalculationError`
types. `CallError::Guest` preserves the named WIT error. `CallError::Runtime`
reports runtime failures separately. Dropping the client drops its host store.

The Node program imports `arithmetic` from `@wisp/library-chain`. JCO's generated
ES module completes asynchronous initialization through top-level `await` before
its exports are used. WIT `u64` values are JavaScript `bigint` values; a failed
result throws a Component error with `payload === 'overflow'`.

B installs the same Cargo package as the native program. On
`wasm32-unknown-unknown`, the package selects guest import bindings. B maps its
WIT's imported types to the package's generated types with `wit-bindgen`'s `with`
option and calls the same public client. Its target-specific Cargo graph must
contain no host runtime. WAC embeds A and B in a new artifact; outer Wasmtime
executes the checkout export.

## Evidence

`out/library-chain/report.json` records all three outputs, registry requests,
artifact hashes, lockfile hashes, source hashes and the command transcript.
`guest-cargo-tree.txt` records the target-specific guest build graph.
`composed.wit` shows the decoded final contract. Composition runs twice and must
produce identical bytes without changing A's original Component.

Preparation snapshots the Wisp executable and records its hash, source revision,
worktree status and generator source hashes before packaging. Replay uses that
same executable, so rebuilding a development checkout does not change the
admission rules midway through an acceptance run. `packaging-sources/` preserves
the corresponding source files present during preparation.

Every replay also runs Rust admission and
[`admission.py`](admission.py)'s eight mutation controls. These alter generated
code, contracts and claimed support while recalculating archive hashes. Admission
must reject them. CI stores the reports, locks and exact archives for 90 days on
Linux x86_64 and macOS ARM. The workflow's presence is not evidence that a CI run
has passed.

The local registry implements the documented
[Cargo sparse index protocol](https://doc.rust-lang.org/cargo/reference/registry-index.html)
and npm's metadata and archive fetches. It is an acceptance fixture. It does not
provide production authentication or publication endpoints.

This test covers the first record/enum/result library chain. It does not claim
that the existing Go and browser package generators support these new contracts.
