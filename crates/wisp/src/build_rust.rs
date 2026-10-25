//! Offline Cargo orchestration for the pinned Wasm-hosted Rust bootstrap.
use crate::build::{digest, read, write};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct Project {
    pub library: String,
    pub source: PathBuf,
}
pub fn project(root: &Path) -> Result<Project> {
    let manifest: toml::Value = toml::from_str(std::str::from_utf8(&read(
        &root.join("Cargo.toml"),
        1024 * 1024,
    )?)?)?;
    let name = manifest
        .get("package")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .context("Rust build requires a package Cargo.toml, not a virtual workspace")?;
    let library = manifest
        .get("lib")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(name)
        .replace('-', "_");
    ensure!(
        valid_identifier(&library),
        "invalid Rust library target name"
    );
    let relative = manifest
        .get("lib")
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("src/lib.rs");
    let path = Path::new(relative);
    ensure!(
        !path.is_absolute()
            && path.components().all(|c| matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )),
        "Rust library source path must stay inside its project"
    );
    let source = root
        .join(path)
        .canonicalize()
        .context("Rust source build requires a library target, normally src/lib.rs")?;
    ensure!(
        source.starts_with(root),
        "Rust library source escapes project root"
    );
    Ok(Project { library, source })
}
fn valid_identifier(name: &str) -> bool {
    name.bytes()
        .enumerate()
        .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
        && !name.is_empty()
}
pub struct Toolchain {
    pub root: PathBuf,
    pub runner: PathBuf,
    pub runner_sha256: String,
    pub manifest_sha256: String,
    pub inventory_sha256: String,
    pub compiler_sha256: String,
    pub runtime: crate::compiler_hosts::ExecutionHost,
    pub manifest: Value,
    pub notices: BTreeMap<String, String>,
}
fn toolchain_root(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    if path.is_dir() {
        return Ok(path);
    }
    if path.file_name().is_some_and(|n| n == "toolchain.json") {
        return Ok(path.parent().context("Rust toolchain parent")?.to_owned());
    }
    ensure!(
        path.file_name().is_some_and(|n| n == "runner.py"),
        "WISP_RUST must select the Rust toolchain root, toolchain.json or scripts/runner.py"
    );
    Ok(path
        .parent()
        .and_then(|p| p.parent())
        .context("Rust runner toolchain root")?
        .to_owned())
}
impl Toolchain {
    pub fn compiler_host(path: &Path) -> Result<String> {
        let bytes = read(&toolchain_root(path)?.join("toolchain.json"), 1024 * 1024)?;
        let manifest: Value = crate::release::strict_json(&bytes)?;
        ensure!(
            manifest["compilerHost"] == "wasm32-wasip3",
            "Rust compiler host must be wasm32-wasip3"
        );
        Ok("wasm32-wasip3".to_owned())
    }
    pub fn load(path: &Path, runtime_path: &Path) -> Result<Self> {
        let root = toolchain_root(path)?;
        let manifest_bytes = read(&root.join("toolchain.json"), 1024 * 1024)?;
        let manifest: Value = crate::release::strict_json(&manifest_bytes)?;
        ensure!(
            manifest["schemaVersion"] == 1
                && manifest["compilerHost"] == "wasm32-wasip3"
                && manifest["applicationTarget"] == "wasm32-wasip1",
            "unsupported Rust compiler host or application target"
        );
        let prefix = root.join("build/bootstrap");
        let inventory = read(&prefix.join("SHA256SUMS.json"), 4 * 1024 * 1024)?;
        let files: BTreeMap<String, String> =
            serde_json::from_value(crate::release::strict_json(&inventory)?)?;
        ensure!(
            !files.is_empty() && files.len() <= 2048,
            "Rust toolchain inventory size limit"
        );
        for (name, hash) in &files {
            crate::release::safe_name(name)?;
            ensure!(
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "invalid Rust toolchain checksum"
            );
            let file = prefix.join(name);
            ensure!(
                file.canonicalize()?.starts_with(&prefix),
                "Rust toolchain path escapes its installation"
            );
            ensure!(
                digest(&read(&file, 256 * 1024 * 1024)?) == *hash,
                "Rust toolchain checksum mismatch: {name}"
            );
        }
        fn visit(
            root: &Path,
            directory: &Path,
            actual: &mut std::collections::BTreeSet<String>,
        ) -> Result<()> {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                ensure!(!kind.is_symlink(), "symlink in Rust toolchain installation");
                if kind.is_dir() {
                    visit(root, &entry.path(), actual)?;
                } else {
                    ensure!(kind.is_file(), "non-regular Rust toolchain input");
                    actual.insert(
                        entry
                            .path()
                            .strip_prefix(root)?
                            .to_str()
                            .context("UTF8 Rust toolchain input")?
                            .replace('\\', "/"),
                    );
                }
            }
            Ok(())
        }
        let mut actual = std::collections::BTreeSet::new();
        visit(&prefix, &prefix, &mut actual)?;
        actual.remove("SHA256SUMS.json");
        ensure!(
            actual == files.keys().cloned().collect(),
            "Rust toolchain file set differs from checksum inventory"
        );
        let module = manifest["compiler"]["module"]
            .as_str()
            .context("Rust compiler module")?;
        crate::release::safe_name(module)?;
        let compiler_sha256 = manifest["compiler"]["moduleSha256"]
            .as_str()
            .context("Rust compiler module checksum")?
            .to_owned();
        ensure!(
            files.get(module) == Some(&compiler_sha256),
            "Rust compiler checksum differs from pinned release manifest"
        );
        let runtime_version = manifest["runtime"]["version"]
            .as_str()
            .context("Rust compiler execution host version")?;
        ensure!(
            runtime_version == crate::compiler_hosts::RUST_WASIP3_VERSION,
            "Rust compiler package requires Wasmtime {runtime_version}; Wisp requires {}. Install a Rust compiler package built for this host version",
            crate::compiler_hosts::RUST_WASIP3_VERSION
        );
        // The pinned external runner uses compact CLI flags, including an async
        // stack size that the worker protocol cannot represent. Keep its CLI.
        let runtime = crate::compiler_hosts::runtime(runtime_path, runtime_version)?;
        ensure!(
            manifest.get("llvmProvider").is_none(),
            "WASIp3 Rust compiler must contain its LLVM backend"
        );
        let runner = root.join("scripts/runner.py");
        let runner_sha256 = digest(&read(&runner, 1024 * 1024)?);
        let mut notices = BTreeMap::new();
        for (name, hash) in manifest
            .get("licenses")
            .and_then(|v| v.as_object())
            .context("Rust toolchain license inventory")?
        {
            crate::release::safe_name(name)?;
            let license = if root.join(name).is_file() {
                root.join(name)
            } else {
                root.join("licenses").join(name)
            };
            let bytes = read(&license, 1024 * 1024)?;
            ensure!(
                Some(digest(&bytes).as_str()) == hash.as_str(),
                "Rust toolchain license checksum mismatch: {name}"
            );
            let notice = format!(
                "Rust-{}.txt",
                Path::new(name)
                    .file_name()
                    .context("Rust license filename")?
                    .to_str()
                    .context("UTF8 Rust license filename")?
            );
            notices.insert(notice, String::from_utf8(bytes)?);
        }
        Ok(Self {
            root,
            runner,
            runner_sha256,
            manifest_sha256: digest(&manifest_bytes),
            inventory_sha256: digest(&inventory),
            compiler_sha256,
            runtime,
            manifest,
            notices,
        })
    }
    pub fn receipt(&self) -> Value {
        json!({"path":self.root,"manifestSha256":self.manifest_sha256,"inventorySha256":self.inventory_sha256,
            "runner":{"path":self.runner,"sha256":self.runner_sha256},"compilerSha256":self.compiler_sha256,
            "version":self.manifest["rustVersion"],"host":self.manifest["compilerHost"],"target":self.manifest["applicationTarget"],
            "llvmVersion":self.manifest["llvmVersion"],
            "llvmCompatibility":"linked",
            "wasmtime":{"path":self.runtime.path,"sha256":self.runtime.sha256,"version":self.manifest["runtime"]["version"]}})
    }
}

