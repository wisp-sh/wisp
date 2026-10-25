//! Local Component resolution. These records do not alter language archive schemas.
use crate::identity::parse as identity;
use anyhow::{Context, Result, bail, ensure};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
};
use wit_parser::{Resolve, WorldId, WorldItem};

use crate::limits::MAX_COMPONENT_BYTES as MAX_COMPONENT;
const MAX_JSON: u64 = 1024 * 1024;
const MAX_NODES: usize = 64;
const MAX_EDGES: usize = 256;
const MAX_SEARCH: usize = 5000;
const MAX_GRAPH_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub package: String,
    pub version: VersionReq,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Package {
    pub schema: u32,
    pub kind: String,
    pub identity: String,
    pub version: Version,
    pub component: Artifact,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Requirement>,
    #[serde(default)]
    pub host_imports: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockedNode {
    pub identity: String,
    pub version: Version,
    pub sha256: String,
    pub manifest_sha256: String,
    pub contract_sha256: String,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub type_exports: Vec<String>,
    pub host_imports: Vec<String>,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lock {
    pub schema: u32,
    pub kind: String,
    pub root: String,
    pub manifest_sha256: String,
    pub nodes: BTreeMap<String, LockedNode>,
}

#[derive(Clone, Debug)]
struct Inventory {
    imports: Vec<String>,
    exports: Vec<String>,
    type_exports: Vec<String>,
    contract: String,
}
#[derive(Clone, Debug)]
struct Loaded {
    package: Package,
    bytes: Vec<u8>,
    inventory: Inventory,
    manifest: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn id(package: &Package) -> String {
    format!("{}@{}", package.identity, package.version)
}
fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    // JSON objects are sorted by serde_json's default map representation.
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= limit,
        "invalid file or size limit: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "file size limit: {}",
        path.display()
    );
    Ok(bytes)
}
fn json_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_value(crate::release::strict_json(
        &read(path, MAX_JSON)?,
    )?)?)
}
fn safe_relative(base: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
        "unsafe Component path: {}",
        relative.display()
    );
    let mut path = base.to_path_buf();
    for part in relative.components() {
        path.push(part);
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "symlinked Component path"
        );
    }
    Ok(path)
}
fn names(resolve: &Resolve, world: WorldId, imports: bool) -> Vec<String> {
    let items = if imports {
        &resolve.worlds[world].imports
    } else {
        &resolve.worlds[world].exports
    };
    let mut result = items
        .iter()
        .filter(|(_, item)| !matches!(item, WorldItem::Type { .. }))
        .map(|(key, _)| resolve.name_world_key(key))
        .collect::<Vec<_>>();
    result.sort();
    result
}
fn inventory(bytes: &[u8]) -> Result<Inventory> {
    ensure!(
        bytes.len() <= MAX_COMPONENT as usize && bytes.starts_with(b"\0asm\x0d\0\x01\0"),
        "expected executable Component"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(bytes)
        .context("invalid Component binary")?;
    let decoded =
        std::panic::catch_unwind(|| wit_parser::decoding::decode(bytes)).map_err(|_| {
            anyhow::anyhow!("Component WIT decoder rejected an unsupported type graph")
        })??;
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) = decoded else {
        bail!("expected executable Component, not encoded WIT");
    };
    let package = resolve.worlds[world]
        .package
        .context("Component world package")?;
    let other = resolve
        .packages
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != package)
        .collect::<Vec<_>>();
    let mut printer = wit_component::WitPrinter::default();
    printer.print(&resolve, package, &other)?;
    Ok(Inventory {
        imports: names(&resolve, world, true),
        exports: names(&resolve, world, false),
        type_exports: resolve.worlds[world]
            .exports
            .iter()
            .filter_map(|(name, item)| {
                let WorldItem::Interface { id, .. } = item else {
                    return None;
                };
                let interface = &resolve.interfaces[*id];
                (interface.functions.is_empty() && !interface.types.is_empty())
                    .then(|| resolve.name_world_key(name))
            })
            .collect(),
        contract: digest(printer.output.to_string().as_bytes()),
    })
}
fn check_package(package: &Package, found: &Inventory) -> Result<()> {
    ensure!(
        package.schema == 1 && package.kind == "component-dependency-package",
        "unsupported Component package schema or kind"
    );
    identity(&package.identity)?;
    ensure!(
        package.version.build.is_empty(),
        "Component versions cannot use build metadata"
    );
    ensure!(
        valid_hash(&package.component.sha256),
        "invalid Component sha256"
    );
    ensure!(
        package.dependencies.len() <= MAX_EDGES,
        "dependency edge limit"
    );
    let hosts = package
        .host_imports
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    ensure!(
        hosts.len() == package.host_imports.len(),
        "duplicate host import"
    );
    let mut declared = hosts.clone();
    for (import, requirement) in &package.dependencies {
        identity(&requirement.package)?;
        ensure!(
            import.len() <= 512 && !import.is_empty(),
            "invalid dependency import"
        );
        ensure!(
            declared.insert(import.clone()),
            "import declared as both host and dependency: {import}"
        );
    }
    ensure!(
        declared == found.imports.iter().cloned().collect(),
        "Component imports differ from declared dependencies and host imports for {}",
        id(package)
    );
    ensure!(
        !found.exports.is_empty(),
        "Component must export a runtime interface or function"
    );
    Ok(())
}
fn load(path: &Path) -> Result<Arc<Loaded>> {
    let package: Package = json_file(path)?;
    let component = safe_relative(
        path.parent().context("package directory")?,
        &package.component.path,
    )?;
    let bytes = read(&component, MAX_COMPONENT)?;
    ensure!(
        digest(&bytes) == package.component.sha256,
        "Component checksum mismatch: {}",
        id(&package)
    );
    let inventory = inventory(&bytes)?;
    check_package(&package, &inventory)?;
    let manifest = digest(&canonical(&package)?);
    Ok(Arc::new(Loaded {
        package,
        bytes,
        inventory,
        manifest,
    }))
}
fn registry_path(registry: &Path, package: &str, version: &Version) -> Result<PathBuf> {
    let (namespace, name) = identity(package)?;
    ensure!(
        version.build.is_empty(),
        "Component versions cannot use build metadata"
    );
    let mut path = registry.to_path_buf();
    for part in [namespace, name, &version.to_string()] {
        path.push(part);
        directory_without_symlink(&path)?;
    }
    Ok(path.join("component.json"))
}
fn directory_without_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "symlinked or invalid package/cache directory: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
fn cache_file(cache: &Path, hash: &str, metadata: bool) -> Result<PathBuf> {
    ensure!(valid_hash(hash), "invalid cache hash");
    let directory = cache.join(if metadata { "manifests" } else { "components" });
    directory_without_symlink(&directory)?;
    Ok(directory.join(format!("{hash}.{}", if metadata { "json" } else { "wasm" })))
}
fn store_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    if path.try_exists()? {
        ensure!(
            read(path, MAX_GRAPH_BYTES as u64)? == bytes,
            "cache checksum mismatch: {}",
            path.display()
        );
        return Ok(());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temporary.write_all(bytes)?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure!(
                read(path, MAX_GRAPH_BYTES as u64)? == bytes,
                "cache checksum mismatch: {}",
                path.display()
            );
            Ok(())
        }
        Err(error) => Err(error.error.into()),
    }
}
fn cache_store(cache: &Path, package: &Loaded) -> Result<()> {
    store_file(
        &cache_file(cache, &package.package.component.sha256, false)?,
        &package.bytes,
    )?;
    store_file(
        &cache_file(cache, &package.manifest, true)?,
        &canonical(&package.package)?,
    )
}
fn cached(cache: &Path, node: &LockedNode) -> Result<Arc<Loaded>> {
    let manifest_bytes = read(&cache_file(cache, &node.manifest_sha256, true)?, MAX_JSON)?;
    ensure!(
        digest(&manifest_bytes) == node.manifest_sha256,
        "cache manifest checksum mismatch"
    );
    let package: Package = serde_json::from_value(crate::release::strict_json(&manifest_bytes)?)?;
    let bytes = read(&cache_file(cache, &node.sha256, false)?, MAX_COMPONENT)?;
    ensure!(
        digest(&bytes) == node.sha256,
        "cache checksum mismatch: {}@{}",
        node.identity,
        node.version
    );
    ensure!(
        package.identity == node.identity
            && package.version == node.version
            && package.component.sha256 == node.sha256,
        "locked package identity mismatch"
    );
    let inventory = inventory(&bytes)?;
    check_package(&package, &inventory)?;
    ensure!(
        inventory.imports == node.imports
            && inventory.exports == node.exports
            && inventory.type_exports == node.type_exports
            && inventory.contract == node.contract_sha256
            && package.host_imports == node.host_imports,
        "locked Component contract mismatch"
    );
    Ok(Arc::new(Loaded {
        package,
        bytes,
        inventory,
        manifest: node.manifest_sha256.clone(),
    }))
}

