//! Authenticate installed generator files using the locked npm archive integrity.
use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn bounded(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 256 * 1024 * 1024,
        "invalid generator file: {}",
        path.display()
    );
    Ok(fs::read(path)?)
}

pub fn authenticate_npm_tree(prefix: &Path, cache: &Path, lock: &Value) -> Result<()> {
    let prefix = prefix.canonicalize()?;
    let mut expected = BTreeSet::new();
    for (package_path, package) in lock["packages"].as_object().context("npm package lock")? {
        if package_path.is_empty() {
            continue;
        }
        crate::release::safe_name(package_path)?;
        let installed = prefix.join(package_path);
        if !installed.try_exists()? {
            ensure!(
                package["optional"] == true,
                "missing locked generator package: {package_path}"
            );
            continue;
        }
        ensure!(
            fs::symlink_metadata(&installed)?.is_dir(),
            "symlinked generator package: {package_path}"
        );
        let integrity = package["integrity"]
            .as_str()
            .context("locked npm integrity")?;
        let encoded = integrity
            .strip_prefix("sha512-")
            .context("unsupported npm integrity algorithm")?;
        let hash = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        ensure!(hash.len() == 64, "invalid npm integrity digest");
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        let archive = cache
            .join("_cacache/content-v2/sha512")
            .join(&hex[..2])
            .join(&hex[2..4])
            .join(&hex[4..]);
        let bytes = bounded(&archive).with_context(|| format!("missing authenticated npm cache for {package_path}; run the pinned library setup and retain its npm cache"))?;
        ensure!(
            Sha512::digest(&bytes).as_slice() == hash,
            "generator npm archive integrity mismatch: {package_path}"
        );
        for (name, contents) in crate::release::archive_files(&bytes)? {
            let (_, relative) = name.split_once('/').context("npm archive package root")?;
            crate::release::safe_name(relative)?;
            let path = installed.join(relative);
            ensure!(
                Sha256::digest(bounded(&path).with_context(|| format!(
                    "missing generator member {package_path}/{relative}"
                ))?) == Sha256::digest(&contents),
                "generator file differs from locked archive: {package_path}/{relative}"
            );
            expected.insert(format!("{package_path}/{relative}"));
        }
    }
    fn visit(prefix: &Path, directory: &Path, expected: &BTreeSet<String>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(prefix)?
                .to_str()
                .context("generator path UTF8")?
                .to_owned();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(prefix, &path, expected)?;
            } else if kind.is_symlink() {
                ensure!(
                    path.parent()
                        .and_then(Path::file_name)
                        .is_some_and(|name| name == ".bin"),
                    "unexpected generator symlink: {relative}"
                );
                let target = path.canonicalize()?;
                ensure!(
                    expected.contains(
                        target
                            .strip_prefix(prefix)
                            .context("generator symlink escapes prefix")?
                            .to_str()
                            .context("generator symlink UTF8")?
                    ),
                    "unlocked generator symlink: {relative}"
                );
            } else {
                ensure!(
                    kind.is_file()
                        && (expected.contains(&relative)
                            || relative == "node_modules/.package-lock.json"),
                    "unlocked generator file: {relative}"
                );
            }
        }
        Ok(())
    }
    visit(&prefix, &prefix.join("node_modules"), &expected)
}

pub fn authenticate_jco(path: &Path) -> Result<()> {
    let path = path.canonicalize()?;
    let modules = path
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "node_modules"))
        .context("jco must be installed by the locked library setup")?;
    let prefix = modules.parent().context("generator prefix")?;
    ensure!(
        path == prefix.join("node_modules/@bytecodealliance/jco/dist/jco.js"),
        "unexpected jco entry point"
    );
    let cache = std::env::var_os("WISP_NPM_CACHE")
        .or_else(|| std::env::var_os("npm_config_cache"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".npm")))
        .context("set WISP_NPM_CACHE to the populated npm cache")?;
    let lock = serde_json::from_str(include_str!("../assets/setup-tooling/package-lock.json"))?;
    authenticate_npm_tree(prefix, &cache, &lock)
}

pub fn node() -> Result<PathBuf> {
    let path = std::env::var_os("WISP_NODE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join("node"))
                    .find(|path| path.is_file())
            })
        })
        .context("set WISP_NODE to pinned Node 24.20.0")?;
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "Linux-x86_64",
        ("macos", "aarch64") => "Darwin-arm64",
        _ => anyhow::bail!("unsupported library generator platform"),
    };
    let lock: Value = serde_json::from_str(include_str!("../assets/library-tools.lock.json"))?;
    let path = path.canonicalize()?;
    ensure!(
        format!("{:x}", Sha256::digest(bounded(&path)?))
            == lock["platforms"][platform]["node_sha256"]
                .as_str()
                .context("Node digest")?,
        "Node executable digest mismatch; use the pinned library setup or set WISP_NODE"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticates_transitive_generator_bytes_and_rejects_injection() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("tools");
        let cache = temp.path().join("cache");
        let relative = "node_modules/generator/index.js";
        let file = prefix.join(relative);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "export const trusted = 1;\n").unwrap();
        let bytes = crate::release::tar(&crate::release::Files::from([(
            "package/index.js".into(),
            fs::read(&file).unwrap(),
        )]))
        .unwrap();
        let hash = Sha512::digest(&bytes);
        let integrity = format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(hash)
        );
        let hex = format!("{hash:x}");
        let archive = cache
            .join("_cacache/content-v2/sha512")
            .join(&hex[..2])
            .join(&hex[2..4])
            .join(&hex[4..]);
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(&archive, bytes).unwrap();
        let lock =
            serde_json::json!({"packages":{"node_modules/generator":{"integrity":integrity}}});
        authenticate_npm_tree(&prefix, &cache, &lock).unwrap();
        fs::write(&file, "export const injected = 1;\n").unwrap();
        assert!(
            authenticate_npm_tree(&prefix, &cache, &lock)
                .unwrap_err()
                .to_string()
                .contains("generator file differs")
        );
        fs::write(&file, "export const trusted = 1;\n").unwrap();
        fs::write(prefix.join("node_modules/injected.js"), "injected").unwrap();
        assert!(
            authenticate_npm_tree(&prefix, &cache, &lock)
                .unwrap_err()
                .to_string()
                .contains("unlocked generator file")
        );
        fs::remove_file(prefix.join("node_modules/injected.js")).unwrap();
        fs::write(&archive, "modified cache").unwrap();
        assert!(
            authenticate_npm_tree(&prefix, &cache, &lock)
                .unwrap_err()
                .to_string()
                .contains("archive integrity mismatch")
        );
    }
}
