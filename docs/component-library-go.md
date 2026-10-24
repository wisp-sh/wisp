# Typed Go Component libraries

Use `--go-module MODULE` to publish a Go module. For synchronous scalar APIs,
the same import and functions work in native applications and applications
compiled to Wasm:

```go
import arithmetic "example.org/arithmetic"

func DoubledSum(a, b uint32) uint32 {
    return arithmetic.Add(a, b) * 2
}
```

Native Go builds embed the Component and execute it in process with Wago.
Generated modules pin upstream main commit `dc13730184e8ba6a1a2b3e3887c53a2b880db88c`
as `v0.1.0-beta.13.0.20261023135323-dc13730184e8` in `go.mod` and record the
same version in `wisp-component.json`. Consumers need no separate runtime
installation, C linker, or Rust compiler. The bindings work with
`CGO_ENABLED=0`. The first call creates one shared instance. Calls are
serialized, and a trap panics and poisons that instance.

Native bindings save scalar stack returns before restoring the guest stack
pointer to avoid a pinned-global load bug in Wago's ARM64 compiler. Packages
retain the original `component.wasm` for identity and Wasm imports; admission
regenerates and checks the native execution copy.

`GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0` selects typed Wasm imports instead. Wisp
connects those imports to the packaged Component during a source build. Both
implementations expose the same ordinary scalar returns. Native runtime code
is excluded from the Wasm build. Runtime dependency downloads happen during
installation, never during a library call.

The portable profile supports synchronous scalar functions and scalar aliases.
Richer Go contracts still use the native client API below; their Wasm bindings
remain unsupported. Import-free scalar Rust packages use a bundled Wisp
worker with Wasmtime 50.0.0. Rust packages with host imports, resources, or async
APIs retain their established native hosts. Rich Go clients retain their native
Wasmtime 50.0.0 shim. JavaScript continues to select its
packaged Node or browser implementation through package exports.

`--go-guest-module` remains a compatibility spelling for the portable scalar
profile. New packages use `--go-module`; publishers and consumers do not need
to choose separate native and guest modules.

## Native clients for richer contracts

The Go generator creates an ordinary Go module with a typed API, generated Rust
bindings, and a native static library. Each WIT function has its own private C
entry point and typed Wasmtime call. JSON carries values across the private C
boundary; it does not select functions at runtime.

A client owns one Component instance and Store. Interface views share that
client, so state survives calls through different interfaces:

```go
library, err := catalog.NewClient()
if err != nil { return err }
defer library.Close()

first, err := library.Catalog().Increment()
if err != nil { return err }
current, err := library.Formatting().Current()
// first and current observe the same instance.
```

Calls on a client are serialized. `Close` releases its Store, accepts repeated
calls, and causes later calls to return `ErrClosed`. A runtime trap poisons the
instance; declared WIT errors leave it usable. A finalizer provides a fallback,
but callers should use `Close` to release resources promptly.

Records have exported fields. Names include their defining interface, such as
`TypesRequest`. Aliases retain that representation. Enums and flags have named
constants; variants have typed constructors and accessors. `Option[T]` retains
the distinction between `None` and nested `Some(None)`. `List[uint8]` carries
numbers rather than base64. `TupleN` fields are `V0`, `V1`, and so on. `Float32`,
`Float64`, and `Char` preserve WIT floating-point values and Unicode scalars.
A WIT `result<T,E>` return becomes `(T, error)`; its declared failure is available
through `errors.As` into `*CallError[E]`, in the `Guest` field.

The schema-3 generator accepts named interfaces with scalar, string, record,
enum, flags, variant, list, tuple, option, and result values. Top-level
freestanding exports are methods on `Client`; the jq Component's `evaluate`
export becomes `client.Evaluate(filter, json)`. The installed resource fixture
exercises constructors, methods, static calls, owned and borrowed handles, and
nested resource values. Each owned resource has `Close()`; an ownership transfer
invalidates the original wrapper.

Direct `future<T>` and `stream<T>` results have typed readers. `Future[T].Receive()`
returns `(T, bool, error)` and `Stream[T].Next()` returns `(T, bool, error)`.
`Next()` requests one item at a time. Both readers have `Close()` for early
cancellation. Nested async handles, async parameters, and async resource methods
remain unsupported.

The native Go host supplies the admitted WASI HTTP and socket imports. The
installed networking fixture exercises HTTP, DNS, TCP, failures, and a returned
response resource. The exact pure-WASI jq profile instead uses a closed host:
no inherited network or filesystem access, and clocks trap. A runtime trap
poisons that client; opening a new client creates a fresh Store. Direct
`error-context` values have owned transfer and `Close()`. Wasmtime 50.0.0
retains native source handles for the Store lifetime, so the generated client
permits 127 cumulative context lifts per Store. These claims apply to the
tested contracts.

