# OCaml source support

OCaml source builds use the same 4.14.2 bytecode/WASI compiler package as the
CPDF command recipe. Python discovers public `.mli` declarations and generates
OCaml callbacks plus C ABI bindings. The packaged bytecode compiler emits C;
the shared Wasm LLVM backend compiles and links it. Wasm-hosted `wasm-tools`
assembles the Component. Application builds need no native OCaml, C compiler,
Dune, opam, Node or network access. Native bootstrap compilers are needed only
when building the toolchain itself. Native consumers have their own build tools.

The installed acceptance uses the candidate Wisp worker for compiler and
application execution, including modern Wasm exceptions. An ordinary Wasmtime
50 host with exception support also works. A minimal CLI built without the
required Wasmtime GC support cannot enable this exception configuration.

```toml
[package]
name = "example:ocaml-values"
version = "0.1.0"

[build]
compiler = "ocaml"
project = "."
sources = ["support.ml", "library.ml"]
```

The last `.ml` file's adjacent `.mli` defines the public library. Each `val`
must describe a curried function with positional arguments. A sole `unit`
argument becomes a function with no arguments. Only declared values are exported.
Implementation details can use ordinary OCaml modules, closures, pattern matching,
exceptions and standard-library collections. The compiler enforces the interface.

List local or vendored modules in dependency order. Adjacent `.mli` files are
included automatically. The project snapshot includes `.ml` and `.mli` files,
with at most 4,096 files and 32 MiB total. Dependencies must already be local;
the source adapter does not resolve opam packages or execute Dune/PPX. A source
or interface change between discovery and staging rejects the build. Receipts
hash the actual staged sources and the installed compiler manifest.

An authored `build.wit` can select a different package/interface identity. Its
function names and value types must match the public `.mli`; it cannot request
an unsupported conversion. `entry` can name a different top-level module with
the same public signature. WIT imports and importing installed Component
libraries into OCaml are not implemented.

| OCaml public type | WIT |
| --- | --- |
| `int` | `s32`, restricted to the OCaml 31-bit signed range |
| `int32`, `int64` | `s32`, `s64`, full signed ranges |
| `float`, `bool` | `f64`, `bool` |
| `string` | UTF-8 `string`; use `bytes` for arbitrary binary data |
| `bytes` | `list<u8>` |
| Immutable named record | Named WIT record, preserving field order |
| Constant variant constructors | WIT enum |
| `t list`, `t option` | `list<t>`, `option<t>` |
| `(ok, error) result` | `result<ok, error>`; `unit` payloads are allowed |
| Function returning `unit` | No result |

Public aliases are supported when their targets are supported. Recursive,
polymorphic, abstract, mutable-record, higher-order and labelled/optional-argument
APIs are rejected. Payload variants, objects, resources, asynchronous functions,
threads and dynamic C libraries are not public bindings. The direct command
recipe retains its existing C-stub and archive API for applications such as CPDF.
Public signatures are limited to 64 KiB, 64 functions, 64 type declarations,
64 fields per record and 256 enum cases. Each call's generated result buffers
have a 16 MiB allocation budget. Ordinary OCaml heap allocation remains governed
by the runtime and host limits.

The reactor initializes OCaml once per Component instance. Callback arguments
and results remain GC roots while conversions allocate. Generated post-return
functions free returned strings and collections. A returned OCaml `Error` is a
normal WIT result and permits subsequent calls. An uncaught exception traps;
consumers must create a new instance after a trap.

## Parity checklist and acceptance

Go/C++/Rust provide inferred source APIs, project snapshots, installed compiler
selection, generated native packages and consumer execution. OCaml uses their
existing Wisp packaging and admission path for the public subset above.

| Workflow | OCaml coverage |
| --- | --- |
| Project and multiple modules | `project`, ordered `sources`, automatic adjacent interfaces, local vendored sources |
| Public API discovery | Explicit `.mli` declarations, compiler-checked implementations |
| Common public values | Scalars, strings, bytes, records, lists, options, results and constant variants |
| Offline application compilation | Wasm-hosted bytecode compiler, C backend and Component assembly |
| Binding lifetime and errors | Repeated npm/Rust calls, full GC/compaction, typed errors and traps |
| Source invalidation | Source inventory hashes, changed local dependency, malformed implementation and rejected API |
| Compiler installation | Immutable bundle hashes, fresh candidate packaging and installation, original compiler provenance |
| Command compatibility | Binary file I/O plus CPDF/CamlPDF 2.9.2 source rebuild and command tests |
| Remaining gaps | Component imports into OCaml, opam/Dune/PPX integration, richer public variants and other types listed above |

Run the **OCaml installed source acceptance** workflow with an exact
`wisp-sh/wisp-buildtools` commit. It checks the candidate CLI, downloads pinned
unchanged compiler payloads, packages the changed adapters, installs them into a
fresh prefix and executes application builds with networking disabled. It then
installs the generated npm and Cargo archives and exercises 200 calls with GC.
The `OCaml public installed acceptance` commit status starts pending and records
the exact tested Wisp commit and compiler revision. It passes only after the
complete installed acceptance, including invalidation and failure checks. An
absent, pending or failed status does not satisfy the public acceptance gate.
CPDF regression acceptance runs separately in the private `wisp-sh/wisp-pkgs`
repository, using the same exact candidate revisions and `setup.py --flat-install`
to freshly package and install the changed adapter before `verify.py --rebuild`.
Both runs are required. Their uploaded evidence records candidate hashes and
individual checks. A skipped compiler job or a saved
verification JSON is not acceptance evidence. The workflow publishes no releases
or registry artifacts.

The historical `t3/add-ocaml-support` branch used OCaml 5.4 and Js_of_ocaml 6.2
inside QuickJS. Its module compilation and scalar library goal are covered by
the current backend and these bindings. Its different OCaml version and
JavaScript runtime semantics are not claimed to be equivalent. The historical
branch and worktree remain available; their unrelated history is not merged.
