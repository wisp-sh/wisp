# Rust values with an authored contract

This project keeps its contract in [wit/world.wit](wit/world.wit). Its Rust
library implements upstream-generated `Guest` traits and invokes the generated
`export!` macro. Wisp stages those bindings and compiles the library with the
Wasm-hosted Rust compiler.

The [ordinary Rust example](../rust/) infers the same kinds of values from public
Rust functions.
