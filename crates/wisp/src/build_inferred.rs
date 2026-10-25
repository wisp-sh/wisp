//! Adapt ordinary source values to upstream-generated bindings.
pub(super) use super::infer::{Function, SourceKind, SourceType};
use super::*;
#[path = "build_inferred_cpp.rs"]
pub(super) mod cpp;
#[path = "build_inferred_go.rs"]
pub(super) mod go;
#[path = "build_inferred_rust.rs"]
pub(super) mod rust;
