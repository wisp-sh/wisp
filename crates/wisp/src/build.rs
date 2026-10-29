//! Build source projects with Wasm-hosted compilers, then use library admission.
#[path = "build_adapters.rs"]
mod adapters;
#[path = "build_infer.rs"]
mod infer;
#[path = "build_inferred.rs"]
mod inferred;
#[path = "build_upstream.rs"]
mod upstream;
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};
use wit_parser::{Resolve, WorldId, WorldItem};
const MAX_SOURCE: u64 = 32 * 1024 * 1024;
use crate::limits::MAX_COMPONENT_BYTES as MAX_COMPONENT;
fn check_inferred_sources(
    contracts: &infer::Contracts,
    name: &str,
    files: &BTreeMap<String, String>,
) -> Result<()> {
    ensure!(
        contracts.sources[name]
            .iter()
            .all(|(path, hash)| files.get(path) == Some(hash)),
        "source changed while inferring the public API; rebuild the project"
    );
    Ok(())
}
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().context("output parent")?)?;
    fs::write(path, bytes)?;
    Ok(())
}
pub(super) fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "invalid file or size limit: {}",
        path.display()
    );
    Ok(fs::read(path)?)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    name: String,
    version: semver::Version,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    package: Identity,
    #[serde(default)]
    dependencies: BTreeMap<String, InstalledPackage>,
    #[serde(default)]
    metadata: toml::Table,
    build: Build,
}
/// An admitted library installed by `wisp library fetch`.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledPackage {
    package: String,
    version: semver::Version,
    path: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Build {
    #[serde(default = "default_root")]
    root: String,
    wit: Option<PathBuf>,
    #[serde(default)]
    world: String,
    compiler: Option<Compiler>,
    project: Option<PathBuf>,
    #[serde(default)]
    sources: Vec<PathBuf>,
    #[serde(default)]
    includes: Vec<PathBuf>,
    optimization: Option<String>,
    entry: Option<String>,
    javascript_engine: Option<JavascriptEngine>,
    #[serde(default)]
    typecheck: bool,
    #[serde(default)]
    tools: Tools,
    #[serde(default)]
    units: BTreeMap<String, Unit>,
}
fn default_root() -> String {
    "application".into()
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Tools {
    wasmtime: Option<PathBuf>,
    clang: Option<PathBuf>,
    clang_wasi_sysroot: Option<PathBuf>,
    go: Option<PathBuf>,
    rust: Option<PathBuf>,
    rust_wasmtime: Option<PathBuf>,
    llvm: Option<PathBuf>,
    preview1_adapter: Option<PathBuf>,
    python_host: Option<PathBuf>,
    wasmtime_python: Option<PathBuf>,
    clang_resource_dir: Option<PathBuf>,
    llvm_toolchain: Option<PathBuf>,
    llvm_swift: Option<PathBuf>,
    wasm_tools: Option<PathBuf>,
    wasm_tools_toolchain: Option<PathBuf>,
    jvm: Option<PathBuf>,
    teavm: Option<PathBuf>,
    zig: Option<PathBuf>,
    assemblyscript: Option<PathBuf>,
    nim: Option<PathBuf>,
    javascript: Option<PathBuf>,
    typescript: Option<PathBuf>,
    tsrs: Option<PathBuf>,
    wizer: Option<PathBuf>,
    wizer_wasmtime: Option<PathBuf>,
    python: Option<PathBuf>,
    grain: Option<PathBuf>,
    bend: Option<PathBuf>,
    goose: Option<PathBuf>,
    goose_frontend_host: Option<PathBuf>,
    swift: Option<PathBuf>,
    csharp: Option<PathBuf>,
    java: Option<PathBuf>,
    kotlin: Option<PathBuf>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Unit {
    compiler: Option<Compiler>,
    component: Option<ComponentInput>,
    #[serde(default)]
    world: String,
    #[serde(default)]
    sources: Vec<PathBuf>,
    #[serde(default)]
    includes: Vec<PathBuf>,
    project: Option<PathBuf>,
    entry: Option<String>,
    #[serde(default)]
    optimization: Option<String>,
    javascript_engine: Option<JavascriptEngine>,
    #[serde(default)]
    typecheck: bool,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum JavascriptEngine {
    Quickjs,
    Starlingmonkey,
}
impl JavascriptEngine {
    fn name(self) -> &'static str {
        match self {
            Self::Quickjs => "quickjs",
            Self::Starlingmonkey => "starlingmonkey",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentFile {
    path: PathBuf,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentModule {
    module: String,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum ComponentInput {
    File(ComponentFile),
    Module(ComponentModule),
}
impl Unit {
    fn kind(&self) -> &'static str {
        self.compiler.map_or("component", Compiler::name)
    }
}
fn prebuilt_component(
    root: &Path,
    artifact: &ComponentFile,
    resolve: &Resolve,
    world: WorldId,
) -> Result<Vec<u8>> {
    ensure!(
        artifact.sha256.len() == 64
            && artifact
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid prebuilt Component sha256"
    );
    let bytes = read(&input(root, &artifact.path)?, MAX_COMPONENT)?;
    ensure!(
        digest(&bytes) == artifact.sha256,
        "prebuilt Component sha256 mismatch"
    );
    ensure!(
        bytes.starts_with(b"\0asm\x0d\0\x01\0"),
        "prebuilt input must be an executable WebAssembly Component; wrap Core Wasm with WIT first"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .context("invalid prebuilt Component binary")?;
    let decoded =
        std::panic::catch_unwind(|| wit_parser::decoding::decode(&bytes)).map_err(|_| {
            anyhow::anyhow!("prebuilt Component WIT decoder rejected an unsupported type graph")
        })??;
    let wit_parser::decoding::DecodedWasm::Component(actual, actual_world) = decoded else {
        bail!("prebuilt input must be an executable WebAssembly Component")
    };
    crate::library::validate_type_expansion(&actual, actual_world)?;
    let mut actual = crate::wit_model::AbiModel::from_resolve(&actual, actual_world)?;
    let mut expected = crate::wit_model::AbiModel::from_resolve(resolve, world)?;
    ensure!(
        crate::library::exports_match(&actual, &expected),
        "prebuilt Component export contract differs from selected WIT world"
    );
    actual.exports = std::mem::take(&mut actual.imports);
    expected.exports = std::mem::take(&mut expected.imports);
    ensure!(
        crate::library::exports_match(&actual, &expected),
        "prebuilt Component import contract differs from selected WIT world"
    );
    Ok(bytes)
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Compiler {
    Clang,
    Go,
    Rust,
    Zig,
    Assemblyscript,
    Nim,
    Javascript,
    Typescript,
    Python,
    Grain,
    Bend,
    Goose,
    Swift,
    Csharp,
    Java,
    Kotlin,
}
impl Compiler {
    fn is_adapter(self) -> bool {
        !matches!(self, Self::Clang | Self::Go | Self::Rust)
    }
    fn name(self) -> &'static str {
        match self {
            Self::Clang => "clang",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Zig => "zig",
            Self::Assemblyscript => "assemblyscript",
            Self::Nim => "nim",
            Self::Javascript => "javascript",
            Self::Typescript => "typescript",
            Self::Python => "python",
            Self::Grain => "grain",
            Self::Bend => "bend",
            Self::Goose => "goose",
            Self::Swift => "swift",
            Self::Csharp => "csharp",
            Self::Java => "java",
            Self::Kotlin => "kotlin",
        }
    }
}
pub(super) struct Tool {
    pub path: PathBuf,
    pub sha256: String,
}
fn tool_path(root: &Path, configured: &Option<PathBuf>, environment: &str) -> Result<PathBuf> {
    let selected = std::env::var_os(environment)
        .map(PathBuf::from)
        .or_else(|| configured.clone());
    let path = match selected {
        Some(path) => Some(path),
        None => crate::toolchain_install::resolve(environment)?,
    }
    .with_context(|| format!("configure build.tools or set {environment}"))?;
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    path.canonicalize()
        .with_context(|| format!("tool {}", path.display()))
}
fn tool(root: &Path, configured: &Option<PathBuf>, environment: &str) -> Result<Tool> {
    let path = tool_path(root, configured, environment)?;
    Ok(Tool {
        sha256: digest(&read(&path, 256 * 1024 * 1024)?),
        path,
    })
}
fn input(root: &Path, path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|p| matches!(p, Component::Normal(_) | Component::CurDir)),
        "project paths must be relative and cannot contain '..': {}",
        path.display()
    );
    let result = root.join(path).canonicalize()?;
    ensure!(
        result.starts_with(root),
        "project input escapes root: {}",
        path.display()
    );
    Ok(result)
}
fn copy_tree(
    root: &Path,
    from: &Path,
    to: &Path,
    go: bool,
    files: &mut BTreeMap<String, String>,
    total: &mut u64,
) -> Result<()> {
    let vendor_roots = if go {
        crate::build_go::vendor_roots(root)?
    } else {
        BTreeSet::new()
    };
    copy_tree_selected(root, from, to, go, &vendor_roots, files, total)
}
fn copy_tree_selected(
    root: &Path,
    from: &Path,
    to: &Path,
    go: bool,
    vendor_roots: &BTreeSet<PathBuf>,
    files: &mut BTreeMap<String, String>,
    total: &mut u64,
) -> Result<()> {
    let mut entries = fs::read_dir(from)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if go
            && path.starts_with(root.join("vendor"))
            && !vendor_roots
                .iter()
                .any(|selected| path.starts_with(selected) || selected.starts_with(&path))
        {
            // Preserve Go's vendor inventory even when no vendored package is reachable.
            if path != root.join("vendor/modules.txt") {
                continue;
            }
        }
        let name = entry.file_name();
        let name = name.to_str().context("UTF8 source filename")?;
        if name.starts_with('.')
            || name.starts_with('_')
            || matches!(
                name,
                "target" | "out" | "dist" | "node_modules" | "testdata"
            )
        {
            continue;
        }
        ensure!(
            !entry.file_type()?.is_symlink(),
            "symlink in source input: {}",
            path.display()
        );
        if entry.file_type()?.is_dir() {
            copy_tree_selected(root, &path, to, go, vendor_roots, files, total)?;
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if go {
            ensure!(
                !matches!(ext, Some("s" | "S" | "c" | "cc" | "cpp")),
                "Go does not support local assembly or cgo sources: {}",
                path.display()
            );
            let vendored = path
                .strip_prefix(root)?
                .components()
                .next()
                .is_some_and(|p| p.as_os_str() == "vendor");
            if ext != Some("go")
                && !matches!(name, "go.mod" | "go.sum")
                && !(vendored
                    && (name == "modules.txt"
                        || matches!(ext, Some("wasm" | "wit" | "json" | "h"))
                        || name == "LICENSE"))
            {
                continue;
            }
        } else if !matches!(ext, Some("h" | "hh" | "hpp" | "hxx" | "inc")) {
            continue;
        }
        copy_file(root, &path, to, files, total)?;
    }
    Ok(())
}
fn copy_file(
    root: &Path,
    path: &Path,
    to: &Path,
    files: &mut BTreeMap<String, String>,
    total: &mut u64,
) -> Result<()> {
    let relative = path
        .strip_prefix(root)?
        .to_str()
        .context("UTF8 source path")?
        .replace('\\', "/");
    let bytes = read(path, MAX_SOURCE)?;
    if files.contains_key(&relative) {
        return Ok(());
    }
    *total += bytes.len() as u64;
    ensure!(
        *total <= MAX_SOURCE && files.len() < 4096,
        "source snapshot size/count limit"
    );
    files.insert(relative.clone(), digest(&bytes));
    write(to.join(relative), bytes)
}
pub(super) fn run_wasm(
    runtime: &crate::compiler_hosts::ExecutionHost,
    module: &Path,
    p3: bool,
    args: &[String],
    mounts: &[(&Path, &str)],
    environment: &[(&str, &str)],
    cache: &Path,
    modules: &[crate::build_llvm::CoreModule],
    commands: &mut Vec<Value>,
) -> Result<()> {
    fs::create_dir_all(cache)?;
    if runtime.backend == crate::compiler_hosts::Backend::WorkerProtocol {
        use base64::Engine as _;
        let request = wisp_worker_protocol::Request {
            module: module.to_str().context("UTF8 compiler path")?.into(),
            args: args.to_vec(),
            wasi_p3: p3,
            env: environment
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            mounts: mounts
                .iter()
                .map(|(host, guest)| {
                    Ok(wisp_worker_protocol::Mount {
                        host: host.canonicalize()?.to_str().context("UTF8 mount")?.into(),
                        guest: guest.to_string(),
                        read_only: false,
                    })
                })
                .collect::<Result<_>>()?,
            modules: modules
                .iter()
                .map(|m| {
                    Ok((
                        m.name.clone(),
                        m.tool.path.to_str().context("UTF8 dependency")?.into(),
                    ))
                })
                .collect::<Result<_>>()?,
            ..Default::default()
        };
        let response = crate::worker::execute(&runtime.path, cache, request)?;
        let stdout = base64::engine::general_purpose::STANDARD.decode(&response.stdout)?;
        let stderr = base64::engine::general_purpose::STANDARD.decode(&response.stderr)?;
        commands.push(json!({"program":runtime.path,"operation":"run","module":module,"status":response.exit_code,"stdout":String::from_utf8_lossy(&stdout),"stderr":String::from_utf8_lossy(&stderr)}));
        ensure!(
            response.ok,
            "Wasm compiler failed: {}{}{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr),
            response.error.unwrap_or_default()
        );
        return Ok(());
    }
    let mut argv = vec![
        "run".to_owned(),
        "-S".into(),
        "inherit-env=n".into(),
        "-S".into(),
        "inherit-network=n".into(),
        "-S".into(),
        "tcp=n".into(),
        "-S".into(),
        "udp=n".into(),
        "-S".into(),
        "http=n".into(),
        "-S".into(),
        format!("p3={}", if p3 { "y" } else { "n" }),
    ];
    // Compiler guests are reused across package units. Avoid expensive JIT
    // optimization and reuse compiled code for Go as well as shared LLVM tools.
    argv.extend([
        "-O".into(),
        "opt-level=0".into(),
        "-C".into(),
        "cache=y".into(),
    ]);
    for dependency in modules {
        argv.extend([
            "--preload".into(),
            format!("{}={}", dependency.name, dependency.tool.path.display()),
        ]);
    }
    for (key, value) in environment {
        argv.extend(["--env".into(), format!("{key}={value}")]);
    }
    for (host, guest) in mounts {
        let host = host.canonicalize()?;
        ensure!(
            !host.to_string_lossy().contains("::"),
            "unsupported Wasmtime mount path"
        );
        argv.extend(["--dir".into(), format!("{}::{guest}", host.display())]);
    }
    argv.push(module.to_str().context("UTF8 compiler path")?.into());
    argv.extend_from_slice(args);
    let output = Command::new(&runtime.path)
        .args(&argv)
        .env_clear()
        .env("HOME", cache)
        .env("XDG_CACHE_HOME", cache)
        .stdin(Stdio::null())
        .output()
        .context("running Wasm compiler under Wasmtime")?;
    commands.push(json!({"program":runtime.path,"arguments":argv,"status":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)}));
    ensure!(
        output.status.success(),
        "Wasm compiler failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
fn prospective(path: &Path) -> Result<PathBuf> {
    if path.try_exists()? {
        return Ok(path.canonicalize()?);
    }
    let name = path.file_name().context("directory filename")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(prospective(parent)?.join(name))
}

fn world_text(resolve: &Resolve, world: WorldId) -> Result<String> {
    let mut selected = resolve.clone();
    let package = selected.worlds[world]
        .package
        .context("build world package")?;
    selected.packages[package]
        .worlds
        .retain(|_, id| *id == world);
    let nested = selected
        .packages
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != package)
        .collect::<Vec<_>>();
    let mut printer = wit_component::WitPrinter::default();
    printer.print(&selected, package, &nested)?;
    Ok(printer.output.to_string())
}
fn unit_order(
    name: &str,
    units: &BTreeMap<String, Unit>,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
    ordered: &mut Vec<String>,
) -> Result<()> {
    ensure!(units.contains_key(name), "unknown build unit: {name}");
    if done.contains(name) {
        return Ok(());
    }
    ensure!(
        active.insert(name.into()),
        "build dependency cycle at {name}"
    );
    for dependency in units[name].dependencies.values() {
        unit_order(dependency, units, active, done, ordered)?;
    }
    active.remove(name);
    done.insert(name.into());
    ordered.push(name.into());
    Ok(())
}

pub fn run_cli(args: &[String]) -> Result<Value> {
    const USAGE: &str = "usage: wisp build [PROJECT_DIR|CONFIG.toml] [--out OUTPUT] [--cache DIRECTORY] [--component-only] [--go-module MODULE | --go-guest-module MODULE]";
    let (mut project, mut output, mut cache, mut go_module) = (None, None, None, None);
    let mut component_only = false;
    let mut go_guest = false;
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--component-only" => {
                ensure!(!component_only, USAGE);
                component_only = true;
                at += 1;
            }
            "--out" | "--cache" | "--go-module" | "--go-guest-module" => {
                ensure!(at + 1 < args.len(), USAGE);
                if args[at] == "--go-guest-module" {
                    go_guest = true;
                }
                let slot = match args[at].as_str() {
                    "--out" => &mut output,
                    "--cache" => &mut cache,
                    _ => &mut go_module,
                };
                ensure!(slot.replace(args[at + 1].clone()).is_none(), USAGE);
                at += 2;
            }
            value if !value.starts_with('-') => {
                ensure!(project.replace(value.to_owned()).is_none(), USAGE);
                at += 1;
            }
            _ => bail!(USAGE),
        }
    }
    ensure!(
        !component_only || go_module.is_none(),
        "--go-module requires package generation"
    );
    let path = PathBuf::from(project.unwrap_or_else(|| ".".into()));
    let config_path = if path.is_dir() {
        path.join("wisp.toml")
    } else {
        path
    };
    let config_path = config_path.canonicalize()?;
    let root = config_path.parent().context("project directory")?;
    let config_bytes = read(&config_path, 1024 * 1024)?;
    let mut config: Config =
        toml::from_str(std::str::from_utf8(&config_bytes)?).context("build project config")?;
    crate::library::package_names(&config.package.name)?;
    ensure!(
        config.package.version.build.is_empty(),
        "build package version cannot use build metadata"
    );
    let metadata: crate::library_metadata::Authored =
        toml::Value::Table(config.metadata.clone()).try_into()?;
    let mut metadata = metadata.load(root)?;
    if let Some(compiler) = config.build.compiler.take() {
        ensure!(
            config.build.units.is_empty(),
            "build.compiler cannot be combined with build.units"
        );
        config.build.units.insert(
            config.build.root.clone(),
            Unit {
                compiler: Some(compiler),
                component: None,
                world: std::mem::take(&mut config.build.world),
                project: config.build.project.take().or_else(|| {
                    matches!(compiler, Compiler::Go | Compiler::Rust).then(|| ".".into())
                }),
                sources: std::mem::take(&mut config.build.sources),
                includes: std::mem::take(&mut config.build.includes),
                optimization: config.build.optimization.take(),
                entry: config.build.entry.take(),
                javascript_engine: config.build.javascript_engine,
                typecheck: config.build.typecheck,
                dependencies: BTreeMap::new(),
            },
        );
    } else {
        ensure!(
            config.build.world.is_empty()
                && config.build.project.is_none()
                && config.build.sources.is_empty()
                && config.build.includes.is_empty()
                && config.build.optimization.is_none()
                && config.build.entry.is_none()
                && config.build.javascript_engine.is_none()
                && !config.build.typecheck,
            "project, sources, includes and optimization at build level require build.compiler"
        );
    }
    let inferred = config.build.wit.is_none()
        && config
            .build
            .units
            .values()
            .all(|unit| unit.world.is_empty());
    ensure!(
        inferred || config.dependencies.is_empty(),
        "installed package dependencies currently require inferred source contracts"
    );
    let mut resolve = Resolve::default();
    let inferred_contracts = if inferred {
        Some(infer::generate(&mut config, root, &mut resolve)?)
    } else {
        None
    };
    if let Some(contracts) = &inferred_contracts {
        for (name, notice) in &contracts.notices {
            if let Some(existing) = metadata.notices.insert(name.clone(), notice.clone()) {
                ensure!(existing == *notice, "conflicting dependency notice: {name}");
            }
        }
        metadata.validate()?;
    }
    ensure!(
        !config.build.units.is_empty() && config.build.units.len() <= 32,
        "build requires 1 to 32 units"
    );
    let mut ordered = Vec::new();
    unit_order(
        &config.build.root,
        &config.build.units,
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
        &mut ordered,
    )?;
    ensure!(
        ordered.len() == config.build.units.len(),
        "all build units must be reachable from build.root"
    );
    let package = if let Some(contracts) = &inferred_contracts {
        contracts.package
    } else {
        let path = config.build.wit.clone().unwrap_or_else(|| "wit".into());
        let path = input(root, &path)?;
        if path.is_file() {
            resolve.push_str(&path, std::str::from_utf8(&read(&path, MAX_SOURCE)?)?)?
        } else {
            resolve.push_dir(&path)?.0
        }
    };
    let identity = &resolve.packages[package].name;
    ensure!(
        format!("{}:{}", identity.namespace, identity.name) == config.package.name
            && identity.version.as_ref() == Some(&config.package.version),
        "build WIT identity mismatch"
    );
    let mut worlds = BTreeMap::new();
    let mut prebuilt = BTreeMap::new();
    for (name, unit) in &config.build.units {
        crate::identity::parse(&format!("build:{name}"))?;
        ensure!(
            (unit.javascript_engine.is_none() && !unit.typecheck)
                || matches!(
                    unit.compiler,
                    Some(Compiler::Javascript | Compiler::Typescript)
                ),
            "javascript-engine and typecheck require a JavaScript or TypeScript compiler"
        );
        ensure!(
            matches!(unit.optimization.as_deref(), None | Some("O0" | "O1")),
            "unsupported build optimization: {:?}",
            unit.optimization
        );
        let world = if let Some(contracts) = &inferred_contracts {
            contracts.worlds[name]
        } else {
            resolve.select_world(
                &[package],
                (!unit.world.is_empty()).then_some(unit.world.as_str()),
            )?
        };
        crate::library::validate_type_expansion(&resolve, world)?;
        for (key, item) in &resolve.worlds[world].imports {
            if matches!(item, WorldItem::Type { .. }) {
                continue;
            }
            let import = resolve.name_world_key(key);
            ensure!(
                unit.dependencies.contains_key(&import)
                    || ((unit.component.is_some()
                        || unit.compiler.is_some_and(Compiler::is_adapter))
                        && import.starts_with("wasi:")),
                "unit {name} has no dependency for WIT import {import}"
            );
        }
        let imports = resolve.worlds[world]
            .imports
            .iter()
            .filter(|(_, i)| !matches!(i, WorldItem::Type { .. }))
            .map(|(k, _)| resolve.name_world_key(k))
            .collect::<BTreeSet<_>>();
        ensure!(
            unit.dependencies.keys().all(|k| imports.contains(k)),
            "unit {name} declares a dependency that is not a WIT import"
        );
        match (unit.compiler, &unit.component) {
            (Some(Compiler::Clang), None) => ensure!(
                !unit.sources.is_empty() && unit.project.is_none(),
                "C/C++ unit {name} requires sources and cannot set project"
            ),
            (Some(Compiler::Go), None) => ensure!(
                unit.project.is_some()
                    && unit.sources.is_empty()
                    && unit.includes.is_empty()
                    && unit.optimization.is_none(),
                "Go unit {name} requires project; sources, includes and optimization overrides are not supported"
            ),
            (Some(Compiler::Rust), None) => ensure!(
                unit.project.is_some()
                    && unit.sources.is_empty()
                    && unit.includes.is_empty()
                    && unit.optimization.is_none(),
                "rust unit {name} requires a project; sources, includes and optimization overrides are not supported"
            ),
            (Some(compiler), None) if compiler.is_adapter() => {
                adapters::validate(compiler, unit).with_context(|| {
                    format!(
                        "{compiler_name} unit {name}",
                        compiler_name = compiler.name()
                    )
                })?;
            }
            (None, Some(artifact)) => {
                ensure!(
                    unit.sources.is_empty()
                        && unit.includes.is_empty()
                        && unit.project.is_none()
                        && unit.optimization.is_none(),
                    "prebuilt Component unit {name} cannot set sources, includes, project or optimization"
                );
                let artifact = match artifact {
                    ComponentInput::File(file) => ComponentFile {
                        path: file.path.clone(),
                        sha256: file.sha256.clone(),
                    },
                    ComponentInput::Module(module) => {
                        let mut found = Vec::new();
                        for project in config
                            .build
                            .units
                            .values()
                            .filter_map(|u| u.project.as_ref())
                        {
                            let project = input(root, project)?;
                            if let Some((path, sha256)) =
                                crate::build_go::published_component(&project, &module.module)?
                            {
                                found.push(ComponentFile {
                                    path: path.strip_prefix(root)?.to_owned(),
                                    sha256,
                                });
                            }
                        }
                        ensure!(
                            found.len() == 1,
                            "component module {} must be installed by go mod vendor in exactly one Go project",
                            module.module
                        );
                        found.pop().unwrap()
                    }
                };
                prebuilt.insert(
                    name.clone(),
                    (
                        prebuilt_component(root, &artifact, &resolve, world)
                            .with_context(|| format!("prebuilt Component unit {name}"))?,
                        artifact.path,
                    ),
                );
            }
            _ => bail!("unit {name} must set exactly one of compiler or component"),
        };
        for path in unit
            .sources
            .iter()
            .chain(&unit.includes)
            .chain(unit.project.iter())
        {
            input(root, path)?;
        }
        worlds.insert(name.clone(), world);
    }
    let output = output
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("dist"));
    ensure!(
        !output.try_exists()?,
        "output already exists; choose a fresh directory"
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let cache = cache
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".wisp-cache"));
    let output_location = prospective(&output)?;
    let cache_location = prospective(&cache)?;
    ensure!(
        !cache_location.starts_with(&output_location)
            && !output_location.starts_with(&cache_location),
        "build output and runtime cache must be separate directories"
    );
    fs::create_dir_all(&cache)?;
    let cache = cache.canonicalize()?;
    let runtime = if config
        .build
        .units
        .values()
        .any(|u| matches!(u.compiler, Some(Compiler::Clang | Compiler::Go)))
    {
        let runtime = crate::compiler_hosts::select(
            root,
            &[("WISP_WASMTIME", &config.build.tools.wasmtime)],
            crate::compiler_hosts::COMMAND_VERSION,
            crate::compiler_hosts::Contract::WorkerProtocol,
        )?;
        Some(runtime)
    } else {
        None
    };
    let clang = if config
        .build
        .units
        .values()
        .any(|u| matches!(u.compiler, Some(Compiler::Clang)))
    {
        Some(tool(root, &config.build.tools.clang, "WISP_CLANG")?)
    } else {
        None
    };
    let clang_sysroot = if clang.is_some() {
        let configured = std::env::var_os("WISP_CLANG_WASI_SYSROOT")
            .map(PathBuf::from)
            .or_else(|| config.build.tools.clang_wasi_sysroot.clone());
        let selected = match configured {
            Some(path) => Some(path),
            None => crate::toolchain_install::resolve("WISP_CLANG_WASI_SYSROOT")?,
        };
        match selected {
            Some(path) => {
                let path = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                }
                .canonicalize()?;
                ensure!(
                    path.join("include").is_dir()
                        && path.join("lib/wasm32-wasip1/libc.a").is_file(),
                    "Clang WASI sysroot is missing target headers or libc"
                );
                Some(path)
            }
            None => None,
        }
    } else {
        None
    };
    let go = if config
        .build
        .units
        .values()
        .any(|u| matches!(u.compiler, Some(Compiler::Go)))
    {
        Some(crate::build_go::Toolchain::load(&tool_path(
            root,
            &config.build.tools.go,
            "WISP_GO_TOOLCHAIN",
        )?)?)
    } else {
        None
    };
    let llvm = if clang.is_some() {
        Some(crate::build_llvm::SharedLlvm::load(&tool_path(
            root,
            &config.build.tools.llvm,
            "WISP_LLVM",
        )?)?)
    } else {
        None
    };
    if let Some(toolchain) = &go {
        let license = String::from_utf8(read(&toolchain.root.join("LICENSE"), 256 * 1024)?)?;
        if let Some(existing) = metadata.notices.get("Go-LICENSE.txt") {
            ensure!(
                existing == &license,
                "Go-LICENSE.txt conflicts with the compiled Go runtime license"
            );
        } else {
            metadata.notices.insert("Go-LICENSE.txt".into(), license);
        }
        metadata.validate()?;
    }
    let rust = if config
        .build
        .units
        .values()
        .any(|u| matches!(u.compiler, Some(Compiler::Rust)))
    {
        let configured = tool_path(root, &config.build.tools.rust, "WISP_RUST")?;
        crate::build_rust::Toolchain::compiler_host(&configured)?;
        let runtime = crate::compiler_hosts::select(
            root,
            &[
                ("WISP_RUST_WASMTIME", &config.build.tools.rust_wasmtime),
                ("WISP_WASMTIME", &config.build.tools.wasmtime),
            ],
            crate::compiler_hosts::RUST_WASIP3_VERSION,
            crate::compiler_hosts::Contract::WasmtimeCli,
        )?;
        let toolchain = crate::build_rust::Toolchain::load(&configured, &runtime.path)?;
        for (name, text) in &toolchain.notices {
            if let Some(existing) = metadata.notices.insert(name.clone(), text.clone()) {
                ensure!(existing == *text, "conflicting Rust runtime notice: {name}");
            }
        }
        metadata.validate()?;
        Some(toolchain)
    } else {
        None
    };
    let adapter = if go.is_some() || rust.is_some() || clang_sysroot.is_some() {
        let adapter = tool(
            root,
            &config.build.tools.preview1_adapter,
            "WISP_PREVIEW1_ADAPTER",
        )?;
        ensure!(
            adapter.sha256 == crate::toolchain_install::ADAPTER_SHA256,
            "WASI reactor requires the pinned Wasmtime {} preview1 reactor adapter",
            crate::toolchain_install::ADAPTER_VERSION
        );
        Some(adapter)
    } else {
        None
    };
    let mut adapter_toolchains = BTreeMap::new();
    for unit in config.build.units.values() {
        if let Some(compiler) = unit.compiler.filter(|compiler| compiler.is_adapter()) {
            if !adapter_toolchains.contains_key(compiler.name()) {
                let toolchain = adapters::Toolchain::load(root, &config.build.tools, compiler)?;
                for (name, notice) in toolchain.notices()? {
                    if let Some(existing) = metadata.notices.insert(name.clone(), notice.clone()) {
                        ensure!(
                            existing == notice,
                            "compiler notice conflicts with authored notice {name}"
                        );
                    }
                }
                adapter_toolchains.insert(compiler.name(), toolchain);
            }
        }
    }
    metadata.validate()?;
    let stage = tempfile::tempdir_in(parent)?;
    let build = stage.path().join("build");
    fs::create_dir(&build)?;
    let mut reports = BTreeMap::new();
    let mut descriptors = BTreeMap::new();
    for name in &ordered {
        eprintln!("building {name} with {}", config.build.units[name].kind());
        let unit = &config.build.units[name];
        let world = worlds[name];
        let work = build.join("units").join(name);
        let source = work.join("source");
        fs::create_dir_all(&source)?;
        let scratch = work.join("work");
        fs::create_dir(&scratch)?;
        let mut files = BTreeMap::new();
        let mut total = 0;
        let mut commands = Vec::new();
        let component = if let Some((bytes, path)) = prebuilt.remove(name) {
            let path = path
                .to_str()
                .context("UTF8 Component path")?
                .replace('\\', "/");
            files.insert(path, digest(&bytes));
            write(source.join("component.wasm"), &bytes)?;
            bytes
        } else if unit.compiler.is_some_and(Compiler::is_adapter) {
            let compiler = unit.compiler.unwrap();
            let toolchain = adapter_toolchains.get(compiler.name()).unwrap();
            let component = toolchain.compile(
                root,
                unit,
                &source,
                &scratch,
                &cache,
                &resolve,
                world,
                &mut files,
                &mut total,
                &mut commands,
            )?;
            if let Some(contracts) = &inferred_contracts {
                check_inferred_sources(contracts, name, &files)?;
                if contracts.complete_sources.contains(name) {
                    ensure!(
                        contracts.sources[name] == files,
                        "C# source inventory changed during API discovery; rebuild the project"
                    );
                }
            }
            component
        } else {
            let mut core = match unit.compiler.unwrap() {
                Compiler::Clang => {
                    let mut sources = Vec::new();
                    let mut language = None;
                    let mut includes = BTreeSet::from(["/src".to_owned()]);
                    for path in &unit.sources {
                        let absolute = input(root, path)?;
                        let cpp = match path.extension().and_then(|s| s.to_str()) {
                            Some("c") => false,
                            Some("cc" | "cpp" | "cxx") => true,
                            _ => bail!("unsupported C/C++ source extension: {}", path.display()),
                        };
                        ensure!(
                            language.is_none_or(|prior| prior == cpp),
                            "a C/C++ unit cannot mix C and C++ sources"
                        );
                        language = Some(cpp);
                        copy_file(root, &absolute, &source, &mut files, &mut total)?;
                        let parent = absolute.parent().context("source parent")?;
                        copy_tree(root, parent, &source, false, &mut files, &mut total)?;
                        includes.insert(format!(
                            "/src/{}",
                            parent
                                .strip_prefix(root)?
                                .to_str()
                                .context("include path")?
                        ));
                        sources.push(format!(
                            "/src/{}",
                            absolute
                                .strip_prefix(root)?
                                .to_str()
                                .context("source path")?
                        ));
                    }
                    for include in &unit.includes {
                        let path = input(root, include)?;
                        copy_tree(root, &path, &source, false, &mut files, &mut total)?;
                        includes.insert(format!(
                            "/src/{}",
                            path.strip_prefix(root)?.to_str().context("include path")?
                        ));
                    }
                    let is_cpp = language.unwrap() || inferred;
                    ensure!(
                        clang_sysroot.is_some(),
                        "C/C++ compilation requires WISP_CLANG_WASI_SYSROOT or build.tools.clang-wasi-sysroot"
                    );
                    let generated = work.join("bindings");
                    let (glue, objects) = if let Some(contracts) = &inferred_contracts {
                        check_inferred_sources(contracts, name, &files)?;
                        let result = inferred::cpp::clang(
                            &resolve,
                            world,
                            &contracts.functions[name],
                            &sources,
                            &generated,
                        )?;
                        if let Some(headers) = contracts.cpp_headers.get(name) {
                            for (header, contents) in headers {
                                write(generated.join(header), contents)?;
                            }
                        }
                        result
                    } else {
                        upstream::clang(&resolve, world, is_cpp, &sources, &generated)?
                    };
                    let filename = if is_cpp { "unit.cpp" } else { "unit.c" };
                    write(generated.join(filename), glue)?;
                    let mut args = vec![
                        format!("/bindings/{filename}"),
                        "-o".into(),
                        "/build/core.wasm".into(),
                        format!("-{}", unit.optimization.as_deref().unwrap_or("O0")),
                        "-I".into(),
                        "/bindings".into(),
                    ];
                    let mut mounts: Vec<(&Path, &str)> = vec![
                        (&source, "/src"),
                        (&generated, "/bindings"),
                        (&scratch, "/build"),
                    ];
                    let sdk_root = clang_sysroot
                        .as_ref()
                        .map(|sysroot| {
                            sysroot
                                .parent()
                                .and_then(Path::parent)
                                .context("WASI SDK sysroot parent")
                        })
                        .transpose()?;
                    if let Some(sdk) = sdk_root {
                        mounts.push((sdk, "/wasi-sdk"));
                        args.extend([
                            "--wasi-sysroot".into(),
                            "/wasi-sdk/share/wasi-sysroot".into(),
                        ]);
                        if is_cpp {
                            args.push("-std=c++23".into());
                        }
                    }
                    for object in objects {
                        args.extend(["--link-object".into(), format!("/bindings/{object}")]);
                    }
                    for include in includes {
                        args.extend(["-I".into(), include]);
                    }
                    run_wasm(
                        runtime.as_ref().unwrap(),
                        &clang.as_ref().unwrap().path,
                        true,
                        &args,
                        &mounts,
                        &[],
                        &cache,
                        &llvm.as_ref().unwrap().modules,
                        &mut commands,
                    )?;
                    llvm.as_ref().unwrap().validate_unchanged()?;
                    read(&scratch.join("core.wasm"), MAX_COMPONENT)?
                }
                Compiler::Go => {
                    let project = input(root, unit.project.as_ref().unwrap())?;
                    copy_tree(&project, &project, &source, true, &mut files, &mut total)?;
                    for dependency in unit.dependencies.values() {
                        if let Some(ComponentInput::Module(module)) =
                            &config.build.units[dependency].component
                        {
                            let (_, hash) =
                                crate::build_go::published_component(&source, &module.module)?
                                    .context("staged Go dependency is missing")?;
                            ensure!(reports.get(dependency).is_some_and(|report: &Value|report["componentSha256"] == hash),
                                "Go dependency changed while staging the build");
                        }
                    }
                    if let Some(contracts) = &inferred_contracts {
                        check_inferred_sources(contracts, name, &files)?;
                        inferred::go::generate(
                            &resolve,
                            world,
                            &contracts.functions[name],
                            &source,
                        )?;
                    } else {
                        upstream::go(&resolve, world, &source)?;
                    }
                    crate::build_go::stage_wit_runtime(&source, go.as_ref().unwrap())?;
                    let core = crate::build_go::compile(
                        runtime.as_ref().unwrap(),
                        go.as_ref().unwrap(),
                        &source,
                        &scratch,
                        &cache,
                        &mut commands,
                    )?;
                    core
                }
                Compiler::Rust => {
                    let project = input(root, unit.project.as_ref().unwrap())?;
                    files = crate::build_rust::snapshot(&project, &source)?;
                    for dependency in crate::build_rust::guest_dependencies(&source)? {
                        let hash = dependency.descriptor["component"]["sha256"]
                            .as_str()
                            .context("Rust guest Component hash")?;
                        ensure!(
                            unit.dependencies.values().any(|name| reports
                                .get(name)
                                .is_some_and(|report: &Value| report["componentSha256"] == hash)),
                            "Rust dependency changed while staging the build"
                        );
                    }
                    let wrapper = if let Some(contracts) = &inferred_contracts {
                        check_inferred_sources(contracts, name, &files)?;
                        let project = crate::build_rust::project(&source)?;
                        let wrapper = inferred::rust::rust(
                            &resolve,
                            world,
                            &contracts.functions[name],
                            &project.library,
                        )?;
                        write(work.join("bindings/wisp.rs"), &wrapper)?;
                        Some(wrapper)
                    } else {
                        upstream::rust(&resolve, world, &source)?;
                        None
                    };
                    crate::build_rust::compile(
                        rust.as_ref().unwrap(),
                        &source,
                        &scratch,
                        wrapper.as_deref(),
                        &cache,
                        &mut commands,
                    )?
                }
                _ => unreachable!("adapter compiler handled separately"),
            };
            if matches!(unit.compiler, Some(Compiler::Go)) {
                // C/C++ metadata objects and generated Rust sections already
                // describe the selected world. Go emits source bindings only.
                wit_component::embed_component_metadata(
                    &mut core,
                    &resolve,
                    world,
                    wit_component::StringEncoding::UTF8,
                )?;
            }
            let mut encoder = wit_component::ComponentEncoder::default()
                .validate(true)
                .module(&core)?;
            if matches!(unit.compiler, Some(Compiler::Go | Compiler::Rust))
                || matches!(unit.compiler, Some(Compiler::Clang)) && clang_sysroot.is_some()
            {
                encoder = encoder.adapter(
                    "wasi_snapshot_preview1",
                    &read(&adapter.as_ref().unwrap().path, MAX_COMPONENT)?,
                )?;
            }
            encoder
                .encode()
                .with_context(|| format!("assembling Component for unit {name}"))?
        };
        ensure!(
            component.len() as u64 <= MAX_COMPONENT,
            "build Component size limit"
        );
        adapters::validate_component(&component, &resolve, world)
            .with_context(|| format!("compiled unit {name} differs from its WIT contract"))?;
        write(work.join("component.wasm"), &component)?;
        let world_wit = world_text(&resolve, world)?;
        write(work.join("world.wit"), &world_wit)?;
        let wit_parser::decoding::DecodedWasm::Component(decoded, decoded_world) =
            wit_parser::decoding::decode(&component)?
        else {
            bail!("expected built Component")
        };
        let dependencies=unit.dependencies.iter().map(|(import,unit)|(import.clone(),json!({"package":format!("build:{unit}"),"version":format!("={}",config.package.version)}))).collect::<BTreeMap<_,_>>();
        let hosts = decoded.worlds[decoded_world]
            .imports
            .iter()
            .filter(|(_, item)| !matches!(item, WorldItem::Type { .. }))
            .map(|(key, _)| decoded.name_world_key(key))
            .filter(|key| !dependencies.contains_key(key))
            .collect::<Vec<_>>();
        ensure!(
            hosts.iter().all(|name| name.starts_with("wasi:")),
            "unresolved non-WASI imports in unit {name}"
        );
        let descriptor = json!({"schema":1,"kind":"component-dependency-package","identity":format!("build:{name}"),"version":config.package.version,"component":{"path":"component.wasm","sha256":digest(&component)},"dependencies":dependencies,"hostImports":hosts});
        write(
            work.join("component.json"),
            serde_json::to_vec_pretty(&descriptor)?,
        )?;
        descriptors.insert(name.clone(), descriptor);
        reports.insert(name.clone(),json!({"compiler":unit.kind(),"world":unit.world,"witSha256":digest(world_wit.as_bytes()),"inputs":files,"commands":commands,"componentSha256":digest(&component)}));
    }
    let component = if ordered.len() == 1 {
        read(
            &build
                .join("units")
                .join(&config.build.root)
                .join("component.wasm"),
            MAX_COMPONENT,
        )?
    } else {
        let registry = stage.path().join("registry");
        for (name, descriptor) in &descriptors {
            let dir = registry
                .join("build")
                .join(name)
                .join(config.package.version.to_string());
            write(
                dir.join("component.json"),
                serde_json::to_vec_pretty(descriptor)?,
            )?;
            fs::copy(
                build.join("units").join(name).join("component.wasm"),
                dir.join("component.wasm"),
            )?;
        }
        let root_manifest = build
            .join("units")
            .join(&config.build.root)
            .join("component.json");
        let lock = build.join("component.lock.json");
        let component_cache = stage.path().join("components");
        crate::dependencies::resolve_lock(
            &root_manifest,
            &registry,
            &component_cache,
            &lock,
            None,
        )?;
        let composed = build.join("composition");
        crate::dependencies::compose_lock(&lock, &component_cache, &composed)?;
        read(&composed.join("component.wasm"), MAX_COMPONENT)?
    };
    let authored = stage.path().join("authored");
    write(authored.join("component.wasm"), &component)?;
    write(
        authored.join("wit/world.wit"),
        world_text(&resolve, worlds[&config.build.root])?,
    )?;
    let mut authored_metadata = config.metadata.clone();
    let mut notices = Vec::new();
    for (name, text) in &metadata.notices {
        let name = format!("notices/{name}");
        write(authored.join(&name), text)?;
        notices.push(toml::Value::String(name));
    }
    authored_metadata.insert("notice-files".into(), toml::Value::Array(notices));
    let mut manifest = toml::Table::new();
    manifest.insert(
        "package".into(),
        toml::Value::try_from(
            json!({"name":config.package.name,"version":config.package.version.to_string()}),
        )?,
    );
    manifest.insert(
        "component".into(),
        toml::Value::try_from(json!({"path":"component.wasm","sha256":digest(&component)}))?,
    );
    manifest.insert("metadata".into(), toml::Value::Table(authored_metadata));
    write(
        authored.join("wisp.toml"),
        toml::to_string_pretty(&manifest)?,
    )?;
    let mut tools = json!({"componentEncoder":"0.245.1"});
    if let Some(runtime) = runtime {
        tools["wasmtime"] = json!({"path":runtime.path,"sha256":runtime.sha256,"version":runtime.backend.wasmtime_version(),"backend":runtime.backend.name()});
    }
    if let Some(tool) = clang {
        tools["clang"] = json!({"path":tool.path,"sha256":tool.sha256});
    }
    if let Some(toolchain) = llvm {
        tools["llvm"] = toolchain.receipt();
    }
    if let Some(tool) = go {
        tools["go"] = tool.receipt();
    }
    if let Some(toolchain) = rust {
        tools["rust"] = toolchain.receipt();
    }
    if let Some(tool) = adapter {
        tools["preview1Adapter"] = json!({"path":tool.path,"sha256":tool.sha256});
    }
    for (name, toolchain) in &adapter_toolchains {
        tools[*name] = toolchain.receipt();
    }
    let receipt = json!({"schema":1,"kind":"source-build","contractMode":if inferred {"inferred"}else{"wit"},"identity":config.package.name,"version":config.package.version,"configSha256":digest(&config_bytes),"root":config.build.root,"tools":tools,"units":reports,"componentSha256":digest(&component),"packageGenerated":!component_only});
    write(
        build.join("build.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    let release = if component_only {
        authored
    } else {
        let release = stage.path().join("release");
        let mut args = vec![
            "pack".into(),
            authored.to_str().context("package path")?.into(),
            "--out".into(),
            release.to_str().context("output path")?.into(),
        ];
        if let Some(module) = go_module {
            args.extend([
                if go_guest {
                    "--go-guest-module"
                } else {
                    "--go-module"
                }
                .into(),
                module,
            ]);
        }
        crate::library::run_cli(&args)?;
        release
    };
    fs::rename(&build, release.join("build"))?;
    fs::create_dir(&output)?;
    if let Err(error) = fs::rename(&release, &output) {
        let _ = fs::remove_dir(&output);
        return Err(error.into());
    }
    Ok(json!({"output":output.canonicalize()?,"build":receipt}))
}

