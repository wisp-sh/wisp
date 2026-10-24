//! The pinned WASI host contract for the package-manager library pipeline.
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use wit_parser::{FunctionKind, Resolve, Type, TypeDefKind, TypeId, TypeOwner, WorldId, WorldItem};

fn resource_identity(resolve: &Resolve, id: TypeId) -> Result<String> {
    if let TypeDefKind::Type(Type::Id(next)) = resolve.types[id].kind {
        return resource_identity(resolve, next);
    }
    let def = &resolve.types[id];
    let TypeOwner::Interface(interface) = def.owner else {
        bail!("resource has no interface owner")
    };
    let interface = &resolve.interfaces[interface];
    Ok(format!(
        "{}/{}/{}",
        resolve.packages[interface.package.context("resource package")?].name,
        interface.name.as_deref().context("resource interface")?,
        def.name.as_deref().context("resource name")?
    ))
}

fn same_type(a: &Resolve, left: Type, b: &Resolve, right: Type, depth: usize) -> Result<bool> {
    ensure!(depth < 64, "host contract type depth limit");
    let child = |x, y| same_type(a, x, b, y, depth + 1);
    let optional = |x: Option<Type>, y: Option<Type>| match (x, y) {
        (Some(x), Some(y)) => child(x, y),
        (None, None) => Ok(true),
        _ => Ok(false),
    };
    if let Type::Id(id) = left {
        if let TypeDefKind::Type(ty) = a.types[id].kind {
            return child(ty, right);
        }
    }
    if let Type::Id(id) = right {
        if let TypeDefKind::Type(ty) = b.types[id].kind {
            return child(left, ty);
        }
    }
    let (Type::Id(x), Type::Id(y)) = (left, right) else {
        return Ok(left == right);
    };
    Ok(match (&a.types[x].kind, &b.types[y].kind) {
        (TypeDefKind::Resource, TypeDefKind::Resource) => {
            resource_identity(a, x)? == resource_identity(b, y)?
        }
        (
            TypeDefKind::Handle(wit_parser::Handle::Own(x)),
            TypeDefKind::Handle(wit_parser::Handle::Own(y)),
        )
        | (
            TypeDefKind::Handle(wit_parser::Handle::Borrow(x)),
            TypeDefKind::Handle(wit_parser::Handle::Borrow(y)),
        ) => resource_identity(a, *x)? == resource_identity(b, *y)?,
        (TypeDefKind::List(x), TypeDefKind::List(y))
        | (TypeDefKind::Option(x), TypeDefKind::Option(y)) => child(*x, *y)?,
        (TypeDefKind::Result(x), TypeDefKind::Result(y)) => {
            optional(x.ok, y.ok)? && optional(x.err, y.err)?
        }
        (TypeDefKind::Tuple(x), TypeDefKind::Tuple(y)) => {
            x.types.len() == y.types.len()
                && x.types
                    .iter()
                    .zip(&y.types)
                    .map(|(x, y)| child(*x, *y))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .all(|v| v)
        }
        (TypeDefKind::Record(x), TypeDefKind::Record(y)) => {
            x.fields.len() == y.fields.len()
                && x.fields
                    .iter()
                    .zip(&y.fields)
                    .map(|(x, y)| Ok(x.name == y.name && child(x.ty, y.ty)?))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .all(|v| v)
        }
        (TypeDefKind::Variant(x), TypeDefKind::Variant(y)) => {
            x.cases.len() == y.cases.len()
                && x.cases
                    .iter()
                    .zip(&y.cases)
                    .map(|(x, y)| Ok(x.name == y.name && optional(x.ty, y.ty)?))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .all(|v| v)
        }
        (TypeDefKind::Enum(x), TypeDefKind::Enum(y)) => x
            .cases
            .iter()
            .map(|c| &c.name)
            .eq(y.cases.iter().map(|c| &c.name)),
        (TypeDefKind::Flags(x), TypeDefKind::Flags(y)) => x
            .flags
            .iter()
            .map(|c| &c.name)
            .eq(y.flags.iter().map(|c| &c.name)),
        (TypeDefKind::Future(x), TypeDefKind::Future(y))
        | (TypeDefKind::Stream(x), TypeDefKind::Stream(y)) => optional(*x, *y)?,
        _ => false,
    })
}

