# Authored Component packages

An authored package supplies a Component, its WIT contract and optional metadata:

```text
package-directory/
  wit/world.wit
  wit/deps/
  wkg.toml
  wkg.lock
  wisp.toml
  component.wasm
```

The root WIT package declaration supplies `namespace:name@version`. Select one
world. `wisp.toml` selects the Component and asserts the same identity and version:

```toml
[package]
name = "wisp:jq"
version = "0.1.0"

[component]
path = "component.wasm"
sha256 = "3f4f95f882d2f7e574e1a3dcd01f690a8f72e928fe8c7d151892daf6433b173e"
```

The Component path must stay within the package and cannot use symlinks. Its
digest must match the bytes. Wisp validates those bytes and compares the authored
world's supported export shape with the decoded executable before packaging.
This checks ABI compatibility. It does not prove source ownership or that the
Component came from the supplied source.

Inspect the checked-in example without installing compiler bundles:

```sh
wisp inspect fixtures/jq/component.wasm
wisp inspect --ownership fixtures/jq/component.wasm
```

## Metadata and local WIT dependencies

`wkg.toml` accepts upstream metadata fields `authors`, `description`, `licenses`,
`source`, `homepage` and `revision` as strings. `author`, `license` and
`repository` are aliases. For example:

```toml
[metadata]
description = "A JSON query Component"
licenses = "MIT"
source = "https://github.com/jqlang/jq"
```

Metadata describes the author's claims. Wisp carries it in generated packages
without writing it into the Component or inventing a license when one is absent.
Use the [jq fixture notices](../fixtures/jq/notices/) for that fixture's complete
redistribution terms.

Wisp resolves local `wit/deps` packages and local path overrides. Relative
override paths start at the authored package directory, and the loaded package
identity must match the override name. Registry version resolution, binary
overrides and upstream workspace discovery are unsupported. A version-1 empty
`wkg.lock` is accepted. Nonempty registry locks are refused, and Wisp does not
generate or update this lock. Unknown metadata and configuration fields fail.

## Package formats

Use [Component library packages](component-library-packages.md) for schema-3
libraries and their supported target bindings. The
[target matrix](target-support.md) lists the supported WIT profiles.
