//! Scalar Rust dependencies whose ordinary calls become Component imports.
use anyhow::{Context, Result, bail, ensure};
use heck::ToSnakeCase;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use wit_parser::{FunctionKind, Resolve, Type, TypeDefKind, WorldId, WorldItem};

pub const TARGET: &str = "wasm32-wasip1";
const COMPONENT_LIMIT: u64 = 16 * 1024 * 1024;
const FILE_LIMIT: u64 = 4 * 1024 * 1024;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn crate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name.as_bytes()[0].is_ascii_lowercase()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
            && !matches!(name, "std" | "core" | "alloc" | "proc_macro" | "test"),
        "invalid Rust guest crate name"
    );
    let library = name.replace('-', "_");
    let identifier = rust_ident(&library)?;
    ensure!(
        !identifier.starts_with("r#") && !matches!(library.as_str(), "self" | "super" | "crate"),
        "reserved Rust guest crate name"
    );
    Ok(())
}

fn rust_ident(name: &str) -> Result<String> {
    let ident = name.to_snake_case();
    ensure!(
        !ident.is_empty()
            && ident.as_bytes()[0].is_ascii_lowercase()
            && ident
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "unsafe Rust guest identifier: {name}"
    );
    Ok(if matches!(ident.as_str(), "self" | "super" | "crate") {
        format!("{ident}_")
    } else if matches!(
        ident.as_str(),
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "yield"
            | "try"
            | "gen"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
    ) {
        format!("r#{ident}")
    } else {
        ident
    })
}

fn scalar(resolve: &Resolve, ty: Type, depth: usize) -> Result<(&'static str, &'static str)> {
    ensure!(depth < 32, "Rust guest scalar alias depth limit");
    Ok(match ty {
        Type::Bool => ("bool", "i32"),
        Type::U8 => ("u8", "u32"),
        Type::U16 => ("u16", "u32"),
        Type::U32 => ("u32", "u32"),
        Type::U64 => ("u64", "u64"),
        Type::S8 => ("i8", "i32"),
        Type::S16 => ("i16", "i32"),
        Type::S32 => ("i32", "i32"),
        Type::S64 => ("i64", "i64"),
        Type::F32 => ("f32", "f32"),
        Type::F64 => ("f64", "f64"),
        Type::Id(id) => match resolve.types[id].kind {
            TypeDefKind::Type(ty) => return scalar(resolve, ty, depth + 1),
            _ => {
                bail!("unsupported Rust guest type: only scalars and scalar aliases are supported")
            }
        },
        _ => bail!("unsupported Rust guest type: only scalars and scalar aliases are supported"),
    })
}

fn convert(ty: (&str, &str), value: &str, lower: bool) -> String {
    if ty.0 == "bool" {
        if lower {
            format!("if {value} {{ 1 }} else {{ 0 }}")
        } else {
            format!("({value} != 0)")
        }
    } else {
        format!("({value} as {})", if lower { ty.1 } else { ty.0 })
    }
}

