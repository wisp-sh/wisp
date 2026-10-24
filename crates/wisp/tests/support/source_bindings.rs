// The library projection shares source ABI generation with the CLI. Integration
// crates include library.rs directly, so they need the same generator modules.
#[allow(dead_code)]
#[path = "../../src/build_bindings.rs"]
pub(crate) mod build_bindings;
#[allow(dead_code)]
#[path = "../../src/build_go_memory.rs"]
pub(crate) mod build_go_memory;
#[allow(dead_code)]
#[path = "../../src/build_values.rs"]
pub(crate) mod build_values;
