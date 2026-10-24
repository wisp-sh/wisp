//! Wisp-owned execution hosts shared by independently installed compiler packages.
use crate::build::{digest, read};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const COMMAND_VERSION: &str = "50.0.0";
pub(crate) const RUST_WASIP3_VERSION: &str = COMMAND_VERSION;
const PYTHON_VERSION: &str = "50.0.1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    WasmtimeCli,
    WorkerProtocol,
}

/// The exact `--version` line of a compatible Wisp worker.
pub(crate) fn worker_version_line() -> String {
    format!(
        "wisp-worker 0.2.0 wasmtime {} protocol {}\n",
        wisp_worker_protocol::WASMTIME_VERSION,
        wisp_worker_protocol::VERSION
    )
}

impl Backend {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::WasmtimeCli => "wasmtime-cli",
            Self::WorkerProtocol => "worker-protocol",
        }
    }

    pub(crate) fn wasmtime_version(self) -> &'static str {
        match self {
            Self::WasmtimeCli => COMMAND_VERSION,
            Self::WorkerProtocol => wisp_worker_protocol::WASMTIME_VERSION,
        }
    }
}

pub(crate) struct ExecutionHost {
    pub path: PathBuf,
    pub sha256: String,
    pub backend: Backend,
}

#[derive(Clone, Copy)]
pub(crate) enum Contract {
    // Wisp constructs protocol requests directly and can also use the CLI.
    WorkerProtocol,
    // External runners may require CLI settings absent from the worker protocol.
    WasmtimeCli,
}

pub(crate) fn select(
    root: &Path,
    overrides: &[(&str, &Option<PathBuf>)],
    version: &str,
    contract: Contract,
) -> Result<ExecutionHost> {
    let explicit = overrides.iter().find_map(|(environment, configured)| {
        std::env::var_os(environment)
            .map(PathBuf::from)
            .or_else(|| (*configured).clone())
    });
    let explicit = explicit.map(|path| {
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    });
    select_with(
        explicit.as_deref(),
        version,
        contract,
        crate::worker::discover,
        || crate::toolchain_install::resolve_host(version),
    )
    .with_context(|| {
        let environments = overrides
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(" or ");
        format!(
            "set {environments} or run wisp toolchain setup-hosts to configure the execution host"
        )
    })
}

fn select_with(
    explicit: Option<&Path>,
    version: &str,
    contract: Contract,
    worker: impl FnOnce() -> Result<Option<PathBuf>>,
    installed: impl FnOnce() -> Result<Option<PathBuf>>,
) -> Result<ExecutionHost> {
    ensure!(
        version == COMMAND_VERSION,
        "unsupported Wasmtime execution host version: {version}"
    );
    if let Some(path) = explicit {
        // Goose's adapter uses only the worker-compatible execution options.
        // Preserve the CLI-only contract for runners needing other settings.
        if matches!(contract, Contract::WorkerProtocol) {
            let output = Command::new(path)
                .arg("--version")
                .output()
                .with_context(|| format!("starting Wisp execution host {}", path.display()))?;
            if output.status.success() && output.stdout == worker_version_line().as_bytes() {
                return Ok(ExecutionHost {
                    path: path.canonicalize()?,
                    sha256: digest(&read(path, 256 * 1024 * 1024)?),
                    backend: Backend::WorkerProtocol,
                });
            }
        }
        return runtime(path, version);
    }
    if matches!(contract, Contract::WorkerProtocol) {
        if let Some(worker) = worker()? {
            let output = Command::new(&worker).arg("--version").output()?;
            ensure!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout) == worker_version_line(),
                "incompatible Wisp worker"
            );
            return Ok(ExecutionHost {
                sha256: digest(&read(&worker, 256 * 1024 * 1024)?),
                path: worker,
                backend: Backend::WorkerProtocol,
            });
        }
    }
    let path = installed()?.with_context(|| {
        format!("configure a Wasmtime {version} execution host or run wisp toolchain setup-hosts")
    })?;
    runtime(&path, version)
}

// A path passed by an external runner is an explicit Wasmtime CLI selection.
// Never replace it with an auto-discovered worker with different option support.
pub(crate) fn runtime(path: &Path, version: &str) -> Result<ExecutionHost> {
    ensure!(
        version == COMMAND_VERSION,
        "unsupported Wasmtime execution host version: {version}"
    );
    let path = path
        .canonicalize()
        .with_context(|| format!("Wisp execution host {}", path.display()))?;
    let output = Command::new(&path)
        .arg("--version")
        .output()
        .with_context(|| format!("starting Wisp execution host {}", path.display()))?;
    let actual = String::from_utf8_lossy(&output.stdout);
    let mut words = actual.split_whitespace();
    ensure!(
        output.status.success()
            && words.next() == Some("wasmtime")
            && words.next() == Some(version),
        "Wisp compiler execution host requires Wasmtime {version}, got {}",
        actual.trim()
    );
    Ok(ExecutionHost {
        sha256: digest(&read(&path, 256 * 1024 * 1024)?),
        path,
        backend: Backend::WasmtimeCli,
    })
}