## Distribution

Portable modules record `native_wasm`. Their canonical archive contains both
Go implementations, the original Component, WIT, and pinned Go module metadata.
The generated `go.sum` includes the Wago commit's module and manifest checksums.
Native scalar bindings carry no C bridge, headers, or worker executable.
Runtime dependencies download during module installation; library calls
perform no downloads. The WASI reactor host has empty guest arguments and
environment, no filesystem preopens, and denied networking by default.

A richer native client module contains its native library under
`native/GOOS_GOARCH/libwisp_go.a`. Building a consumer needs Go 1.27.1 and a C
linker; consuming it does not need Rust or download a runtime. The package
supports its recorded native platform. The current builders run on macOS ARM
and Linux x86_64; they do not cross-compile a second native library.

Module paths use lowercase ASCII and a domain, for example
`example.org/team/catalog`. Releases use canonical semantic versions. Versions
2 and later require the matching `/vN` module path suffix. The generator emits
ordinary Go proxy `.zip`, `.mod`, and `.info` payloads. ZIP entries use the
`module@vVERSION/` prefix, sorted paths, fixed timestamps, and fixed permissions.
The proxy `.info` timestamp is the Unix epoch because package generation does
not assign a publication time.

The acceptance test installs the ZIP with `GOPROXY=file://...` into fresh module
and build caches from a directory outside the checkout. It removes both the
proxy and generated source directory before running the consumer with
`GOPROXY=off`. Consumer imports contain no `replace` directive.

## Go modules for Wasm consumers

`wisp library pack PACKAGE --out OUTPUT --go-module example.org/arithmetic`
emits both implementations for a supported scalar contract. `wisp build`
accepts the same option. The module carries `bindings_native.go`,
`bindings_wasip1.go`, `component.wasm`, `component.wit`, `wisp-component.json`,
and ordinary module metadata. Older `wasip1_wasm` archives remain readable.

Install it through the standard Go module proxy, pin its version in `go.mod`,
and vendor with `GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0 go mod vendor`. Wisp
discovers the packaged Component from the Go import graph and generates its
contract and composition mapping. An explicit-contract project can also select
it with `component = { module = "..." }`. The Go compiler compiles the vendored Go package,
and Wisp composes its corresponding Component into the application's output.

Guest admission regenerates the complete module and canonical ZIP, including
bindings, Component hash, WIT and notices. Consumer builds verify the vendored
code and descriptor against the Component, and the version against `go.mod` and
`vendor/modules.txt`. Source hashes and original Component bytes remain in the
build receipt. The [Go registry example](../examples/go-import-wasm/README.md)
includes a real published-and-installed package and all import/export bindings.

## Admission and reproducibility

Portable admission regenerates both implementations, the
Component, module dependencies and checksums, metadata, and the canonical ZIP.
It rejects changed or extra files. Go verifies the runtime dependency through
its module checksums. Consumer source builds authenticate the vendored package
and stage only dependencies reachable on the Wasm import graph, so vendored
native runtime sources and libraries never enter the Wasm compiler.

Richer native Go admission regenerates the Go API, private C header, Rust shim, canonical WIT,
embedded Component, Cargo manifest, and complete pinned Cargo lock. It rejects
extra files and altered generated sources. It rebuilds the native static
library with Rust 1.98.1 and Wasmtime 50.0.0, then compares its hash and the full
provenance record. Altering both native bytes and their reported hash does not
bypass this rebuild. The package includes `WISP-GO-LICENSES.json`: source notices and license metadata
from the complete pinned Cargo graph, with registry archives authenticated against
the lock, plus the pinned Rust standard library copyright notices. Wisp collects these while packing, so the Cargo cache must hold every
archive in the pinned shim lock, including target-specific ones; `cargo fetch
--locked` downloads them. Author notices
are authenticated against the metadata passed to generation and admission.
ZIP admission also checks paths, duplicate entries,
symlinks, file counts, compressed and expanded byte limits, and canonical ZIP
bytes.

Native admission requires the archive's platform and the pinned compiler and
cached dependency sources. It uses fixed crate metadata, remapped shim paths,
`SOURCE_DATE_EPOCH=0`, and deterministic archive timestamps. A generated rustc
wrapper also remaps dependency source and generated build paths. Independent
native target caches produced identical archives on macOS ARM; admission still
fails closed if a different build environment produces different bytes. Source
and archive hashes
do not establish ownership of a public module domain.

Native build caches use `WISP_GO_CARGO_TARGET_DIR` when set, otherwise
`XDG_CACHE_HOME/wisp/library-go/native-target` or
`HOME/.cache/wisp/library-go/native-target`. Cargo runs offline with the embedded
complete lock; dependency caches must be populated during setup.

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
