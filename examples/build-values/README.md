# Rich source values

These ordinary C++, Go and Rust projects export strings, byte buffers, records
and typed errors. Their source defines the public API. Wisp infers the contract
and generates the bindings without application-authored WIT.

After [installing the compilers](../../docs/toolchain-packages.md) and
[configuring the packaging tools](../../docs/project-builds.md#configure-the-compilers),
build any project from the repository root:

```sh
wisp build examples/build-values/cpp --out out/cpp-values
wisp build examples/build-values/go --out out/go-values
wisp build examples/build-values/rust --out out/rust-values
```

Each project provides string and buffer round trips, a record with owned data
and a 64-bit field, success and error returns, and a nine-string record that
requires indirect canonical parameters. Rust and C++ also accept typed results
as arguments and exercise mixed floating-point and integer result payloads.

The executable verifier builds all three and runs their installed npm packages:

```sh
python3 tests/source-build/values.py --cli target/debug/wisp --out out/values-check
```

The [full acceptance runner](../../docs/source-build-ci.md) also installs and
executes their native Rust packages after registry shutdown. It verifies C++
and Rust importing the Go package, nested results, unit result payloads and
repeated buffer ownership transfers. Current inference and ownership limits are
listed in [project builds](../../docs/project-builds.md).
