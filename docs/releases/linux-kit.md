# Linux CLI release kit

Dispatch **Release Linux CLI kit** from the source commit to release, supplying a
new immutable tag such as `cli-2026.10.26`. The workflow builds matching `wisp`
and `wisp-worker` executables from `Cargo.lock`, then runs
`tools/release/prepare.py` on a disposable Ubuntu 24.04 x86_64 runner.

The script builds Wisp's pinned Wasmtime host once, installs the pinned Preview 1
adapter, downloads checksum-verified Node and Go archives, copies Rust 1.98.1
with its wasm32 standard library, installs locked npm tooling and the pinned
upstream binding generator, and fetches both generated-library Cargo dependency
graphs. The resulting kit contains real toolchain binaries rather than rustup
shims pointing at the builder's home directory. A relocation check verifies the
execution host before publication.

The workflow publishes `wisp-linux-x86_64.tar.gz`, its schema 1 native-kit manifest
`wisp-linux-x86_64.json`, and `SHA256SUMS`. Buildtools consumes the manifest by
its exact SHA-256 and adds compiler bundles in its separate runtime release
workflow. The CLI kit alone is not a source compiler installation.

The runner needs native C tooling, Python 3.12, patch, rustup, and at least 30 GiB
free disk after recovery of disposable runner tooling. Both Rust builds use one
worker. Scratch space belongs on disk under the runner temporary directory.
Do not run this release build on a shared host without first checking its
memory, swap, cgroup limits and disk budget. No release assets are replaced when
a tag already exists.