/// Generate a dependency API while preserving the Component's canonical import names.
pub fn bindings(resolve: &Resolve, world: WorldId) -> Result<String> {
    let reactor = crate::library_wasi::reactor_host(resolve, world)?;
    let mut exports = resolve.clone();
    exports.worlds[world].imports.clear();
    if let Ok(bindings) = crate::build_bindings::bindings(&exports, world) {
        if crate::build_values::needed(&bindings) {
            ensure!(
                resolve.worlds[world]
                    .imports
                    .values()
                    .all(|item| matches!(item, WorldItem::Type { .. }))
                    || reactor,
                "Rust guest dependencies require a closed Component or supported WASI reactor imports"
            );
            return crate::build_values::rust_guest(&bindings);
        }
    }
    let world = &resolve.worlds[world];
    ensure!(
        world
            .imports
            .values()
            .all(|item| matches!(item, WorldItem::Type { .. }))
            || reactor,
        "Rust guest dependencies require a closed Component or supported WASI reactor imports"
    );
    let callable_exports = world
        .exports
        .values()
        .filter(|item| !matches!(item, WorldItem::Type { .. }))
        .count();
    let mut source =
        String::from("// Code generated by wisp. DO NOT EDIT.\n#![allow(unused_parens)]\n\n");
    let mut outer_names = BTreeSet::new();
    let mut ordinal = 0;
    for (key, item) in &world.exports {
        let canonical_module = resolve.name_world_key(key);
        let (module, functions) = match item {
            WorldItem::Function(function) => (None, vec![function]),
            WorldItem::Interface { id, .. } => {
                let interface = &resolve.interfaces[*id];
                for ty in interface.types.values() {
                    scalar(resolve, Type::Id(*ty), 0)?;
                }
                let name = interface
                    .name
                    .as_deref()
                    .context("Rust guest interface name")?;
                let module = if name == "api" && callable_exports == 1 {
                    None
                } else {
                    Some(rust_ident(name)?)
                };
                (module, interface.functions.values().collect())
            }
            WorldItem::Type { id, .. } => {
                scalar(resolve, Type::Id(*id), 0)?;
                continue;
            }
        };
        if let Some(module) = &module {
            ensure!(
                outer_names.insert(module.clone()),
                "Rust guest binding name collision: {module}"
            );
            source.push_str(&format!("pub mod {module} {{\n"));
        }
        let mut local_names = BTreeSet::new();
        for function in functions {
            ensure!(
                matches!(function.kind, FunctionKind::Freestanding) && function.params.len() <= 16,
                "unsupported Rust guest function: synchronous scalar free functions with at most 16 parameters are required"
            );
            let name = rust_ident(&function.name)?;
            ensure!(
                local_names.insert(name.clone()),
                "Rust guest binding name collision: {name}"
            );
            if module.is_none() {
                ensure!(
                    outer_names.insert(name.clone()),
                    "Rust guest binding name collision: {name}"
                );
            }
            let params = function
                .params
                .iter()
                .map(|p| scalar(resolve, p.ty, 0))
                .collect::<Result<Vec<_>>>()?;
            let result = function
                .result
                .map(|ty| scalar(resolve, ty, 0))
                .transpose()?;
            let signature = |abi: bool| {
                params
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| format!("a{index}: {}", if abi { ty.1 } else { ty.0 }))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let result_type = |abi: bool| {
                result
                    .map(|ty| format!(" -> {}", if abi { ty.1 } else { ty.0 }))
                    .unwrap_or_default()
            };
            let imported_module = if matches!(item, WorldItem::Function(_)) {
                "$root"
            } else {
                &canonical_module
            };
            source.push_str(&format!(
                "#[cfg(target_arch = \"wasm32\")]\n#[link(wasm_import_module = {imported_module:?})]\nextern \"C\" {{\n    #[link_name = {:?}]\n    fn __wisp_import_{ordinal}({}){};\n}}\n\n",
                function.name, signature(true), result_type(true)
            ));
            // Host editors can check signatures without linking a native symbol with the
            // same name as a guest operation. Executing a guest API on the host fails.
            source.push_str(&format!(
                "#[cfg(not(target_arch = \"wasm32\"))]\n#[allow(unused_variables)]\nunsafe fn __wisp_import_{ordinal}({}){} {{\n    panic!(\"Wisp guest dependencies require wasm32-wasip1\")\n}}\n\n",
                signature(true), result_type(true)
            ));
            let arguments = params
                .iter()
                .enumerate()
                .map(|(index, ty)| convert(*ty, &format!("a{index}"), true))
                .collect::<Vec<_>>()
                .join(", ");
            let call = format!("unsafe {{ __wisp_import_{ordinal}({arguments}) }}");
            let body = result.map(|ty| convert(ty, &call, false)).unwrap_or(call);
            source.push_str(&format!(
                "pub fn {name}({}){} {{\n    {body}\n}}\n\n",
                signature(false),
                result_type(false)
            ));
            ordinal += 1;
            ensure!(ordinal <= 1024, "Rust guest function count limit");
        }
        if module.is_some() {
            source.push_str("}\n\n");
        }
    }
    ensure!(ordinal > 0, "Rust guest dependency has no callable exports");
    Ok(source)
}

fn decode(component: &[u8]) -> Result<(Resolve, WorldId)> {
    ensure!(
        component.len() <= COMPONENT_LIMIT as usize,
        "Rust guest Component size limit"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(component)?;
    let decoded = std::panic::catch_unwind(|| wit_parser::decoding::decode(component))
        .map_err(|_| anyhow::anyhow!("Rust guest WIT decoder rejected the type graph"))??;
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) = decoded else {
        bail!("Rust guest dependency requires an executable Component")
    };
    Ok((resolve, world))
}

