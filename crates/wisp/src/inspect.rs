use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub(crate) fn expand(value: &Value, doc: &Value, depth: usize) -> Result<Value> {
    if depth > 256 {
        bail!("WIT type expansion depth limit");
    }
    Ok(match value {
        Value::Number(n) => expand(
            &doc["types"][n.as_u64().context("invalid type index")? as usize]["kind"],
            doc,
            depth + 1,
        )?,
        Value::Object(map) if map.len() == 1 && map.contains_key("type") => {
            expand(&map["type"], doc, depth + 1)?
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), expand(v, doc, depth + 1)?)))
                .collect::<Result<_>>()?,
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| expand(v, doc, depth + 1))
                .collect::<Result<_>>()?,
        ),
        _ => value.clone(),
    })
}
fn max_index(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::Array(a) => a.iter().filter_map(max_index).max(),
        Value::Object(o) => o.values().filter_map(max_index).max(),
        _ => None,
    }
}
pub fn ownership(path: &Path) -> Result<Value> {
    let bytes = fs::read(path)?;
    let wit_parser::decoding::DecodedWasm::Component(resolve, world_id) =
        wit_parser::decoding::decode(&bytes)?
    else {
        bail!("expected executable Component");
    };
    let world = &resolve.worlds[world_id];
    let root = resolve.packages[world.package.context("world has no package")?]
        .name
        .to_string();
    let mut packages = Vec::new();
    for (_, package) in resolve.packages.iter() {
        let mut interfaces: Vec<_> = package.interfaces.keys().cloned().collect();
        interfaces.sort();
        packages.push(json!({"identity":package.name.to_string(),"interfaces":interfaces}));
    }
    packages.sort_by_key(|p| p["identity"].as_str().unwrap().to_owned());
    let mut imports = Vec::new();
    for (key, item) in &world.imports {
        if let wit_parser::WorldItem::Interface { id, .. } = item {
            let interface = &resolve.interfaces[*id];
            let owner = resolve.packages
                [interface.package.context("unowned imported interface")?]
            .name
            .to_string();
            imports.push(json!({"package":owner,"interface":interface.name,"import":resolve.name_world_key(key)}));
        } else {
            imports.push(json!({"import":resolve.name_world_key(key),"package":null}));
        }
    }
    imports.sort_by_key(|p| p["import"].as_str().unwrap().to_owned());
    Ok(json!({"schema":1,"root":root,"packages":packages,"worldImports":imports}))
}
pub fn inspect(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if !bytes.starts_with(b"\0asm\x0d\0\x01\0") {
        bail!("expected a binary Component, not a core module or WIT");
    }
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .context("invalid Component binary")?;
    let decoded = wit_parser::decoding::decode(&bytes)?;
    let wit_parser::decoding::DecodedWasm::Component(resolve, world_id) = decoded else {
        bail!("expected executable Component, not encoded WIT package");
    };
    let contract_model = crate::wit_model::AbiModel::from_resolve(&resolve, world_id)?;
    let binding_inventory = contract_model.emit_binding_inventory();
    let canonical_contract = contract_model.canonical_contract()?;
    let contract_fingerprint = contract_model.contract_fingerprint()?;
    let contract = contract_model.canonical_json()?;
    let doc = serde_json::to_value(&resolve)?;
    let world = &doc["worlds"][world_id.index()];
    let mut imports = Vec::new();
    for (name, item) in world["imports"].as_object().context("world imports")? {
        // World-local type declarations live in wit_parser's imports map but
        // are compile-time names, not runtime Component imports.
        if !item["type"].is_null() {
            continue;
        }
        let name = if let Some(id) = item["interface"]["id"].as_u64() {
            let interface = &doc["interfaces"][id as usize];
            let pkg = doc["packages"]
                [interface["package"].as_u64().context("package owner")? as usize]["name"]
                .as_str()
                .context("package identity")?;
            let (name, version) = pkg.split_once('@').unwrap_or((pkg, ""));
            format!(
                "{}/{}{}",
                name,
                interface["name"].as_str().context("interface name")?,
                if version.is_empty() {
                    String::new()
                } else {
                    format!("@{version}")
                }
            )
        } else {
            name.clone()
        };
        imports.push(name);
    }
    imports.sort();
    let mut exports = Vec::new();
    for (name, item) in world["exports"].as_object().context("world exports")? {
        let f = &item["function"];
        if f.is_null() {
            exports.push(json!({"name":name,"unsupported":item}));
        } else if f["kind"] != "freestanding" {
            exports.push(json!({"name":name,"unsupported":f["kind"]}));
        } else {
            let params = f["params"]
                .as_array()
                .context("function params")?
                .iter()
                .map(|p| expand(&p["type"], &doc, 0))
                .collect::<Result<Vec<_>>>()?;
            exports
                .push(json!({"name":name,"params":params,"result":expand(&f["result"],&doc,0)?}));
        }
    }
    if let Ok(plan) = contract_model.binding_plan() {
        exports = plan
            .executable_export_shape()
            .as_array()
            .context("generated executable export shape")?
            .clone();
    }
    let mut report = json!({"contract":1,"sha256":digest(&bytes),"imports":imports,"exports":exports,"witContract":contract,"bindingInventory":binding_inventory,"canonicalContract":canonical_contract,"contractFingerprint":contract_fingerprint});
    if !imports.is_empty() {
        let bound = max_index(&doc["interfaces"])
            .map(|n| n as usize + 1)
            .unwrap_or(0);
        let types = doc["types"].as_array().context("types")?;
        // The bound counts every integer field, including package IDs, so clamp
        // it to the type array when those IDs exceed its length.
        let bound = bound.min(types.len());
        let packages: Vec<_> = doc["packages"]
            .as_array()
            .context("packages")?
            .iter()
            .filter(|p| p["interfaces"].as_object().is_some_and(|i| !i.is_empty()))
            .collect();
        let graph =
            json!({"interfaces":doc["interfaces"],"types":types[..bound],"packages":packages});
        report["importContract"] = json!(digest(&serde_json::to_vec(&graph)?));
    }
    Ok(report)
}
