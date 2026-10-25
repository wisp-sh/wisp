# C with the WASI standard library

This authored WIT library implements `add` using `malloc` and `free`.
The upstream C generator supplies declarations and metadata. Wisp compiles and
links with its Wasm-hosted Clang and the WASI SDK target sysroot.

```sh
wisp build examples/build-c --component-only --out out/c
wasmtime run --invoke 'add(20,22)' out/c/component.wasm
```