/// Guest compilation can remove unused imported functions and aliases. Compare
/// each retained declaration structurally while preserving resource identities.
pub fn imports_compatible(
    actual: &Resolve,
    world: WorldId,
    expected: &Resolve,
    expected_world: WorldId,
) -> Result<()> {
    for item in actual.worlds[world].imports.values() {
        let WorldItem::Interface { id, .. } = item else {
            bail!("unsupported-library-host-import")
        };
        let interface = &actual.interfaces[*id];
        let package = &actual.packages[interface.package.context("import package")?].name;
        let reference = expected.worlds[expected_world]
            .imports
            .values()
            .find_map(|item| {
                let WorldItem::Interface { id, .. } = item else {
                    return None;
                };
                let other = &expected.interfaces[*id];
                (other.name == interface.name
                    && other
                        .package
                        .is_some_and(|id| &expected.packages[id].name == package))
                .then_some(other)
            })
            .context("unsupported-library-host-contract: undeclared imported interface")?;
        for (name, id) in &interface.types {
            let other = reference
                .types
                .get(name)
                .context("unsupported-library-host-contract: undeclared imported type")?;
            ensure!(
                same_type(actual, Type::Id(*id), expected, Type::Id(*other), 0)?,
                "unsupported-library-host-contract: type {package}/{name} differs"
            );
        }
        for (name, function) in &interface.functions {
            let other = reference
                .functions
                .get(name)
                .context("unsupported-library-host-contract: undeclared imported function")?;
            let kind = |kind: &FunctionKind| match kind {
                FunctionKind::Freestanding => 0,
                FunctionKind::Method(_) => 1,
                FunctionKind::Static(_) => 2,
                FunctionKind::Constructor(_) => 3,
                FunctionKind::AsyncFreestanding => 4,
                FunctionKind::AsyncMethod(_) => 5,
                FunctionKind::AsyncStatic(_) => 6,
            };
            ensure!(
                kind(&function.kind) == kind(&other.kind)
                    && function.params.len() == other.params.len(),
                "unsupported-library-host-contract: function {name} differs"
            );
            for (left, right) in function.params.iter().zip(&other.params) {
                ensure!(
                    left.name == right.name && same_type(actual, left.ty, expected, right.ty, 0)?,
                    "unsupported-library-host-contract: parameter {name}/{} differs",
                    left.name
                );
            }
            let equal = match (function.result, other.result) {
                (None, None) => true,
                (Some(x), Some(y)) => same_type(actual, x, expected, y, 0)?,
                _ => false,
            };
            ensure!(
                equal,
                "unsupported-library-host-contract: result {name} differs"
            );
        }
    }
    Ok(())
}

const WIT: [(&str, &str); 7] = [
    (
        "random.wit",
        include_str!("../assets/wasi-library/random.wit"),
    ),
    (
        "filesystem.wit",
        include_str!("../assets/wasi-library/filesystem.wit"),
    ),
    ("cli.wit", include_str!("../assets/wasi-library/cli.wit")),
    ("io.wit", include_str!("../assets/wasi-library/io.wit")),
    (
        "clocks.wit",
        include_str!("../assets/wasi-library/clocks.wit"),
    ),
    (
        "sockets.wit",
        include_str!("../assets/wasi-library/sockets.wit"),
    ),
    ("http.wit", include_str!("../assets/wasi-library/http.wit")),
];

// Keep the older contract as upstream bytes rather than renaming the current
// spec: resource identities and retained signatures must match that version.
const WIT_0_2_9: [(&str, &str); 7] = [
    (
        "random.wit",
        include_str!("../assets/wasi-library/0.2.9/random.wit"),
    ),
    (
        "filesystem.wit",
        include_str!("../assets/wasi-library/0.2.9/filesystem.wit"),
    ),
    (
        "cli.wit",
        include_str!("../assets/wasi-library/0.2.9/cli.wit"),
    ),
    (
        "io.wit",
        include_str!("../assets/wasi-library/0.2.9/io.wit"),
    ),
    (
        "clocks.wit",
        include_str!("../assets/wasi-library/0.2.9/clocks.wit"),
    ),
    (
        "sockets.wit",
        include_str!("../assets/wasi-library/0.2.9/sockets.wit"),
    ),
    (
        "http.wit",
        include_str!("../assets/wasi-library/0.2.9/http.wit"),
    ),
];

