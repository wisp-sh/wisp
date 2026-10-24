# Locked Component dependencies

`wisp deps` resolves Components from a local directory registry and composes a
verified cache into a new executable Component. Original files remain unchanged.
The commands use the guest tools selected by the
[Component architecture decision](component-library-architecture.md), including
WAC 0.10.1. Provision them with the documented proof setup first. `WISP_WAC` can
select an absolute binary path; Wisp verifies the pinned binary hash before
running it.

Cargo.lock and
package-lock.json continue to select language packages and their build
dependencies. The Component lock selects executable Component bytes and the
connections between their interfaces. It does not replace either language lock.

## Package identity and registry layout

A logical identity has the form `namespace:name`. The shared Rust identity module
maps `shop:prices` to Cargo package `wisp-shop--prices` and npm package
`@shop/prices`. Go naming is not defined by this format.

A Component package version identifies one immutable Component hash and one
immutable dependency manifest. WIT interface identities include their own exact
names and versions. Package release versions and WIT interface versions are
distinct. A compatible package patch may retain the same WIT interface identity;
WAC must still validate its actual signatures against every consumer.

The local registry contains these files:

```text
registry/shop/prices/1.0.0/component.json
registry/shop/prices/1.0.0/component.wasm
registry/shop/prices/1.1.0/component.json
registry/shop/prices/1.1.0/component.wasm
```

Each version directory contains a descriptor like this:

```json
{
  "schema": 1,
  "kind": "component-dependency-package",
  "identity": "shop:prices",
  "version": "1.0.0",
  "component": {
    "path": "component.wasm",
    "sha256": "REPLACE_WITH_THE_COMPONENT_SHA256"
  },
  "dependencies": {},
  "hostImports": []
}
```

The root application uses the same descriptor, with its own identity, Component
path and dependencies. For example, an application importing a prices interface
declares:

```json
"dependencies": {
  "shop:prices/pricing@1.0.0": {
    "package": "shop:prices",
    "version": "^1.0"
  }
}
```

Keys are exact runtime import names decoded from the Component. Every runtime
import must appear in `dependencies` or in `hostImports`, once. Extra declarations
and missing imports fail. `hostImports` explicitly identifies imports that stay
at the outer runtime boundary; declaring them does not establish that a chosen
runtime implements them. Composition reports the final host imports before
execution. WIT type aliases are not runtime imports.

## Resolve, reuse and update

```sh
wisp deps lock application/component.json \
  --registry registry --cache component-cache --out component.lock.json
wisp deps compose component.lock.json \
  --cache component-cache --out composed-application
```

The resolver selects the highest matching releases in deterministic order and
backtracks when transitive version requirements conflict. Shared imports of the
same package and exact interface use one selected Component instance. Cycles,
missing artifacts and incompatible requirements fail before a lock is written.
Rust validates and decodes every Component, including cache hits, through
`wit_parser::Resolve`. It derives import/export inventories and a canonical WIT
hash from that model. An authenticated WAC process then checks the complete
interface connections. A semver match alone never admits a composition.

An existing lock keeps its selected bytes. Running `deps lock` again does not
upgrade packages or rewrite the lock. The root descriptor must still have the
same canonical content. Wisp compares registry versions that remain available
against the lock and rejects changed bytes or dependency metadata under the same
identity and version. If the registry is unavailable, a complete cache suffices
to verify the existing lock. If the cache is empty, the registry can repopulate
the exact locked entries without choosing newer releases.

To unlock one package identity:

```sh
wisp deps lock application/component.json \
  --registry registry --cache component-cache --out component.lock.json \
  --update shop:prices
```

The update may select a new matching release of `shop:prices`. Other existing
package selections remain fixed; newly introduced dependencies can be selected.
If the new graph cannot satisfy those fixed choices, the command fails and keeps
the old lock. A changed root descriptor requires a new lock path. Updates are
local registry operations; these commands have no remote registry transport.

Two releases of one package may coexist only when consumers name distinct,
fully qualified, versioned WIT interfaces. Two requirements for the same exact
interface cannot silently select different releases. Wisp never rewrites
interface versions or merges resource identities by unqualified type names.
WAC retains the Component Model's instance and resource identity checks.

## Lock, cache and composition records

The lock has `schema: 1` and `kind: "component-dependencies"`. Its `root` names an
`identity@version` node. Every node records its Component hash, canonical
descriptor hash, canonical WIT hash, decoded imports and exports, type-only
interface exports, declared host
imports, and import-to-node edges. Unknown fields and duplicate JSON keys fail.

The cache stores exact bytes under:

```text
component-cache/components/SHA256.wasm
component-cache/manifests/SHA256.json
```

Cache objects are verified before use. Existing objects with altered bytes are
rejected, not repaired silently. The composed result is a fresh artifact under
`composed-application/component.wasm`, accompanied by `component.lock.json`, the
generated `composition.wac`, and `composition.json`. The report records the lock,
input graph, output and contract hashes, the WAC binary hash, and remaining host
imports. When an exported interface uses types supplied by a dependency, the
composer preserves the dependency’s imported type-only interface export before
exporting the root. `preservedTypeExports` records these interface-to-provider
entries; this retains Component type identities across the composition.
Component dependencies are embedded in the result; execution performs
no registry lookup or code installation. Host networking, if an outer runtime
provides it, is separate from dependency loading.

An existing output directory is preserved. Wisp constructs and validates a
composition in a temporary directory before publishing a fresh output directory.
The original library Components remain separate cache artifacts.

Current limits are 64 nodes, 256 dependency edges, 128 versions per registry
package, 5,000 solver states, 16 MiB per Component, 128 MiB of loaded candidate or
graph bytes, and 1 MiB per JSON document. Versions with build metadata are
rejected. Composition tools currently run on Linux x86_64 and macOS ARM. This
does not change native language-package platform support.

## Executable coverage

After provisioning the Component proof tools:

```sh
cargo test --locked --test dependencies -- --test-threads=1
```

These tests build and execute C → B → A, share A between consumers, remove the
registry before cache-only composition, and compare successful output bytes
across cache reinstallation. They exercise targeted updates to another A release,
same-version substitution, incompatible WIT signatures, requirement conflicts,
cycles, missing packages, cache tampering, two explicit interface versions, and
outer host imports. They do not establish support for a public Component
registry, authenticated publisher ownership, or runtime safety of arbitrary
Components. Hashes identify bytes and detect changes.
