# TypeScript catalog

```sh
wisp build examples/build-typescript-catalog --out out/typescript-catalog
wisp library validate out/typescript-catalog
```

Install the JavaScript, tsrs, Wizer and wasm-tools compiler packages first. The
selected Wisp execution host must include the approved Wizer capability, or set
`WISP_WIZER_WASMTIME` to the pinned official Wasmtime 50.0.0 host. Type checking
also requires the external Wasmtime Python 50.0.1 host.

The project uses local TypeScript modules, strict tsconfig checking, authored
WIT, and StarlingMonkey. `catalog` and `formatting` share module state. Their
contract includes records, lists, options, flags, tuples, variants and typed
errors. Set `javascript-engine = "quickjs"` to use the compatibility engine.

The source compiler and binding splicer run in Wasm. StarlingMonkey initialization
runs in Wasm and the approved native Wizer host captures its state. Build receipts
describe that snapshot exception. No native TypeScript compiler or Node process
compiles the application.
