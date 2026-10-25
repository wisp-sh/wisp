# Ordinary Go rich values

[main.go](main.go) defines the API using ordinary Go functions and types. Wisp
infers its WIT contract, generates upstream wit-bindgen glue, and adapts the
generated values to the source types. The Go compiler and linker remain Wasm
programs. The project needs no authored WIT or generated-package imports.

The example includes strings, bytes, structs, `(Packet, error)` results, string
slices, enum slices, optional enum pointers, and slices of optional enums.
`nil` maps to `None`; a non-nil pointer maps to `Some` of its value. Pointers
never appear in the Component API. `ModeFast` and `ModeCareful` become the enum
cases `fast` and `careful`. Their integer values remain a Go implementation
detail. Recursive records and ambiguous enum declarations are rejected.

With the compiler tools configured, run from the repository root:

```sh
wisp build examples/build-values/go --out out/go-values
```

The authored-WIT version is retained separately in
[build-go-values-authored](../../build-go-values-authored/README.md).
