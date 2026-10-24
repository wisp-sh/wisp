use sha2::Digest;
// Runtime files carried by generated language packages.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};

pub const MAX_WORKER_BYTES: usize = 256 * 1024 * 1024;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Manifest {
    schema: u32,
    protocol: u32,
    worker_version: String,
    wasmtime_version: String,
    artifacts: BTreeMap<String, Artifact>,
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: String,
    sha256: String,
}

fn manifest(bytes: &[u8]) -> Result<Manifest> {
    ensure!(bytes.len() <= 16 * 1024, "worker manifest size limit");
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    // This also rejects duplicate keys and noncanonical encodings.
    ensure!(
        serde_json::to_vec_pretty(&serde_json::to_value(&manifest)?)? == bytes,
        "noncanonical worker manifest"
    );
    ensure!(
        manifest.schema == 1
            && manifest.protocol == crate::VERSION
            && manifest.worker_version == "0.2.0"
            && manifest.wasmtime_version == crate::WASMTIME_VERSION,
        "unsupported bundled worker version"
    );
    ensure!(
        !manifest.artifacts.is_empty() && manifest.artifacts.len() <= 2,
        "invalid worker artifact count"
    );
    for (platform, artifact) in &manifest.artifacts {
        ensure!(
            matches!(platform.as_str(), "linux-amd64" | "darwin-arm64"),
            "unsupported worker platform"
        );
        ensure!(
            artifact.path == format!("worker/{platform}/wisp-worker"),
            "invalid worker artifact path"
        );
        ensure!(
            artifact.sha256.len() == 64
                && artifact
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid worker artifact checksum"
        );
    }
    Ok(manifest)
}

/// Check release-carried integrity without resolving or executing a local worker.
pub fn validate(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<String>> {
    let manifest = manifest(
        files
            .get("worker/manifest.json")
            .context("missing worker manifest")?,
    )?;
    ensure!(
        files.len() == manifest.artifacts.len() + 1,
        "unexpected worker bundle files"
    );
    for artifact in manifest.artifacts.values() {
        let bytes = files
            .get(&artifact.path)
            .context("missing worker artifact")?;
        ensure!(
            !bytes.is_empty() && bytes.len() < MAX_WORKER_BYTES,
            "worker exceeds bundle limit"
        );
        ensure!(
            artifact.sha256 == format!("{:x}", sha2::Sha256::digest(bytes)),
            "worker artifact checksum mismatch"
        );
    }
    Ok(manifest.artifacts.into_keys().collect())
}

/// Trust comes from an independently selected manifest, never the uploaded one.
pub fn authenticate_with_manifest(files: &BTreeMap<String, Vec<u8>>, trusted: &[u8]) -> Result<()> {
    validate(files)?;
    let actual = manifest(&files["worker/manifest.json"])?;
    let trusted = manifest(trusted)?;
    for (platform, artifact) in actual.artifacts {
        ensure!(
            trusted
                .artifacts
                .get(&platform)
                .is_some_and(|pin| pin.sha256 == artifact.sha256),
            "untrusted worker artifact for {platform}; configure WISP_WORKER_TRUST_MANIFEST with independently verified runtime hashes"
        );
    }
    Ok(())
}

/// Servers may pin foreign-platform artifacts without installing those binaries.
/// Local development defaults to the explicitly selected or installed worker.
pub fn authenticate(files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    let trusted = if let Some(path) = std::env::var_os("WISP_WORKER_TRUST_MANIFEST") {
        let file = fs::File::open(path).context("opening WISP_WORKER_TRUST_MANIFEST")?;
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
        bytes
    } else {
        self::files()
            .context("install a worker or configure WISP_WORKER_TRUST_MANIFEST for admission")?
            .remove("worker/manifest.json")
            .context("local worker manifest")?
    };
    authenticate_with_manifest(files, &trusted)
}

pub fn files() -> Result<BTreeMap<String, Vec<u8>>> {
    let mut workers = BTreeMap::new();
    if let Some(root) = std::env::var_os("WISP_WORKER_BUNDLE_DIR") {
        let root = PathBuf::from(root);
        for platform in ["linux-amd64", "darwin-arm64"] {
            let path = root.join(platform).join("wisp-worker");
            if path.is_file() {
                workers.insert(platform.to_owned(), path);
            }
        }
    } else {
        let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => "linux-amd64",
            ("macos", "aarch64") => "darwin-arm64",
            _ => anyhow::bail!("no bundled worker for this platform"),
        };
        workers.insert(platform.to_owned(), resolve()?);
    }
    ensure!(
        !workers.is_empty(),
        "WISP_WORKER_BUNDLE_DIR contains no supported workers"
    );
    let mut files = BTreeMap::new();
    let mut artifacts = BTreeMap::new();
    for (platform, path) in workers {
        let bytes = fs::read(&path).with_context(|| format!("bundling {}", path.display()))?;
        ensure!(
            !bytes.is_empty() && bytes.len() < MAX_WORKER_BYTES,
            "worker exceeds bundle limit"
        );
        let path = format!("worker/{platform}/wisp-worker");
        artifacts.insert(
            platform,
            json!({"path":path,"sha256":format!("{:x}", sha2::Sha256::digest(&bytes))}),
        );
        files.insert(path, bytes);
    }
    files.insert("worker/manifest.json".into(), serde_json::to_vec_pretty(&json!({
        "schema":1,"protocol":1,"workerVersion":"0.2.0","wasmtimeVersion":crate::WASMTIME_VERSION,"artifacts":artifacts
    }))?);
    validate(&files)?;
    Ok(files)
}

pub fn resolve() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WISP_WORKER") {
        return PathBuf::from(path).canonicalize().context("WISP_WORKER");
    }
    let sibling = std::env::current_exe()?.with_file_name(if cfg!(windows) {
        "wisp-worker.exe"
    } else {
        "wisp-worker"
    });
    if sibling.is_file() {
        return Ok(sibling);
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let path = dir.join("wisp-worker");
        if path.is_file() {
            return Ok(path.canonicalize()?);
        }
    }
    anyhow::bail!("install wisp-worker beside wisp or set WISP_WORKER")
}