fn visit(
    name: &str,
    nodes: &BTreeMap<String, LockedNode>,
    active: &mut BTreeSet<String>,
    seen: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) -> Result<()> {
    ensure!(
        !active.contains(name),
        "Component dependency cycle at {name}"
    );
    if seen.contains(name) {
        return Ok(());
    }
    let node = nodes
        .get(name)
        .with_context(|| format!("missing locked dependency {name}"))?;
    active.insert(name.into());
    for child in node.dependencies.values() {
        visit(child, nodes, active, seen, order)?;
    }
    active.remove(name);
    seen.insert(name.into());
    order.push(name.into());
    Ok(())
}
fn order(lock: &Lock) -> Result<Vec<String>> {
    ensure!(
        lock.schema == 1 && lock.kind == "component-dependencies",
        "unsupported Component lock schema or kind"
    );
    ensure!(
        !lock.nodes.is_empty() && lock.nodes.len() <= MAX_NODES,
        "Component dependency node limit"
    );
    ensure!(
        lock.nodes
            .values()
            .map(|n| n.dependencies.len())
            .sum::<usize>()
            <= MAX_EDGES,
        "dependency edge limit"
    );
    ensure!(
        valid_hash(&lock.manifest_sha256),
        "invalid root manifest hash"
    );
    let mut result = Vec::new();
    visit(
        &lock.root,
        &lock.nodes,
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
        &mut result,
    )?;
    ensure!(
        result.len() == lock.nodes.len(),
        "unreachable locked Component"
    );
    Ok(result)
}
fn validate_cache(lock: &Lock, cache: &Path) -> Result<BTreeMap<String, Arc<Loaded>>> {
    order(lock)?;
    let mut loaded = BTreeMap::new();
    let mut bytes = 0;
    let mut slots = BTreeMap::new();
    for (name, node) in &lock.nodes {
        ensure!(
            name == &format!("{}@{}", node.identity, node.version),
            "locked node identity mismatch"
        );
        let package = cached(cache, node)?;
        bytes += package.bytes.len();
        ensure!(bytes <= MAX_GRAPH_BYTES, "aggregate Component size limit");
        ensure!(
            package.package.dependencies.len() == node.dependencies.len(),
            "locked dependency inventory mismatch"
        );
        for (import, requirement) in &package.package.dependencies {
            let child = node
                .dependencies
                .get(import)
                .context("missing locked import binding")?;
            let target = lock.nodes.get(child).context("missing locked dependency")?;
            ensure!(
                target.identity == requirement.package
                    && requirement.version.matches(&target.version),
                "incompatible locked requirement for {import}"
            );
            ensure!(
                target.exports.contains(import),
                "dependency {child} does not export {import}"
            );
            let key = (requirement.package.clone(), import.clone());
            if let Some(previous) = slots.insert(key, child.clone()) {
                ensure!(
                    previous == *child,
                    "two package versions cannot share the same exact interface identity: {import}"
                );
            }
        }
        loaded.insert(name.clone(), package);
    }
    ensure!(
        loaded[&lock.root].manifest == lock.manifest_sha256,
        "root manifest hash mismatch"
    );
    let mut versions: BTreeMap<&str, BTreeSet<&Version>> = BTreeMap::new();
    for node in lock.nodes.values() {
        versions
            .entry(&node.identity)
            .or_default()
            .insert(&node.version);
    }
    for ((package, import), _) in &slots {
        if versions[package.as_str()].len() > 1 {
            let valid = import.rsplit_once('@').is_some_and(|(name, version)| {
                Version::parse(version).is_ok()
                    && name.rsplit_once('/').is_some_and(|(owner, interface)| {
                        identity(owner).is_ok()
                            && identity(&format!("interface:{interface}")).is_ok()
                    })
            });
            ensure!(
                valid,
                "coexisting package versions require explicit versioned interface identities: {import}"
            );
        }
    }
    Ok(loaded)
}

