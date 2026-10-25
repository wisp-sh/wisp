# Python library with a typed Component API

This example re-exports functions from the local `arithmetic_helpers` package.
Its `project = "."` setting bundles the package alongside the entrypoint.
It exports ordinary Python functions for Unicode strings, integer
lists, records and typed errors. Its authored WIT contract defines the API.

Provision the base Python toolchain and this contract's runtime template from
the buildtools checkout:

```sh
python3 python/bootstrap.py
python3 python/bootstrap_contract.py --wit /path/to/wisp/examples/build-python/library.wit --world library --rebuild
export WISP_PYTHON_TOOLCHAIN="$PWD/python"
```

Then build from the Wisp checkout with its compiler execution hosts configured:

```sh
wisp build examples/build-python --component-only --out out/python-library
wasmtime run --invoke 'greet("世界")' out/python-library/component.wasm
wasmtime run --invoke 'total([20,22])' out/python-library/component.wasm
wasmtime run --invoke 'checked(-1)' out/python-library/component.wasm
```

The results are `"Hello, 世界!"`, `42` and `err("value must be nonnegative")`.
The `birthday` function accepts and returns the generated `Person` record.

A template belongs to a public contract. Changing function bodies requires only
`wisp build`; changing the WIT API requires provisioning a matching template.
Application source compiles and initializes inside guest CPython. Native
componentize-py is used only to provision the reusable runtime template.

Changes to any module under the source root are included in the build receipt.
Existing templates need a one-time rebuild to enable the package import loader.
