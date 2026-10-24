#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
export TMPDIR="$ROOT/tmp"
mkdir -p "$TMPDIR" build/jq evidence
SDK="$ROOT/tools/wasi-sdk/bin"
"$ROOT/tools/wit-bindgen/wit-bindgen" c wit --out-dir build
(
 cd build/jq
 CC="$SDK/clang" AR="$SDK/llvm-ar" RANLIB="$SDK/llvm-ranlib" \
 CFLAGS='-O2 -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS' \
 LDFLAGS='-lwasi-emulated-signal -lwasi-emulated-process-clocks' \
 ../../tools/jq/configure --host=wasm32-wasi --disable-shared --enable-static \
 --disable-docs --disable-maintainer-mode --with-oniguruma=builtin
) > evidence/configure.log 2>&1
make -C build/jq/vendor/oniguruma -j"$(nproc)" > evidence/oniguruma-build.log 2>&1
make -C build/jq src/builtin.inc src/config_opts.inc src/version.h > evidence/generated-build.log 2>&1
make -C build/jq -j"$(nproc)" libjq.la > evidence/make.log 2>&1
"$SDK/clang" -O2 -Wall -Wextra -Ibuild -Itools/jq/src -mexec-model=reactor \
 src/component.c build/jq_component.c build/jq_component_component_type.o \
 build/jq/.libs/libjq.a build/jq/vendor/oniguruma/src/.libs/libonig.a \
 -lwasi-emulated-signal -lwasi-emulated-process-clocks -lm \
 -Wl,-z,stack-size=1048576 -o build/core.wasm > evidence/link.log 2>&1
sha256sum -c provenance/adapter.sha256
WT="$ROOT/tools/wasm-tools/wasm-tools"
"$WT" component new build/core.wasm --adapt tools/wasi_snapshot_preview1.reactor.wasm -o component.wasm
"$WT" validate component.wasm
"$WT" component wit component.wasm > evidence/component.wit
"$WT" print build/core.wasm > evidence/core.wat
python3 -c 'import pathlib; p=pathlib.Path("evidence/core.wat"); pathlib.Path("evidence/core-imports.txt").write_text("\n".join(x for x in p.read_text().splitlines() if x.lstrip().startswith("(import"))+"\n")'
sha256sum component.wasm build/core.wasm > evidence/artifacts.sha256
wc -c component.wasm build/core.wasm
