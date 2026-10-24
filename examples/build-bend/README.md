# Build a Bend 2 Component

`math.bend` exports integer addition and float multiplication. Wisp infers
their WIT signatures from the `U32` and `F32` annotations.

First provision the [Bend compiler adapter](../../tools/bend/README.md#bootstrap).
Then run:

```sh
export WISP_BEND_TOOLCHAIN="$PWD/tools/bend"
wisp build examples/build-bend --component-only --out out/bend-math
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/bend-math/component.wasm
"$WISP_WASMTIME" run --invoke 'multiply(1.5,2.0)' out/bend-math/component.wasm
```

The calls return `42` and `3`. Bend's checker and compiler run inside Wasm,
then Wisp's JavaScript compiler package creates the application Component.
This backend runs sequentially and provides no GPU execution.
