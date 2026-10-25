# Build ordinary C++ values

[main.cpp](main.cpp) defines ordinary functions and records. Wisp infers the
contract and generates upstream C++ bindings in the staged build directory.
The existing `echo`, `buffer`, `packet`, `checked`, `relay`, `numeric32`,
`numeric64` and `wide` functions retain their source names.

The same file exposes `std::string`, vectors of strings, optional enums,
vectors of optional enums, nested records and `std::expected`. `Mode` uses
explicit numeric values to demonstrate that source enum values are independent
of WIT case indices.

```sh
wisp build examples/build-values/cpp --component-only --out out/cpp-values
```

Configure the Wasm compiler, WASI SDK sysroot and upstream generator through
[project builds](../../../docs/project-builds.md). `wisp_types.hpp` is a generated
source compatibility helper for owned string/byte buffers and results. Wisp
converts those values to upstream types; the helper defines no Component ABI.
