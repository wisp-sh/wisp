# Toolchain packages

Compiler bundles use `<tool>.wisp.zip`. They share Wisp's release container,
strict JSON, package identity, SHA-256, and complete archive inventory conventions.
Their explicit package kind is `toolchain`, with schema 4. This kind covers
multi-file compilers, standard libraries, shared SDKs, and Python adapters.

Validate a saved bundle without extracting or executing it:

```sh
wisp toolchain validate dist/clang.wisp.zip
```

The command reports identity, upstream version, measured archive and installed
sizes, entrypoints, and declared host requirements. Execution hosts belong to
Wisp and are shared across compiler installations. Compiler packages do not
include Wasmtime executables or the native Wasmtime Python binding library.
Inspect the configured hosts separately:

```sh
wisp toolchain hosts
```

The report checks `WISP_WASMTIME`, `WISP_RUST_WASMTIME`, and the host Python
`wasmtime` module. Set `WISP_WASMTIME_PYTHON` to its parent module directory when
the binding is outside the Python installation. Each available host includes its
native library or executable path, byte size and SHA-256. Python bindings without
distribution metadata report `versionVerified: false`; their smoke test still
runs a Wasm export. Missing or incompatible hosts appear in the report with an
error, without changing any installed files.

The compiler-package builder
in `wisp-buildtools` creates these bundles. The Component library commands keep
their own formats and bounds. They do not install or publish toolchain bundles.

## Install and select compiler versions

Install saved compiler bundles into a separate toolchain prefix. Each install
checks the whole ZIP, compressed archive, inventory, file hashes and modes before
extracting anything. It never runs a package entrypoint.

```sh
wisp toolchain install downloads/clang.wisp.zip --prefix "$HOME/.local/share/wisp/toolchains" --sha256 FULL_ZIP_SHA256
wisp toolchain install downloads/llvm.wisp.zip --prefix "$HOME/.local/share/wisp/toolchains"
wisp toolchain install downloads/go.wisp.zip --prefix "$HOME/.local/share/wisp/toolchains"
wisp toolchain install downloads/rust.wisp.zip --prefix "$HOME/.local/share/wisp/toolchains"
wisp toolchain install downloads/wasm-tools.wisp.zip --prefix "$HOME/.local/share/wisp/toolchains"
```

`FULL_ZIP_SHA256` is the lowercase SHA-256 from a trusted download manifest.
Replace the placeholder with that digest. The optional `--sha256` checks the
transport file before admission. Without it, admission checks internal integrity
but does not authenticate the package author.

An install selects that package version and writes its exact archive, transport
and manifest digests to `selection.json`. Installed payloads live under
`packages/TOOL/VERSION/payload`. Installing identical bytes again verifies and
reuses the version. A different bundle for the same identity and version fails,
including a change confined to release metadata. Publish a new package version
to change an installed release.

```sh
wisp toolchain list --prefix "$HOME/.local/share/wisp/toolchains"
wisp toolchain select wisp:clang@0.1.0 --prefix "$HOME/.local/share/wisp/toolchains"
```

Version selection checks the stored inventory before changing the selection.
Updates to the selected versions use an atomic rename under a prefix lock. A
failed extraction leaves the previous selection intact. If a process dies while
holding `.install-lock`, remove that empty directory after checking that no
installation is running.

