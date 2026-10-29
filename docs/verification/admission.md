# Admission and generator authentication

Admission is the check `wisp library validate` runs before a release is
reported as valid or published. It regenerates everything Wisp generated and
compares it byte for byte with the submitted release. This page records the
exact rules and trust model. For everyday use see
[Component library packages](../component-library-packages.md).

## Component library packages

Release inputs must be regular files. Admission rejects symlinks and special
files, checks the opened file's size, and bounds the read even if the file grows.
`go.mod` and `go.info` each have a 16 KiB limit. The Component has a 64 MiB limit;
release JSON and package archives have a 128 MiB limit. Rust guest generation
also limits its second read of release JSON to 1 MiB.

Admission validates the Component, decoded contract, names, versions, targets, requirements, tools, archive framing, metadata, notices, and hashes. It regenerates Rust source, WIT, package descriptors, and the complete JCO output, then compares them with the submitted archives. Optional Go admission regenerates its API, runtime metadata, and package files. Richer native clients also require reconstruction of their native archive using the declared platform and pinned inputs. For Rust worker packages, admission reconstructs generated source using the
release-carried worker platform manifest. It checks worker version `0.2.0`,
protocol `1`, Wasmtime `50.0.0`, exact paths and file inventory, and each worker's
SHA-256. It also authenticates those worker hashes against an independent trust
source. Structural self-hashes alone are insufficient.

Set `WISP_WORKER_TRUST_MANIFEST` to an independently trusted canonical
`worker/manifest.json`, using the same schema as the package's worker manifest:

```sh
export WISP_WORKER_TRUST_MANIFEST=/trusted/wisp/worker/manifest.json
wisp library validate out/library-release
```

With this configured manifest, admission can validate foreign artifacts without
an installed worker or a matching local build. It does not execute a worker.
Without the variable, local development authenticates against hashes from the
selected local worker bundle. That fallback requires the trusted local bundle
and does not accept arbitrary release-carried worker hashes.

Obtain the trust manifest through an independently authenticated build or
release process and verify its provenance before configuring it. Never copy an
untrusted uploaded manifest into the trust store. There are no canonical
published worker artifacts to pin globally. Matching a trusted SHA-256 proves
that the bundled bytes match the approved artifact; it does not establish that
the executable is safe. Publishers remain responsible for native executable
trust. Recalculating archive hashes does not authorize changed generated source
or replacement worker executables. [Negative controls](../../tests/library-chain/admission.py) mutate code, descriptors, contracts, targets, and archive members.

Generator authentication verifies the Node executable against the platform lock. It authenticates every installed JCO dependency file against cached npm tarballs whose integrity values come from the checked-in package lock, and rejects added files. The npm tarball cache must remain available during packing and validation. Cargo checksums identify registry contents; they do not establish provenance or code safety.

The [target matrix](target-evidence.md) records verified, unverified and unsupported combinations. CI configuration and local execution evidence remain separate. Acceptance reports bind commands, lockfiles, package hashes, guest dependency graphs, repeated composition hashes, and a frozen packaging executable with source and asset snapshots. Replay removes a stale success report before doing any work.

## Go modules

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
