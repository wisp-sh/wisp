//! Third-party notices for the Go package's embedded Rust static library.
//!
//! The notices are collected while packing instead of being vendored. Every
//! registry archive in the pinned shim lock is read from the local Cargo cache
//! and authenticated against its lock checksum. The Rust standard library
//! notices come from the pinned toolchain and are checked against a fixed hash.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::PathBuf, process::Command};

const SHIM_LOCK: &str = include_str!("../assets/library-go-Cargo.lock");
const RUST_TOOLCHAIN: &str = "1.98.1";
const RUST_VERSION: &str = "rustc 1.98.1 (48a229cea 2026-09-01)";
const RUST_COPYRIGHT_PATH: &str = "share/doc/rust/COPYRIGHT-library.html";
const RUST_COPYRIGHT_SHA256: &str =
    "68129500b616d5838629e68f55ff3aed5e096dacf60ce9eb41bbe599a563afa6";
const NOTICE_LIMIT: u64 = 4 * 1024 * 1024;
const NOTICE_PREFIXES: [&str; 5] = ["LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bundle {
    schema: u32,
    cargo_lock_sha256: String,
    scope: &'static str,
    packages: Vec<Package>,
    rust_toolchain: Toolchain,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Package {
    name: String,
    version: String,
    source: String,
    archive_sha256: String,
    license: Option<String>,
    license_file: Option<String>,
    notices: Vec<Notice>,
}

#[derive(Serialize)]
struct Toolchain {
    version: String,
    notices: Vec<Notice>,
    status: &'static str,
}

#[derive(Serialize)]
struct Notice {
    path: String,
    sha256: String,
    text: String,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Render `WISP-GO-LICENSES.json` for the pinned shim graph.
pub fn bundle() -> Result<Vec<u8>> {
    let bundle = Bundle {
        schema: 1,
        cargo_lock_sha256: sha256(SHIM_LOCK.as_bytes()),
        scope: "all registry packages in the pinned Go static library Cargo lock, including target-specific packages",
        packages: packages()?,
        rust_toolchain: toolchain()?,
    };
    let mut data = serde_json::to_vec_pretty(&bundle)?;
    data.push(b'\n');
    Ok(data)
}

fn packages() -> Result<Vec<Package>> {
    let lock: toml::Value = toml::from_str(SHIM_LOCK).context("parse Go shim Cargo lock")?;
    let mut locked = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .context("Go shim lock packages")?
        .iter()
        .filter(|package| package.get("checksum").is_some())
        .map(|package| {
            let field = |key: &str| {
                package
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .with_context(|| format!("Go shim lock package {key}"))
            };
            Ok((
                field("name")?,
                field("version")?,
                field("source")?,
                field("checksum")?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    locked.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    let cache = cargo_home()?.join("registry/cache");
    let mut missing = Vec::new();
    let mut packages = Vec::new();
    for (name, version, source, checksum) in locked {
        let filename = format!("{name}-{version}.crate");
        let Some(archive) = cached_archive(&cache, &filename, &checksum)? else {
            missing.push(filename);
            continue;
        };
        let (license, license_file, notices) = notices(&archive, &format!("{name}-{version}/"))
            .with_context(|| format!("read notices from {filename}"))?;
        packages.push(Package {
            name,
            version,
            source,
            archive_sha256: checksum,
            license,
            license_file,
            notices,
        });
    }
    ensure!(
        missing.is_empty(),
        "Go package notices need every crate in the pinned shim lock; {} archive(s) are \
         missing from the Cargo cache or fail their lock checksum ({}). Fetch the lock with \
         `cargo fetch --locked` while online",
        missing.len(),
        missing.join(", ")
    );
    Ok(packages)
}

fn cargo_home() -> Result<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .context("Cargo home is required")
}

fn cached_archive(
    cache: &std::path::Path,
    filename: &str,
    checksum: &str,
) -> Result<Option<Vec<u8>>> {
    let Ok(registries) = fs::read_dir(cache) else {
        return Ok(None);
    };
    let mut candidates = registries
        .map(|entry| Ok(entry?.path().join(filename)))
        .collect::<std::io::Result<Vec<_>>>()?;
    candidates.sort();
    for candidate in candidates {
        if let Ok(bytes) = fs::read(&candidate)
            && sha256(&bytes) == checksum
        {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

type PackageNotices = (Option<String>, Option<String>, Vec<Notice>);

fn notices(archive: &[u8], prefix: &str) -> Result<PackageNotices> {
    let mut members = Vec::new();
    let mut entries = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in entries.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry.path()?.to_string_lossy().into_owned();
        let Some(relative) = name.strip_prefix(prefix).map(str::to_owned) else {
            continue;
        };
        let size = entry.size();
        let mut content = Vec::new();
        if size <= NOTICE_LIMIT {
            entry.read_to_end(&mut content)?;
        }
        members.push((name, relative, size, content));
    }
    let manifest = members
        .iter()
        .rev()
        .find(|(_, relative, _, _)| relative == "Cargo.toml")
        .context("crate archive lacks Cargo.toml")?;
    let manifest: toml::Value = toml::from_str(std::str::from_utf8(&manifest.3)?)?;
    let metadata = manifest.get("package").context("crate manifest package")?;
    let string = |key: &str| -> Result<Option<String>> {
        match metadata.get(key) {
            None => Ok(None),
            Some(value) => Ok(Some(
                value
                    .as_str()
                    .with_context(|| format!("crate manifest {key} is not a string"))?
                    .to_owned(),
            )),
        }
    };
    let license = string("license")?;
    let license_file = string("license-file")?;
    members.sort_by(|left, right| left.0.cmp(&right.0));
    let mut notices = Vec::new();
    for (name, relative, size, content) in members {
        if !(is_notice_name(&relative) || license_file.as_deref() == Some(relative.as_str())) {
            continue;
        }
        if size > NOTICE_LIMIT {
            bail!("oversized license text: {name}");
        }
        notices.push(Notice {
            path: relative,
            sha256: sha256(&content),
            text: String::from_utf8(content).context("license text is not UTF-8")?,
        });
    }
    Ok((license, license_file, notices))
}

fn is_notice_name(relative: &str) -> bool {
    let basename = relative
        .rsplit('/')
        .next()
        .unwrap_or(relative)
        .to_uppercase();
    NOTICE_PREFIXES
        .iter()
        .any(|prefix| basename.starts_with(prefix))
}

fn toolchain() -> Result<Toolchain> {
    let rustc = |arguments: &[&str]| -> Result<String> {
        let output = Command::new("rustc")
            .args(arguments)
            .env("RUSTUP_TOOLCHAIN", RUST_TOOLCHAIN)
            .env("RUSTUP_AUTO_INSTALL", "0")
            .output()
            .context("run the pinned Rust compiler")?;
        ensure!(
            output.status.success(),
            "rustc {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    };
    let version = rustc(&["--version"])?;
    ensure!(
        version == RUST_VERSION,
        "unexpected Rust compiler: {version}"
    );
    let path = PathBuf::from(rustc(&["--print", "sysroot"])?).join(RUST_COPYRIGHT_PATH);
    let data = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        sha256(&data) == RUST_COPYRIGHT_SHA256,
        "pinned Rust standard library copyright checksum mismatch: {}",
        path.display()
    );
    Ok(Toolchain {
        version,
        notices: vec![Notice {
            path: RUST_COPYRIGHT_PATH.into(),
            sha256: RUST_COPYRIGHT_SHA256.into(),
            text: String::from_utf8(data).context("Rust copyright notice is not UTF-8")?,
        }],
        status: "pinned Rust distribution copyright files included",
    })
}

#[cfg(test)]
mod tests {
    /// Requires the pinned toolchain and a Cargo cache holding the full shim lock.
    /// Set `WISP_DUMP_GO_LICENSES=PATH` to write the bundle for review.
    #[test]
    #[ignore = "needs Rust 1.98.1 and the fetched Go shim Cargo graph"]
    fn bundle_covers_the_pinned_shim_graph() {
        let bundle = super::bundle().unwrap();
        if let Some(path) = std::env::var_os("WISP_DUMP_GO_LICENSES") {
            std::fs::write(path, &bundle).unwrap();
        }
        assert_eq!(
            super::sha256(&bundle),
            "e48b2763c8913ab49384109103649c21d1e35929961201477b7c0a172d694494",
            "update this digest only after reviewing the changed notices"
        );
    }
}
