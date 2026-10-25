# Contributing to Wisp

Start with the [README](README.md) and [target support](docs/target-support.md).
Include a small reproducer when reporting a bug, along with the Wisp version,
host platform, command and expected result. Remove credentials and private paths
from reports and attachments.

## Development setup

The workspace contains the CLI, execution worker and worker protocol. Install
both executables when testing installed consumers:

```sh
cargo install --path crates/wisp --locked
cargo install --path crates/wisp-worker --locked
```

Source compiler acceptance also needs the
[compiler bundles](docs/toolchain-packages.md) and
[shared hosts](tools/shared-hosts/README.md). The test harnesses require Python
3.11 or later. Compiler and browser setup downloads large dependencies.

Source builds and guest-package generator tests use the pinned upstream CLI:

```sh
python3 tools/upstream-bindings/setup.py --out .tools/upstream-bindings
export WISP_WIT_BINDGEN="$PWD/.tools/upstream-bindings/wit-bindgen"
```

Application compilers still run in Wasm; this CLI only generates binding source.

## Bounded checks

Run from the repository root. These checks do not provision compiler bundles or
run the complete installed-consumer suite:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --locked -p wisp --test cli
python -m unittest discover -s tests/source-build -p 'test_*.py'
git diff --check
```

The CLI tests do not require a provisioned toolchain. Additional test suites
may require the installed executables, compiler bundles, a native compiler or a
browser. Keep the matching worker beside the CLI or set `WISP_WORKER`.

## Full acceptance

The [source compiler CI guide](docs/source-build-ci.md) describes acquisition and
execution from clean compiler bundles. That job needs an explicit manifest URL
and digest. The [Component chain harness](tests/component-chain/README.md)
has its own pinned setup and offline verification script. The installed
library suites under `tests/library-*` run after `tests/library-chain/setup.py`;
see [the component library workflow](.github/workflows/component-library-chain.yml).

Generated logs and reports belong in the ignored `evidence/` and `out/`
directories. Keep regression fixtures small, and commit their source locks,
rebuild scripts and redistribution notices alongside the tests that use them.

## Changes to contracts and fixtures

Add regression coverage when behavior changes. Keep unsupported WIT shapes
explicit and update the public support matrix when a supported profile changes.

Treat fixture binaries and archives as locked inputs. Rebuild them with the
documented recipe, review changed hashes and preserve third-party notices.
Use compiler path remapping when producing fixtures so build-machine paths do
not become part of distributed bytes. Do not remove debug-symbol support from
the compiler or package format to hide paths in one fixture.
