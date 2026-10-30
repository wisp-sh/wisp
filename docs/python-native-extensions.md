# WASI Python extensions: design investigation

Native extensions remain unsupported by Wisp's Python source adapter. A wheel
containing a host `.so`, `.pyd` or platform-specific tag is rejected. Supporting
NumPy needs a WASI build that matches the embedded interpreter and a way to link
its extension modules into the Component. This note does not implement either.

## Upstream starting points

[Pyodide](https://github.com/pyodide/pyodide/blob/main/README.md) demonstrates
NumPy and other compiled Python packages on WebAssembly. It targets Emscripten
and supplies JavaScript integration. Its wheels cannot be treated as WASI
binaries. Its package recipes are useful references for compiler flags, patches
and numerical-library dependencies.

[wasi-wheels](https://github.com/benbrandt/wasi-wheels) targets wheels consumed by
componentize-py and documents work on pydantic-core and regex. Its published
recipes need an ABI and version audit against Wisp's CPython 3.14 and
componentize-py 0.25.1 pins before reuse.

The [NumPy WASI discussion](https://github.com/numpy/numpy/issues/25859)
describes componentize-py's build-time symbol tables for extension linking.
This points toward linking verified extensions during compiler-package
provisioning rather than loading arbitrary shared objects from the filesystem.
[CPython's WASI build instructions](https://github.com/python/cpython/blob/main/Platforms/WASI/README.md)
cover interpreter cross-compilation prerequisites. Interpreter platform support
alone does not establish compatibility for a particular extension wheel.

## Proposed boundary

Provision an extension-enabled Python compiler package from pinned CPython,
componentize-py, WASI SDK, extension sources, patches and numerical dependencies.
Pin every source archive and emitted extension by version and SHA-256. The
native cross-compilers would bootstrap that package; application source would
still parse, compile and initialize inside guest CPython.

Start with a small extension such as regex. Validate its ABI, import behavior,
exception handling and symbol resolution inside the target Component host before
attempting NumPy. NumPy additionally needs a reproducible numerical backend,
including BLAS/LAPACK where enabled, with an explicit choice for threading and
SIMD. Audit unavailable OS calls and filesystem assumptions. Do not silently
substitute host libraries.

The current runtime snapshots imports at provisioning time. An extension-enabled
package must preload extension modules and their Python dependencies, or provide
a verified guest loader. Runtime cache identities must include extension hashes,
CPython ABI, SDK/linker versions, build flags and preloaded modules as well as the
public contract. Loading a new extension is a toolchain provisioning operation.
It must not execute application initializers on native CPython.

Expose arrays through explicit WIT lists or records first. A NumPy ndarray is not
a canonical ABI value; dimensions, dtype, layout and copying behavior need a
separate public contract. Measure runtime memory and Component size against
Wisp's 64 MiB bound before admitting a package. Acceptance must cover imports,
array arithmetic, typed errors, deterministic builds, denied host capabilities
and consumption from Rust, Node and Go. No extension support status should
change until those checks run with pinned artifacts.