Wisp reads the selected tools directly. Set `WISP_TOOLCHAIN_HOME` for a custom
prefix. The default is `$XDG_DATA_HOME/wisp/toolchains`, or
`$HOME/.local/share/wisp/toolchains` when `XDG_DATA_HOME` is unset. Explicit
project tool paths and existing `WISP_*` overrides take precedence. Installed
Clang, Go, Rust, Zig, AssemblyScript, Nim, JavaScript, TypeScript, Python,
Grain, Bend 2, Goose, Swift, C#, Java and Kotlin resolve their selected compiler packages without a
`wisp-buildtools` source checkout. TypeScript shares the JavaScript package.
Install each compiler's runtime dependency packages too. Zig, Nim and Goose use Clang
and LLVM; JavaScript, Python, C# and Grain use wasm-tools; Swift uses llvm-swift and
wasm-tools. Java and Kotlin use jvm and teavm, whose C backend also needs Clang,
LLVM and wasm-tools. Installing a compiler bundle does not fetch dependencies.
Bend uses the JavaScript and wasm-tools packages and Wasmtime Python Component
bindings. Its [bootstrap instructions](../tools/bend/README.md#bootstrap) build
the pinned checker/compiler Component before application builds.
Zig stages its standard library beside the temporary project so builds leave
the installed package unchanged, including when a guest compiler fails.

The language adapters use `WISP_LANGUAGE_TOOLCHAIN`, with the language name in
uppercase. `WISP_JAVASCRIPT_TOOLCHAIN` and `WISP_TYPESCRIPT_TOOLCHAIN` select the
same package root. Shared adapter imports use `WISP_LLVM_TOOLCHAIN`,
`WISP_JVM_TOOLCHAIN`, `WISP_TEAVM_TOOLCHAIN` and `WISP_WASM_TOOLS_TOOLCHAIN`.
These variables point to package directories. `WISP_LLVM` and `WISP_LLVM_SWIFT`
point to their shared runtime directories, and `WISP_WASM_TOOLS` points to the
Core Wasm tool itself. `wisp toolchain env` exports all selected paths.

Buildtools adapter packages built before this integration must be repackaged
before using independently installed support packages. The updated adapters
honor the explicit shared package roots instead of requiring sibling source
directories. For TeaVM and Kotlin, repackaging checks the original compiler
bootstrap inputs and distribution artifacts, permits changes only to their
known host adapter scripts, and records the original manifest digest and old
and new script digests under `adapterRepackage`. Compiler source or build recipe
changes still require rebuilding the distribution. The original distribution
manifest stays unchanged.

For other tools that consume those environment variables, generate shell exports:

```sh
wisp toolchain env --prefix "$HOME/.local/share/wisp/toolchains" > /tmp/wisp-compiler-env.sh
. /tmp/wisp-compiler-env.sh
```

This command prints shell text. Other toolchain commands print JSON. It checks
selected file hashes and modes before printing paths, and quotes shell values.
The installation stores relative payload paths, so moving the whole prefix and
regenerating its environment works.

## Provision shared execution hosts

Install `wisp-worker` beside `wisp`, or select it with `WISP_WORKER`. For
50.0.0 compiler execution, Wisp prefers this worker, which embeds upstream
Wasmtime 50.0.0 and supplies shared core-module imports through its library API.
Copy both executables when creating a CLI snapshot.

Wisp also embeds checksum-pinned Wasmtime source recipes, source locks and
patches for the fallback CLI host. The provisioning commands below still build
that host. The Wasmtime 50.0.0 command host runs Clang and Go when no worker is
available. The Rust WASIp3 compiler always uses the CLI host because its runner
requires CLI options outside the worker protocol. Rust compiler packages must
declare `wasm32-wasip3` and runtime version `50.0.0` in their verified
`toolchain.json`. Packages pinned to an earlier runtime need a new buildtools
release before Wisp can use them.

Provision the hosts with:

```sh
wisp toolchain setup-hosts --prefix "$HOME/.local/share/wisp/toolchains" --cache "$HOME/.cache/wisp/host-build" --host all --jobs 2
wisp toolchain hosts --prefix "$HOME/.local/share/wisp/toolchains"
wisp toolchain doctor --prefix "$HOME/.local/share/wisp/toolchains"
```

Host setup currently supports Linux x86_64. It requires Python 3.11 or newer,
`patch`, a native C toolchain, rustup with Rust 1.99.0, and registry access or a
populated Cargo cache. It builds the pinned source with its original locked
dependencies and Wisp's embedded patches. This source build can take several
minutes and several GiB of temporary disk. `--host command`
provisions the shared host. Hosts live under `hosts/wasmtime-VERSION`, shared by all
installed compiler versions. A failed build does not install a host.

Every host setup also provisions Wisp's exact Wasmtime 50.0.0 preview1 reactor
adapter. Its [official release](https://github.com/bytecodealliance/wasmtime/releases/tag/v50.0.0)
asset has SHA-256
`73b01f85e42e99c890efbbf619cd765e04e5809da36d976b87fc64abd87f5812`.
It lives under `hosts/preview1-50.0.0/` and takes precedence over the wasm-tools
compiler package's adapter, which may target a different Wasmtime release.
`--host adapter` installs just this support file. The verified download cache is
`CACHE/downloads/SHA256`; valid cached bytes work offline, while changed bytes
fail. A temporary file and directory keep failed downloads out of the installed
path.

Setup verifies binary and recipe receipts and runs capability checks before the
host directory appears at its final path. The 50.0.0 test imports a separate core
module into a Component and returns 42. It also rejects a missing provider.
Reusing an installed host reruns these checks. The unmodified upstream Wasmtime
CLI cannot replace this patched CLI host. The Wisp worker uses the upstream
library API for module imports.

`wisp toolchain doctor --prefix PREFIX` verifies
all selected package inventories, including file contents, permissions, added
files and symlinks. They also verify the installed host receipt and run
those capability tests. The separate `adapter` report checks the pinned adapter
file and receipt. Missing or broken hosts appear with `available: false`
and an error in the JSON report. The top-level `valid` field covers compiler
package integrity; check `hosts.available` and `adapter.available` when requiring
the complete compiler runtime.

AssemblyScript, Bend and Swift use external Wasmtime Python bindings. Set
`WISP_WASMTIME_PYTHON` when their module directory is outside the Python
installation. Toolchain installation and host setup do not provision those
bindings.

## Container and manifest

The outer ZIP contains exactly two regular ZIP_STORED entries:

```text
release.json
toolchain.tar.gz
```

ZIP entries appear in the order shown and have no flags, encryption, data
descriptors, extra fields, comments, or ZIP64 records. Local and central headers
agree. Records cover the complete file,
with no prepended data, trailing data, gaps, or overlapping entries.

`release.json` has exactly these fields:

```json
{
  "schema": 4,
  "kind": "toolchain",
  "identity": "wisp:clang",
  "version": "0.1.0",
  "tool": "clang",
  "upstreamVersion": "llvm-24-d297af3",
  "platform": "wasm32-wasi",
  "archive": {
    "file": "toolchain.tar.gz",
    "size": 123,
    "sha256": "64 lowercase hexadecimal characters",
    "inventory": {
      "clang/compiler.wasm": {
        "size": 123,
        "sha256": "64 lowercase hexadecimal characters",
        "mode": 420,
        "format": "wasm-component",
        "role": "compiler"
      }
    }
  },
  "entrypoints": [
    {"name": "clang", "path": "clang/compiler.wasm", "format": "wasm-component"}
  ],
  "requirements": [
    {
      "name": "wasmtime",
      "kind": "runtime-host",
      "version": "50.0.0",
      "included": false,
      "note": "Wisp provides the shared execution host"
    }
  ],
  "sizes": {
    "installedBytes": 123,
    "wasmBytes": 123,
    "nativeBytes": 0,
    "supportBytes": 0
  },
  "bootstrapRequirements": ["Native LLVM bootstrap compiler"]
}
```

This example illustrates the shape. Its placeholder digest and sizes do not
describe a valid package.

Identity uses the same `namespace:name` parser as Component library packages.
The namespace must be `wisp`, and the name must equal `tool`. Package `version`
is canonical SemVer and independent of the upstream compiler's version string.
`upstreamVersion` preserves that upstream string. The initial package version is
`0.1.0`. Compiler payloads use platform `wasm32-wasi` and must contain zero
native binary bytes. Their host requirements describe the separate Wisp runtime.
Admission also supports `linux-x86_64` packages that explicitly include native
payloads.

Every inventory path is relative to the repository-shaped payload. The matching
tar name is `wisp-buildtools/<path>`. Inventory entries are complete and have
exactly `size`, `sha256`, `mode`, `format`, and `role`. Modes are decimal 420 or
493, corresponding to octal 0644 and 0755. Paths are canonical printable ASCII
POSIX paths, at most 240 characters including the package root. Absolute paths,
backslashes, empty segments, dot segments, collisions between files and parent
directories, and duplicate members fail validation.

Allowed formats are `wasm-component`, `wasm-core`, `wasm-archive`, `native`,
`host-script`, and `data`. Validation checks Wasm, ar, ELF, and MZ signatures against
the declared formats. A host script has a `.py` or `.sh` path or a shebang.
Binary signatures cannot be hidden as data. The `wasm-archive` format identifies
archives distributed for Wasm compilation; validation checks the ar signature,
not the target of every object inside the archive.

Allowed roles are `compiler`, `stdlib`, `headers`, `runtime`, `adapter`,
`licenses`, `metadata`, `sdk`, and `documentation`. Entrypoint names are unique,
their paths must occur in the inventory, and their formats must agree with those
files. SDK-only packages can have an empty entrypoint list.

Requirements have exactly `name`, `kind`, `version`, `included`, and `note`.
Names are unique. Kinds are `runtime-host`, `orchestrator`, `shared-tool`, and
`system-library`. Version can be empty when no version is declared. `included`
is a JSON boolean. Requirements describe what a consumer needs after download;
`bootstrapRequirements` separately records requirements for rebuilding the
compiler package.

## Size accounting and admission

`archive.size` is the exact compressed tar length. The ZIP transport length also
includes its manifest and ZIP headers. Installed sizes sum regular-file content
bytes, without filesystem block allocation or unpacker metadata:

- `installedBytes` sums every inventoried file.
- `wasmBytes` sums `wasm-component`, `wasm-core`, and `wasm-archive` files.
- `nativeBytes` sums `native` files.
- `supportBytes` sums the remaining files.

Admission recomputes the archive hash, every file hash, modes, and these totals.
The gzip stream must be one complete member with a valid CRC and no following
bytes. The tar uses USTAR regular files, zero user/group IDs and timestamps,
empty user/group names, zero payload padding, and complete zero
termination. Links, directories, PAX records, sparse and special files, duplicate
members, and nonzero bytes after tar termination fail validation.

| Input | Limit |
| --- | --- |
| Complete ZIP | 2 GiB |
| Manifest | 16 MiB |
| Expanded tar including headers and padding | 4 GiB |
| Individual file | 2 GiB |
| Regular files | 50,000 |
| Entrypoints, requirements, or bootstrap requirements | 64 each |

Payload admission streams compressed data and each file through bounded buffers.
Only the bounded manifest and its inventory remain in memory. It does not unpack
the bundle, run entrypoints, prove compiler behavior, or authenticate the author.
Roles, requirements, upstream versions, and bootstrap requirements remain author
metadata; admission verifies their shape and binds them to the saved release.
