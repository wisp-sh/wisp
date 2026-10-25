//! Stage an authenticated upstream Go runtime without fetching dependencies.
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs, path::Path};

fn source_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    ensure!(
        fs::symlink_metadata(directory)?.is_dir(),
        "invalid Go WIT runtime directory"
    );
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(!kind.is_symlink(), "symlink in Go WIT runtime");
        if kind.is_dir() {
            source_files(root, &entry.path(), files)?;
        } else {
            ensure!(
                kind.is_file() && entry.metadata()?.len() <= 64 * 1024 && files.len() < 64,
                "Go WIT runtime input limit"
            );
            let path = entry.path();
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("Go WIT runtime path")?
                .replace('\\', "/");
            files.insert(name, fs::read(path)?);
        }
    }
    Ok(())
}

pub fn stage(source: &Path, support: &Path) -> Result<()> {
    let pin: serde_json::Value = serde_json::from_str(include_str!("go_wit_runtime.lock.json"))?;
    let expected = pin["files"].as_object().context("Go WIT runtime pin")?;
    let mut files = BTreeMap::new();
    source_files(support, support, &mut files).with_context(|| format!("read pinned Go WIT runtime at {}; install a toolchain with wit-runtime or set WISP_GO_WIT_RUNTIME", support.display()))?;
    ensure!(
        files.len() == expected.len(),
        "Go WIT runtime file set differs from v0.2.3 pin"
    );
    for (name, bytes) in &files {
        ensure!(
            expected.get(name).and_then(|v| v.as_str())
                == Some(crate::build::digest(bytes).as_str()),
            "Go WIT runtime differs from v0.2.3 pin: {name}"
        );
    }
    let destination = source.join("vendor/go.bytecodealliance.org/pkg");
    for directory in [
        source.join("vendor"),
        source.join("vendor/go.bytecodealliance.org"),
        destination.clone(),
    ] {
        if let Ok(meta) = fs::symlink_metadata(directory) {
            ensure!(meta.is_dir(), "invalid vendored Go WIT runtime directory");
        }
    }
    if destination.try_exists()? {
        let mut existing = BTreeMap::new();
        source_files(&destination, &destination, &mut existing)?;
        for (name, bytes) in &existing {
            ensure!(
                files.get(name) == Some(bytes),
                "vendored Go WIT runtime differs from v0.2.3 pin: {name}"
            );
        }
    }
    for (name, bytes) in files {
        let path = destination.join(name);
        fs::create_dir_all(path.parent().context("Go WIT runtime parent")?)?;
        fs::write(path, bytes)?;
    }
    Ok(())
}