#[derive(Clone)]
struct Pending {
    parent: String,
    import: String,
    requirement: Requirement,
}
#[derive(Clone, Default)]
struct State {
    loaded: BTreeMap<String, Arc<Loaded>>,
    edges: BTreeMap<String, BTreeMap<String, String>>,
    slots: BTreeMap<(String, String), String>,
    pending: VecDeque<Pending>,
}
impl State {
    fn add(&mut self, package: Arc<Loaded>) {
        let name = id(&package.package);
        if self.loaded.contains_key(&name) {
            return;
        }
        for (import, requirement) in &package.package.dependencies {
            self.pending.push_back(Pending {
                parent: name.clone(),
                import: import.clone(),
                requirement: requirement.clone(),
            });
        }
        self.edges.insert(name.clone(), BTreeMap::new());
        self.loaded.insert(name, package);
    }
    fn reaches(&self, from: &str, target: &str, seen: &mut BTreeSet<String>) -> bool {
        from == target
            || (seen.insert(from.into())
                && self.edges.get(from).is_some_and(|edges| {
                    edges
                        .values()
                        .any(|child| self.reaches(child, target, seen))
                }))
    }
    fn bind(&mut self, pending: &Pending, package: Arc<Loaded>) -> Result<()> {
        let name = id(&package.package);
        ensure!(
            package.inventory.exports.contains(&pending.import),
            "dependency {name} does not export {}",
            pending.import
        );
        if let Some(existing) = self.loaded.get(&name) {
            ensure!(
                existing.manifest == package.manifest,
                "same identity has conflicting manifests: {name}"
            );
        }
        self.add(package);
        ensure!(
            !self.reaches(&name, &pending.parent, &mut BTreeSet::new()),
            "Component dependency cycle at {name}"
        );
        self.edges
            .get_mut(&pending.parent)
            .unwrap()
            .insert(pending.import.clone(), name.clone());
        self.slots.insert(
            (pending.requirement.package.clone(), pending.import.clone()),
            name,
        );
        ensure!(
            self.loaded.len() <= MAX_NODES,
            "Component dependency node limit"
        );
        ensure!(
            self.edges.values().map(|e| e.len()).sum::<usize>() + self.pending.len() <= MAX_EDGES,
            "dependency edge limit"
        );
        Ok(())
    }
}
struct Resolver<'a> {
    registry: &'a Path,
    frozen: BTreeMap<(String, String), Arc<Loaded>>,
    available: BTreeMap<String, Vec<Arc<Loaded>>>,
    attempts: usize,
    loaded_bytes: usize,
}
impl Resolver<'_> {
    fn candidates(&mut self, pending: &Pending) -> Result<Vec<Arc<Loaded>>> {
        let key = (pending.requirement.package.clone(), pending.import.clone());
        if let Some(package) = self.frozen.get(&key) {
            return Ok(vec![package.clone()]);
        }
        if !self.available.contains_key(&pending.requirement.package) {
            let (namespace, name) = identity(&pending.requirement.package)?;
            directory_without_symlink(&self.registry.join(namespace))?;
            let directory = self.registry.join(namespace).join(name);
            directory_without_symlink(&directory)?;
            let mut packages = Vec::new();
            let entries = fs::read_dir(&directory).with_context(|| {
                format!("missing Component package {}", pending.requirement.package)
            })?;
            for entry in entries {
                let entry = entry?;
                ensure!(
                    !entry.file_type()?.is_symlink(),
                    "symlinked registry version"
                );
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let version = Version::parse(&entry.file_name().to_string_lossy())
                    .context("invalid registry version directory")?;
                let package = load(&entry.path().join("component.json"))?;
                ensure!(
                    package.package.identity == pending.requirement.package
                        && package.package.version == version,
                    "registry identity mismatch"
                );
                self.loaded_bytes += package.bytes.len();
                ensure!(
                    self.loaded_bytes <= MAX_GRAPH_BYTES,
                    "registry candidate size limit"
                );
                packages.push(package);
                ensure!(packages.len() <= 128, "registry version limit");
            }
            packages.sort_by(|a, b| b.package.version.cmp(&a.package.version));
            self.available
                .insert(pending.requirement.package.clone(), packages);
        }
        Ok(self.available[&pending.requirement.package].clone())
    }
    fn solve(&mut self, mut state: State) -> Result<State> {
        self.attempts += 1;
        ensure!(
            self.attempts <= MAX_SEARCH,
            "Component resolution search limit"
        );
        let Some(pending) = state.pending.pop_front() else {
            return Ok(state);
        };
        let key = (pending.requirement.package.clone(), pending.import.clone());
        if let Some(name) = state.slots.get(&key) {
            let package = state.loaded[name].clone();
            ensure!(
                pending
                    .requirement
                    .version
                    .matches(&package.package.version),
                "incompatible requirements for {} and interface {}",
                pending.requirement.package,
                pending.import
            );
            state.bind(&pending, package)?;
            return self.solve(state);
        }
        let candidates = self.candidates(&pending)?;
        let mut failure = None;
        for package in candidates {
            if !pending
                .requirement
                .version
                .matches(&package.package.version)
            {
                continue;
            }
            let mut attempt = state.clone();
            match attempt
                .bind(&pending, package)
                .and_then(|_| self.solve(attempt))
            {
                Ok(solved) => return Ok(solved),
                Err(error) => failure = Some(error),
            }
        }
        Err(failure.unwrap_or_else(|| {
            anyhow::anyhow!(
                "incompatible or missing versions for {} {} importing {}",
                pending.requirement.package,
                pending.requirement.version,
                pending.import
            )
        }))
    }
}