#[cfg(test)]
mod naming_tests {
    use super::*;

    #[test]
    fn accepts_compiler_names() {
        for name in [
            "clang",
            "go",
            "rust",
            "zig",
            "assemblyscript",
            "nim",
            "javascript",
            "typescript",
            "python",
            "grain",
            "goose",
            "bend",
            "swift",
            "csharp",
            "java",
            "kotlin",
        ] {
            let config: Config = toml::from_str(&format!(
                "[package]\nname='test:names'\nversion='0.1.0'\n[build]\ncompiler='{name}'\n"
            ))
            .unwrap();
            assert_eq!(config.build.compiler.unwrap().name(), name);
            assert_eq!(
                config.build.compiler.unwrap().is_adapter(),
                !matches!(name, "clang" | "go" | "rust")
            );
        }
        assert!(
            toml::from_str::<Config>(
                "[package]\nname='test:names'\nversion='0.1.0'\n[build]\ncompiler='cpp'\n"
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_tool_keys_and_rejects_unknown_keys() {
        let tools: Tools = toml::from_str("clang='compiler.wasm'\ngo='go-toolchain'\n").unwrap();
        assert_eq!(tools.clang, Some(PathBuf::from("compiler.wasm")));
        assert_eq!(tools.go, Some(PathBuf::from("go-toolchain")));
        assert!(toml::from_str::<Tools>("cpp='a'\n").is_err());
        assert!(toml::from_str::<Tools>("compiler='a'\n").is_err());
    }
}
