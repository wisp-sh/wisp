# Nested values and shared interface types

This fixture extends the installed library test with strings, nested records,
lists, options, tuples, flags, payload variants and typed results. Three exported
interfaces share the types defined by `wisp:catalog/types@0.1.0`.

The native and Node consumers check every field of a nested success value, two
named errors, a scalar return and a string return. The Component consumer imports
the same generated Cargo package and forwards its public methods. Its imported
types map to the package's types through `wit-bindgen`'s `with` option.

The native consumer creates one `Library` and obtains both interface clients
from it. It increments a counter through `catalog` and reads the changed value
through `formatting`. Node and B exercise the same shared state. B's
`shared-state` export performs both increments in one runtime invocation and
returns 2, so creating separate Component instances per interface fails this
check.

After the parent directory's setup:

```sh
python3 tests/library-chain/run.py --general --output-dir out/catalog-chain
python3 tests/library-chain/run.py --general --replay --output-dir out/catalog-chain
```

[`cases.json`](cases.json) defines the outer runtime calls and exact expected
WAVE results. Six calls execute directly against A and through composed B.
B's additional `shared-state` call verifies its two imported interfaces share A.
The runner compares the outputs with the expected results. The ordinary
package-manager installation, offline replay, graph checks and source evidence
are the same as the first library chain.

The composition explicitly exports A's shared `types` interface before B's
checkout interface. WAC 0.10.1 rejects an output that only exports checkout for
this graph, reporting `instance not valid to be used as export`. Keeping the
type-only provider as an output export preserves the shared types and validates.
The resulting Component has both exports and no unresolved imports. Removing
that one export from the generated `chain.wac` reproduces the tool limitation.

For Node, `list<u32>` results are `Uint32Array` values. Flags are objects with
boolean members, payload variants contain `tag` and `val`, and named WIT errors
appear in the thrown Component error's `payload`.
