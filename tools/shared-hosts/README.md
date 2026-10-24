# Shared compiler execution host

Compiler packages contain Wasm compilers and language inputs. Wisp owns their
native execution hosts, independently of those packages.

```sh
python3 tools/shared-hosts/build.py --jobs 2
python3 tools/shared-hosts/build.py --check
export WISP_WASMTIME="$PWD/.tools/shared-hosts/wisp-wasmtime-50.0.0/wasmtime"
wisp toolchain hosts
```

This builds the checksum-pinned Wasmtime 50.0.0 published CLI crate with Rust
1.99.0. Rustup must have that toolchain installed. Cargo fetches dependencies
from its original lockfile. Building needs native Rust, a C toolchain, `patch`,
Python 3.11 or newer, and access to the crates registry or a populated cache.
The small feature selection retains command execution, WASIp1, WASIp2, WASIp3,
Cranelift, Wasm text, threads and compilation caching. LLVM remains in Wasm.

The patch enables `--preload NAME=FILE` for imported core modules in a Component.
Upstream's CLI already supports core preloads, but rejects them for Components.
Wisp passes those modules to the Component linker's `module` API. The guest's
dynamic linker still instantiates the shared objects and resolves their symbols.
The host neither merges their code into the compiler nor implements LLVM calls.

The build checks a Component that imports a separate core module and calls its
export to obtain 42. Omitting the module must fail. A successful build writes
`receipt.json` beside the host, with source, patch, Cargo lockfile and binary
hashes and the smoke result. This directory is Wisp's cache, outside compiler
packages.

Rust uses the same Wasmtime 50.0.0 Component host. Its compiler is a
`wasm32-wasip3` Component with LLVM linked into it. `WISP_RUST_WASMTIME`
can select a separate installation of that same host version.

AssemblyScript uses the shared Wasmtime 50.0.1 Python binding selected with
`WISP_WASMTIME_PYTHON`.
