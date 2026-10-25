# Go importing C++ through WIT

[wit/world.wit](wit/world.wit) declares the C++ arithmetic provider and Go
calculator consumer. Both use upstream-generated APIs. Wisp compiles both
implementations inside Wasm and composes their Components.

The consumer's `RichCheck` calls C++ with a string list and optional enum,
including an absent option. `DoubledSum` also calls the scalar provider.

```sh
wisp build examples/build-mixed --component-only --out out/mixed
```

The final Component exports `calculator.doubled-sum` and
`calculator.rich-check`. The latter returns 42 after checking its imports.
Compiler setup is described in [project builds](../../docs/project-builds.md).