fn identity(resolve: &Resolve, world: WorldId, version: &str) -> Result<String> {
    let version = semver::Version::parse(version).context("Rust guest package version")?;
    let mut identities = BTreeSet::new();
    for item in resolve.worlds[world].exports.values() {
        if let WorldItem::Interface { id, .. } = item {
            let package = resolve.interfaces[*id]
                .package
                .context("Rust guest interface package")?;
            let name = &resolve.packages[package].name;
            if let Some(exported_version) = &name.version {
                ensure!(
                    exported_version == &version,
                    "Rust guest package version differs from Component interface version"
                );
            }
            identities.insert(format!("{}:{}", name.namespace, name.name));
        }
    }
    if identities.is_empty() {
        let package = resolve.worlds[world]
            .package
            .context("Rust guest world package")?;
        let name = &resolve.packages[package].name;
        identities.insert(format!("{}:{}", name.namespace, name.name));
    }
    ensure!(
        identities.len() == 1,
        "Rust guest exports must belong to one Component package"
    );
    let identity = identities.into_iter().next().unwrap();
    crate::identity::parse(&identity)?;
    Ok(identity)
}

fn notice_path(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 240
            && !name.contains('\\')
            && name
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
            && (name == "WISP-NOTICES.txt" || name.starts_with("notices/")),
        "unsafe Rust guest notice path"
    );
    ensure!(
        !matches!(
            Path::new(name)
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("rs" | "toml" | "json" | "wasm")
        ),
        "Rust guest notice must not be executable or configuration"
    );
    Ok(())
}

#[cfg(test)]
pub fn files(component: &[u8], name: &str, version: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    files_with_notices(component, name, version, &BTreeMap::new())
}

pub fn files_with_notices(
    component: &[u8],
    name: &str,
    version: &str,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    crate_name(name)?;
    let parsed_version = semver::Version::parse(version).context("Rust guest version")?;
    ensure!(
        parsed_version.to_string() == version,
        "non-canonical Rust guest version"
    );
    let (resolve, world) = decode(component)?;
    let identity = identity(&resolve, world, version)?;
    let package = resolve.worlds[world]
        .package
        .context("Rust guest world package")?;
    let others = resolve
        .packages
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != package)
        .collect::<Vec<_>>();
    let mut printer = wit_component::WitPrinter::default();
    printer.print(&resolve, package, &others)?;
    ensure!(notices.len() <= 17, "Rust guest notice count limit");
    let mut notice_hashes = BTreeMap::new();
    let mut notice_size = 0;
    for (path, bytes) in notices {
        notice_path(path)?;
        notice_size += bytes.len();
        ensure!(
            bytes.len() <= 256 * 1024 && notice_size <= 1024 * 1024,
            "Rust guest notice size limit"
        );
        notice_hashes.insert(path.clone(), digest(bytes));
    }
    let descriptor = json!({"schema":1,"kind":"wisp-rust-component-guest","package":identity,"version":version,"crate":name,
        "component":{"path":"component.wasm","sha256":digest(component)},"target":TARGET,"notices":notice_hashes});
    let cargo = format!(
        "[package]\nname = {name:?}\nversion = {version:?}\nedition = \"2021\"\n\n[lib]\nname = {:?}\npath = \"src/lib.rs\"\n",
        name.replace('-', "_")
    );
    let mut files = BTreeMap::from([
        ("Cargo.toml".into(), cargo.into_bytes()),
        ("src/lib.rs".into(), bindings(&resolve, world)?.into_bytes()),
        ("component.wasm".into(), component.to_vec()),
        (
            "component.wit".into(),
            printer.output.to_string().into_bytes(),
        ),
        (
            "wisp-component.json".into(),
            serde_json::to_vec_pretty(&descriptor)?,
        ),
    ]);
    for (path, bytes) in notices {
        ensure!(
            files.insert(path.clone(), bytes.clone()).is_none(),
            "Rust guest notice path collision"
        );
    }
    Ok(files)
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading Rust guest input {}", path.display()))?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "invalid Rust guest input: {}",
        path.display()
    );
    Ok(fs::read(path)?)
}