/// Resolve the bundled packages without writing them to a temporary directory.
fn resolve_contract(
    contract: &[(&str, &str)],
    source: &str,
    world_name: &str,
) -> Result<(Resolve, WorldId)> {
    let mut resolve = Resolve::default();
    // Both bundled versions have this dependency order: clocks needs io;
    // filesystem and sockets need io/clocks; cli additionally needs random,
    // filesystem, and sockets; http needs io/clocks/random/cli.
    for name in [
        "io.wit",
        "random.wit",
        "clocks.wit",
        "filesystem.wit",
        "sockets.wit",
        "cli.wit",
        "http.wit",
    ] {
        let (_, text) = contract
            .iter()
            .find(|(file, _)| *file == name)
            .with_context(|| format!("missing bundled WASI package {name}"))?;
        resolve.push_source(name, text)?;
    }
    let package = resolve.push_source("world.wit", source)?;
    let world = resolve.select_world(&[package], Some(world_name))?;
    Ok((resolve, world))
}

/// Check entire imported interfaces against the bundled standard, including resources.
pub fn enabled(resolve: &Resolve, world: WorldId) -> Result<bool> {
    if pure_browser_host(resolve, world)? {
        return Ok(true);
    }
    if resolve.worlds[world].imports.is_empty() {
        return Ok(false);
    }
    let reactor = reactor_host(resolve, world)?;
    let imported = resolve.worlds[world]
        .imports
        .values()
        .filter_map(|item| match item {
            WorldItem::Interface { id, .. } => Some(*id),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    fn exported_type(
        resolve: &Resolve,
        ty: Type,
        imports: &std::collections::BTreeSet<wit_parser::InterfaceId>,
        depth: usize,
    ) -> Result<()> {
        ensure!(depth < 64, "exported host type depth limit");
        let Type::Id(id) = ty else { return Ok(()) };
        let def = &resolve.types[id];
        ensure!(
            !matches!(def.owner, TypeOwner::Interface(owner) if imports.contains(&owner)),
            "unsupported-library-exported-host-type: exported APIs cannot expose types owned by WASI host interfaces"
        );
        let mut children = Vec::new();
        match &def.kind {
            TypeDefKind::Type(t) | TypeDefKind::List(t) | TypeDefKind::Option(t) => {
                children.push(*t)
            }
            TypeDefKind::Tuple(t) => children.extend(&t.types),
            TypeDefKind::Record(r) => children.extend(r.fields.iter().map(|f| f.ty)),
            TypeDefKind::Variant(v) => children.extend(v.cases.iter().filter_map(|c| c.ty)),
            TypeDefKind::Result(r) => children.extend(r.ok.iter().chain(r.err.iter()).copied()),
            TypeDefKind::Future(t) | TypeDefKind::Stream(t) => children.extend(t.iter().copied()),
            TypeDefKind::Handle(wit_parser::Handle::Own(id) | wit_parser::Handle::Borrow(id)) => {
                children.push(Type::Id(*id))
            }
            _ => (),
        }
        for child in children {
            exported_type(resolve, child, imports, depth + 1)?;
        }
        Ok(())
    }
    for item in resolve.worlds[world].exports.values() {
        if let WorldItem::Interface { id, .. } = item {
            for id in resolve.interfaces[*id].types.values() {
                exported_type(resolve, Type::Id(*id), &imported, 0)?;
            }
            for function in resolve.interfaces[*id].functions.values() {
                for p in &function.params {
                    exported_type(resolve, p.ty, &imported, 0)?;
                }
                if let Some(ty) = function.result {
                    exported_type(resolve, ty, &imported, 0)?;
                }
            }
        }
    }
    if reactor {
        return Ok(true);
    }
    let mut source = String::from("package wisp:host-contract;\nworld host {\n");
    for item in resolve.worlds[world].imports.values() {
        let WorldItem::Interface { id, .. } = item else {
            bail!("unsupported-library-host-import: expected a standard WASI interface")
        };
        let interface = &resolve.interfaces[*id];
        let package = &resolve.packages[interface.package.context("import package")?].name;
        let name = interface.name.as_deref().context("import interface name")?;
        ensure!(
            package.namespace == "wasi"
                && package
                    .version
                    .as_ref()
                    .is_some_and(|v| v.to_string() == "0.2.12")
                && match package.name.as_str() {
                    "io" => matches!(name, "error" | "poll" | "streams"),
                    "clocks" => matches!(name, "monotonic-clock" | "wall-clock"),
                    "http" => matches!(name, "types" | "outgoing-handler"),
                    "sockets" => matches!(
                        name,
                        "network"
                            | "instance-network"
                            | "ip-name-lookup"
                            | "tcp"
                            | "tcp-create-socket"
                    ),
                    _ => false,
                },
            "unsupported-library-host-import: {package}/{name}"
        );
        source.push_str(&format!("import wasi:{}/{name}@0.2.12;\n", package.name));
    }
    source.push_str("}\n");
    let (expected, expected_world) = resolve_contract(&WIT, &source, "host")?;
    imports_compatible(resolve, world, &expected, expected_world)?;
    Ok(true)
}

/// The synchronous reactor profile permits standard CLI, I/O, clocks and
/// randomness without network or filesystem preopens. Older CPython runtimes
/// retain socket imports; the reactor host denies those capabilities.
pub fn reactor_host(resolve: &Resolve, world: WorldId) -> Result<bool> {
    // Preserve the existing browser-capable jq profile and its release contract.
    if pure_browser_host(resolve, world)? {
        return Ok(false);
    }
    let mut imports = Vec::new();
    let mut version = None;
    for item in resolve.worlds[world].imports.values() {
        let WorldItem::Interface { id, .. } = item else {
            return Ok(false);
        };
        let interface = &resolve.interfaces[*id];
        let Some(package) = interface.package.map(|id| &resolve.packages[id].name) else {
            return Ok(false);
        };
        let Some(name) = interface.name.as_deref() else {
            return Ok(false);
        };
        let current = package.version.as_ref().map(ToString::to_string);
        let current = match current.as_deref() {
            Some("0.2.9") => "0.2.9",
            Some("0.2.12") => "0.2.12",
            _ => return Ok(false),
        };
        if package.namespace != "wasi" || version.is_some_and(|previous| previous != current) {
            return Ok(false);
        }
        version = Some(current);
        let allowed = match package.name.as_str() {
            "cli" => matches!(
                name,
                "environment"
                    | "exit"
                    | "stdin"
                    | "stdout"
                    | "stderr"
                    | "terminal-input"
                    | "terminal-output"
                    | "terminal-stdin"
                    | "terminal-stdout"
                    | "terminal-stderr"
            ),
            "io" => matches!(name, "error" | "poll" | "streams"),
            "clocks" => matches!(name, "monotonic-clock" | "wall-clock"),
            "filesystem" => matches!(name, "types" | "preopens"),
            "random" => matches!(name, "random" | "insecure-seed"),
            "sockets" if current == "0.2.9" => matches!(
                name,
                "network"
                    | "instance-network"
                    | "tcp"
                    | "tcp-create-socket"
                    | "udp"
                    | "udp-create-socket"
                    | "ip-name-lookup"
            ),
            _ => false,
        };
        if !allowed {
            return Ok(false);
        }
        imports.push(format!("import wasi:{}/{name}@{current};\n", package.name));
    }
    if imports.is_empty() {
        return Ok(false);
    }
    let contract = if version == Some("0.2.9") {
        WIT_0_2_9.as_slice()
    } else {
        WIT.as_slice()
    };
    let (expected, expected_world) = resolve_contract(
        contract,
        &format!(
            "package wisp:reactor-host;\nworld host {{\n{}}}\n",
            imports.join("")
        ),
        "host",
    )?;
    imports_compatible(resolve, world, &expected, expected_world)?;
    Ok(true)
}

pub const REACTOR_LINKER: &str = "wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;";
pub const REACTOR_NODE: &str = include_str!("../assets/wasi-library/reactor-host.js");

/// Narrow browser-only admission for the pure WASI imports of the jq fixture.
/// This does not relax the network-capable native/Node host policy.
pub fn pure_browser_host(resolve: &Resolve, world: WorldId) -> Result<bool> {
    const PURE: [(&str, &str); 16] = [
        ("cli", "environment"),
        ("cli", "exit"),
        ("io", "error"),
        ("io", "streams"),
        ("cli", "stdin"),
        ("cli", "stdout"),
        ("cli", "stderr"),
        ("cli", "terminal-input"),
        ("cli", "terminal-output"),
        ("cli", "terminal-stdin"),
        ("cli", "terminal-stdout"),
        ("cli", "terminal-stderr"),
        ("clocks", "monotonic-clock"),
        ("clocks", "wall-clock"),
        ("filesystem", "types"),
        ("filesystem", "preopens"),
    ];
    let imports = &resolve.worlds[world].imports;
    if imports.len() != PURE.len() {
        return Ok(false);
    }
    let mut seen = std::collections::BTreeSet::new();
    for item in imports.values() {
        let WorldItem::Interface { id, .. } = item else {
            return Ok(false);
        };
        let interface = &resolve.interfaces[*id];
        let Some(package) = interface.package.map(|id| &resolve.packages[id].name) else {
            return Ok(false);
        };
        let Some(name) = interface.name.as_deref() else {
            return Ok(false);
        };
        if package.namespace != "wasi"
            || package
                .version
                .as_ref()
                .is_none_or(|v| v.to_string() != "0.2.12")
            || !PURE.contains(&(package.name.as_str(), name))
            || !seen.insert((package.name.as_str(), name))
        {
            return Ok(false);
        }
    }
    let mut source = String::from("package wisp:host-contract;\nworld host {\n");
    for (package, name) in PURE {
        source.push_str(&format!("import wasi:{package}/{name}@0.2.12;\n"));
    }
    source.push_str("}\n");
    let (expected, expected_world) = resolve_contract(&WIT, &source, "host")?;
    if imports_compatible(resolve, world, &expected, expected_world).is_err() {
        return Ok(false);
    }
    // The two generated JS entry points expose evaluate(filter, json). Do not
    // advertise browser support for another component with the same imports.
    let exports = &resolve.worlds[world].exports;
    if exports.len() != 1 || !matches!(exports.values().next(), Some(WorldItem::Function(_))) {
        return Ok(false);
    }
    let model = crate::wit_model::AbiModel::from_resolve(resolve, world)?;
    let function = &model.exports[0].functions[0];
    let signature = serde_json::to_value(function)?;
    Ok(signature["name"] == "evaluate"
        && signature["kind"] == "freestanding"
        && signature["params"] == serde_json::json!([["filter", "string"], ["json", "string"]])
        && signature["result"]
            == serde_json::json!({"result":{"ok":{"list":"string"},"err":"string"}}))
}

/// Additional files for a separately transpiled `jco --instantiation sync --name guest`
/// browser directory. Its wasm files and guest.js remain package-local.
pub fn browser_files(wasm_names: &[String]) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(!wasm_names.is_empty(), "missing browser core modules");
    let mut names = std::collections::BTreeSet::new();
    for name in wasm_names {
        ensure!(
            name.starts_with("guest.")
                && name.ends_with(".wasm")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                && names.insert(name),
            "invalid browser core module name"
        );
    }
    let json = serde_json::to_string(&names)?;
    let entry =
        include_str!("../assets/wasi-library/pure-browser.js.in").replace("@MODULES@", &json);
    Ok(BTreeMap::from([
        ("browser.js".into(), entry.into_bytes()),
        (
            "index.js".into(),
            include_str!("../assets/wasi-library/pure-node.js.in")
                .replace("@MODULES@", &json)
                .into_bytes(),
        ),
        (
            "pure-host.js".into(),
            include_bytes!("../assets/templates/pure-host.js").to_vec(),
        ),
    ]))
}

pub const STATE: &str = r#"
#[cfg(not(target_arch = "wasm32"))]
struct WasiState {
    ctx: wasmtime_wasi::WasiCtx,
    http: wasmtime_wasi_http::WasiHttpCtx,
    table: wasmtime::component::ResourceTable,
}
#[cfg(not(target_arch = "wasm32"))]
impl wasmtime_wasi::WasiView for WasiState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl wasmtime_wasi_http::WasiHttpView for WasiState {
    fn http(&mut self) -> wasmtime_wasi_http::WasiHttpCtxView<'_> {
        wasmtime_wasi_http::WasiHttpCtxView { ctx: &mut self.http, table: &mut self.table,
            hooks: wasmtime_wasi_http::default_hooks() }
    }
}
"#;
/// The jq-only host is capability-free: no inherited network, filesystem or
/// process streams. Shadow the default WASI clocks with traps at the linker,
/// rather than substituting a zero clock (which would still expose a clock).
pub const PURE_LINKER: &str = r#"
fn pure_wasi_linker(linker: &mut wasmtime::component::Linker<WasiState>) -> wasmtime::Result<()> {
    linker.allow_shadowing(true);
    wasmtime_wasi::p2::add_to_linker_sync(linker)?;
    for (interface, functions, reason) in [
        ("wasi:clocks/wall-clock@0.2.12", &["now", "resolution"][..], "WISP_DENIED: wall clock"),
        ("wasi:clocks/monotonic-clock@0.2.12", &["now", "resolution", "subscribe-instant", "subscribe-duration"][..], "WISP_DENIED: monotonic clock"),
    ] {
        let mut instance = linker.instance(interface)?;
        for &function in functions {
            instance.func_new(function, move |_, _, _, _| Err(wasmtime::Error::msg(reason)))?;
        }
    }
    Ok(())
}
"#;
pub const PURE_STORE: &str = r#"WasiState {
    ctx: wasmtime_wasi::WasiCtx::builder()
        .allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false).build(),
    http: wasmtime_wasi_http::WasiHttpCtx::new(),
    table: wasmtime::component::ResourceTable::new(),
}"#;
pub const PURE_LINKER_CALL: &str = "pure_wasi_linker(&mut linker)?;";
pub const STORE: &str = r#"WasiState {
    ctx: wasmtime_wasi::WasiCtx::builder().inherit_network().allow_ip_name_lookup(true).allow_tcp(true).build(),
    http: wasmtime_wasi_http::WasiHttpCtx::new(),
    table: wasmtime::component::ResourceTable::new(),
}"#;
pub const LINKER: &str = r#"
wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker)?;
"#;
pub const DEPENDENCIES: &str = r#"
wasmtime-wasi = { version = "=50.0.0", default-features = false, features = ["p2"] }
wasmtime-wasi-http = { version = "=50.0.0", default-features = false, features = ["p2", "default-send-request"] }
"#;

