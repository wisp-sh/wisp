# Python source acceptance

Python uses the same Wisp source-build and installed compiler entrypoints as Go,
C++ and Rust. Its compiler is guest CPython 3.14. Native componentize-py and the
bundled bootstrap tools create reusable WIT-only runtime templates; they never
parse or execute application source. The application receipt distinguishes that
bootstrap from Wasm source packaging, guest compilation/initialization, export
projection and validation.

The comparison below follows `build.rs`, the language inference adapters,
`tests/source-build/run.py`, `tests/source-build/values.py` and
`tests/library-chain/run.py`. A listed acceptance case is a required check,
not a claim that an unexecuted workflow passed.

| Workflow | Go / C++ / Rust baseline | Python requirement and evidence |
| --- | --- | --- |
| Project inputs | Go packages and modules, C++ source/header sets, Cargo projects | One API entrypoint with imported modules, regular/namespace packages, reexports and flat or `src/` layout. `tests/python-source/run.py` builds both layouts. |
| Public API discovery | Source declarations produce typed WIT; authored WIT remains available | Guest AST resolves public annotated functions, aliases, imported dataclasses and enums. Explicit `__all__` selects exports. Buildtools guest inference tests cover diagnostics without executing source. |
| Types and bindings | Scalar and rich-value source fixtures plus generated consumer bindings | `int` is `s64`, `float` is `f64`; strings, bytes, lists, options, records, enums and result returns. Installed Node checks use large-integer-safe values and generated bindings. Authored WIT adds explicit widths, tuples, flags and variants. |
| Offline dependencies | Authenticated Go modules/proxy data and Cargo vendor inputs; C++ project inputs | Local wheels/vendor inventories must match SHA-256 locks, versions, target Python constraints, extras and WASI markers. No pip or build backend runs. Acceptance disables networking for installation, source builds and consumers. |
| Application compilation | Wasm compiler execution with recorded compiler identities | WASI packager, guest CPython compilation, WASI export projection and validation. Receipts must select the installed compiler and declare that bootstrap did not read application source. |
| Calls and errors | Generated installed libraries exercise values, errors and state | The installed Node package performs 64 calls on one instance, including state, records, enums, bytes, options and typed errors, then checks an ordinary exception. |
| Invalidation | Source and dependency inventories participate in build receipts | Module/resource edits change hashes and behavior; body edits reuse templates; API edits provision a new template. Output/cache paths are excluded explicitly. Python stages its discovered inventory without a second unrestricted tree scan. |
| Compiler packaging | Admitted, checksum-pinned `.wisp.zip` bundles installed into a fresh prefix | Download the Python and shared wasm-tools bundles from an executed buildtools run at the immutable source pin in `tests/python-source/toolchain.lock.json`. Verify installed inventories before and after use. |
| Installed consumers | Generated Rust/npm packages installed and executed by library-chain acceptance | Build Python through the installed compiler, install its generated `node.tgz` offline and execute the rich consumer. This is separate from PyPI publication. |
| CI | Source compiler acceptance is dispatch-only; push checks alone are insufficient | `python-source-acceptance.yml` verifies compiler execution, artifact provenance and the source pin before downloading. A skipped compiler/test/package step fails acquisition. Bounded evidence uploads run even on failure. |
| Examples and restrictions | Source examples and target support documentation identify supported workflows | `examples/build-python` contains inferred and authored APIs, multiple modules, public types and package resources. The restrictions below remain part of the support contract. |

## Run the acceptance workflow

Use a successful `Checks and compiler builds` run from `wisp-sh/wisp-buildtools`
whose head exactly matches the checked-in Python toolchain pin. Dispatch
`Python source and installed compiler acceptance` on the Wisp branch with that
run ID as `toolchain_run_id`. The workflow checks the actual compiler test and
packaging steps, installs the artifacts into a fresh prefix, and runs the changed
Wisp CLI. It does not publish a release or registry package.

The evidence artifact records the Wisp CLI hash, immutable compiler source pin,
GitHub run/job/artifact provenance, installed inventories, each build receipt,
consumer result and bounded command logs. Both `passed` and `complete` in
`out/python-source/result.json` must be true. A successful evidence-contract job
on push or PR does not constitute compiler acceptance.

## Boundaries

Python requires Linux x86_64 for automatic native template bootstrap. Templates
can be reused without rebuilding them, but a new public contract or stdlib import
set may require a new template. Normal native execution hosts are distinct from
application compilers.

Pure-Python dependencies can still require OS facilities unavailable under WASI.
Native extensions, bytecode-only dependencies, async functions, WIT resources,
futures/streams and imported application interfaces remain unsupported. Python
source does not yet consume Wisp library dependencies through generated imports.
Dynamic APIs, recursive public types and missing annotations fail with diagnostics.
See [project builds](project-builds.md) and the paired buildtools Python README
for the exact source/data limits, lock format and supported annotation subset.

Outputs default to `dist/`; `out/` is also excluded at the dependency root.
Wisp refuses to overwrite an existing output directory, so use a fresh destination
for each build. `--cache` may point inside the project and is excluded from Python
input discovery. Other Wasm files remain package data. Standalone buildtools
`compile.py` replaces its selected output atomically and excludes that exact file;
standalone `infer.py` accepts repeatable `--exclude-path` arguments.
