//! Versioned, immutable compiler installations and their Wisp-owned execution hosts.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_BUNDLE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_JSON: u64 = 16 * 1024 * 1024;
pub(crate) const ADAPTER_VERSION: &str = "50.0.0";
pub(crate) const ADAPTER_SHA256: &str =
    "73b01f85e42e99c890efbbf619cd765e04e5809da36d976b87fc64abd87f5812";
const ADAPTER_URL: &str = "https://github.com/bytecodealliance/wasmtime/releases/download/v50.0.0/wasi_snapshot_preview1.reactor.wasm";
const ADAPTER_FILE: &str = "wasi_snapshot_preview1.reactor.wasm";
const RECIPES: &[(&str, &[u8])] = &[
    (
        "build.py",
        include_bytes!("../../../tools/shared-hosts/build.py"),
    ),
    (
        "wasmtime.lock.json",
        include_bytes!("../../../tools/shared-hosts/wasmtime.lock.json"),
    ),
    (
        "component-preload.patch",
        include_bytes!("../../../tools/shared-hosts/component-preload.patch"),
    ),
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Pin {
    version: String,
    archive_sha256: String,
    bundle_sha256: String,
    manifest_sha256: String,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    schema: u32,
    packages: BTreeMap<String, Pin>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    identity: String,
    pin: Pin,
}

fn hash(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut sha = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        sha.update(&bytes[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}
fn bytes_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn json_file(path: &Path) -> Result<Value> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "installation JSON must be a regular file: {}",
        path.display()
    );
    let file = fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= MAX_JSON,
        "installation JSON size limit"
    );
    let mut bytes = Vec::new();
    file.take(MAX_JSON + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_JSON,
        "installation JSON size limit"
    );
    crate::release::strict_json(&bytes)
}
fn real_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "installation directory is a symlink or special file: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)?,
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
fn home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WISP_TOOLCHAIN_HOME") {
        return Ok(path.into());
    }
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path).join("wisp/toolchains"));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("set WISP_TOOLCHAIN_HOME or HOME")?)
            .join(".local/share/wisp/toolchains"),
    )
}
fn prefix(path: &Path) -> Result<PathBuf> {
    fs::create_dir_all(path)?;
    let path = path.canonicalize()?;
    real_directory(&path)?;
    Ok(path)
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}
fn lock(prefix: &Path) -> Result<Lock> {
    let path = prefix.join(".install-lock");
    fs::create_dir(&path).with_context(|| format!(
        "another toolchain operation holds {}; retry when it finishes, or remove the directory after a crashed operation", path.display()))?;
    Ok(Lock(path))
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("JSON parent")?)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file_mut().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}
fn identity(value: &str) -> Result<&str> {
    let (namespace, name) = crate::identity::parse(value)?;
    ensure!(namespace == "wisp", "toolchain namespace must be wisp");
    Ok(name)
}
fn validate_pin(pin: &Pin) -> Result<()> {
    ensure!(
        semver::Version::parse(&pin.version)?.to_string() == pin.version,
        "noncanonical installed version"
    );
    for hash in [
        &pin.bundle_sha256,
        &pin.archive_sha256,
        &pin.manifest_sha256,
    ] {
        ensure!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "invalid installed SHA-256"
        );
    }
    Ok(())
}
fn selection(prefix: &Path) -> Result<Selection> {
    let path = prefix.join("selection.json");
    if !path.try_exists()? {
        return Ok(Selection {
            schema: 1,
            ..Selection::default()
        });
    }
    ensure!(
        fs::symlink_metadata(&path)?.is_file()
            && !fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "selection must be a regular file"
    );
    let selected: Selection = serde_json::from_value(json_file(&path)?)?;
    ensure!(
        selected.schema == 1 && selected.packages.len() <= 64,
        "unsupported toolchain selection"
    );
    for (id, pin) in &selected.packages {
        identity(id)?;
        validate_pin(pin)?;
    }
    Ok(selected)
}
fn installation(prefix: &Path, id: &str, version: &str) -> Result<PathBuf> {
    let name = identity(id)?;
    ensure!(
        semver::Version::parse(version)?.to_string() == version,
        "noncanonical installed version"
    );
    let path = prefix.join("packages").join(name).join(version);
    for path in [
        prefix.join("packages"),
        prefix.join("packages").join(name),
        path.clone(),
    ] {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("missing installed toolchain {}", path.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid installation directory: {}",
            path.display()
        );
    }
    Ok(path)
}
fn visit(root: &Path, directory: &Path, paths: &mut BTreeSet<String>) -> Result<()> {
    ensure!(
        paths.len() <= 50_000,
        "installed toolchain file count limit"
    );
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "symlink in installed toolchain: {}",
            entry.path().display()
        );
        if kind.is_dir() {
            visit(root, &entry.path(), paths)?;
        } else {
            ensure!(kind.is_file(), "special file in installed toolchain");
            paths.insert(
                entry
                    .path()
                    .strip_prefix(root)?
                    .to_str()
                    .context("UTF-8 installed path")?
                    .to_owned(),
            );
        }
    }
    Ok(())
}
fn verify(prefix: &Path, id: &str, pin: &Pin) -> Result<(PathBuf, Value)> {
    validate_pin(pin)?;
    let directory = installation(prefix, id, &pin.version)?;
    let names = fs::read_dir(&directory)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    ensure!(
        names
            == ["payload", "receipt.json", "release.json"]
                .into_iter()
                .map(std::ffi::OsString::from)
                .collect(),
        "installed toolchain metadata file set mismatch"
    );
    let manifest_path = directory.join("release.json");
    for path in [&manifest_path, &directory.join("receipt.json")] {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "installed metadata is not a regular file"
        );
    }
    ensure!(
        hash(&manifest_path)? == pin.manifest_sha256,
        "installed toolchain manifest SHA-256 mismatch"
    );
    let record = json_file(&manifest_path)?;
    crate::toolchain_package::validate_metadata(&record)?;
    let receipt: Receipt = serde_json::from_value(json_file(&directory.join("receipt.json"))?)?;
    ensure!(
        receipt.schema == 1 && receipt.identity == id && receipt.pin == *pin,
        "installed toolchain receipt differs from selection"
    );
    ensure!(
        record["identity"] == id
            && record["version"] == pin.version
            && record["archive"]["sha256"] == pin.archive_sha256,
        "installed toolchain identity or archive digest mismatch"
    );
    let root = directory.join("payload");
    let metadata = fs::symlink_metadata(&root)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "installed payload root must be a real directory"
    );
    let mut actual = BTreeSet::new();
    visit(&root, &root, &mut actual)?;
    let inventory = record["archive"]["inventory"].as_object().unwrap();
    ensure!(
        actual == inventory.keys().cloned().collect(),
        "installed toolchain file set differs from inventory"
    );
    for (path, expected) in inventory {
        let path = root.join(path);
        let metadata = fs::metadata(&path)?;
        ensure!(
            metadata.len() == expected["size"].as_u64().unwrap()
                && hash(&path)? == expected["sha256"],
            "installed toolchain checksum or size mismatch: {}",
            path.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o7777 == expected["mode"].as_u64().unwrap() as u32,
                "installed toolchain mode mismatch: {}",
                path.display()
            );
        }
    }
    Ok((root, record))
}
fn extract(bundle: &Path, root: &Path) -> Result<()> {
    fs::create_dir(root)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(bundle)?)?;
    let decoder = flate2::read::GzDecoder::new(zip.by_name("toolchain.tar.gz")?);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.into_owned();
        let name = name
            .to_str()
            .context("UTF-8 archive member")?
            .strip_prefix("wisp-buildtools/")
            .context("toolchain archive root")?;
        crate::release::safe_name(name)?;
        ensure!(
            entry.header().entry_type().is_file(),
            "nonregular admitted archive member"
        );
        let path = root.join(name);
        fs::create_dir_all(path.parent().context("payload parent")?)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        std::io::copy(&mut entry, &mut file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(entry.header().mode()?))?;
        }
    }
    Ok(())
}
fn install(bundle: &Path, root: &Path, expected: Option<&str>) -> Result<Value> {
    let root = prefix(root)?;
    let _lock = lock(&root)?;
    let stage = tempfile::tempdir_in(&root)?;
    let snapshot = stage.path().join("bundle.wisp.zip");
    let source = fs::File::open(bundle)?;
    ensure!(
        source.metadata()?.len() <= MAX_BUNDLE,
        "toolchain package size limit"
    );
    let mut target = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&snapshot)?;
    ensure!(
        std::io::copy(&mut source.take(MAX_BUNDLE + 1), &mut target)? <= MAX_BUNDLE,
        "toolchain package size limit"
    );
    target.sync_all()?;
    let bundle_sha256 = hash(&snapshot)?;
    if let Some(expected) = expected {
        ensure!(
            bundle_sha256 == expected,
            "toolchain bundle SHA-256 does not match pin"
        );
    }
    // A private snapshot prevents the caller's source changing between validation and extraction.
    let record = crate::toolchain_package::admit(&snapshot)?;
    ensure!(
        record["platform"] == "wasm32-wasi"
            || (std::env::consts::OS == "linux" && std::env::consts::ARCH == "x86_64"),
        "native toolchain requires Linux x86_64"
    );
    let id = record["identity"].as_str().unwrap();
    let name = identity(id)?;
    let manifest = serde_json::to_vec_pretty(&record)?;
    let pin = Pin {
        version: record["version"].as_str().unwrap().into(),
        archive_sha256: record["archive"]["sha256"].as_str().unwrap().into(),
        bundle_sha256,
        manifest_sha256: bytes_hash(&manifest),
    };
    let packages = root.join("packages");
    real_directory(&packages)?;
    let versions = packages.join(name);
    real_directory(&versions)?;
    let destination = versions.join(&pin.version);
    let mut selected = selection(&root)?;
    let mut reused = false;
    if fs::symlink_metadata(&destination).is_ok() {
        let stored: Receipt =
            serde_json::from_value(json_file(&destination.join("receipt.json"))?)?;
        ensure!(
            stored.pin == pin && stored.identity == id,
            "toolchain version already exists with a different digest: {id}@{}",
            pin.version
        );
        verify(&root, id, &pin)?;
        reused = true;
    } else {
        let install = stage.path().join("install");
        fs::create_dir(&install)?;
        // Nothing from the package is written or executed until full admission succeeds.
        extract(&snapshot, &install.join("payload"))?;
        fs::write(install.join("release.json"), &manifest)?;
        atomic_json(
            &install.join("receipt.json"),
            &Receipt {
                schema: 1,
                identity: id.into(),
                pin: pin.clone(),
            },
        )?;
        fs::rename(&install, &destination)?;
        if let Err(error) = verify(&root, id, &pin) {
            let _ = fs::remove_dir_all(&destination);
            return Err(error);
        }
    }
    selected.packages.insert(id.into(), pin.clone());
    atomic_json(&root.join("selection.json"), &selected)?;
    Ok(
        json!({"installed":true,"selected":true,"reused":reused,"identity":id,"version":pin.version,"pin":pin,"path":destination,"prefix":root}),
    )
}
fn select(spec: &str, root: &Path) -> Result<Value> {
    let (id, version) = spec
        .rsplit_once('@')
        .context("select requires wisp:tool@VERSION")?;
    identity(id)?;
    let root = prefix(root)?;
    let _lock = lock(&root)?;
    let directory = installation(&root, id, version)?;
    let receipt: Receipt = serde_json::from_value(json_file(&directory.join("receipt.json"))?)?;
    ensure!(
        receipt.schema == 1 && receipt.identity == id && receipt.pin.version == version,
        "installed toolchain receipt mismatch"
    );
    verify(&root, id, &receipt.pin)?;
    let mut selected = selection(&root)?;
    selected.packages.insert(id.into(), receipt.pin.clone());
    atomic_json(&root.join("selection.json"), &selected)?;
    Ok(json!({"selected":true,"identity":id,"version":version,"pin":receipt.pin}))
}
fn entrypoint(record: &Value, name: &str) -> Result<PathBuf> {
    record["entrypoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .and_then(|entry| entry["path"].as_str())
        .map(PathBuf::from)
        .with_context(|| format!("installed package has no {name} entrypoint"))
}
fn package_path(root: &Path, selected: &Selection, environment: &str) -> Result<Option<PathBuf>> {
    let (id, entry, depth) = match environment {
        "WISP_CLANG" | "WISP_CLANG_RESOURCE_DIR" | "WISP_CLANG_WASI_SYSROOT" => {
            ("wisp:clang", "clang", 0)
        }
        "WISP_GO_TOOLCHAIN" => ("wisp:go", "compile", 2),
        "WISP_LLVM" => ("wisp:llvm", "llvm24", 2),
        "WISP_LLVM_TOOLCHAIN" => ("wisp:llvm", "llvm24", 4),
        "WISP_RUST" => ("wisp:rust", "rustc", 3),
        "WISP_ZIG_TOOLCHAIN" => ("wisp:zig", "build", 1),
        "WISP_ASSEMBLYSCRIPT_TOOLCHAIN" => ("wisp:assemblyscript", "build", 1),
        "WISP_NIM_TOOLCHAIN" => ("wisp:nim", "build", 1),
        "WISP_JAVASCRIPT_TOOLCHAIN" | "WISP_TYPESCRIPT_TOOLCHAIN" => {
            ("wisp:javascript", "build", 1)
        }
        "WISP_PYTHON_TOOLCHAIN" => ("wisp:python", "build", 1),
        "WISP_TSRS_TOOLCHAIN" => ("wisp:tsrs", "tsrs", 2),
        "WISP_WIZER_TOOLCHAIN" => ("wisp:wizer", "snapshot", 1),
        "WISP_GRAIN_TOOLCHAIN" => ("wisp:grain", "build", 1),
        "WISP_BEND_TOOLCHAIN" => ("wisp:bend", "build", 1),
        "WISP_GOOSE_TOOLCHAIN" => ("wisp:goose", "build", 1),
        "WISP_SWIFT_TOOLCHAIN" => ("wisp:swift", "build", 1),
        "WISP_JAVA_TOOLCHAIN" => ("wisp:java", "build", 1),
        "WISP_KOTLIN_TOOLCHAIN" => ("wisp:kotlin", "build", 1),
        "WISP_LLVM_SWIFT" => ("wisp:llvm-swift", "swift-llvm", 1),
        "WISP_JVM_TOOLCHAIN" => ("wisp:jvm", "jvm", 2),
        "WISP_TEAVM_TOOLCHAIN" => ("wisp:teavm", "build", 1),
        "WISP_WASM_TOOLS" => ("wisp:wasm-tools", "wasm-tools", 0),
        "WISP_WASM_TOOLS_TOOLCHAIN" => ("wisp:wasm-tools", "wasm-tools", 2),
        "WISP_PREVIEW1_ADAPTER" => ("wisp:wasm-tools", "preview1-reactor-adapter", 0),
        _ => return Ok(None),
    };
    let Some(pin) = selected.packages.get(id) else {
        return Ok(None);
    };
    let (payload, record) = verify(root, id, pin)?;
    if environment == "WISP_CLANG_WASI_SYSROOT" {
        let sysroot = payload.join("clang/wasi-sdk/share/wasi-sysroot");
        return Ok(sysroot
            .join("lib/wasm32-wasip1/libc.a")
            .is_file()
            .then_some(sysroot));
    }
    if environment == "WISP_CLANG_RESOURCE_DIR" {
        let include = "clang/include";
        if record["archive"]["inventory"]
            .as_object()
            .unwrap()
            .keys()
            .any(|path| path.starts_with("clang/include/"))
        {
            return Ok(Some(payload.join(include)));
        }
        return Ok(None);
    }
    let relative = entrypoint(&record, entry)?;
    let path = if depth > 0 {
        // Derive consumer roots from the admitted release entrypoint, not a checkout path.
        relative
            .ancestors()
            .nth(depth)
            .context("compiler package entrypoint layout")?
            .to_path_buf()
    } else {
        relative
    };
    let path = payload.join(path);
    ensure!(
        path.try_exists()?,
        "installed compiler entrypoint path is missing"
    );
    Ok(Some(path))
}

pub(crate) fn resolve(environment: &str) -> Result<Option<PathBuf>> {
    let root = home()?;
    resolve_in(&root, environment)
}
/// Select a host for a known compiler profile without consulting the globally
/// selected compiler, which a project or environment override may replace.
pub(crate) fn resolve_host(version: &str) -> Result<Option<PathBuf>> {
    resolve_host_in(&home()?, version)
}
fn resolve_host_in(root: &Path, version: &str) -> Result<Option<PathBuf>> {
    let path = root
        .join("hosts")
        .join(format!("wasmtime-{version}"))
        .join("wasmtime");
    if !path.try_exists()? {
        return Ok(None);
    }
    verify_host(&path, version)?;
    Ok(Some(path.canonicalize()?))
}
fn resolve_in(root: &Path, environment: &str) -> Result<Option<PathBuf>> {
    if environment == "WISP_PREVIEW1_ADAPTER" {
        let path = adapter_path(root);
        if path.try_exists()? {
            verify_adapter(root)?;
            return Ok(Some(path));
        }
        let path = package_path(root, &selection(root)?, environment)?;
        if let Some(path) = &path {
            ensure!(
                hash(path)? == ADAPTER_SHA256,
                "installed wasm-tools adapter is incompatible with Wisp's pinned Wasmtime {ADAPTER_VERSION} reactor adapter; run wisp toolchain setup-hosts --host adapter"
            );
        }
        return Ok(path);
    }
    if ["WISP_WASMTIME", "WISP_RUST_WASMTIME"].contains(&environment) {
        if environment == "WISP_RUST_WASMTIME" {
            if let Some(rust) = package_path(root, &selection(root)?, "WISP_RUST")? {
                crate::build_rust::Toolchain::compiler_host(&rust)?;
            }
        }
        return resolve_host_in(root, crate::compiler_hosts::COMMAND_VERSION);
    }
    package_path(root, &selection(root)?, environment)
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn environment(root: &Path) -> Result<String> {
    let root = root.canonicalize()?;
    let mut text = format!(
        "export WISP_TOOLCHAIN_HOME={}\n",
        shell_quote(root.to_str().context("UTF-8 toolchain prefix")?)
    );
    for name in [
        "WISP_CLANG",
        "WISP_CLANG_RESOURCE_DIR",
        "WISP_CLANG_WASI_SYSROOT",
        "WISP_GO_TOOLCHAIN",
        "WISP_LLVM",
        "WISP_LLVM_TOOLCHAIN",
        "WISP_RUST",
        "WISP_ZIG_TOOLCHAIN",
        "WISP_ASSEMBLYSCRIPT_TOOLCHAIN",
        "WISP_NIM_TOOLCHAIN",
        "WISP_JAVASCRIPT_TOOLCHAIN",
        "WISP_TYPESCRIPT_TOOLCHAIN",
        "WISP_TSRS_TOOLCHAIN",
        "WISP_WIZER_TOOLCHAIN",
        "WISP_PYTHON_TOOLCHAIN",
        "WISP_GRAIN_TOOLCHAIN",
        "WISP_GOOSE_TOOLCHAIN",
        "WISP_BEND_TOOLCHAIN",
        "WISP_SWIFT_TOOLCHAIN",
        "WISP_JAVA_TOOLCHAIN",
        "WISP_KOTLIN_TOOLCHAIN",
        "WISP_LLVM_SWIFT",
        "WISP_JVM_TOOLCHAIN",
        "WISP_TEAVM_TOOLCHAIN",
        "WISP_PREVIEW1_ADAPTER",
        "WISP_WASM_TOOLS",
        "WISP_WASM_TOOLS_TOOLCHAIN",
        "WISP_WASMTIME",
        "WISP_RUST_WASMTIME",
    ] {
        if let Some(path) = resolve_in(&root, name)? {
            text.push_str(&format!(
                "export {name}={}\n",
                shell_quote(path.to_str().context("UTF-8 toolchain path")?)
            ));
        }
    }
    Ok(text)
}
fn host_recipe(version: &str) -> Result<(&'static str, &'static [u8], &'static [u8])> {
    let (script, lock, patch) = match version {
        crate::compiler_hosts::COMMAND_VERSION => {
            ("build.py", "wasmtime.lock.json", "component-preload.patch")
        }
        _ => bail!("unsupported Wisp host version"),
    };
    let data = |name| RECIPES.iter().find(|(path, _)| *path == name).unwrap().1;
    Ok((script, data(lock), data(patch)))
}
fn verify_host(path: &Path, version: &str) -> Result<()> {
    let directory = path.parent().context("host parent")?;
    let (_, lock, patch) = host_recipe(version)?;
    for parent in [
        directory.to_owned(),
        directory
            .parent()
            .context("host installation parent")?
            .to_owned(),
    ] {
        let metadata = fs::symlink_metadata(parent)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid Wisp execution host directory"
        );
    }
    for path in [path.to_owned(), directory.join("receipt.json")] {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Wisp execution host must contain regular files"
        );
    }
    let receipt = json_file(&directory.join("receipt.json"))?;
    ensure!(
        receipt["schema"] == 1
            && receipt["owner"] == "wisp"
            && receipt["version"] == version
            && receipt["source"] == serde_json::from_slice::<Value>(lock)?
            && receipt["patchSha256"] == bytes_hash(patch)
            && receipt["binarySha256"] == hash(path)?,
        "Wisp execution host receipt, pinned recipe or binary mismatch"
    );
    let smoke = &receipt["smoke"];
    ensure!(
        smoke["externalModuleImport"] == "wisp-test-provider"
            && smoke["value"] == 42
            && smoke["missingProviderRejected"] == true,
        "Wisp command host lacks Component core-module import proof"
    );
    Ok(())
}
fn recipes(directory: &Path) -> Result<()> {
    for (name, bytes) in RECIPES {
        fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}
fn check_host(path: &Path, version: &str) -> Result<Value> {
    verify_host(path, version)?;
    let temporary = tempfile::tempdir()?;
    recipes(temporary.path())?;
    let (script, _, _) = host_recipe(version)?;
    let output = Command::new("python3")
        .arg("-I")
        .arg(temporary.path().join(script))
        .arg("--check")
        .arg("--prefix")
        .arg(path.parent().unwrap())
        .output()?;
    ensure!(
        output.status.success(),
        "Wisp host capability smoke failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(
        json!({"available":true,"version":version,"path":path,"sha256":hash(path)?,"proof":serde_json::from_slice::<Value>(&output.stdout)?}),
    )
}
fn hosts(root: &Path, run_smoke: bool) -> Value {
    let results = [crate::compiler_hosts::COMMAND_VERSION].into_iter().map(|version| {
        let path = root.join("hosts").join(format!("wasmtime-{version}")).join("wasmtime");
        let result = if run_smoke { check_host(&path, version) } else { verify_host(&path, version).map(|_| json!({"available":true,"version":version,"path":path})) };
        result.unwrap_or_else(|error| json!({"available":false,"version":version,"path":path,"error":format!("{error:#}")}))
    }).collect::<Vec<_>>();
    json!({"owner":"wisp","available":results.iter().all(|host| host["available"] == true),"hosts":results})
}
fn adapter_path(root: &Path) -> PathBuf {
    root.join("hosts")
        .join(format!("preview1-{ADAPTER_VERSION}"))
        .join(ADAPTER_FILE)
}
fn verify_adapter(root: &Path) -> Result<Value> {
    let path = adapter_path(root);
    let directory = path.parent().unwrap();
    for parent in [directory, directory.parent().unwrap()] {
        let metadata = fs::symlink_metadata(parent)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid preview1 adapter directory"
        );
    }
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 1024 * 1024,
        "preview1 adapter must be a bounded regular file"
    );
    ensure!(
        hash(&path)? == ADAPTER_SHA256,
        "preview1 adapter checksum mismatch"
    );
    let receipt = json_file(&directory.join("receipt.json"))?;
    ensure!(
        receipt
            == json!({"schema":1,"owner":"wisp","version":ADAPTER_VERSION,"url":ADAPTER_URL,"sha256":ADAPTER_SHA256}),
        "preview1 adapter receipt differs from the pinned source"
    );
    Ok(
        json!({"available":true,"version":ADAPTER_VERSION,"path":path,"sha256":ADAPTER_SHA256,"sizeBytes":metadata.len()}),
    )
}
fn adapter_status(root: &Path) -> Value {
    verify_adapter(root).unwrap_or_else(|error| json!({"available":false,"version":ADAPTER_VERSION,"path":adapter_path(root),"error":format!("{error:#}")}))
}
fn setup_adapter(root: &Path, cache: &Path) -> Result<Value> {
    let destination = adapter_path(root).parent().unwrap().to_owned();
    if destination.try_exists()? {
        return verify_adapter(root);
    }
    let downloads = cache.join("downloads");
    real_directory(&downloads)?;
    let cached = downloads.join(ADAPTER_SHA256);
    if !cached.try_exists()? {
        let mut temporary = tempfile::NamedTempFile::new_in(&downloads)?;
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        let mut response = client
            .get(ADAPTER_URL)
            .send()?
            .error_for_status()?
            .take(1024 * 1024 + 1);
        let size = std::io::copy(&mut response, &mut temporary)?;
        ensure!(size <= 1024 * 1024, "preview1 adapter download size limit");
        temporary.as_file_mut().sync_all()?;
        ensure!(
            hash(temporary.path())? == ADAPTER_SHA256,
            "preview1 adapter download checksum mismatch"
        );
        temporary.persist(&cached).map_err(|error| error.error)?;
    }
    let metadata = fs::symlink_metadata(&cached)?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= 1024 * 1024
            && hash(&cached)? == ADAPTER_SHA256,
        "preview1 adapter cache checksum or file mismatch"
    );
    let stage = tempfile::tempdir_in(root.join("hosts"))?;
    fs::copy(&cached, stage.path().join(ADAPTER_FILE))?;
    ensure!(
        hash(&stage.path().join(ADAPTER_FILE))? == ADAPTER_SHA256,
        "preview1 adapter changed while copying its verified cache"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            stage.path().join(ADAPTER_FILE),
            fs::Permissions::from_mode(0o644),
        )?;
    }
    atomic_json(
        &stage.path().join("receipt.json"),
        &json!({"schema":1,"owner":"wisp","version":ADAPTER_VERSION,"url":ADAPTER_URL,"sha256":ADAPTER_SHA256}),
    )?;
    fs::rename(stage.path(), &destination)?;
    verify_adapter(root)
}
fn setup_hosts(root: &Path, cache: &Path, selected: &str, jobs: &str) -> Result<Value> {
    ensure!(
        std::env::consts::OS == "linux" && std::env::consts::ARCH == "x86_64",
        "compiler host setup supports Linux x86_64"
    );
    ensure!(
        ["all", "command", "adapter"].contains(&selected),
        "--host must be command, adapter or all"
    );
    let count: usize = jobs.parse().context("--jobs must be 1 through 8")?;
    ensure!((1..=8).contains(&count), "--jobs must be 1 through 8");
    let root = prefix(root)?;
    let _lock = lock(&root)?;
    let hosts = root.join("hosts");
    real_directory(&hosts)?;
    fs::create_dir_all(cache)?;
    let cache = cache.canonicalize()?;
    let recipe_dir = tempfile::tempdir_in(&root)?;
    recipes(recipe_dir.path())?;
    let mut installed = Vec::new();
    for (name, version) in [("command", crate::compiler_hosts::COMMAND_VERSION)] {
        if selected != "all" && selected != name {
            continue;
        }
        let destination = hosts.join(format!("wasmtime-{version}"));
        if destination.try_exists()? {
            installed.push(check_host(&destination.join("wasmtime"), version)?);
            continue;
        }
        let stage = tempfile::tempdir_in(&hosts)?;
        let (script, _, _) = host_recipe(version)?;
        // These are Wisp's embedded source recipes, never package-provided scripts.
        let status = Command::new("python3").arg("-I").arg(recipe_dir.path().join(script))
            .arg("--prefix").arg(stage.path()).arg("--cache").arg(cache.join(name)).arg("--jobs").arg(jobs)
            .env("CARGO_NET_OFFLINE", "false").stdout(Stdio::null()).stderr(Stdio::inherit()).status()
            .context("building the pinned Wisp execution host requires Python 3.11+, patch, native C tooling and rustup with Rust 1.99.0")?;
        ensure!(
            status.success(),
            "Wisp {name} host build failed; no host was installed"
        );
        check_host(&stage.path().join("wasmtime"), version)?;
        fs::rename(stage.path(), &destination)?;
        installed.push(check_host(&destination.join("wasmtime"), version)?);
    }
    let adapter = setup_adapter(&root, &cache)?;
    Ok(json!({"owner":"wisp","installed":installed,"adapter":adapter,"prefix":root}))
}
pub(crate) fn doctor(root: &Path) -> Result<Value> {
    let root = root.canonicalize()?;
    ensure!(
        root.join("selection.json").try_exists()?,
        "no installed compiler selection at {}",
        root.display()
    );
    let selected = selection(&root)?;
    ensure!(
        !selected.packages.is_empty(),
        "no selected compiler packages"
    );
    let mut packages = Vec::new();
    for (id, pin) in &selected.packages {
        let (path, record) = verify(&root, id, pin)?;
        packages.push(json!({"identity":id,"pin":pin,"path":path,"files":record["archive"]["inventory"].as_object().unwrap().len(),"valid":true}));
    }
    let hosts = hosts(&root, true);
    Ok(
        json!({"valid":true,"prefix":root,"packages":packages,"hosts":hosts,"adapter":adapter_status(&root)}),
    )
}
fn list(root: &Path) -> Result<Value> {
    let selected = selection(root)?;
    let mut installed = Vec::new();
    let packages = root.join("packages");
    if packages.try_exists()? {
        for tool in fs::read_dir(packages)? {
            let tool = tool?;
            ensure!(
                tool.file_type()?.is_dir(),
                "invalid installed toolchain directory"
            );
            let id = format!(
                "wisp:{}",
                tool.file_name().to_str().context("UTF-8 toolchain name")?
            );
            identity(&id)?;
            for version in fs::read_dir(tool.path())? {
                let version = version?;
                ensure!(
                    version.file_type()?.is_dir(),
                    "invalid installed version directory"
                );
                let receipt: Receipt =
                    serde_json::from_value(json_file(&version.path().join("receipt.json"))?)?;
                verify(root, &id, &receipt.pin)?;
                installed.push(json!({"identity":id,"pin":receipt.pin,"selected":selected.packages.get(&id) == Some(&receipt.pin)}));
            }
        }
    }
    installed.sort_by_key(|entry| entry.to_string());
    Ok(json!({"prefix":root,"installed":installed}))
}