fn wac() -> Result<(PathBuf, String)> {
    let path = std::env::var_os("WISP_WAC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.tools/component-library-chain/wac")
        });
    let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "f7315f2ebf764efc0c7e9688c854f972a8ac41ed1b68fd01f4192226307a8c53",
        ("linux", "x86_64") => "250c11762916ba733c7d22b62487580f21270ec9dde4f13460ea69d300e25406",
        _ => bail!("Component composition tools support Linux x86_64 and macOS ARM"),
    };
    let hash = digest(
        &read(&path, 64 * 1024 * 1024)
            .context("provision WAC with component-library-chain/run.py --setup or set WISP_WAC")?,
    );
    ensure!(hash == expected, "WAC tool checksum mismatch");
    let output = Command::new(&path).arg("--version").output()?;
    let version = String::from_utf8(output.stdout)?;
    ensure!(
        output.status.success() && version.trim() == "wac-cli 0.10.1",
        "expected WAC 0.10.1"
    );
    Ok((path, hash))
}
fn compose_into(lock: &Lock, cache: &Path, directory: &Path) -> Result<Value> {
    let ordered = order(lock)?;
    let (tool, tool_hash) = wac()?;
    let mut source = String::from("package wisp:resolved-components;\n");
    let aliases = ordered
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), format!("node{i}")))
        .collect::<BTreeMap<_, _>>();
    let mut args = vec![
        "compose".to_owned(),
        directory
            .join("composition.wac")
            .to_string_lossy()
            .into_owned(),
    ];
    let mut host_imports = BTreeSet::new();
    let mut type_exports: BTreeMap<String, String> = BTreeMap::new();
    for name in &ordered {
        let node = &lock.nodes[name];
        let alias = &aliases[name];
        source.push_str(&format!("let {alias} = new wisp:{alias} {{\n"));
        for (import, child) in &node.dependencies {
            source.push_str(&format!("  {import:?}: {}[{import:?}],\n", aliases[child]));
        }
        if !node.host_imports.is_empty() {
            source.push_str("  ...\n");
        }
        source.push_str("};\n");
        host_imports.extend(node.host_imports.iter().cloned());
        for interface in &node.type_exports {
            if !lock.nodes[&lock.root].exports.contains(interface)
                && lock
                    .nodes
                    .values()
                    .any(|parent| parent.dependencies.get(interface) == Some(name))
            {
                if let Some(previous) = type_exports.insert(interface.clone(), name.clone()) {
                    ensure!(
                        previous == *name,
                        "ambiguous shared type provider for {interface}"
                    );
                }
            }
        }
        args.push("--dep".into());
        args.push(format!(
            "wisp:{alias}={}",
            cache_file(cache, &node.sha256, false)?
                .canonicalize()?
                .display()
        ));
    }
    // An exported interface may alias a type from a dependency's type-only
    // interface. Preserve that provider before exposing the dependent export.
    for (interface, provider) in &type_exports {
        source.push_str(&format!("export {}[{interface:?}];\n", aliases[provider]));
    }
    source.push_str(&format!("export {}...;\n", aliases[&lock.root]));
    fs::write(directory.join("composition.wac"), &source)?;
    let output = directory.join("component.wasm");
    args.extend(["--output".into(), output.to_string_lossy().into_owned()]);
    let process = Command::new(&tool)
        .args(&args)
        .current_dir(directory)
        .output()?;
    ensure!(
        process.status.success(),
        "Component interface compatibility failed before execution: {}",
        String::from_utf8_lossy(&process.stderr)
    );
    let bytes = read(&output, MAX_COMPONENT)?;
    let found = inventory(&bytes)?;
    ensure!(
        found.imports.iter().cloned().collect::<BTreeSet<_>>() == host_imports,
        "composition contains unresolved Component dependencies"
    );
    let mut expected_exports = lock.nodes[&lock.root]
        .exports
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    expected_exports.extend(type_exports.keys().cloned());
    ensure!(
        found.exports.iter().cloned().collect::<BTreeSet<_>>() == expected_exports,
        "composition changed root exports"
    );
    Ok(
        json!({"schema":1,"kind":"component-composition","lockSha256":digest(&canonical(lock)?),
        "componentSha256":digest(&bytes),"contractSha256":found.contract,"hostImports":found.imports,"preservedTypeExports":type_exports,
        "inputs":lock.nodes,"compositionSourceSha256":digest(source.as_bytes()),
        "tool":{"name":"wac","version":"0.10.1","sha256":tool_hash}}),
    )
}

