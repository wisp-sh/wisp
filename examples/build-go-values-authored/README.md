# Rich Go values from WIT

[wit/world.wit](wit/world.wit) defines strings, byte buffers, records, typed
results, string lists, named enums, options, and lists of options.
[export_demo_go_values_api/implementation.go](export_demo_go_values_api/implementation.go)
implements the upstream backend's generated export API. `api.Packet`,
`api.Mode`, and `api.Batch` come from the generated `demo_go_values_api` package;
options and results use upstream `witTypes.Option` and `witTypes.Result`.

Wisp generates these packages during staging, adds a main-package import of
`wit_exports`, and uses the Wasm-hosted Go compiler and linker. The pinned
runtime source comes from the local Go toolchain support package.

From the repository root, with compilers and packaging tools configured:

```sh
wisp build examples/build-go-values-authored --out out/go-values
node examples/build-go-values-authored/verify.mjs out/go-values/node/index.js
```

Use a fresh output directory. To inspect the generator's function signatures
without compiling, run `wit-bindgen go --format=false --generate-stubs --world
library --pkg-name example.org/go-values --out-dir /path/to/scratch
examples/build-go-values-authored/wit/world.wit` with the pinned CLI. Generate stubs into
a scratch directory so they do not replace the implementation.
