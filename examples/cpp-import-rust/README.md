# Import a Rust package from ordinary C++

[main.cpp](cpp/main.cpp) includes the dependency's generated C++ header:

```cpp
#include <arithmetic.hpp>

unsigned doubled_sum(unsigned a, unsigned b) {
    return arithmetic::add(a, b) * 2;
}
```

The implementation is [ordinary Rust](../build-rust/rust/src/lib.rs), published
as `demo:rust-math@0.1.0`. The [project config](wisp.toml) selects the admitted
release in `deps/arithmetic` using dependency alias `arithmetic`. Wisp generates
`arithmetic.hpp`, private canonical ABI bindings and Component composition.

With the toolchains configured:

```sh
source .tools/build/env.sh
examples/cpp-import-rust/run.sh out/my-cpp-import-rust
# 42
```

Only the C++ toolchain compiles this application. The dependency contains the already
compiled Rust Component, so a Rust compiler and Rust source are unnecessary.
The example also forwards a boolean and a 64-bit integer through Rust.

[compile_flags.txt](compile_flags.txt) selects the Wasm target and the generated
header directory for clangd. The checked-in
[arithmetic.hpp](bindings/cpp/arithmetic.hpp) gives editors the real API before
a build. Its include path is relative to the example and survives moving the
checkout. Wisp regenerates the actual build header from the installed package;
the inspection copy is not a build input.

The final Component includes both implementations and retains Rust's WASI
requirements. Run it under a Component-capable runtime. To package it for
publication, omit `--component-only` when running `wisp build`.

See [the local registry verification](../rust-import-wasm/README.md#verify-publication-and-imports)
for publication, `wisp library fetch`, and offline execution of all four Rust
import flows. Application authors supply neither WIT nor ABI annotations.