pub fn resolve_lock(
    manifest: &Path,
    registry: &Path,
    cache: &Path,
    output: &Path,
    update: Option<&str>,
) -> Result<Lock> {
    let root = load(manifest)?;
    if let Some(package) = update {
        identity(package)?;
    }
    let existing = if output.try_exists()? {
        Some(json_file::<Lock>(output)?)
    } else {
        None
    };
    ensure!(
        update.is_none() || existing.is_some(),
        "targeted update requires an existing lock"
    );
    let mut frozen = BTreeMap::new();
    if let Some(lock) = &existing {
        order(lock)?;
        ensure!(
            lock.manifest_sha256 == root.manifest && lock.root == id(&root.package),
            "root manifest changed; choose a new lock"
        );
        if let Some(package) = update {
            ensure!(
                lock.nodes.values().any(|n| n.identity == package)
                    && package != root.package.identity,
                "update identity is not a locked dependency: {package}"
            );
        }
        // Compare present registry records even on cache hits. A published identity
        // cannot change its bytes or dependency metadata under an existing lock.
        for (name, node) in &lock.nodes {
            if name == &lock.root {
                cache_store(cache, &root)?;
                continue;
            }
            let path = registry_path(registry, &node.identity, &node.version)?;
            if path.try_exists()? {
                let found = load(&path)?;
                ensure!(
                    found.manifest == node.manifest_sha256
                        && found.package.component.sha256 == node.sha256,
                    "locked identity changed: {name}"
                );
                cache_store(cache, &found)?;
            }
        }
        let loaded = validate_cache(lock, cache)?;
        if update.is_none() {
            let proof = tempfile::tempdir_in(cache)?;
            compose_into(lock, cache, proof.path())?;
            return Ok(lock.clone());
        }
        for (name, node) in &lock.nodes {
            for (import, child) in &node.dependencies {
                let package = &loaded[child];
                if Some(package.package.identity.as_str()) != update {
                    frozen.insert(
                        (package.package.identity.clone(), import.clone()),
                        package.clone(),
                    );
                }
            }
            let _ = name;
        }
    }
    let mut state = State::default();
    state.add(root.clone());
    let mut resolver = Resolver {
        registry,
        frozen,
        available: BTreeMap::new(),
        attempts: 0,
        loaded_bytes: root.bytes.len(),
    };
    let state = resolver.solve(state)?;
    let mut nodes = BTreeMap::new();
    for (name, loaded) in &state.loaded {
        cache_store(cache, loaded)?;
        nodes.insert(
            name.clone(),
            LockedNode {
                identity: loaded.package.identity.clone(),
                version: loaded.package.version.clone(),
                sha256: loaded.package.component.sha256.clone(),
                manifest_sha256: loaded.manifest.clone(),
                contract_sha256: loaded.inventory.contract.clone(),
                imports: loaded.inventory.imports.clone(),
                exports: loaded.inventory.exports.clone(),
                type_exports: loaded.inventory.type_exports.clone(),
                host_imports: loaded.package.host_imports.clone(),
                dependencies: state.edges[name].clone(),
            },
        );
    }
    let lock = Lock {
        schema: 1,
        kind: "component-dependencies".into(),
        root: id(&root.package),
        manifest_sha256: root.manifest.clone(),
        nodes,
    };
    validate_cache(&lock, cache)?;
    let proof = tempfile::tempdir_in(cache)?;
    compose_into(&lock, cache, proof.path())?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(&lock)?)?;
    file.write_all(b"\n")?;
    if existing.is_some() {
        file.persist(output).map_err(|e| e.error)?;
    } else {
        file.persist_noclobber(output).map_err(|e| e.error)?;
    }
    Ok(lock)
}