pub fn snapshot(project: &Path, to: &Path) -> Result<BTreeMap<String, String>> {
    fn visit(
        root: &Path,
        directory: &Path,
        to: &Path,
        files: &mut BTreeMap<String, String>,
        total: &mut u64,
    ) -> Result<()> {
        let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let name = name.to_str().context("UTF8 Rust source filename")?;
            if matches!(
                name,
                ".git"
                    | ".wisp-cache"
                    | ".wisp-generated.rs"
                    | "target"
                    | "out"
                    | "dist"
                    | "node_modules"
                    | "__pycache__"
            ) {
                continue;
            }
            let file_type = entry.file_type()?;
            ensure!(
                !file_type.is_symlink(),
                "symlink in Rust source input: {}",
                entry.path().display()
            );
            if file_type.is_dir() {
                visit(root, &entry.path(), to, files, total)?;
                continue;
            }
            ensure!(file_type.is_file(), "non-regular Rust source input");
            let bytes = read(&entry.path(), 32 * 1024 * 1024)?;
            *total += bytes.len() as u64;
            ensure!(
                *total <= 32 * 1024 * 1024 && files.len() < 4096,
                "Rust source snapshot size/count limit"
            );
            let relative = entry
                .path()
                .strip_prefix(root)?
                .to_str()
                .context("UTF8 Rust source path")?
                .replace('\\', "/");
            files.insert(relative.clone(), digest(&bytes));
            write(to.join(relative), bytes)?;
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(project, project, to, &mut files, &mut 0)?;
    Ok(files)
}

pub fn compile(
    toolchain: &Toolchain,
    source: &Path,
    scratch: &Path,
    wrapper: Option<&str>,
    cache: &Path,
    commands: &mut Vec<Value>,
) -> Result<Vec<u8>> {
    let mut arguments = vec![
        toolchain
            .runner
            .to_str()
            .context("UTF8 Rust runner path")?
            .to_owned(),
        "build".into(),
        "--manifest-path".into(),
        source
            .join("Cargo.toml")
            .to_str()
            .context("UTF8 Rust manifest path")?
            .into(),
        "--crate-type".into(),
        "cdylib".into(),
        "--out-dir".into(),
        scratch
            .join("rust")
            .to_str()
            .context("UTF8 Rust build path")?
            .into(),
        "--report".into(),
        scratch
            .join("rust/build.json")
            .to_str()
            .context("UTF8 Rust receipt path")?
            .into(),
        "--wasmtime".into(),
        toolchain
            .runtime
            .path
            .to_str()
            .context("UTF8 Rust runtime path")?
            .into(),
        "--offline".into(),
    ];
    if let Some(wrapper) = wrapper {
        let wrapper_path = source.join(".wisp-generated.rs");
        write(&wrapper_path, wrapper)?;
        arguments.extend([
            "--wrapper-source".into(),
            wrapper_path
                .to_str()
                .context("UTF8 Rust wrapper path")?
                .into(),
        ]);
    }
    if source.join("vendor").is_dir() {
        arguments.extend([
            "--vendor-dir".into(),
            source
                .join("vendor")
                .to_str()
                .context("UTF8 Rust vendor path")?
                .into(),
        ]);
    }
    let mut command = Command::new("python3");
    let result = command
        .arg("-I")
        .args(&arguments)
        .env("XDG_CACHE_HOME", cache)
        .stdin(std::process::Stdio::null())
        .output()
        .context("running Wasm Rust compiler orchestrator")?;
    commands.push(json!({"kind":"rust-orchestration","program":"python3","arguments":std::iter::once("-I".to_owned()).chain(arguments).collect::<Vec<_>>(),"status":result.status.code(),
        "stdout":String::from_utf8_lossy(&result.stdout),"stderr":String::from_utf8_lossy(&result.stderr)}));
    ensure!(
        result.status.success(),
        "Wasm Rust compiler failed: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    ensure!(
        digest(&read(&toolchain.runner, 1024 * 1024)?) == toolchain.runner_sha256,
        "Rust compiler runner changed during build"
    );
    let report: Value =
        crate::release::strict_json(&read(&scratch.join("rust/build.json"), 4 * 1024 * 1024)?)?;
    validate_report(toolchain, source, scratch, &report, commands)
}

fn compiler_module_matches(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\0asm\x0d\0\x01\0")
}

/// Authenticate the runner's execution records against the staged inputs and output.
fn validate_report(
    toolchain: &Toolchain,
    source: &Path,
    scratch: &Path,
    report: &Value,
    commands: &mut Vec<Value>,
) -> Result<Vec<u8>> {
    ensure!(
        digest(&read(&toolchain.runtime.path, 256 * 1024 * 1024)?) == toolchain.runtime.sha256,
        "Rust Wasmtime runtime changed during build"
    );
    ensure!(
        report.get("toolchainDependencies").is_none()
            || report["toolchainDependencies"]
                .as_array()
                .is_some_and(Vec::is_empty),
        "linked Rust compiler receipt contains unexpected toolchain dependencies"
    );
    let compiler = toolchain
        .root
        .join("build/bootstrap")
        .join(
            toolchain.manifest["compiler"]["module"]
                .as_str()
                .context("Rust compiler module")?,
        )
        .canonicalize()?;
    let compiler_bytes = read(&compiler, 256 * 1024 * 1024)?;
    ensure!(
        compiler_module_matches(&compiler_bytes)
            && digest(&compiler_bytes) == toolchain.compiler_sha256,
        "Rust compiler changed or does not match its Wasm compiler host"
    );
    let manifest = source.join("Cargo.toml").canonicalize()?;
    let build_root = scratch.join("rust").canonicalize()?;
    let library = project(source)?.library;
    ensure!(
        report["schemaVersion"] == 1
            && report["manifestPath"].as_str() == manifest.to_str()
            && report["target"] == "wasm32-wasip1"
            && report["libraryName"] == library
            && report["compiler"]["path"].as_str() == compiler.to_str()
            && report["compiler"]["sha256"] == toolchain.compiler_sha256
            && report["compiler"]["host"] == "wasm",
        "Rust compiler receipt differs from admitted Wasm toolchain or staged project"
    );
    let executions = report["executions"]
        .as_array()
        .context("Rust execution receipt")?;
    ensure!(
        executions.iter().any(|e| e["kind"] == "compiler"),
        "Rust build has no Wasm compiler execution"
    );
    let mut programs = Vec::new();
    for execution in executions {
        ensure!(
            matches!(
                execution["kind"].as_str(),
                Some("compiler" | "build-script" | "procedural-macro")
            ),
            "unsupported Rust compiler execution kind"
        );
        ensure!(
            execution["exitCode"] == 0,
            "Rust receipt contains a failed execution"
        );
        let program = Path::new(
            execution["program"]
                .as_str()
                .context("Rust execution program")?,
        );
        ensure!(
            program.is_absolute(),
            "Rust execution program must be absolute"
        );
        let program = program.canonicalize()?;
        let directory = Path::new(
            execution["cwd"]
                .as_str()
                .context("Rust execution working directory")?,
        );
        ensure!(
            directory.is_absolute() && directory.is_dir(),
            "Rust execution working directory must be an absolute directory"
        );
        let arguments = execution["argv"]
            .as_array()
            .context("Rust execution arguments")?;
        ensure!(
            arguments.len() > 3
                && arguments.iter().all(Value::is_string)
                && arguments[0].as_str() == toolchain.runtime.path.to_str()
                && arguments[1] == "run"
                && arguments
                    .iter()
                    .any(|argument| argument.as_str() == program.to_str()),
            "Rust execution receipt does not use the configured Wasmtime program"
        );
        let program_bytes = read(&program, 256 * 1024 * 1024)?;
        ensure!(
            (if execution["kind"] == "compiler" {
                compiler_module_matches(&program_bytes)
            } else {
                program_bytes.starts_with(b"\0asm\x01\0\0\0")
            }) && execution["sha256"] == digest(&program_bytes),
            "Rust execution program is not the reported Wasm module"
        );
        if execution["kind"] == "compiler" {
            ensure!(
                program == compiler && execution["sha256"] == toolchain.compiler_sha256,
                "Rust source was compiled by an unexpected compiler"
            );
        } else {
            ensure!(
                program.starts_with(&build_root),
                "Rust build script or procedural macro is outside the fresh build directory"
            );
        }
        programs.push(program);
        commands.push(execution.clone());
    }
    ensure!(
        report["dependencies"].is_array(),
        "Rust compiler receipt is missing its dependency inventory"
    );
    for (field, kind) in [
        ("buildScripts", "build-script"),
        ("proceduralMacros", "procedural-macro"),
    ] {
        for record in report[field]
            .as_array()
            .with_context(|| format!("Rust compiler receipt is missing {field}"))?
        {
            ensure!(
                record["package"].is_string(),
                "invalid Rust {kind} package record"
            );
            let program = Path::new(
                record["program"]
                    .as_str()
                    .with_context(|| format!("Rust {kind} program"))?,
            );
            ensure!(
                program.is_absolute() && program.canonicalize()?.starts_with(&build_root),
                "Rust {kind} is outside the fresh build directory"
            );
            let bytes = read(program, 256 * 1024 * 1024)?;
            ensure!(
                bytes.starts_with(b"\0asm\x01\0\0\0") && record["sha256"] == digest(&bytes),
                "Rust {kind} program is not the reported Wasm module"
            );
            let index = record["execution"]
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .filter(|index| *index < executions.len())
                .with_context(|| format!("Rust {kind} has no execution record"))?;
            if kind == "procedural-macro" && record["loader"] == "compiler-wasm-runtime" {
                ensure!(
                    executions[index]["kind"] == "compiler",
                    "in-process procedural macro must execute in the Wasm compiler"
                );
            } else {
                ensure!(
                    executions[index]["kind"] == kind
                        && programs[index] == program.canonicalize()?,
                    "Rust {kind} did not execute through Wasmtime"
                );
            }
        }
    }
    let artifact = scratch.join("rust/app.wasm").canonicalize()?;
    ensure!(
        artifact.starts_with(&build_root)
            && report["artifact"]["path"].as_str() == artifact.to_str(),
        "Rust compiler artifact path differs from the fresh output directory"
    );
    let bytes = read(&artifact, crate::limits::MAX_COMPONENT_BYTES)?;
    ensure!(
        bytes.starts_with(b"\0asm\x01\0\0\0") && report["artifact"]["sha256"] == digest(&bytes),
        "Rust compiler output checksum differs from receipt"
    );
    Ok(bytes)
}

#[derive(Debug)]
pub struct GuestDependency {
    pub directory: PathBuf,
    pub descriptor: Value,
}
/// Find executable packages through ordinary Cargo path dependencies or cargo vendor.
pub fn guest_dependencies(project: &Path) -> Result<Vec<GuestDependency>> {
    fn visit(
        root: &Path,
        current: &Path,
        visiting: &mut std::collections::BTreeSet<PathBuf>,
        cache: &mut BTreeMap<PathBuf, std::collections::BTreeSet<PathBuf>>,
        result: &mut BTreeMap<PathBuf, GuestDependency>,
    ) -> Result<std::collections::BTreeSet<PathBuf>> {
        if let Some(components) = cache.get(current) {
            return Ok(components.clone());
        }
        ensure!(
            cache.len() + visiting.len() < 128,
            "Rust local dependency graph size limit"
        );
        ensure!(
            visiting.insert(current.to_owned()),
            "cyclic Cargo dependency graph is unsupported by Rust Component discovery"
        );
        let manifest: toml::Value = toml::from_str(std::str::from_utf8(&read(
            &current.join("Cargo.toml"),
            1024 * 1024,
        )?)?)?;
        let mut tables = Vec::new();
        for (key, build) in [("dependencies", false), ("build-dependencies", true)] {
            if let Some(dependencies) = manifest.get(key).and_then(|value| value.as_table()) {
                tables.push((dependencies, None, build));
            }
        }
        if let Some(targets) = manifest.get("target").and_then(|value| value.as_table()) {
            for (target, table) in targets {
                for (key, build) in [("dependencies", false), ("build-dependencies", true)] {
                    if let Some(dependencies) = table.get(key).and_then(|value| value.as_table()) {
                        tables.push((dependencies, Some(target.as_str()), build));
                    }
                }
            }
        }
        let mut reachable = std::collections::BTreeSet::new();
        for (dependencies, target, build) in tables {
            for (alias, requirement) in dependencies {
                let path = requirement.get("path").and_then(|v| v.as_str());
                let package = requirement
                    .get("package")
                    .and_then(|v| v.as_str())
                    .unwrap_or(alias);
                let version = requirement
                    .as_str()
                    .or_else(|| requirement.get("version").and_then(|v| v.as_str()));
                let version = version.map(semver::VersionReq::parse).transpose()?;
                let mut candidates = Vec::new();
                if let Some(path) = path {
                    let directory = current
                        .join(path)
                        .canonicalize()
                        .with_context(|| format!("Cargo dependency {alias}"))?;
                    ensure!(
                        directory.starts_with(root),
                        "Cargo path dependency {alias} escapes the Rust project; install dependencies inside the project before building"
                    );
                    candidates.push(directory);
                } else if root.join("vendor").is_dir() {
                    for entry in fs::read_dir(root.join("vendor"))? {
                        let directory = entry?.path();
                        if !directory.join("Cargo.toml").is_file() {
                            continue;
                        }
                        let candidate: toml::Value = toml::from_str(std::str::from_utf8(&read(
                            &directory.join("Cargo.toml"),
                            1024 * 1024,
                        )?)?)?;
                        if candidate
                            .get("package")
                            .and_then(|v| v.get("name"))
                            .and_then(|v| v.as_str())
                            != Some(package)
                        {
                            continue;
                        }
                        let parsed = semver::Version::parse(
                            candidate
                                .get("package")
                                .and_then(|v| v.get("version"))
                                .and_then(|v| v.as_str())
                                .context("vendored Rust package version")?,
                        )?;
                        if version
                            .as_ref()
                            .is_none_or(|version| version.matches(&parsed))
                        {
                            let directory = directory.canonicalize()?;
                            ensure!(
                                directory.starts_with(root),
                                "vendored Cargo dependency {alias} escapes the Rust project"
                            );
                            candidates.push(directory);
                        }
                    }
                    ensure!(
                        candidates.len() <= 1,
                        "multiple vendored versions match Cargo dependency {alias}; select one exact version"
                    );
                }
                let mut components = std::collections::BTreeSet::new();
                for directory in candidates {
                    if !directory.join("wisp-component.json").is_file() {
                        components.extend(visit(root, &directory, visiting, cache, result)?);
                        continue;
                    }
                    let descriptor = crate::library::rust_guest::installed(&directory)
                        .with_context(|| format!("Cargo Component dependency {alias}"))?;
                    ensure!(
                        descriptor["crate"] == package,
                        "Cargo dependency {alias} package name differs from installed Rust guest crate"
                    );
                    let parsed = semver::Version::parse(
                        descriptor["version"]
                            .as_str()
                            .context("Rust guest version")?,
                    )?;
                    ensure!(
                        version
                            .as_ref()
                            .is_none_or(|version| version.matches(&parsed)),
                        "Cargo dependency {alias} version differs from installed Component"
                    );
                    components.insert(directory.clone());
                    result.insert(
                        directory.clone(),
                        GuestDependency {
                            directory,
                            descriptor,
                        },
                    );
                }
                if !components.is_empty() {
                    ensure!(
                        target.is_none(),
                        "target-specific Cargo Component dependencies are not yet supported: {alias} under {}",
                        target.unwrap_or("")
                    );
                    ensure!(
                        !build,
                        "Cargo build-script dependencies containing Components are not yet supported: {alias}"
                    );
                    ensure!(
                        requirement.get("optional").and_then(|v| v.as_bool()) != Some(true),
                        "optional Cargo dependencies containing Components are not yet supported: {alias}"
                    );
                }
                reachable.extend(components);
            }
        }
        visiting.remove(current);
        cache.insert(current.to_owned(), reachable.clone());
        Ok(reachable)
    }
    let mut result = BTreeMap::new();
    let project = project.canonicalize()?;
    visit(
        &project,
        &project,
        &mut std::collections::BTreeSet::new(),
        &mut BTreeMap::new(),
        &mut result,
    )?;
    Ok(result.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires WISP_RUST and WISP_RUST_WASMTIME selecting the packaged WASIp3 compiler"]
    fn packaged_wasip3_compiler_builds_a_library_and_wasm_build_script() {
        let compiler = PathBuf::from(std::env::var_os("WISP_RUST").expect("WISP_RUST"));
        let runtime =
            PathBuf::from(std::env::var_os("WISP_RUST_WASMTIME").expect("WISP_RUST_WASMTIME"));
        let toolchain = Toolchain::load(&compiler, &runtime).unwrap();
        assert_eq!(toolchain.manifest["compilerHost"], "wasm32-wasip3");
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir_all(source.join("src")).unwrap();
        write(
            source.join("Cargo.toml"),
            "[package]\nname='application'\nversion='0.1.0'\nedition='2021'\n[workspace]\n",
        )
        .unwrap();
        write(
            source.join("Cargo.lock"),
            "version = 4\n[[package]]\nname = \"application\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        write(
            source.join("build.rs"),
            "fn main() { println!(\"cargo:rustc-cfg=wisp_smoke\"); }\n",
        )
        .unwrap();
        write(
            source.join("src/lib.rs"),
            "#[cfg(wisp_smoke)] pub fn answer() -> u32 { vec![20,22].into_iter().sum() }\n",
        )
        .unwrap();
        let mut commands = Vec::new();
        let cache = std::env::var_os("WISP_RUST_TEST_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.path().join("cache"));
        let bytes = compile(&toolchain, &source, &directory.path().join("scratch"),
            Some("extern crate application; #[no_mangle] pub extern \"C\" fn answer() -> u32 { application::answer() }\n"),
            &cache, &mut commands).unwrap();
        assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
        assert!(
            commands
                .iter()
                .any(|record| record["kind"] == "build-script")
        );
        assert!(commands.iter().any(|record| record["kind"] == "compiler"));
    }

    #[test]
    #[cfg(unix)]
    fn wasip3_toolchain_loads_without_a_shared_llvm_provider() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("compiler");
        fs::create_dir_all(root.join("build/bootstrap")).unwrap();
        fs::create_dir(root.join("scripts")).unwrap();
        let bytes = b"\0asm\x0d\0\x01\0";
        let hash = digest(bytes);
        write(root.join("build/bootstrap/rustc.wasm"), bytes).unwrap();
        write(
            root.join("build/bootstrap/SHA256SUMS.json"),
            serde_json::to_vec(&json!({"rustc.wasm":hash})).unwrap(),
        )
        .unwrap();
        write(root.join("scripts/runner.py"), b"# pinned runner").unwrap();
        write(
            root.join("toolchain.json"),
            serde_json::to_vec(&json!({
                "schemaVersion":1,"rustVersion":"1.99.1","compilerHost":"wasm32-wasip3",
                "applicationTarget":"wasm32-wasip1","llvmVersion":"24.0.0git",
                "compiler":{"module":"rustc.wasm","moduleSha256":hash},
                "runtime":{"version":crate::compiler_hosts::RUST_WASIP3_VERSION},"licenses":{}
            }))
            .unwrap(),
        )
        .unwrap();
        let runtime = directory.path().join("wasmtime");
        write(
            &runtime,
            format!(
                "#!/bin/sh\nprintf 'wasmtime {}\\n'\n",
                crate::compiler_hosts::RUST_WASIP3_VERSION
            ),
        )
        .unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
        let toolchain = Toolchain::load(&root, &runtime).unwrap();
        assert_eq!(toolchain.runtime.path, runtime.canonicalize().unwrap());
        assert_eq!(
            toolchain.runtime.backend,
            crate::compiler_hosts::Backend::WasmtimeCli
        );
        assert_eq!(
            Toolchain::compiler_host(&toolchain.runner).unwrap(),
            "wasm32-wasip3"
        );
        assert_eq!(toolchain.receipt()["llvmCompatibility"], "linked");
        assert!(toolchain.receipt()["llvmProvider"].is_null());
        assert!(compiler_module_matches(bytes));
        assert!(!compiler_module_matches(b"\0asm\x01\0\0\0"));

        let manifest_path = root.join("toolchain.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["runtime"]["version"] = json!("49.0.1");
        write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Toolchain::load(&root, &runtime).err().unwrap().to_string();
        assert!(error.contains("Rust compiler package requires Wasmtime 49.0.1"));
        assert!(error.contains("Install a Rust compiler package built for this host version"));
    }

    #[test]
    fn wasip3_receipts_authenticate_component_compilers_and_linked_llvm() {
        let (_root, mut toolchain, source, scratch, mut report) = receipt_fixture();
        let compiler = toolchain.root.join("build/bootstrap/rustc.wasm");
        let bytes = b"\0asm\x0d\0\x01\0";
        write(&compiler, bytes).unwrap();
        toolchain.compiler_sha256 = digest(bytes);
        toolchain.manifest["compilerHost"] = json!("wasm32-wasip3");
        report
            .as_object_mut()
            .unwrap()
            .remove("toolchainDependencies");
        report["compiler"]["sha256"] = json!(toolchain.compiler_sha256);
        report["executions"][0]["sha256"] = json!(toolchain.compiler_sha256);
        assert!(validate_report(&toolchain, &source, &scratch, &report, &mut Vec::new()).is_ok());
        report["toolchainDependencies"] = json!([]);
        assert!(validate_report(&toolchain, &source, &scratch, &report, &mut Vec::new()).is_ok());
        report["toolchainDependencies"] = json!([{"package":"unexpected"}]);
        assert!(validate_report(&toolchain, &source, &scratch, &report, &mut Vec::new()).is_err());
    }

    fn receipt_fixture() -> (tempfile::TempDir, Toolchain, PathBuf, PathBuf, Value) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let scratch = root.path().join("scratch");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(scratch.join("rust")).unwrap();
        fs::write(
            source.join("Cargo.toml"),
            "[package]\nname='application'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(source.join("src/lib.rs"), "").unwrap();
        let module = "rustc.wasm";
        let compiler = root.path().join("build/bootstrap").join(module);
        fs::create_dir_all(compiler.parent().unwrap()).unwrap();
        fs::write(&compiler, b"\0asm\x0d\0\x01\0").unwrap();
        let runtime = root.path().join("wasmtime");
        fs::write(&runtime, b"pinned-runtime").unwrap();
        let artifact = scratch.join("rust/app.wasm");
        fs::write(&artifact, b"\0asm\x01\0\0\0").unwrap();
        let compiler_sha256 = digest(&fs::read(&compiler).unwrap());
        let toolchain = Toolchain {
            root: root.path().to_owned(),
            runner: root.path().join("runner.py"),
            runner_sha256: String::new(),
            manifest_sha256: String::new(),
            inventory_sha256: String::new(),
            compiler_sha256: compiler_sha256.clone(),
            runtime: crate::compiler_hosts::ExecutionHost {
                sha256: digest(&fs::read(&runtime).unwrap()),
                path: runtime.clone(),
                backend: crate::compiler_hosts::Backend::WasmtimeCli,
            },
            manifest: json!({"compilerHost":"wasm32-wasip3","compiler":{"module":module}}),
            notices: BTreeMap::new(),
        };
        let report = json!({
            "schemaVersion":1,"manifestPath":source.join("Cargo.toml"),
            "target":"wasm32-wasip1","libraryName":"application",
            "compiler":{"path":compiler,"sha256":compiler_sha256,"host":"wasm"},
            "toolchainDependencies":[],
            "dependencies":[],"buildScripts":[],"proceduralMacros":[],
            "executions":[{"kind":"compiler","program":compiler,"sha256":compiler_sha256,
                "argv":[runtime,"run",compiler,"--version"],"cwd":source,"exitCode":0}],
            "artifact":{"path":artifact,"sha256":digest(&fs::read(&artifact).unwrap())}
        });
        (root, toolchain, source, scratch, report)
    }

    #[test]
    fn compiler_receipts_require_the_staged_manifest_and_actual_wasm_execution() {
        let (_root, toolchain, source, scratch, report) = receipt_fixture();
        assert!(validate_report(&toolchain, &source, &scratch, &report, &mut Vec::new()).is_ok());
        for (pointer, replacement) in [
            ("/schemaVersion", json!(2)),
            ("/manifestPath", json!("/different/Cargo.toml")),
            ("/libraryName", json!("other")),
            ("/compiler/host", json!("native")),
            ("/toolchainDependencies", json!([{"package":"unexpected"}])),
            ("/executions/0/exitCode", json!(1)),
            ("/executions/0/argv/0", json!("/different/wasmtime")),
            ("/executions/0/argv/3", json!({"argument":"--version"})),
            ("/executions/0/cwd", json!("relative/directory")),
            ("/executions/0/program", json!("rustc.wasm")),
            ("/executions/0/sha256", json!("0".repeat(64))),
            ("/buildScripts", Value::Null),
            ("/proceduralMacros", Value::Null),
            ("/artifact/sha256", json!("0".repeat(64))),
        ] {
            let mut invalid = report.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                validate_report(&toolchain, &source, &scratch, &invalid, &mut Vec::new()).is_err(),
                "receipt accepted altered {pointer}"
            );
        }
        fs::write(&toolchain.runtime.path, b"changed-runtime").unwrap();
        assert!(
            validate_report(&toolchain, &source, &scratch, &report, &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains("runtime changed")
        );
    }

    #[test]
    fn compiler_receipts_link_build_scripts_to_their_wasm_execution() {
        let (_root, toolchain, source, scratch, mut report) = receipt_fixture();
        let script = scratch.join("rust/build-script.wasm");
        fs::write(&script, b"\0asm\x01\0\0\0").unwrap();
        let hash = digest(&fs::read(&script).unwrap());
        report["executions"].as_array_mut().unwrap().push(json!({
            "kind":"build-script","program":script,"sha256":hash,
            "argv":[toolchain.runtime.path,"run",script,"--"],"cwd":source,"exitCode":0
        }));
        report["buildScripts"] =
            json!([{"package":"application","program":script,"sha256":hash,"execution":1}]);
        assert!(validate_report(&toolchain, &source, &scratch, &report, &mut Vec::new()).is_ok());
        report["buildScripts"][0]["execution"] = json!(0);
        assert!(
            validate_report(&toolchain, &source, &scratch, &report, &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains("did not execute through Wasmtime")
        );
    }

    fn guest_fixture(directory: &Path) {
        use wasm_encoder::*;
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        module.section(&types);
        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);
        let mut exports = ExportSection::new();
        exports.export("test:guest/api@0.1.0#answer", ExportKind::Func, 0);
        module.section(&exports);
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(42));
        function.instruction(&Instruction::End);
        let mut code = CodeSection::new();
        code.function(&function);
        module.section(&code);
        let mut core = module.finish();
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve.push_str("guest.wit", "package test:guest@0.1.0; interface api { answer: func() -> u32; } world library { export api; }").unwrap();
        let world = resolve.select_world(&[package], Some("library")).unwrap();
        wit_component::embed_component_metadata(
            &mut core,
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
        )
        .unwrap();
        let component = wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&core)
            .unwrap()
            .encode()
            .unwrap();
        for (name, bytes) in
            crate::library::rust_guest::files(&component, "guest", "0.1.0").unwrap()
        {
            write(directory.join(name), bytes).unwrap();
        }
    }

    #[test]
    fn discovers_components_through_local_and_vendored_rust_wrappers() {
        for vendored in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let wrapper = root.path().join(if vendored {
                "vendor/wrapper"
            } else {
                "wrapper"
            });
            let guest = root.path().join("vendor/guest");
            fs::create_dir_all(&wrapper).unwrap();
            fs::write(
                root.path().join("Cargo.toml"),
                format!(
                    "[package]\nname='application'\nversion='0.1.0'\n[dependencies]\nwrapper={}\n",
                    if vendored {
                        "'=0.1.0'"
                    } else {
                        "{path='wrapper'}"
                    }
                ),
            )
            .unwrap();
            fs::write(
                wrapper.join("Cargo.toml"),
                "[package]\nname='wrapper'\nversion='0.1.0'\n[dependencies]\nguest='=0.1.0'\n",
            )
            .unwrap();
            guest_fixture(&guest);
            let dependencies = guest_dependencies(root.path()).unwrap();
            assert_eq!(dependencies.len(), 1);
            assert_eq!(dependencies[0].directory, guest);
            assert_eq!(dependencies[0].descriptor["crate"], "guest");
        }
    }

    #[test]
    fn target_specific_component_dependencies_fail_before_compilation() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("Cargo.toml"), "[package]\nname='application'\nversion='0.1.0'\n[target.'cfg(target_os = \"wasi\")'.dependencies]\nguest={path='guest'}\n").unwrap();
        guest_fixture(&root.path().join("guest"));
        assert!(
            guest_dependencies(root.path())
                .unwrap_err()
                .to_string()
                .contains("target-specific Cargo Component dependencies")
        );
    }

    #[test]
    fn ordinary_target_specific_dependencies_do_not_require_components() {
        let root = tempfile::tempdir().unwrap();
        for directory in ["dependency", "build-dependency"] {
            fs::create_dir(root.path().join(directory)).unwrap();
            fs::write(
                root.path().join(directory).join("Cargo.toml"),
                format!("[package]\nname='{directory}'\nversion='0.1.0'\n"),
            )
            .unwrap();
        }
        fs::write(root.path().join("Cargo.toml"), "[package]\nname='application'\nversion='0.1.0'\n[target.'cfg(any())'.dependencies]\ndependency={path='dependency'}\n[target.'cfg(target_os = \"wasi\")'.build-dependencies]\nbuild-dependency={path='build-dependency'}\n").unwrap();
        assert!(guest_dependencies(root.path()).unwrap().is_empty());
    }

    #[test]
    fn cached_wrappers_cannot_hide_conditional_or_optional_components() {
        for vendored in [false, true] {
            for condition in ["target", "optional", "build"] {
                let root = tempfile::tempdir().unwrap();
                let wrapper = root.path().join(if vendored {
                    "vendor/wrapper"
                } else {
                    "wrapper"
                });
                fs::create_dir_all(&wrapper).unwrap();
                fs::write(
                    wrapper.join("Cargo.toml"),
                    "[package]\nname='wrapper'\nversion='0.1.0'\n[dependencies]\nguest='=0.1.0'\n",
                )
                .unwrap();
                guest_fixture(&root.path().join("vendor/guest"));
                let requirement = if vendored {
                    "{package='wrapper',version='=0.1.0'}"
                } else {
                    "{package='wrapper',path='wrapper'}"
                };
                let conditional = match condition {
                    "target" => {
                        format!("[target.'cfg(any())'.dependencies]\nz_conditional={requirement}\n")
                    }
                    "optional" => format!(
                        "z_optional={}\n",
                        requirement.replace('}', ",optional=true}")
                    ),
                    "build" => format!("[build-dependencies]\nz_build={requirement}\n"),
                    _ => unreachable!(),
                };
                // The unconditional edge visits the wrapper first. Later edges
                // must inspect its cached Component set, even when no new
                // provider would be inserted into the final result map.
                fs::write(root.path().join("Cargo.toml"), format!("[package]\nname='application'\nversion='0.1.0'\n[dependencies]\na_unconditional={requirement}\n{conditional}")).unwrap();
                let error = guest_dependencies(root.path()).unwrap_err().to_string();
                let expected = match condition {
                    "target" => "target-specific Cargo Component",
                    "optional" => "optional Cargo dependencies containing Components",
                    "build" => "build-script dependencies containing Components",
                    _ => unreachable!(),
                };
                assert!(
                    error.contains(expected),
                    "{condition}, vendored={vendored}: {error}"
                );
            }
        }
    }

    #[test]
    fn transitive_conditional_components_are_rejected_inside_ordinary_wrappers() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("wrapper")).unwrap();
        fs::write(root.path().join("Cargo.toml"), "[package]\nname='application'\nversion='0.1.0'\n[dependencies]\nwrapper={path='wrapper'}\n").unwrap();
        fs::write(root.path().join("wrapper/Cargo.toml"), "[package]\nname='wrapper'\nversion='0.1.0'\n[target.'cfg(target_os = \"wasi\")'.dependencies]\nguest='=0.1.0'\n").unwrap();
        guest_fixture(&root.path().join("vendor/guest"));
        assert!(
            guest_dependencies(root.path())
                .unwrap_err()
                .to_string()
                .contains("target-specific Cargo Component")
        );
    }

    #[test]
    #[ignore = "requires the prepared Serde acceptance project and published Cargo vendor manifests"]
    fn admits_actual_vendored_serde_graph() {
        let vendor = PathBuf::from(std::env::var_os("WISP_TEST_RUST_VENDOR").unwrap());
        let project = PathBuf::from(std::env::var_os("WISP_TEST_SERDE_PROJECT").unwrap());
        let root = tempfile::tempdir().unwrap();
        fs::copy(project.join("Cargo.toml"), root.path().join("Cargo.toml")).unwrap();
        for entry in fs::read_dir(vendor).unwrap() {
            let entry = entry.unwrap();
            if entry.path().join("Cargo.toml").is_file() {
                let directory = root.path().join("vendor").join(entry.file_name());
                fs::create_dir_all(&directory).unwrap();
                fs::copy(
                    entry.path().join("Cargo.toml"),
                    directory.join("Cargo.toml"),
                )
                .unwrap();
            }
        }
        for name in ["serde_core", "serde_json"] {
            let manifest =
                fs::read_to_string(root.path().join("vendor").join(name).join("Cargo.toml"))
                    .unwrap();
            assert!(manifest.contains("cfg(any())"));
        }
        assert!(guest_dependencies(root.path()).unwrap().is_empty());
    }
    #[test]
    fn snapshots_cargo_vendor_integrity_and_assets_without_build_products() {
        let root = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".cargo")).unwrap();
        fs::create_dir_all(root.path().join("vendor/dependency")).unwrap();
        fs::create_dir_all(root.path().join("target/debug")).unwrap();
        fs::write(
            root.path().join(".cargo/config.toml"),
            "[net]\noffline=true\n",
        )
        .unwrap();
        fs::write(
            root.path().join("vendor/dependency/.cargo-checksum.json"),
            "{\"files\":{}}",
        )
        .unwrap();
        fs::write(root.path().join("target/debug/ignored"), "ignored").unwrap();
        fs::write(root.path().join("asset.json"), "{\"answer\":42}").unwrap();
        let files = snapshot(root.path(), destination.path()).unwrap();
        assert!(files.contains_key(".cargo/config.toml"));
        assert!(files.contains_key("vendor/dependency/.cargo-checksum.json"));
        assert!(files.contains_key("asset.json"));
        assert!(!files.contains_key("target/debug/ignored"));
        assert_eq!(
            fs::read(destination.path().join("asset.json")).unwrap(),
            b"{\"answer\":42}"
        );
    }
    #[test]
    fn cargo_dependency_paths_cannot_escape_staged_project() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("project")).unwrap();
        fs::create_dir(root.path().join("outside")).unwrap();
        fs::write(
            root.path().join("outside/Cargo.toml"),
            "[package]\nname='outside'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(root.path().join("project/Cargo.toml"),"[package]\nname='application'\nversion='0.1.0'\n[dependencies]\noutside={path='../outside'}\n").unwrap();
        let error = guest_dependencies(&root.path().join("project"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("escapes the Rust project"));
    }
    #[test]
    fn library_source_path_cannot_escape_project() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname='application'\nversion='0.1.0'\n[lib]\npath='../outside.rs'\n",
        )
        .unwrap();
        assert!(
            project(root.path())
                .err()
                .unwrap()
                .to_string()
                .contains("inside its project")
        );
    }
}
