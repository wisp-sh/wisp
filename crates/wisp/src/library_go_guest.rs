//! Ordinary Go modules whose calls become Component imports on wasip1/wasm.
use anyhow::{Context, Result, bail, ensure};
use heck::ToUpperCamelCase;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use wit_parser::{FunctionKind, Resolve, Type, TypeDefKind, WorldId, WorldItem};

pub const PLATFORM: &str = "wasip1_wasm";

#[path = "library_go_upstream.rs"]
pub mod upstream;

pub fn module_file(module: &str) -> Vec<u8> {
    format!(
        "module {module}\n\ngo 1.27.0\n\nrequire {} {}\n",
        upstream::RUNTIME_MODULE,
        upstream::RUNTIME_VERSION
    )
    .into_bytes()
}

/// Validate the synchronous subset before invoking upstream code generation.
/// Resources and async values need different compiler/runtime contracts.
fn synchronous(resolve: &Resolve, ty: Type, seen: &mut BTreeSet<wit_parser::TypeId>) -> Result<()> {
    let Type::Id(id) = ty else {
        ensure!(
            ty != Type::ErrorContext,
            "Go guest error-context is unsupported"
        );
        return Ok(());
    };
    if !seen.insert(id) {
        return Ok(());
    }
    let mut visit = |ty| synchronous(resolve, ty, seen);
    match &resolve.types[id].kind {
        TypeDefKind::Type(ty) | TypeDefKind::List(ty) | TypeDefKind::Option(ty) => visit(*ty)?,
        TypeDefKind::Record(record) => {
            for field in &record.fields {
                visit(field.ty)?;
            }
        }
        TypeDefKind::Variant(variant) => {
            for case in &variant.cases {
                if let Some(ty) = case.ty {
                    visit(ty)?;
                }
            }
        }
        TypeDefKind::Tuple(tuple) => {
            ensure!(tuple.types.len() <= 16, "Go guest tuple size limit");
            for ty in &tuple.types {
                visit(*ty)?;
            }
        }
        TypeDefKind::Result(result) => {
            for ty in result.ok.iter().chain(&result.err) {
                visit(*ty)?;
            }
        }
        TypeDefKind::Enum(_) => (),
        TypeDefKind::Flags(flags) => ensure!(flags.flags.len() <= 64, "Go guest flags size limit"),
        _ => bail!(
            "Go guest supports synchronous values; resources and async values are unsupported"
        ),
    }
    Ok(())
}

fn guest_world(resolve: &Resolve, world: WorldId) -> Result<Resolve> {
    super::validate_type_expansion(resolve, world)?;
    let reactor = crate::library_wasi::reactor_host(resolve, world)?;
    ensure!(
        resolve.worlds[world]
            .imports
            .values()
            .all(|item| matches!(item, WorldItem::Type { .. }))
            || reactor,
        "Go guest requires a closed Component or supported WASI reactor imports"
    );
    let mut seen = BTreeSet::new();
    let mut functions = 0;
    for item in resolve.worlds[world].exports.values() {
        let exports = match item {
            WorldItem::Function(f) => vec![f],
            WorldItem::Interface { id, .. } => {
                for ty in resolve.interfaces[*id].types.values() {
                    synchronous(resolve, Type::Id(*ty), &mut seen)?;
                }
                resolve.interfaces[*id].functions.values().collect()
            }
            WorldItem::Type { id, .. } => {
                synchronous(resolve, Type::Id(*id), &mut seen)?;
                continue;
            }
        };
        for f in exports {
            ensure!(
                matches!(f.kind, FunctionKind::Freestanding),
                "Go guest requires synchronous freestanding functions"
            );
            for ty in f.params.iter().map(|p| p.ty).chain(f.result) {
                synchronous(resolve, ty, &mut seen)?;
            }
            functions += 1;
        }
    }
    ensure!(
        functions > 0 && functions <= 1024,
        "Go guest function count limit"
    );
    let mut consumer = resolve.clone();
    let w = &mut consumer.worlds[world];
    // Keep world-level type imports used by exported function signatures. Host
    // WASI imports belong to the dependency's instance, not to its consumer.
    w.imports
        .retain(|_, item| matches!(item, WorldItem::Type { .. }));
    for (key, item) in std::mem::take(&mut w.exports) {
        ensure!(
            w.imports.insert(key, item).is_none(),
            "Go guest world name collision"
        );
    }
    Ok(consumer)
}

/// Capability detection must not depend on a native generator being installed.
pub fn supports(component: &[u8]) -> bool {
    super::decode_component(component)
        .and_then(|(resolve, world)| guest_world(&resolve, world).map(|_| ()))
        .is_ok()
}

fn printed_world(resolve: &Resolve, world: WorldId) -> Result<String> {
    let package = resolve.worlds[world]
        .package
        .context("Go guest world package")?;
    let others = resolve
        .packages
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != package)
        .collect::<Vec<_>>();
    let mut printer = wit_component::WitPrinter::default();
    printer.print(resolve, package, &others)?;
    Ok(printer.output.to_string())
}

