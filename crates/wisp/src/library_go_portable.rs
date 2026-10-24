//! One scalar Go API with native Wago and Component guest implementations.
use anyhow::{Context, Result, ensure};
use heck::ToUpperCamelCase;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
use wit_parser::{Resolve, Type, TypeDefKind, WorldId, WorldItem};

pub const PLATFORM: &str = "native_wasm";
pub const RUNTIME: &str = "github.com/wago-org/wago";
// Upstream main, dc13730184e8ba6a1a2b3e3887c53a2b880db88c.
pub const RUNTIME_VERSION: &str = "v0.1.0-beta.13.0.20261023135323-dc13730184e8";
pub fn module_file(module: &str) -> Vec<u8> {
    format!("module {module}\n\ngo 1.27.0\n\nrequire (\n\tgithub.com/wago-org/component-model v0.1.7-0.20261005115302-e43b64caa85b\n\t{RUNTIME} {RUNTIME_VERSION}\n\tgithub.com/wago-org/wasi v0.3.2-0.20261005152821-e461db531d72\n)\n\nrequire golang.org/x/sys v0.30.0 // indirect\n").into_bytes()
}

fn scalar(resolve: &Resolve, ty: Type) -> Result<&'static str> {
    Ok(match ty {
        Type::Bool => "bool",
        Type::U8 => "uint8",
        Type::U16 => "uint16",
        Type::U32 => "uint32",
        Type::U64 => "uint64",
        Type::S8 => "int8",
        Type::S16 => "int16",
        Type::S32 => "int32",
        Type::S64 => "int64",
        Type::F32 => "float32",
        Type::F64 => "float64",
        Type::Id(id) => match resolve.types[id].kind {
            TypeDefKind::Type(ty) => return scalar(resolve, ty),
            _ => anyhow::bail!("unsupported portable Go value"),
        },
        _ => anyhow::bail!("unsupported portable Go value"),
    })
}
// Wago represents narrow integers as their signed or unsigned 32-bit ABI value.
fn abi_type(ty: &str) -> &str {
    match ty {
        "uint8" | "uint16" => "uint32",
        "int8" | "int16" => "int32",
        _ => ty,
    }
}

fn native(resolve: &Resolve, world: WorldId) -> Result<String> {
    let mut go = String::new();
    let world = &resolve.worlds[world];
    for (key, item) in &world.exports {
        let (interface, prefix, functions) = match item {
            WorldItem::Function(f) => (String::new(), String::new(), vec![f]),
            WorldItem::Interface { id, .. } => {
                let interface = &resolve.interfaces[*id];
                let name = interface.name.as_deref().context("Go interface name")?;
                (
                    resolve.name_world_key(key),
                    if name == "api" && world.exports.len() == 1 {
                        String::new()
                    } else {
                        name.to_upper_camel_case()
                    },
                    interface.functions.values().collect(),
                )
            }
            WorldItem::Type { .. } => continue,
        };
        for f in functions {
            let params = f
                .params
                .iter()
                .map(|p| scalar(resolve, p.ty))
                .collect::<Result<Vec<_>>>()?;
            let result = f.result.map(|t| scalar(resolve, t)).transpose()?;
            let signature = params
                .iter()
                .enumerate()
                .map(|(i, ty)| format!("a{i} {ty}"))
                .collect::<Vec<_>>()
                .join(", ");
            let args = params
                .iter()
                .enumerate()
                .map(|(i, ty)| format!(", {}(a{i})", abi_type(ty)))
                .collect::<String>();
            let result_ty = result.map(|ty| format!(" {ty}")).unwrap_or_default();
            let call = format!("wispCall({interface:?}, {:?}{args})", f.name);
            let body = result
                .map(|ty| format!("return {ty}({call}[0].({}))", abi_type(ty)))
                .unwrap_or(call);
            go.push_str(&format!(
                "func {prefix}{}({signature}){result_ty} {{\n{body}\n}}\n",
                f.name.to_upper_camel_case()
            ));
        }
    }
    Ok(
        include_str!("../assets/templates/library-go-portable.go.in")
            .replace("@GO_FUNCTIONS@", &go),
    )
}

pub fn supports(component: &[u8]) -> bool {
    super::decode_component(component)
        .and_then(|(resolve, world)| {
            super::validate_type_expansion(&resolve, world)?;
            super::guest::bindings(&resolve, world).map(|_| ())
        })
        .is_ok()
}
pub fn files(
    component: &[u8],
    module: &str,
    version: &str,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = super::guest::files(component, module, version, notices)?;
    let native_component = super::wago::component(component)?;
    let native_path = if native_component != component {
        files.insert("component_native.wasm".into(), native_component);
        "component_native.wasm"
    } else {
        "component.wasm"
    };
    let (resolve, world) = super::decode_component(component)?;
    let reactor = crate::library_wasi::reactor_host(&resolve, world)?;
    let native = native(&resolve, world)?
        .replace("@NATIVE_COMPONENT@", native_path)
        .replace(
            "@WASI_IMPORT@",
            if reactor {
                "p2 \"github.com/wago-org/wasi/p2\""
            } else {
                ""
            },
        )
        .replace(
            "@WASI_OPTIONS@",
            if reactor {
                "p2.Options(p2.Config{})..."
            } else {
                ""
            },
        );
    files.insert("go.mod".into(), module_file(module));
    files.insert(
        "go.sum".into(),
        include_bytes!("../assets/library-go-wago.sum").to_vec(),
    );
    let mut descriptor: Value = serde_json::from_slice(&files["wisp-component.json"])?;
    descriptor["kind"] = json!("wisp-go-component-library");
    descriptor["platform"] = json!(PLATFORM);
    descriptor["runtime"] = json!({"module":RUNTIME,"version":RUNTIME_VERSION});
    files.insert(
        "wisp-component.json".into(),
        serde_json::to_vec_pretty(&descriptor)?,
    );
    files.insert("bindings_native.go".into(), super::format_go(&native)?);
    Ok(files)
}
pub fn generate(
    component: &[u8],
    output: &Path,
    module: &str,
    version: &str,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<Value> {
    let files = files(component, module, version, notices)?;
    ensure!(!output.try_exists()?, "Go output already exists");
    for (path, bytes) in files {
        let path = output.join(path);
        fs::create_dir_all(path.parent().context("Go parent")?)?;
        fs::write(path, bytes)?;
    }
    Ok(json!({"platform":PLATFORM}))
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
        &mut (256 * 1024 * 1024),
    )?;
    ensure!(
        actual == files(component, module, version, notices)?,
        "portable Go generated package differs"
    );
    ensure!(
        super::module_archive(temp.path(), module, version)?.zip == bytes,
        "portable Go module archive is not canonical"
    );
    Ok(json!({"platform":PLATFORM}))
}
