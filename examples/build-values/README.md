# Rich source values

These C++, Go and Rust projects expose ordinary source functions. Wisp infers
their WIT contracts and adapts their native values to upstream wit-bindgen APIs. They cover strings, byte buffers, general lists, optional enums,
nested lists of options, records and synchronous results.

Wisp generates bindings in its staged source tree and compiles every
implementation with a Wasm-hosted compiler. C++ uses WASI SDK libc/libc++.
Go uses the pinned upstream runtime shipped with its toolchain.

After configuring the compilers and `WISP_WIT_BINDGEN`:

```sh
wisp build examples/build-values/cpp --component-only --out out/cpp-values
wisp build examples/build-values/go --component-only --out out/go-values
wisp build examples/build-values/rust --component-only --out out/rust-values
```

The Go [runtime verifier](go/verify.mjs) also works against transpiled C++ and
Rust Components because all three expose the same WIT function shapes.

See [project builds](../../docs/project-builds.md) for compiler setup and API rules.