pub(crate) fn run_cli(args: &[String]) -> Result<Value> {
    let Some(command) = args.first().map(String::as_str) else {
        bail!(
            "usage: wisp toolchain <validate|install|select|list|env|doctor|hosts|setup-hosts> ..."
        );
    };
    let mut options = BTreeMap::<String, String>::new();
    let mut positional = Vec::new();
    let mut args = args[1..].iter();
    while let Some(arg) = args.next() {
        if arg.starts_with("--") {
            ensure!(
                ["--prefix", "--sha256", "--cache", "--host", "--jobs"].contains(&arg.as_str()),
                "unknown toolchain option {arg}"
            );
            let value = args
                .next()
                .with_context(|| format!("missing value for {arg}"))?;
            ensure!(
                options.insert(arg.clone(), value.clone()).is_none(),
                "duplicate option {arg}"
            );
        } else {
            positional.push(arg.as_str());
        }
    }
    let allowed: &[&str] = match command {
        "install" => &["--prefix", "--sha256"],
        "setup-hosts" => &["--prefix", "--cache", "--host", "--jobs"],
        "select" | "list" | "env" | "doctor" | "hosts" => &["--prefix"],
        _ => bail!("unknown toolchain command {command}"),
    };
    ensure!(
        options
            .keys()
            .all(|option| allowed.contains(&option.as_str())),
        "option does not apply to {command}"
    );
    let root = options
        .get("--prefix")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(home)?;
    match (command, positional.as_slice()) {
        ("install", [bundle]) => install(
            Path::new(bundle),
            &root,
            options.get("--sha256").map(String::as_str),
        ),
        ("select", [spec]) => select(spec, &root),
        ("list", []) => list(&root),
        ("env", []) => Ok(json!(environment(&root)?)),
        ("doctor", []) => doctor(&root),
        ("hosts", []) => Ok(hosts(&root, true)),
        ("setup-hosts", []) => setup_hosts(
            &root,
            Path::new(
                options
                    .get("--cache")
                    .context("setup-hosts requires --cache DIR")?,
            ),
            options.get("--host").map(String::as_str).unwrap_or("all"),
            options.get("--jobs").map(String::as_str).unwrap_or("2"),
        ),
        _ => bail!("wrong arguments for wisp toolchain {command}"),
    }
}
