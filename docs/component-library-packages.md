# Component library packages

`wisp library pack` creates packages that call an authored Component. One Cargo package serves native Rust applications and `wasm32-unknown-unknown` Component guests. Import-free native scalar APIs start a bundled Wisp worker using upstream Wasmtime 50.0.0. Consumers install no separate runtime. APIs with host imports, resources or async values retain their established native Wasmtime bindings. Component guests call imports generated from the same WIT and contain no host runtime. The npm package contains JCO bindings and Core Wasm. An optional Go module selects pinned native Wago or Wasm imports automatically for scalar APIs. Richer Go contracts retain their native client bindings.

The [installed package fixture](../tests/library-chain/README.md) installs through isolated Cargo and npm registries, stops both servers, and executes the native Rust, Node, and composed Component consumers. Its general fixture checks nested records, strings, lists, tuples, options, flags, variants, typed errors, shared types, and several interfaces sharing one instance. The [architecture decision](verification/architecture.md) explains the binding and composition choices.

## Authoring and packaging

An authored directory contains `wisp.toml`, the original Component, and a `wit/` directory with one selected world. The packer checks the Component hash and compares decoded WIT with the authored contract. Identity and version are explicit.

```toml
[package]
name = "wisp:catalog"
version = "0.1.0"

[component]
path = "component.wasm"
sha256 = "<sha256 of the original Component>"

[metadata]
description = "Catalog operations"
license = "MIT"
repository = "https://example.com/catalog"
authors = ["Catalog maintainers"]
notice-files = ["LICENSE"]
```

Build or install both `wisp` and `wisp-worker` before packaging:

```sh
cargo build --locked -p wisp -p wisp-worker
```

Keep the executables together when installing or snapshotting them.
`WISP_WORKER` selects an explicit worker path. `WISP_WORKER_BUNDLE_DIR` can supply
workers for supported platforms using `PLATFORM/wisp-worker` paths. Generated
Rust packages carry their selected platform workers so consumers need no local
Wisp installation.

Set `WISP_NODE` to the pinned Node 24.20.0 executable and `WISP_JCO` to the pinned JCO 1.33.0 entry point. [setup.py](../tests/library-chain/setup.py) provisions both and fetches the locked dependencies used by the acceptance fixtures.

```sh
wisp library pack path/to/catalog --out out/catalog
wisp library validate out/catalog

# Also emit a Go module. Scalar APIs work in native and Wasm builds.
wisp library pack path/to/catalog --out out/catalog-with-go \
  --go-module example.com/catalog
```

Packing requires a populated Cargo cache and a fresh output directory. It packages offline and admits the completed artifacts before reporting success. A failure can leave diagnostic output. The original Component is copied without rewriting its implementation.

`library-release.json` uses `schema: 3` and `kind: "component-library"`. It records the Component contract, identity, version, package hashes, tools, target support and requirements, and author metadata. The release contains `component.wasm`, `rust.crate`, `node.tgz`, and optional `go.zip`, `go.mod`, and `go.info`, plus unpacked directories for inspection. Author notices are preserved in the packages. The Go archive also carries notices from its checksum-verified native dependency graph and the pinned Rust standard library.

Validate a release with `wisp library validate`.

## Public calls and instance ownership

Use one `Library` when calls through different interfaces must share state:

```rust
use wisp_wisp__catalog::Library;

let library = Library::new()?;
let mut catalog = library.catalog();
let mut formatting = library.formatting();
assert_eq!(catalog.increment()?, 1);
assert_eq!(formatting.current()?, 1);
```

Native interface clients cloned from that `Library` share a Store. Calling an interface's `Client::new()` creates a new native instance. Dropping the last library or resource wrapper releases its Store. Native traps poison the shared instance; subsequent calls fail. A declared WIT error returns `CallError::Guest` and leaves the instance usable. Initialization and runtime failures return `CallError::Runtime`. Guest traps propagate to the outer runtime.

```javascript
import { catalog, formatting } from '@wisp/catalog';
const next = catalog.increment();
if (formatting.current() !== next) throw new Error('state mismatch');
```