/// Authenticate installed executable inputs against the embedded Component.
pub fn installed(directory: &Path) -> Result<Value> {
    ensure!(
        fs::symlink_metadata(directory)?.is_dir(),
        "Rust guest package must be a real directory"
    );
    let descriptor_bytes = read(&directory.join("wisp-component.json"), 1024 * 1024)?;
    let descriptor: Value = serde_json::from_slice(&descriptor_bytes)?;
    ensure!(
        descriptor["schema"] == 1
            && descriptor["kind"] == "wisp-rust-component-guest"
            && descriptor["target"] == TARGET,
        "invalid Rust guest descriptor"
    );
    ensure!(
        descriptor["component"]["path"] == "component.wasm",
        "unsafe Rust guest Component path"
    );
    let name = descriptor["crate"]
        .as_str()
        .context("Rust guest crate name")?;
    let version = descriptor["version"]
        .as_str()
        .context("Rust guest package version")?;
    let component = read(&directory.join("component.wasm"), COMPONENT_LIMIT)?;
    let mut notices = BTreeMap::new();
    let hashes = descriptor["notices"]
        .as_object()
        .context("Rust guest notice inventory")?;
    ensure!(hashes.len() <= 17, "Rust guest notice count limit");
    for (name, expected) in hashes {
        notice_path(name)?;
        let bytes = read(&directory.join(name), 256 * 1024)?;
        ensure!(
            expected.as_str() == Some(digest(&bytes).as_str()),
            "Rust guest notice checksum mismatch: {name}"
        );
        notices.insert(name.clone(), bytes);
    }
    let expected = files_with_notices(&component, name, version, &notices)?;
    ensure!(
        descriptor_bytes == expected["wisp-component.json"],
        "installed Rust guest descriptor differs from Component"
    );
    for (path, bytes) in &expected {
        ensure!(
            read(
                &directory.join(path),
                if path == "component.wasm" {
                    COMPONENT_LIMIT
                } else {
                    FILE_LIMIT
                }
            )? == *bytes,
            "installed Rust guest package differs: {path}"
        );
    }
    fn inspect(
        root: &Path,
        directory: &Path,
        expected: &BTreeMap<String, Vec<u8>>,
        count: &mut usize,
    ) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            *count += 1;
            ensure!(*count <= 256, "Rust guest file count limit");
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "symlink in installed Rust guest package"
            );
            let path = entry.path();
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("Rust guest filename")?
                .replace('\\', "/");
            if kind.is_dir() {
                ensure!(
                    expected
                        .keys()
                        .any(|name| name.starts_with(&format!("{relative}/"))),
                    "unexpected Rust guest directory: {relative}"
                );
                inspect(root, &path, expected, count)?;
            } else if relative == "Cargo.lock" {
                let _: toml::Value =
                    toml::from_str(std::str::from_utf8(&read(&path, 1024 * 1024)?)?)?;
            } else if relative == ".cargo-checksum.json" {
                let _: Value = serde_json::from_slice(&read(&path, 1024 * 1024)?)?;
            } else {
                ensure!(
                    kind.is_file() && expected.contains_key(&relative),
                    "unexpected Rust guest file: {relative}"
                );
            }
        }
        Ok(())
    }
    inspect(directory, directory, &expected, &mut 0)?;
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component() -> Vec<u8> {
        use wasm_encoder::*;
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types
            .ty()
            .function([ValType::I32, ValType::I32], [ValType::I32]);
        types.ty().function([ValType::I32], [ValType::I32]);
        types.ty().function([ValType::I64], [ValType::I64]);
        module.section(&types);
        let mut functions = FunctionSection::new();
        for ty in [0, 1, 2] {
            functions.function(ty);
        }
        module.section(&functions);
        let mut exports = ExportSection::new();
        for (index, name) in ["add", "negate", "widen"].into_iter().enumerate() {
            exports.export(
                &format!("test:guest/api@0.1.0#{name}"),
                ExportKind::Func,
                index as u32,
            );
        }
        module.section(&exports);
        let mut code = CodeSection::new();
        for instructions in [
            vec![
                Instruction::LocalGet(0),
                Instruction::LocalGet(1),
                Instruction::I32Add,
                Instruction::End,
            ],
            vec![
                Instruction::LocalGet(0),
                Instruction::I32Eqz,
                Instruction::End,
            ],
            vec![
                Instruction::LocalGet(0),
                Instruction::I64Const(1),
                Instruction::I64Add,
                Instruction::End,
            ],
        ] {
            let mut function = Function::new([]);
            for instruction in instructions {
                function.instruction(&instruction);
            }
            code.function(&function);
        }
        module.section(&code);
        let mut core = module.finish();
        let mut resolve = Resolve::default();
        let package = resolve.push_str("guest.wit", "package test:guest@0.1.0; interface api { add: func(a: u32, b: u32) -> u32; negate: func(value: bool) -> bool; widen: func(value: u64) -> u64; } world library { export api; }").unwrap();
        let world = resolve.select_world(&[package], Some("library")).unwrap();
        wit_component::embed_component_metadata(
            &mut core,
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
        )
        .unwrap();
        wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&core)
            .unwrap()
            .encode()
            .unwrap()
    }

    fn write_files(directory: &Path, files: &BTreeMap<String, Vec<u8>>) {
        for (name, bytes) in files {
            let path = directory.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
    }

    fn wit_bindings(wit: &str) -> Result<String> {
        let mut resolve = Resolve::default();
        let package = resolve.push_str("guest.wit", wit)?;
        let world = resolve.select_world(&[package], Some("library"))?;
        bindings(&resolve, world)
    }

    #[test]
    fn generated_scalar_crate_keeps_canonical_imports_and_has_no_dependencies() {
        let component = component();
        let files = files(&component, "guest-arithmetic", "0.1.0").unwrap();
        let descriptor: Value = serde_json::from_slice(&files["wisp-component.json"]).unwrap();
        assert_eq!(descriptor["package"], "test:guest");
        assert_eq!(descriptor["crate"], "guest-arithmetic");
        assert_eq!(descriptor["component"]["sha256"], digest(&component));
        let cargo: toml::Value =
            toml::from_str(std::str::from_utf8(&files["Cargo.toml"]).unwrap()).unwrap();
        assert_eq!(cargo["package"]["edition"].as_str(), Some("2021"));
        assert!(cargo.get("dependencies").is_none());
        let source = std::str::from_utf8(&files["src/lib.rs"]).unwrap();
        assert!(source.contains("pub fn add(a0: u32, a1: u32) -> u32"));
        assert!(source.contains("pub fn negate(a0: bool) -> bool"));
        assert!(source.contains("pub fn widen(a0: u64) -> u64"));
        assert!(source.contains("wasm_import_module = \"test:guest/api@0.1.0\""));
        assert!(source.contains("if a0 { 1 } else { 0 }"));
        assert!(source.contains("!= 0"));
        assert!(source.contains("Wisp guest dependencies require wasm32-wasip1"));
        assert!(!source.contains("pub mod api"));
        let installed_dir = tempfile::tempdir().unwrap();
        write_files(installed_dir.path(), &files);
        assert_eq!(installed(installed_dir.path()).unwrap(), descriptor);
    }

    #[test]
    fn named_interfaces_become_modules_and_scalar_aliases_are_resolved() {
        let source = wit_bindings("package test:guest; interface arithmetic { type small = s16; calculate: func(value: small) -> small; } world library { export arithmetic; }").unwrap();
        assert!(source.contains("pub mod arithmetic"));
        assert!(source.contains("pub fn calculate(a0: i16) -> i16"));
        assert!(source.contains("wasm_import_module = \"test:guest/arithmetic\""));
        assert!(source.contains("fn __wisp_import_0(a0: i32) -> i32"));
    }

    #[test]
    fn keyword_names_are_escaped_and_root_imports_are_exact() {
        let source = wit_bindings(
            "package test:guest; world library { export %type: func(value: bool) -> bool; }",
        )
        .unwrap();
        assert!(source.contains("pub fn r#type(a0: bool) -> bool"));
        assert!(source.contains("wasm_import_module = \"$root\""));
        assert!(source.contains("link_name = \"type\""));
    }

    #[test]
    fn unsupported_signatures_imports_and_collisions_are_rejected() {
        for wit in [
            "package test:guest; interface api { echo: func(value: list<u16>) -> list<u16>; } world library { export api; }",
            "package test:guest; interface api { resource thing; } world library { export api; }",
            "package test:guest; interface api { work: async func() -> u32; } world library { export api; }",
            "package test:guest; interface dep { add: func(a: u32, b: u32) -> u32; } interface api { add: func(a: u32, b: u32) -> u32; } world library { import dep; export api; }",
            "package test:guest; world library { export %self: func(); export self-: func(); }",
        ] {
            assert!(wit_bindings(wit).is_err(), "unexpectedly accepted {wit}");
        }
    }

    #[test]
    fn owned_values_generate_dependency_free_guest_apis() {
        let source = wit_bindings("package test:guest; interface api { record point { label:string, data:list<u8> } echo:func(value:point)->result<point,string>; } world library { export api; }").unwrap();
        syn::parse_file(&source).expect("generated owned guest bindings must parse");
        assert!(source.contains("pub struct WispPoint"));
        assert!(source.contains("pub fn r#echo(a0:WispPoint) -> Result<WispPoint, String>"));
        assert!(source.contains("std::alloc::dealloc"));
        assert!(!source.contains("#[export_name=\"cabi_realloc\"]"));
    }

    #[test]
    fn installed_rejects_rehashed_descriptor_and_generated_source_changes() {
        let original = files(&component(), "guest-arithmetic", "0.1.0").unwrap();
        let directory = tempfile::tempdir().unwrap();
        write_files(directory.path(), &original);
        let source_path = directory.path().join("src/lib.rs");
        fs::write(
            &source_path,
            "pub fn add(a: u32, b: u32) -> u32 { a.wrapping_sub(b) }\n",
        )
        .unwrap();
        let mut descriptor: Value =
            serde_json::from_slice(&original["wisp-component.json"]).unwrap();
        descriptor["sourceSha256"] = json!(digest(&fs::read(source_path).unwrap()));
        fs::write(
            directory.path().join("wisp-component.json"),
            serde_json::to_vec_pretty(&descriptor).unwrap(),
        )
        .unwrap();
        assert!(installed(directory.path()).is_err());
        write_files(directory.path(), &original);
        fs::write(
            directory.path().join("Cargo.toml"),
            b"[package]\nname='guest-arithmetic'\nversion='0.1.0'\nbuild='build.rs'\n",
        )
        .unwrap();
        assert!(installed(directory.path()).is_err());
    }

    #[test]
    fn notices_are_carried_checked_and_never_treated_as_source() {
        let notices = BTreeMap::from([
            ("WISP-NOTICES.txt".into(), b"Author notice\n".to_vec()),
            ("notices/LICENSE".into(), b"MIT\n".to_vec()),
        ]);
        let files =
            files_with_notices(&component(), "guest-arithmetic", "0.1.0", &notices).unwrap();
        let directory = tempfile::tempdir().unwrap();
        write_files(directory.path(), &files);
        installed(directory.path()).unwrap();
        fs::write(directory.path().join("notices/LICENSE"), "modified").unwrap();
        assert!(installed(directory.path()).is_err());
        for path in [
            "../outside",
            "notices/../build.rs",
            "notices/extra.rs",
            "/tmp/NOTICE",
            "notices\\LICENSE",
        ] {
            assert!(
                files_with_notices(
                    &component(),
                    "guest-arithmetic",
                    "0.1.0",
                    &BTreeMap::from([(path.into(), vec![])])
                )
                .is_err()
            );
        }
    }

    #[test]
    fn installed_allows_inert_cargo_metadata_but_rejects_extra_code() {
        let files = files(&component(), "guest-arithmetic", "0.1.0").unwrap();
        let directory = tempfile::tempdir().unwrap();
        write_files(directory.path(), &files);
        fs::write(directory.path().join("Cargo.lock"), "version = 4\n").unwrap();
        fs::write(directory.path().join(".cargo-checksum.json"), "{}").unwrap();
        installed(directory.path()).unwrap();
        for name in ["build.rs", "src/extra.rs", ".cargo/config.toml"] {
            let path = directory.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "injected").unwrap();
            assert!(installed(directory.path()).is_err());
            fs::remove_file(path).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn installed_rejects_symlinked_inputs_and_package_roots() {
        let files = files(&component(), "guest-arithmetic", "0.1.0").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("package");
        write_files(&package, &files);
        let link = directory.path().join("linked-package");
        std::os::unix::fs::symlink(&package, &link).unwrap();
        assert!(installed(&link).is_err());
        let source = package.join("src/lib.rs");
        fs::remove_file(&source).unwrap();
        let outside = directory.path().join("bindings.rs");
        fs::write(&outside, &files["src/lib.rs"]).unwrap();
        std::os::unix::fs::symlink(&outside, &source).unwrap();
        assert!(installed(&package).is_err());
    }

    #[test]
    fn invalid_crate_names_versions_and_noncomponents_are_rejected() {
        let component = component();
        for name in [
            "",
            "../escape",
            "Upper",
            "quote\"name",
            "1crate",
            "std",
            "type",
            "self",
            "with space",
        ] {
            assert!(files(&component, name, "0.1.0").is_err());
        }
        assert!(files(&component, "guest-arithmetic", "0.2.0").is_err());
        assert!(files(b"\0asm\x01\0\0\0", "guest-arithmetic", "0.1.0").is_err());
    }
}
