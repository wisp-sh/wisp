//! Pinned upstream source generation. Application compilation uses Wasm tools.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const GENERATOR_VERSION: &str = crate::generator_tools::WIT_BINDGEN_VERSION;
pub const RUNTIME_MODULE: &str = "go.bytecodealliance.org/pkg";
pub const RUNTIME_VERSION: &str = "v0.2.3";
pub const RUNTIME_SUM: &str = "go.bytecodealliance.org/pkg v0.2.3 h1:So9zHg8ahW+TH1JmgBHgIYuKHcinLiEcpV1tqVEZvLA=\ngo.bytecodealliance.org/pkg v0.2.3/go.mod h1:OjA+V8g3uUFixeCKFfamm6sYhTJdg8fvwEdJ2GO0GSk=\n";

fn work_root() -> Result<PathBuf> {
    let root = if let Some(path) = std::env::var_os("WISP_WIT_BINDGEN_WORKDIR") {
        PathBuf::from(path)
    } else if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(path).join("wisp/wit-bindgen")
    } else {
        PathBuf::from(std::env::var_os("HOME").context("set WISP_WIT_BINDGEN_WORKDIR")?)
            .join(".cache/wisp/wit-bindgen")
    };
    fs::create_dir_all(&root)?;
    Ok(root)
}

/// Generate a library's bindings with the official CLI. No Go tool, formatter,
/// dependency installer, or compiler runs here. Keep the CLI and its runtime in
/// step; both version checks also apply during archive admission/regeneration.
pub fn generate(wit: &str, world: &str, module: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    let executable = crate::generator_tools::wit_bindgen_executable()?;
    let temp = tempfile::Builder::new()
        .prefix("generate-")
        .tempdir_in(work_root()?)?;
    let input = temp.path().join("world.wit");
    let output = temp.path().join("output");
    fs::write(&input, wit)?;
    let result = Command::new(&executable)
        .args([
            "go",
            "--format=false",
            "--pkg-name",
            module,
            "--print-remote-pkg-version",
            "--world",
            world,
            "--out-dir",
        ])
        .arg(&output)
        .arg(&input)
        .output()
        .context("generate upstream Go bindings")?;
    ensure!(
        result.status.success(),
        "upstream Go bindings failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    ensure!(
        std::str::from_utf8(&result.stdout)?.trim()
            == format!("{RUNTIME_MODULE} {RUNTIME_VERSION}"),
        "unexpected upstream Go runtime version"
    );
    fn collect(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<String, Vec<u8>>,
        size: &mut usize,
    ) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(!kind.is_symlink(), "symlink in upstream Go output");
            if kind.is_dir() {
                collect(root, &entry.path(), files, size)?;
            } else {
                ensure!(kind.is_file(), "non-file upstream Go output");
                let path = entry.path();
                let name = path
                    .strip_prefix(root)?
                    .to_str()
                    .context("UTF8 upstream Go filename")?
                    .replace('\\', "/");
                ensure!(
                    name.ends_with(".go") || name.ends_with("/empty.s"),
                    "unexpected upstream Go output: {name}"
                );
                ensure!(
                    entry.metadata()?.len() <= 4 * 1024 * 1024 && files.len() < 1024,
                    "upstream Go output limit"
                );
                let bytes = fs::read(path)?;
                *size += bytes.len();
                ensure!(*size <= 16 * 1024 * 1024, "upstream Go total output limit");
                files.insert(name, bytes);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    collect(&output, &output, &mut files, &mut 0)?;
    ensure!(!files.is_empty(), "no upstream Go bindings");
    Ok(files)
}