fn command_status(environment: &str, version: &str, note: &str) -> Value {
    let shared_rust = environment == "WISP_RUST_WASMTIME" && version == RUST_WASIP3_VERSION;
    let absent = None;
    let mut overrides = vec![(environment, &absent)];
    if shared_rust {
        overrides.push(("WISP_WASMTIME", &absent));
    }
    let result = select(
        Path::new("."),
        &overrides,
        version,
        if shared_rust {
            Contract::WasmtimeCli
        } else {
            Contract::WorkerProtocol
        },
    );
    match result {
        Ok(host) => json!({"environment":environment,"requiredVersion":version,"available":true,
            "path":host.path,"sha256":host.sha256,"backend":host.backend.name(),
            "sizeBytes":std::fs::metadata(&host.path).ok().map(|metadata|metadata.len()),"note":note}),
        Err(error) => json!({"environment":environment,"requiredVersion":version,"available":false,
            "error":format!("{error:#}"),"note":note}),
    }
}

fn python_status() -> Value {
    // Isolated mode matches Wisp's Python adapters. Explicit host modules are
    // selected separately from compiler payloads and never inferred from them.
    let script = r#"
import hashlib, importlib.metadata, json, os, pathlib, sys
selected = os.environ.get('WISP_WASMTIME_PYTHON')
if selected:
    sys.path.insert(0, str(pathlib.Path(selected).resolve(strict=True)))
import wasmtime
from wasmtime import _ffi
engine = wasmtime.Engine()
store = wasmtime.Store(engine)
module = wasmtime.Module(engine, '(module (func (export "value") (result i32) i32.const 42))')
assert wasmtime.Instance(store, module, []).exports(store)['value'](store) == 42
version = None
try:
    distribution = importlib.metadata.distribution('wasmtime')
    if pathlib.Path(distribution.locate_file('wasmtime')).resolve() == pathlib.Path(wasmtime.__file__).resolve().parent:
        version = distribution.version
except importlib.metadata.PackageNotFoundError:
    pass
library = pathlib.Path(_ffi.filename).resolve(strict=True)
print(json.dumps({'module': str(pathlib.Path(wasmtime.__file__).resolve()), 'library': str(library),
    'sha256': hashlib.file_digest(library.open('rb'), 'sha256').hexdigest(),
    'sizeBytes': library.stat().st_size, 'version': version}))
"#;
    let result = (|| -> Result<Value> {
        let output = Command::new("python3")
            .args(["-I", "-c", script])
            .output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let mut report: Value = serde_json::from_slice(&output.stdout)?;
        if let Some(version) = report["version"].as_str() {
            ensure!(
                version == PYTHON_VERSION,
                "requires Python wasmtime {PYTHON_VERSION}, got {version}"
            );
        }
        report["environment"] = json!("WISP_WASMTIME_PYTHON");
        report["requiredVersion"] = json!(PYTHON_VERSION);
        report["versionVerified"] = json!(report["version"].is_string());
        report["available"] = json!(true);
        report["note"] = json!(
            "AssemblyScript requires the Python host API and custom imports. The Wasmtime CLI does not provide that API."
        );
        Ok(report)
    })();
    match result {
        Ok(report) => report,
        Err(error) => json!({"environment":"WISP_WASMTIME_PYTHON","requiredVersion":PYTHON_VERSION,
            "available":false,"error":format!("{error:#}")}),
    }
}

fn rust_status() -> Value {
    let selected = std::env::var_os("WISP_RUST")
        .map(std::path::PathBuf::from)
        .map(|path| Ok(Some(path)))
        .unwrap_or_else(|| crate::toolchain_install::resolve("WISP_RUST"));
    let host = selected.and_then(|path| {
        path.map(|path| crate::build_rust::Toolchain::compiler_host(&path))
            .transpose()
    });
    match host {
        Ok(_) => command_status(
            "WISP_RUST_WASMTIME",
            RUST_WASIP3_VERSION,
            "Rust WASIp3 uses the shared Component execution host with its linked LLVM backend.",
        ),
        Err(error) => json!({"environment":"WISP_RUST_WASMTIME", "available":false,
            "error":format!("{error:#}")}),
    }
}

