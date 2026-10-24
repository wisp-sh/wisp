# Typed asynchronous library values

This fixture checks a `future<record { label: string, values: list<u32> }>` and a `stream<string>`. Their producer functions are asynchronous WIT exports. The record and strings pass through the same installed Cargo package in native Rust and a Component guest. The npm consumer imports the generated package by name.

Provision with `python3 tests/library-chain/setup.py`, build the current CLI, then run:

```sh
python3 tests/library-async/run.py
python3 tests/library-async/run.py --replay
```

The first command installs through isolated loopback Cargo and npm registries. It stops both servers before building or running consumers. Replay uses Cargo locks, the npm lock and cache, a frozen packaging executable, and the original library Component. CI runs replay with network access denied by the OS. Source and asset snapshots identify the exact generator used at installation.

Native Rust checks repeated future completion, ordered string streams, stream exhaustion, cancellation by dropping an unread future and an active stream, and rejection of reads after explicit close. `Library::drain().await` completes background Component tasks once the caller has consumed or closed their readers; it rejects open readers to avoid waiting for a close that needs the same Store. The installed consumer also closes a reader while another call is pending on the same executor, checks the overlapping call returns a busy error, and verifies cleanup after the pending call finishes. The guest performs the same typed calls and returns 42. WAC composes it with the original library, and both Node/JCO and an outer Wasmtime 50.0.0 instance execute the composed bytes. The native probe also requires Wasmtime's concurrent state tables to be empty after cleanup. The guest Cargo graph must contain no host runtime.

The generated readers close on drop. A stream reads one item per `next()` call, so the host does not collect an unbounded stream eagerly. Future payloads and stream items use generated typed Wasmtime bindings. Standard synchronous functions on the same interface remain synchronous Rust methods. Async WIT functions use owned guest arguments because wit-bindgen 0.61.1's async imports take ownership.

The current emitter accepts direct future and stream results with ordinary typed payloads. Nested async handles, async parameters, and async resource methods are rejected until they have ownership conversions and executable coverage. Async browser execution is not included in the target list. Under Wasmtime 50, lifted error contexts are retained until the Store is dropped.

## Pinned JCO limitations

Node 24.20.0 requires `--experimental-wasm-jspi` for these generated modules. JCO 1.33.0 reads the record future and string stream correctly, and `AsyncIterator.return()` cancels a string stream. Its public `FutureValue` has a `then` method but no close, cancel, or disposal method. An unread future keeps its guest writer alive. Reproduce that separately:

```sh
.tools/library-chain-node/node/bin/node --experimental-wasm-jspi \
  tests/library-async/future-cancellation.mjs \
  "$PWD/out/library-async-chain/release/node/index.js"
```

The fixture's `delayed` export calls `yield_async()` before returning its future. With that yield, JCO 1.33.0 executes the composed A-to-B graph and returns 42. The earlier fixture without the yield failed during context lookup, so this result establishes the tested graph rather than general composed async support.

The cleanup API follows Wasmtime's [FutureReader ownership requirement](https://docs.rs/wasmtime/50.0.0/wasmtime/component/struct.FutureReader.html) and [Accessor task completion API](https://docs.rs/wasmtime/50.0.0/wasmtime/component/struct.Accessor.html#method.poll_no_interesting_tasks).
