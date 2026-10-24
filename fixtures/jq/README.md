# Real jq fixture

`component.wasm` is the unchanged 1,481,276-byte Component from a prior local source build of jq 1.8.1 and its bundled Oniguruma 6.9.10. SHA-256:

```
3f4f95f882d2f7e574e1a3dcd01f690a8f72e928fe8c7d151892daf6433b173e
```

The maintained integration is `src/component.c` plus `wit/jq.wit`. This is libjq compiled from C, not another jq implementation or a CLI subprocess. `provenance/` retains the upstream tag metadata and the adapter checksum used by the build recipe. These are source inputs, not signed build attestations. Package tests use this directory's binary.

To fetch checksum-pinned sources/tool inputs and rebuild locally, with network access and Linux amd64 host build utilities:

```sh
python fixtures/jq/bootstrap.py
bash fixtures/jq/build.sh
```

The bootstrap installs under this fixture directory and checks `downloads.lock.json` inputs. It does not need the packaging tool prefix. The recipe uses the checked-in adapter hash, WASI SDK 31.0, wit-bindgen 0.61.0, wasm-tools 1.245.0 and the Wasmtime 50.0.0 reactor adapter. It invokes the upstream jq release configure scripts. Python 3.11+, Bash, GNU tar, Make and ordinary Linux host utilities are prerequisites. Build caches and compiler tools are deliberately not distributed with the source.

Package generation and the installed four-target regressions exercise this exact carried binary. Cross-machine byte reproducibility and signed source provenance are not established. A rebuild that changes the binary needs a corresponding manifest digest/import review, not automatic acceptance under the old hash.

jq and Oniguruma notices are retained in `notices/`. The platform does not validate licensing, ownership, source URLs or build recipes. Do not run the source recipe through the upload service.
