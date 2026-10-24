//! Native scalar bindings using a bundled execution worker.
use anyhow::{Context, Result, bail};
use heck::ToUpperCamelCase;
use wit_parser::{Resolve, Type, TypeDefKind, WorldId, WorldItem};

fn scalar(resolve: &Resolve, ty: Type) -> Result<&'static str> {
    Ok(match ty {
        Type::Bool => "bool",
        Type::U8 => "u8",
        Type::U16 => "u16",
        Type::U32 => "u32",
        Type::U64 => "u64",
        Type::S8 => "i8",
        Type::S16 => "i16",
        Type::S32 => "i32",
        Type::S64 => "i64",
        Type::F32 => "f32",
        Type::F64 => "f64",
        Type::Id(id) => match resolve.types[id].kind {
            TypeDefKind::Type(ty) => return scalar(resolve, ty),
            _ => bail!("non-scalar worker binding"),
        },
        _ => bail!("non-scalar worker binding"),
    })
}
pub fn supports(resolve: &Resolve, world: WorldId) -> bool {
    // Imported capabilities still use the established native hosts. The worker
    // does not yet implement their complete admission and capability policies.
    if !resolve.worlds[world].imports.is_empty() {
        return false;
    }
    resolve.worlds[world]
        .exports
        .values()
        .all(|item| match item {
            WorldItem::Function(f) => {
                f.params.iter().all(|p| scalar(resolve, p.ty).is_ok())
                    && f.result.is_none_or(|t| scalar(resolve, t).is_ok())
            }
            WorldItem::Interface { id, .. } => {
                resolve.interfaces[*id]
                    .types
                    .values()
                    .all(|id| scalar(resolve, Type::Id(*id)).is_ok())
                    && resolve.interfaces[*id].functions.values().all(|f| {
                        matches!(f.kind, wit_parser::FunctionKind::Freestanding)
                            && f.params.iter().all(|p| scalar(resolve, p.ty).is_ok())
                            && f.result.is_none_or(|t| scalar(resolve, t).is_ok())
                    })
            }
            _ => false,
        })
}
pub fn source(resolve: &Resolve, world: WorldId, files: &crate::release::Files) -> Result<String> {
    let mut source = include_str!("../assets/templates/library-worker.rs").to_owned();
    let mut selections = String::new();
    for platform in wisp_worker_protocol::bundle::validate(files)? {
        let path = format!("worker/{platform}/wisp-worker");
        selections.push_str(&format!(
            "{platform:?} => include_bytes!({:?}),",
            format!("../{path}")
        ));
    }
    source = source.replace("@WORKER_SELECTION@", &selections);
    let protocol = include_str!("../../wisp-worker-protocol/src/lib.rs")
        .replace("pub mod client;", "")
        .replace("pub mod command;", "")
        .replace("pub mod bundle;", "");
    let client = include_str!("../../wisp-worker-protocol/src/client.rs")
        .replace("use crate::{", "use super::{");
    source.push_str(&format!(
        "\nmod protocol {{\n{protocol}\npub mod client {{\n{client}\n}}\n}}\n"
    ));
    for (key, item) in &resolve.worlds[world].exports {
        let (name, interface, functions, types) = match item {
            WorldItem::Function(f) => ("world".to_owned(), String::new(), vec![f], vec![]),
            WorldItem::Interface { id, .. } => {
                let i = &resolve.interfaces[*id];
                (
                    i.name.as_deref().context("worker interface")?.to_owned(),
                    resolve.name_world_key(key),
                    i.functions.values().collect(),
                    i.types.iter().collect(),
                )
            }
            _ => bail!("unsupported worker export"),
        };
        let module = super::ident(&name);
        // World functions are grouped below to avoid repeated modules.
        if matches!(item, WorldItem::Function(_)) {
            continue;
        }
        source.push_str(&format!("pub mod {module} {{\npub struct Client {{ pub(super) library: super::Library }}\nimpl Client {{ pub fn new()->Result<Self,super::CallError<std::convert::Infallible>> {{ Ok(Self {{ library: super::Library::new()? }}) }}\n"));
        for f in functions {
            source.push_str(&method(resolve, f, &interface)?);
        }
        source.push_str("}\n");
        for (name, id) in types {
            source.push_str(&format!(
                "pub type {} = {};\n",
                name.to_upper_camel_case(),
                scalar(resolve, Type::Id(*id))?
            ));
        }
        source.push_str(&format!("}}\nimpl Library {{pub fn {module}(&self)->{module}::Client {{{module}::Client {{library:self.clone()}}}}}}\n"));
    }
    let direct = resolve.worlds[world]
        .exports
        .values()
        .filter_map(|i| {
            if let WorldItem::Function(f) = i {
                Some(f)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !direct.is_empty() {
        source.push_str("pub mod world {pub struct Client {pub(super) library:super::Library} impl Client {pub fn new()->Result<Self,super::CallError<std::convert::Infallible>> {Ok(Self {library:super::Library::new()?})}\n");
        for f in direct {
            source.push_str(&method(resolve, f, "")?);
        }
        source.push_str("}} impl Library {pub fn world(&self)->world::Client {world::Client {library:self.clone()}}}\n");
    }
    Ok(source)
}
fn method(resolve: &Resolve, f: &wit_parser::Function, interface: &str) -> Result<String> {
    let params = f
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| Ok(format!("a{i}:{}", scalar(resolve, p.ty)?)))
        .collect::<Result<Vec<_>>>()?;
    let values = f
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            Ok(match scalar(resolve, p.ty)? {
                "f32" => format!("serde_json::json!({{\"f32\":a{i}.to_bits()}})"),
                "f64" => format!("serde_json::json!({{\"f64\":a{i}.to_bits()}})"),
                _ => format!("serde_json::json!(a{i})"),
            })
        })
        .collect::<Result<Vec<_>>>()?
        .join(",");
    let ty = f
        .result
        .map(|t| scalar(resolve, t))
        .transpose()?
        .unwrap_or("()");
    let decode = match ty {
        "f32" => {
            "Ok(f32::from_bits(result[0].get(\"f32\").and_then(serde_json::Value::as_u64).ok_or_else(||super::CallError::Runtime(\"invalid float result\".into()))? as u32))"
        }
        "f64" => {
            "Ok(f64::from_bits(result[0].get(\"f64\").and_then(serde_json::Value::as_u64).ok_or_else(||super::CallError::Runtime(\"invalid float result\".into()))?))"
        }
        "()" => "Ok(())",
        _ => {
            "serde_json::from_value(result.into_iter().next().ok_or_else(||super::CallError::Runtime(\"missing result\".into()))?).map_err(|e|super::CallError::Runtime(e.to_string()))"
        }
    };
    Ok(format!(
        "pub fn {}(&mut self{}{}) -> Result<{ty},super::CallError<std::convert::Infallible>> {{let result=self.library.call({interface:?},{:?},vec![{values}]).map_err(super::CallError::Runtime)?;{} }}\n",
        super::ident(&f.name),
        if params.is_empty() { "" } else { "," },
        params.join(","),
        f.name,
        decode
    ))
}
