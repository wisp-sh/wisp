//! Versioned local IPC shared by Wisp, compiler adapters and packaged bindings.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
pub mod bundle;
pub mod client;
pub mod command;

pub const VERSION: u32 = 1;
/// Wasmtime release embedded in the worker and recorded in bundled worker manifests.
pub const WASMTIME_VERSION: &str = "50.0.0";
pub const MAX_FRAME: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub host: String,
    pub guest: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub memory_bytes: usize,
    pub output_bytes: usize,
    pub timeout_ms: u64,
    pub fuel: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 4_294_901_760,
            output_bytes: 16 * 1024 * 1024,
            timeout_ms: 600_000,
            fuel: u64::MAX,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub operation: String,
    pub module: String,
    pub sha256: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub mounts: Vec<Mount>,
    pub modules: BTreeMap<String, String>,
    pub stdin: String,
    pub instance: u64,
    pub interface: String,
    pub function: String,
    /// Component options use exactly one tag: `{"none":null}` or
    /// `{"some":VALUE}`. Nested options retain a tag at every level.
    /// Resource handles contain `instance` and `handle`; owned parameters
    /// consume their handle, while borrowed parameters retain it.
    /// Future and stream transfers, including nested types, are unsupported.
    pub params: Vec<Value>,
    pub handle: u64,
    pub wasi_p3: bool,
    pub network: bool,
    pub limits: Limits,
}
impl Default for Request {
    fn default() -> Self {
        Self {
            version: VERSION,
            id: 0,
            operation: "run".into(),
            module: String::new(),
            sha256: None,
            args: vec![],
            env: BTreeMap::new(),
            mounts: vec![],
            modules: BTreeMap::new(),
            stdin: String::new(),
            instance: 0,
            interface: String::new(),
            function: String::new(),
            params: vec![],
            handle: 0,
            wasi_p3: false,
            network: false,
            limits: Limits::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    pub ok: bool,
    pub exit_code: i32,
    /// Guest output is base64, never interleaved with control frames.
    pub stdout: String,
    pub stderr: String,
    pub instance: u64,
    /// Component values use the same tagged encoding as [`Request::params`].
    pub results: Vec<Value>,
    pub error: Option<String>,
}
impl Response {
    pub fn new(id: u64) -> Self {
        Self {
            version: VERSION,
            id,
            ok: true,
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            instance: 0,
            results: vec![],
            error: None,
        }
    }
}