Node uses JCO's WIT representation, including BigInt for `u64`, typed arrays for numeric lists, and discriminated objects for variants. A WIT error result throws JCO's Component error with its typed payload. All interfaces exported by one imported ESM module share that module's instance.

Resource wrappers retain their originating library. Borrowed and owned arguments are checked against that instance. Explicit close and Rust drop release owned resources. The [resource fixture](../tests/library-resources/) exercises constructors, methods, static operations, own/borrow arguments, typed resource results, nested records and variants, lists, options, aliases across interfaces, and calls after close or trap.

For asynchronous functions, the Rust API uses `async fn`. Direct `future<T>` results expose `receive().await`, and direct `stream<T>` results expose `next().await`. Both readers support close and native Rust drop. `Library::drain().await` waits for background Component tasks after callers have consumed or closed their readers and rejects open readers. Node requires `--experimental-wasm-jspi` on the pinned Node version. The [async fixture](../tests/library-async/README.md) documents the verified record and string payloads, ownership limits, and JCO behavior. Async support is not inferred merely from successful WIT parsing.

The [Go API](component-library-go.md) provides generated ordinary value types, typed guest errors, explicit `Close`, and clients for interfaces sharing one native instance. Installed Go consumers execute the tested resource, future, stream, error-context, and WASI HTTP/DNS/TCP contracts. Nested async handles and async resource methods remain unsupported. Scalar packages use the same import and functions in native and Wasm builds. Richer native clients carry a platform-specific archive whose reconstruction is validated before admission succeeds.

## Installation and Component dependencies

Package names follow one mapping. `namespace:name` becomes Cargo `wisp-namespace--name`, npm `@namespace/name`, and a caller-supplied Go module path. The double hyphen keeps Cargo identities such as `a-b:c` and `a:b-c` distinct. These names do not establish ownership of public registry accounts.

The Cargo package uses a bundled Wasmtime 50.0.0 worker for import-free scalar native APIs. Native APIs with host imports or richer values retain Wasmtime 50.0.0 bindings. Component guests use wit-bindgen 0.61.1. The consumer's Cargo lock records the dependency graph. Scalar Go modules use Wago at upstream commit `dc13730184e8`, with no worker executable. Installation can fetch dependencies; subsequent builds use `--locked --offline`. The npm package contains its JavaScript, declarations, Core Wasm, and any required vendored WASI host code. It has no npm dependencies or install scripts. ESM initialization uses top-level await. Execution does not install code.

A Component consumer maps shared WIT types to the installed Cargo package's import bindings with wit-bindgen's `with` option, builds its core Wasm, and composes it with the original library Component. The [Component dependency commands](component-dependencies.md) resolve versioned packages into a lock and content cache, then compose from that cache without a registry. Type-only provider interfaces are preserved when exported values refer to their types. Remaining imports must be admitted host interfaces.

For applications compiled by `wisp build`, `wisp library rust-guest ADMITTED_RELEASE --out SDK_DIR` generates a separate dependency-free Cargo crate targeting `wasm32-wasip1`. It carries the admitted Component, notices and authenticated import wrappers for synchronous scalars, strings, byte buffers, records and results. Resources, asynchronous values and other list elements are outside this projection. It does not change the native or `wasm32-unknown-unknown` implementations in the ordinary Cargo archive. See [source-built Rust consumers](project-builds.md#rust-compiler).

[Publication preparation](library-publication.md) validates and snapshots the release before sending anything. Publication uploads exact admitted bytes to configured Cargo, npm, and OCI registries and records each confirmed upload independently. Local registry tests cover retries and interrupted publication. They do not establish interoperability with every public registry.

## Admission

`wisp library validate` regenerates every generated package from the Component and
contract and rejects any difference. Rust worker packages are also checked against
an independently trusted worker manifest:

```sh
export WISP_WORKER_TRUST_MANIFEST=/trusted/wisp/worker/manifest.json
wisp library validate out/library-release
```

Admission checks identity and byte integrity; it does not prove that a package is
safe to run. See [admission and generator authentication](verification/admission.md#component-library-packages)
for the full rules and trust model.
