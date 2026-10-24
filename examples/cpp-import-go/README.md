# Import a Go package from ordinary C++

[The C++ application](cpp/main.cpp) includes a package header and calls Go:

```cpp
#include <arithmetic.hpp>

unsigned doubled_sum(unsigned a, unsigned b) {
    return arithmetic::add(a, b) * 2;
}
```

[The Go package](go/src/main.go) implements ordinary exported functions:

```go
package main

func Add(a, b uint32) uint32 { return a + b }
func Negate(value bool) bool { return !value }
func Widen(value uint64) uint64 { return value + 1 }

func main() {}
```

Wisp infers both APIs. It generates the C++ header, Go export wrappers, WIT
contracts and dependency wiring. Neither application supplies WIT, ABI
annotations or special function names. The C++ example also calls Go with
boolean and 64-bit values.

## Open in an editor

[compile_flags.txt](compile_flags.txt) gives clangd the Wasm target, C++ language
settings and generated-header include directory. The generated
[arithmetic.hpp](bindings/cpp/arithmetic.hpp) is included in the checkout, so
completion, go-to-definition and type diagnostics work when opening the repo,
before running a build. The paths are relative to the project and work after
moving or copying the checkout. No editor-specific settings are required for
clangd-based editors.

Wisp still generates the actual compilation header from the installed package
during each build. When updating the dependency, refresh the checked-in header
from `build/units/application/bindings/arithmetic.hpp` in the successful output.
The inspection copy of `unit.cpp` uses a source include relative to this
checkout; the actual compiler input retains its sandbox path.

## Build the C++ application

[The project config](wisp.toml) selects the C++ compiler and an installed package:

```toml
[dependencies.arithmetic]
package = "demo:go-arithmetic"
version = "0.1.0"
path = "deps/arithmetic"

[build]
compiler = "clang"
sources = ["cpp/main.cpp"]
```

The dependency alias `arithmetic` gives the generated header its name,
`arithmetic.hpp`, and its C++ namespace, `arithmetic`. The dependency path is
an admitted release installed by `wisp library fetch`. The included
[installed package](deps/arithmetic) contains the Go Component and its admitted
release archives. Wisp validates the release, checks the requested identity and
version, and generates the header from the Component's exports.

With [the tools configured](../../docs/project-builds.md#configure-the-compilers),
run from the repository root:

```sh
cargo build --locked -p wisp
examples/cpp-import-go/run.sh out/my-cpp-wasm
# 42
```

This build needs the C++ toolchain, Wasmtime, WAC and the library admission
tools. It needs no Go compiler, Go source, Preview 1 adapter, registry connection
or native C++ compiler. The Go runtime and its adapter are
already inside the installed Component. Wisp carries its license notice into
the resulting release.

The result is `out/my-cpp-wasm/component.wasm`. Its C++ export calls the Go
`Add` implementation and doubles the result. Choose a fresh output directory
for each build. To produce Cargo and npm packages for publication as well:

```sh
target/debug/wisp build examples/cpp-import-go --out out/cpp-release
```

The final Component retains Go's WASI host requirements. Run it with a
Component-capable runtime. The generated npm package supplies the established
Go reactor host; browser execution is excluded.

## Build and publish the Go package

With the Go toolchain and reactor adapter configured:

```sh
target/debug/wisp build examples/cpp-import-go/go --out out/go-arithmetic
```

The Go producer uses only this compiler selection:

```toml
[build]
compiler = "go"
project = "src"
```

Publish the result through the existing `wisp library prepare` and
`wisp library publish` commands. Install it with `wisp library fetch` into the
consumer's `deps/arithmetic` directory, using the publication's Component
manifest digest. See [library publication](../../docs/library-publication.md)
for registry configuration and fetch commands. The Go package uses the same
Component registry transport as packages produced by other languages; C++
does not import Go source or link against a native Go library.

## Run the registry demonstration

```sh
python3 examples/cpp-import-go/registry-demo.py \
  --release out/go-arithmetic --out out/cpp-go-registry
```

The demo publishes the Go release to disposable Cargo/npm/OCI registries and
installs it with `wisp library fetch`. It then shuts down the registries and
builds a separate C++ consumer with Go source absent and unusable Go compiler
and adapter paths. It checks `42`, a boolean result and a 64-bit result, verifies
license preservation, and rejects changed Component bytes and an incorrect
required version. It needs no native Go executable.

## Inspect the generated code

These generated snapshots expose the implementation without becoming inputs
to the application build:

- [arithmetic.hpp](bindings/cpp/arithmetic.hpp) implements the public C++ API
  using private functions with Wasm import attributes.
- [unit.cpp](bindings/cpp/unit.cpp) exports the ordinary C++ functions under
  their inferred canonical names; [wisp.h](bindings/cpp/wisp.h) is its private
  binding header.
- [Go export wrappers](bindings/go/wisp_bindings.go) connect canonical Wasm
  exports to `Add`, `Negate` and `Widen` and reserve Go's adapter storage.
- [composition.wac](bindings/composition.wac) connects the C++ imports to the
  installed Go Component. [component.wit](bindings/component.wit) shows the
  final exports and remaining WASI host imports.

The generated header includes this private import and public wrapper:

```cpp
namespace arithmetic {
namespace detail {
extern "C" __attribute__((
    import_module("demo:go-arithmetic/api@0.1.0"), import_name("add")))
    __UINT32_TYPE__ wisp_import_arithmetic_0(__UINT32_TYPE__ a0, __UINT32_TYPE__ a1);
}
inline __UINT32_TYPE__ add(__UINT32_TYPE__ a0, __UINT32_TYPE__ a1) {
    return detail::wisp_import_arithmetic_0(a0, a1);
}
}
```

The matching Go wrapper is generated when building the producer:

```go
//go:wasmexport demo:go-arithmetic/api@0.1.0#add
func wispExport0(a0 uint32, a1 uint32) uint32 {
    return uint32(Add(uint32(a0), uint32(a1)))
}
```

A call enters C++, crosses the generated import into Go, runs `Add`, and
returns to C++ to double the result. Both implementations are embedded in the
final Component. Wisp retains the actual headers, contracts, compiler commands,
dependency lock and composition report under `build/` in each output.

Generated C++ dependency headers support synchronous scalars, strings, byte
buffers, records and typed results. This example uses scalar calls. Rich APIs
use `wisp::string`, `wisp::bytes` and `wisp::result<T, E>` from the generated
`wisp_types.hpp`; imported buffers retain their data until their last copy is
destroyed. See [source binding and ownership rules](../../docs/project-builds.md#write-ordinary-application-code).
A single `api` interface becomes calls such as
`arithmetic::add`; other interfaces get their own nested namespace. Function
names use snake case; C++ keywords get a trailing underscore. This is a Wasm
package import, built with the C++ toolchain, rather than a native C++ library target.