/// Canonical lowering/lifting is generated by wit-bindgen. The root package is
/// a facade over upstream packages, so portable scalar APIs retain their names.
/// Generated nested packages are also public for consumers using upstream types.
pub fn import_files(
    resolve: &Resolve,
    world: WorldId,
    module: &str,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let consumer = guest_world(resolve, world)?;
    let mut files = upstream::generate(
        &printed_world(&consumer, world)?,
        &consumer.worlds[world].name,
        module,
    )?;
    files.retain(|path, _| path != "wit_exports/wit_exports.go" && path.ends_with(".go"));
    let mut imports = String::new();
    let mut declarations = String::new();
    let mut names = BTreeSet::new();
    let mut packages = BTreeMap::new();
    // Find the exact upstream package by its canonical import directive rather
    // than duplicating upstream's package/version name mangling.
    for (key, item) in &resolve.worlds[world].exports {
        let (prefix, functions, types) = match item {
            WorldItem::Function(f) => (String::new(), vec![f], Vec::new()),
            WorldItem::Interface { id, .. } => {
                let interface = &resolve.interfaces[*id];
                let prefix = if interface.name.as_deref() == Some("api")
                    && resolve.worlds[world].exports.len() == 1
                {
                    String::new()
                } else {
                    interface
                        .name
                        .as_deref()
                        .context("Go guest interface name")?
                        .to_upper_camel_case()
                };
                (
                    prefix,
                    interface.functions.values().collect(),
                    interface.types.iter().collect(),
                )
            }
            WorldItem::Type { .. } => continue,
        };
        if functions.is_empty() {
            continue;
        }
        let import_module = if matches!(item, WorldItem::Function(_)) {
            "$root".into()
        } else {
            resolve.name_world_key(key)
        };
        let marker = format!("//go:wasmimport {import_module} {}\n", functions[0].name);
        let candidates = files
            .iter()
            .filter(|(_, bytes)| std::str::from_utf8(bytes).is_ok_and(|s| s.contains(&marker)))
            .map(|(path, _)| {
                path.rsplit_once('/')
                    .map(|(directory, _)| directory.to_owned())
                    .context("upstream Go package directory")
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            candidates.len() == 1,
            "cannot identify upstream Go import package"
        );
        let package = &candidates[0];
        let next = packages.len();
        let alias = packages
            .entry(package.clone())
            .or_insert_with(|| format!("wisp{next}"));
        for f in functions {
            let name = format!("{prefix}{}", f.name.to_upper_camel_case());
            ensure!(
                names.insert(name.clone()),
                "Go guest binding name collision"
            );
            declarations.push_str(&format!(
                "var {name} = {alias}.{}\n",
                f.name.to_upper_camel_case()
            ));
        }
        for (name, id) in types {
            let name = name.to_upper_camel_case();
            let public = format!("{prefix}{name}");
            ensure!(names.insert(public.clone()), "Go guest type name collision");
            declarations.push_str(&format!("type {public} = {alias}.{name}\n"));
            let cases = match &resolve.types[*id].kind {
                TypeDefKind::Enum(e) => e.cases.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
                TypeDefKind::Flags(f) => f.flags.iter().map(|f| f.name.as_str()).collect(),
                TypeDefKind::Variant(v) => v.cases.iter().map(|c| c.name.as_str()).collect(),
                _ => vec![],
            };
            for case in cases {
                let variant = format!("{name}{}", case.to_upper_camel_case());
                let public = format!("{prefix}{variant}");
                ensure!(
                    names.insert(public.clone()),
                    "Go guest constant name collision"
                );
                declarations.push_str(&format!("const {public} = {alias}.{variant}\n"));
                if matches!(resolve.types[*id].kind, TypeDefKind::Variant(_)) {
                    let public = format!("{prefix}Make{variant}");
                    ensure!(
                        names.insert(public.clone()),
                        "Go guest constructor name collision"
                    );
                    declarations.push_str(&format!("var {public} = {alias}.Make{variant}\n"));
                }
            }
        }
    }
    for (package, alias) in packages {
        imports.push_str(&format!("{alias} \"{module}/{package}\"\n"));
    }
    files.insert("bindings_wasip1.go".into(), format!("// Code generated by wisp using wit-bindgen {}. DO NOT EDIT.\n\npackage component\n\nimport (\n{imports})\n\n{declarations}", upstream::GENERATOR_VERSION).into_bytes());
    for bytes in files.values_mut() {
        let mut tagged = b"//go:build wasip1 && wasm\n\n".to_vec();
        tagged.append(bytes);
        *bytes = tagged;
    }
    Ok(files)
}

pub fn files(
    component: &[u8],
    module: &str,
    version: &str,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    super::validate_module(module, version)?;
    let (resolve, world) = super::decode_component(component)?;
    super::validate_type_expansion(&resolve, world)?;
    let package = resolve.worlds[world]
        .package
        .context("Go guest world package")?;
    let others = resolve
        .packages
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != package)
        .collect::<Vec<_>>();
    let mut printer = wit_component::WitPrinter::default();
    printer.print(&resolve, package, &others)?;
    let descriptor = json!({"schema":1,"kind":"wisp-go-component-guest","module":module,"version":version,
        "component":{"path":"component.wasm","sha256":super::digest(component)},"platform":PLATFORM,
        "bindings":{"generator":"wit-bindgen","version":upstream::GENERATOR_VERSION},
        "runtime":{"module":upstream::RUNTIME_MODULE,"version":upstream::RUNTIME_VERSION}});
    let mod_file = module_file(module);
    let mut files = import_files(&resolve, world, module)?;
    files.extend(BTreeMap::from([
        ("go.mod".into(), mod_file),
        ("go.sum".into(), upstream::RUNTIME_SUM.as_bytes().to_vec()),
        ("component.wasm".into(), component.to_vec()),
        (
            "component.wit".into(),
            printer.output.to_string().into_bytes(),
        ),
        (
            "wisp-component.json".into(),
            serde_json::to_vec_pretty(&descriptor)?,
        ),
    ]));
    let mut size = 0;
    ensure!(notices.len() <= 17, "Go guest notice count limit");
    for (name, bytes) in notices {
        ensure!(
            !name.is_empty()
                && name.len() <= 240
                && !name.contains('\\')
                && name
                    .split('/')
                    .all(|p| !p.is_empty() && p != "." && p != ".."),
            "unsafe Go guest notice path"
        );
        ensure!(
            (name == "WISP-NOTICES.txt" || name.starts_with("notices/")) && !name.ends_with(".go"),
            "invalid Go guest notice"
        );
        size += bytes.len();
        ensure!(
            bytes.len() <= 256 * 1024 && size <= 1024 * 1024,
            "Go guest notice size limit"
        );
        ensure!(
            files.insert(name.clone(), bytes.clone()).is_none(),
            "Go guest notice collision"
        );
    }
    Ok(files)
}
#[allow(dead_code)]
pub fn generate(
    component: &[u8],
    output: &Path,
    module: &str,
    version: &str,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    ensure!(!output.try_exists()?, "Go guest output already exists");
    for (path, bytes) in files(component, module, version, notices)? {
        let path = output.join(path);
        fs::create_dir_all(path.parent().context("Go guest parent")?)?;
        fs::write(path, bytes)?;
    }
    Ok(())
}
pub fn verify_archive(
    component: &[u8],
    module: &str,
    version: &str,
    bytes: &[u8],
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<Value> {
    let temp = super::unpack_module_archive(bytes, module, version)?;
    let mut actual = BTreeMap::new();
    super::package_files(
        temp.path(),
        temp.path(),
        &mut actual,
        &mut (32 * 1024 * 1024),
    )?;
    ensure!(
        actual == files(component, module, version, notices)?,
        "Go guest generated package differs"
    );
    ensure!(
        super::module_archive(temp.path(), module, version)?.zip == bytes,
        "Go guest module archive is not canonical"
    );
    Ok(json!({"platform":PLATFORM}))
}
/// go mod vendor strips go.mod and can omit notice subdirectories. Authenticate
/// every executable/binding input retained by the normal Go module installer.
pub fn vendored(directory: &Path, module: &str) -> Result<Value> {
    let descriptor: Value =
        serde_json::from_slice(&read(&directory.join("wisp-component.json"), 1024 * 1024)?)?;
    let version = descriptor["version"]
        .as_str()
        .context("Go guest module version")?;
    super::validate_module(module, version)?;
    let component = read(&directory.join("component.wasm"), 16 * 1024 * 1024)?;
    let portable = descriptor["platform"] == super::portable::PLATFORM;
    let expected_files = if portable {
        super::portable::files(&component, module, version, &BTreeMap::new())?
    } else {
        files(&component, module, version, &BTreeMap::new())?
    };
    let expected_names = expected_files.keys().cloned().collect::<BTreeSet<_>>();
    for (name, expected) in expected_files {
        if name == "go.mod" || name == "go.sum" {
            continue;
        }
        ensure!(
            read(&directory.join(&name), 32 * 1024 * 1024)? == expected,
            "vendored Go guest package differs: {name}"
        );
    }
    fn inspect_tree(root: &Path, directory: &Path, expected: &BTreeSet<String>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            ensure!(
                !entry.file_type()?.is_symlink(),
                "symlink in vendored Go package"
            );
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                inspect_tree(root, &path, expected)?;
                continue;
            }
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("Go filename")?
                .replace('\\', "/");
            if matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("go" | "h" | "c" | "cc" | "cpp" | "cxx" | "s" | "S")
            ) {
                ensure!(
                    expected.contains(&name),
                    "unexpected Go package source: {name}"
                );
            }
        }
        Ok(())
    }
    inspect_tree(directory, directory, &expected_names)?;
    Ok(descriptor)
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "invalid Go guest input"
    );
    Ok(fs::read(path)?)
}
