# OCaml library

```sh
wisp build examples/build-ocaml --out /path/to/new/release
```

Install the `ocaml`, `llvm`, `clang` and `wasm-tools` compiler bundles first.
The last implementation's `library.mli` declares the public API. Wisp infers WIT
from that interface; the OCaml compiler checks `library.ml` against it. Listed
sources compile in dependency order, and adjacent interfaces are added before
implementations. `support.ml` is a local dependency, not a public export.

This example exports records, binary bytes, Unicode text, lists, options,
results, 64-bit integers and stateful calls. `collect` exercises GC and compaction.
`safe_divide` returns a recoverable typed error. `explode` raises an unexpected
exception and traps the Component instance.

See [OCaml source support](../../docs/ocaml-support.md) for type mappings,
restrictions and hosted acceptance.