pub(crate) fn status() -> Value {
    let hosts = vec![
        command_status(
            "WISP_WASMTIME",
            COMMAND_VERSION,
            "Shared Component and WASIp1 command host.",
        ),
        rust_status(),
        python_status(),
    ];
    json!({"owner":"wisp","includedInCompilerPackages":false,
        "available":hosts.iter().all(|host| host["available"] == true),"hosts":hosts})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn host(version: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wasmtime");
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf 'wasmtime {version}\\n'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (directory, path)
    }
    #[test]
    #[cfg(unix)]
    fn rejects_unknown_execution_host_versions() {
        let (_directory, path) = host(COMMAND_VERSION);
        let error = runtime(&path, "0.0.0").err().unwrap();
        assert!(
            error
                .to_string()
                .contains("unsupported Wasmtime execution host version")
        );
    }

    #[test]
    #[cfg(unix)]
    fn explicit_wasmtime_wins_without_discovering_a_worker() {
        let (_directory, path) = host(COMMAND_VERSION);
        let selected = select_with(
            Some(&path),
            COMMAND_VERSION,
            Contract::WorkerProtocol,
            || panic!("an explicit Wasmtime selection must win over worker discovery"),
            || panic!("an explicit selection must not resolve an installed host"),
        )
        .unwrap();
        assert_eq!(selected.path, path.canonicalize().unwrap());
        assert_eq!(selected.backend, Backend::WasmtimeCli);

        let error = select_with(
            Some(&path.with_file_name("missing")),
            COMMAND_VERSION,
            Contract::WorkerProtocol,
            || panic!("a broken explicit override must not fall back"),
            || panic!("a broken explicit override must not fall back"),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("Wisp execution host"));
    }

    #[test]
    #[cfg(unix)]
    fn worker_selection_does_not_require_an_installed_wasmtime() {
        let (_directory, path) = host(COMMAND_VERSION);
        std::fs::write(
            &path,
            &format!(
                "#!/bin/sh\nprintf '{}\\n'\n",
                worker_version_line().trim_end()
            ),
        )
        .unwrap();
        let selected = select_with(
            None,
            COMMAND_VERSION,
            Contract::WorkerProtocol,
            || Ok(Some(path.clone())),
            || panic!("worker selection must happen before installed host resolution"),
        )
        .unwrap();
        assert_eq!(selected.path, path);
        assert_eq!(selected.backend, Backend::WorkerProtocol);
    }

    #[test]
    #[cfg(unix)]
    fn explicit_worker_is_accepted_only_for_worker_compatible_runners() {
        let (_directory, path) = host(COMMAND_VERSION);
        std::fs::write(
            &path,
            &format!(
                "#!/bin/sh\nprintf '{}\\n'\n",
                worker_version_line().trim_end()
            ),
        )
        .unwrap();
        let selected = select_with(
            Some(&path),
            COMMAND_VERSION,
            Contract::WorkerProtocol,
            || panic!("explicit worker must win"),
            || panic!("explicit worker must win"),
        )
        .unwrap();
        assert_eq!(selected.backend, Backend::WorkerProtocol);
        assert!(
            select_with(
                Some(&path),
                COMMAND_VERSION,
                Contract::WasmtimeCli,
                || panic!("no worker discovery"),
                || panic!("no fallback")
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    fn external_runner_contract_requires_the_wasmtime_cli() {
        let (_directory, path) = host(COMMAND_VERSION);
        let selected = select_with(
            None,
            COMMAND_VERSION,
            Contract::WasmtimeCli,
            || panic!("external runners must not discover a worker"),
            || Ok(Some(path.clone())),
        )
        .unwrap();
        assert_eq!(selected.path, path.canonicalize().unwrap());
        assert_eq!(selected.backend, Backend::WasmtimeCli);
    }

    #[test]
    #[cfg(unix)]
    fn absent_workers_fall_back_but_broken_workers_fail() {
        let (_directory, path) = host(COMMAND_VERSION);
        let selected = select_with(
            None,
            COMMAND_VERSION,
            Contract::WorkerProtocol,
            || Ok(None),
            || Ok(Some(path.clone())),
        )
        .unwrap();
        assert_eq!(selected.backend, Backend::WasmtimeCli);
        let error = select_with(
            None,
            COMMAND_VERSION,
            Contract::WorkerProtocol,
            || anyhow::bail!("configured WISP_WORKER is unavailable"),
            || panic!("a broken worker override must not fall back"),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("WISP_WORKER"));
        let error = select_with(
            None,
            COMMAND_VERSION,
            Contract::WorkerProtocol,
            || Ok(Some(path)),
            || panic!("an incompatible worker must not fall back"),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("incompatible Wisp worker"));
    }
}