pub fn compose_lock(lock_path: &Path, cache: &Path, output: &Path) -> Result<Value> {
    ensure!(
        !output.try_exists()?,
        "output already exists; choose a fresh directory"
    );
    let lock: Lock = json_file(lock_path)?;
    validate_cache(&lock, cache)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    let report = compose_into(&lock, cache, stage.path())?;
    fs::write(
        stage.path().join("composition.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        stage.path().join("component.lock.json"),
        serde_json::to_vec_pretty(&lock)?,
    )?;
    fs::create_dir(output)?;
    if let Err(error) = fs::rename(stage.path(), output) {
        let _ = fs::remove_dir(output);
        return Err(error.into());
    }
    Ok(report)
}

pub fn run_cli(args: &[String]) -> Result<Value> {
    const USAGE: &str = "usage: wisp deps lock MANIFEST --registry DIRECTORY --cache DIRECTORY --out LOCK [--update ID] | wisp deps compose LOCK --cache DIRECTORY --out DIRECTORY";
    ensure!(args.len() >= 2, USAGE);
    let mut options = BTreeMap::new();
    for pair in args[2..].chunks(2) {
        ensure!(
            pair.len() == 2
                && pair[0].starts_with("--")
                && options.insert(pair[0].as_str(), pair[1].as_str()).is_none(),
            USAGE
        );
    }
    let required = |name| {
        options
            .get(name)
            .copied()
            .with_context(|| format!("missing {name}; {USAGE}"))
    };
    match args[0].as_str() {
        "lock" => {
            ensure!(
                options
                    .keys()
                    .all(|key| matches!(*key, "--registry" | "--cache" | "--out" | "--update")),
                USAGE
            );
            Ok(serde_json::to_value(resolve_lock(
                Path::new(&args[1]),
                Path::new(required("--registry")?),
                Path::new(required("--cache")?),
                Path::new(required("--out")?),
                options.get("--update").copied(),
            )?)?)
        }
        "compose" => {
            ensure!(
                options
                    .keys()
                    .all(|key| matches!(*key, "--cache" | "--out")),
                USAGE
            );
            compose_lock(
                Path::new(&args[1]),
                Path::new(required("--cache")?),
                Path::new(required("--out")?),
            )
        }
        _ => bail!(USAGE),
    }
}
