# Python library with an inferred Component API

`main.py` exposes annotated functions and imports its dataclass and enum from
`models.py`. It reexports `greet` from a local package, which reads a bundled
text resource through `importlib.resources`. Wisp
asks guest CPython to infer the contract, then obtains a matching runtime from
the installed Python toolchain. No authored WIT or manual template command is
needed. Local package imports still work through `project = "."`.

Install a current Python toolchain and configure Wisp's execution hosts. For a
local buildtools checkout, run `python3 python/bootstrap.py` once there and set
`WISP_PYTHON_TOOLCHAIN` to its `python` directory. Automatic provisioning currently
uses the package's Linux x86_64 bootstrap tools. Then:

```sh
wisp build examples/build-python --component-only --out out/python-library
wasmtime run --invoke 'greet("世界")' out/python-library/component.wasm
wasmtime run --invoke 'total([20,22])' out/python-library/component.wasm
wasmtime run --invoke 'checked(-1)' out/python-library/component.wasm
```

The results are `"Hello, 世界!"`, `42` and `err("value must be nonnegative")`.
The API also accepts records, enums, byte buffers and optional strings. Python
`int` maps to WIT `s64`, and `float` maps to `f64`. `Result[T, E]` declares a typed
error. Return the success value directly or raise `componentize_py_types.Err(E)`.

The first build of an API can be slow. The toolchain provisions its CPython
snapshot using bundled, pinned native componentize-py and bootstrap tools. That
step reads only WIT. Wisp caches it under `.wisp-cache/python/`, or your `--cache`
directory. Function-body changes reuse the template; contract or discovered stdlib
import changes select a new one. Stdlib imports are discovered inside Wasm. Application parsing, compilation and initialization run inside Wasm.

`wisp-authored.toml`, `authored.py` and `library.wit` retain the authored-WIT
variant, including its explicit integer widths:

```sh
wisp build examples/build-python/wisp-authored.toml --component-only --out out/python-authored
```

Both paths provision missing templates automatically and build without network
access. For third-party modules, add pinned project dependencies plus a
`python-deps.lock.json` pointing to hash-checked local wheels or vendor trees.
See [project builds](../../docs/project-builds.md) for the lock format and limits.