pub fn node_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = [
        (
            "wasi-node/nodejs/http.js",
            include_bytes!("../assets/wasi-node/http.js").as_slice(),
        ),
        (
            "wasi-node/nodejs/io.js",
            include_bytes!("../assets/wasi-node/io.js").as_slice(),
        ),
        (
            "wasi-node/nodejs/sockets.js",
            include_bytes!("../assets/wasi-node/sockets.js").as_slice(),
        ),
        (
            "wasi-node/nodejs/clocks.js",
            include_bytes!("../assets/wasi-library/clocks.js").as_slice(),
        ),
        (
            "wasi-node/io/calls.js",
            include_bytes!("../assets/wasi-node/io/calls.js").as_slice(),
        ),
        (
            "wasi-node/io/worker-io.js",
            include_bytes!("../assets/wasi-node/io/worker-io.js").as_slice(),
        ),
        (
            "wasi-node/io/worker-thread.bundle.js",
            include_bytes!("../assets/wasi-node/io/worker-thread.bundle.js").as_slice(),
        ),
        (
            "wasi-node/synckit/index.js",
            include_bytes!("../assets/wasi-node/synckit/index.js").as_slice(),
        ),
        (
            "wasi-node/LICENSE",
            include_bytes!("../assets/wasi-node/LICENSE").as_slice(),
        ),
        (
            "browser.js",
            b"throw new Error('unsupported-browser-wasi-network');\n".as_slice(),
        ),
    ]
    .into_iter()
    .map(|(path, bytes)| (path.into(), bytes.to_vec()))
    .collect::<BTreeMap<String, Vec<u8>>>();
    // preview2-shim 0.24.1's DNS worker wraps each resolved IP in a one-item
    // array. Numeric IP lookups already return the canonical variant directly.
    let sockets = include_str!("../assets/wasi-node/sockets.js").replace(
        "return this.#data[this.#curItem++];",
        "const address = this.#data[this.#curItem++];\n            return Array.isArray(address) && address.length === 1 ? address[0] : address;",
    );
    files.insert("wasi-node/nodejs/sockets.js".into(), sockets.into_bytes());
    files.insert("wasi-node/WISP-CHANGES.txt".into(), b"Wisp normalizes preview2-shim 0.24.1 DNS worker responses from a one-item array to the WASI ip-address variant. The bundled worker also retains the historical Wisp self-import flattening and disabled optional filesystem advice dependency. See wasi-node/LICENSE.\n".to_vec());
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reactor_admits_previous_pinned_cpython_contract() {
        let (resolve, world) = resolve_contract(
            &WIT_0_2_9,
            "package test:python; world root {\n\
             import wasi:io/poll@0.2.9;\n\
             import wasi:io/streams@0.2.9;\n\
             import wasi:cli/environment@0.2.9;\n\
             import wasi:cli/stdout@0.2.9;\n\
             import wasi:clocks/monotonic-clock@0.2.9;\n\
             import wasi:filesystem/types@0.2.9;\n\
             import wasi:filesystem/preopens@0.2.9;\n\
             import wasi:random/random@0.2.9;\n\
             import wasi:sockets/network@0.2.9;\n\
             import wasi:sockets/instance-network@0.2.9;\n\
             import wasi:sockets/tcp@0.2.9;\n\
             import wasi:sockets/tcp-create-socket@0.2.9;\n\
             import wasi:sockets/udp@0.2.9;\n\
             import wasi:sockets/udp-create-socket@0.2.9;\n\
             import wasi:sockets/ip-name-lookup@0.2.9;\n\
             export ping: func() -> u32; }",
            "root",
        )
        .unwrap();
        assert!(reactor_host(&resolve, world).unwrap());
        assert!(enabled(&resolve, world).unwrap());
        assert!(!pure_browser_host(&resolve, world).unwrap());
    }

    #[test]
    fn reactor_rejects_forged_previous_contract_and_unpinned_versions() {
        for version in ["0.2.9", "0.2.10"] {
            let mut resolve = Resolve::default();
            resolve.push_str("random.wit", &format!(
                "package wasi:random@{version}; interface insecure-seed {{ insecure-seed: func() -> u32; }}"
            )).unwrap();
            let package = resolve.push_str("root.wit", &format!(
                "package test:library; world root {{ import wasi:random/insecure-seed@{version}; }}"
            )).unwrap();
            let world = resolve.select_world(&[package], Some("root")).unwrap();
            if version == "0.2.9" {
                assert!(
                    reactor_host(&resolve, world)
                        .unwrap_err()
                        .to_string()
                        .contains("host-contract")
                );
            } else {
                assert!(!reactor_host(&resolve, world).unwrap());
                assert!(enabled(&resolve, world).is_err());
            }
        }
    }

    #[test]
    fn reactor_rejects_forged_previous_socket_contract() {
        let mut resolve = Resolve::default();
        resolve
            .push_str(
                "sockets.wit",
                "package wasi:sockets@0.2.9; interface network { fake: func(); }",
            )
            .unwrap();
        let package = resolve
            .push_str(
                "root.wit",
                "package test:library; world root { import wasi:sockets/network@0.2.9; }",
            )
            .unwrap();
        let world = resolve.select_world(&[package], Some("root")).unwrap();
        assert!(
            reactor_host(&resolve, world)
                .unwrap_err()
                .to_string()
                .contains("host-contract")
        );
    }

    #[test]
    fn reactor_admits_pinned_insecure_seed() {
        let mut resolve = Resolve::default();
        resolve.push_str("random.wit", WIT[0].1).unwrap();
        let package = resolve
            .push_str(
                "root.wit",
                "package test:library; world root { import wasi:random/insecure-seed@0.2.12; }",
            )
            .unwrap();
        let world = resolve.select_world(&[package], Some("root")).unwrap();
        assert!(reactor_host(&resolve, world).unwrap());
        assert!(enabled(&resolve, world).unwrap());
        assert!(!pure_browser_host(&resolve, world).unwrap());
    }
    #[test]
    fn reactor_rejects_forged_insecure_seed_signatures() {
        for function in [
            "insecure-seed: func() -> u64;",
            "insecure-seed: func() -> tuple<u64, u32>;",
            "insecure-seed: func(seed: u64) -> tuple<u64, u64>;",
            "fake: func() -> tuple<u64, u64>;",
        ] {
            let mut resolve = Resolve::default();
            resolve
                .push_str(
                    "random.wit",
                    &format!(
                        "package wasi:random@0.2.12; interface insecure-seed {{ {function} }}"
                    ),
                )
                .unwrap();
            let package = resolve
                .push_str(
                    "root.wit",
                    "package test:library; world root { import wasi:random/insecure-seed@0.2.12; }",
                )
                .unwrap();
            let world = resolve.select_world(&[package], Some("root")).unwrap();
            for result in [reactor_host(&resolve, world), enabled(&resolve, world)] {
                assert!(
                    result.unwrap_err().to_string().contains("host-contract"),
                    "accepted forged insecure-seed function: {function}"
                );
            }
        }
    }
    #[test]
    fn admits_unchanged_jq_pure_wasi_imports() {
        let bytes = include_bytes!("../../../fixtures/jq/component.wasm");
        let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
            wit_parser::decoding::decode(bytes).unwrap()
        else {
            panic!("expected component")
        };
        assert!(pure_browser_host(&resolve, world).unwrap());
        assert!(!reactor_host(&resolve, world).unwrap());
        let mut other = resolve.clone();
        let export = other.worlds[world].exports.values_mut().next().unwrap();
        if let WorldItem::Function(function) = export {
            function.name = "unrelated".into();
        }
        assert!(!pure_browser_host(&other, world).unwrap());
    }
    #[test]
    fn pure_browser_entry_uses_only_local_wasm_and_recovers_after_exception() {
        let files = browser_files(&["guest.core.wasm".into(), "guest.core2.wasm".into()]).unwrap();
        let entry = std::str::from_utf8(&files["browser.js"]).unwrap();
        assert!(entry.contains("./guest.js"));
        assert!(entry.contains("./pure-host.js"));
        assert!(entry.contains("redirect: 'error'"));
        assert!(entry.contains("guest = null"));
        assert!(!entry.contains("node:"));
        assert!(!entry.contains("https:"));
        assert_eq!(
            files["pure-host.js"],
            include_bytes!("../assets/templates/pure-host.js")
        );
    }
    #[test]
    fn rejects_same_named_forged_standard_import() {
        let mut resolve = Resolve::default();
        resolve
            .push_str(
                "forged.wit",
                "package wasi:sockets@0.2.12; interface network { fake: func(); }",
            )
            .unwrap();
        let package = resolve
            .push_str(
                "root.wit",
                "package test:library; world root { import wasi:sockets/network@0.2.12; }",
            )
            .unwrap();
        let world = resolve.select_world(&[package], Some("root")).unwrap();
        assert!(
            enabled(&resolve, world)
                .unwrap_err()
                .to_string()
                .contains("host-contract")
        );
    }
}
