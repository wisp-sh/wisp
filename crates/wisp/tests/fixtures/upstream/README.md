# Upstream compatibility fixture

`example-d.wit` is an unmodified copy of
`crates/wasm-pkg-core/tests/fixtures/transitive-local/example-d/wit/world.wit`
from https://github.com/bytecodealliance/wasm-pkg-tools at
`5a4c2ab721e12511f39bb9cb42cf71fe76f6c89a`.

The upstream workspace declares `Apache-2.0 WITH LLVM-exception`. Its complete
LICENSE is retained beside the fixture. This fixture tests local WIT package
identity and override parsing without importing the upstream OCI client.
