//! Go libraries backed by contract-specific typed Rust Component calls.
#[path = "library_go_guest.rs"]
pub mod guest;
#[path = "library_go_licenses.rs"]
mod licenses;
#[path = "library_go_portable.rs"]
pub mod portable;
#[path = "library_go_wago.rs"]
mod wago;
use anyhow::{Context, Result, bail, ensure};
use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};
use wit_parser::{
    FunctionKind, Handle, Resolve, Type, TypeDefKind, TypeId, TypeOwner, WorldId, WorldItem,
};

#[derive(Clone, Copy)]
enum ReaderKind {
    Future,
    Stream,
}
fn is_error_context(resolve: &Resolve, ty: Type, depth: usize) -> Result<bool> {
    ensure!(depth < 64, "Go error-context alias depth limit");
    Ok(match ty {
        Type::ErrorContext => true,
        Type::Id(id) => match resolve.types[id].kind {
            TypeDefKind::Type(inner) => is_error_context(resolve, inner, depth + 1)?,
            _ => false,
        },
        _ => false,
    })
}
fn contains_error_context(
    resolve: &Resolve,
    ty: Type,
    seen: &mut BTreeSet<TypeId>,
    depth: usize,
) -> Result<bool> {
    ensure!(depth < 64, "Go error-context type depth limit");
    if ty == Type::ErrorContext {
        return Ok(true);
    }
    let Type::Id(id) = ty else { return Ok(false) };
    if !seen.insert(id) {
        return Ok(false);
    }
    let result = match &resolve.types[id].kind {
        TypeDefKind::Type(ty) | TypeDefKind::List(ty) | TypeDefKind::Option(ty) => {
            contains_error_context(resolve, *ty, seen, depth + 1)?
        }
        TypeDefKind::Record(record) => record.fields.iter().try_fold(false, |found, field| {
            Ok::<bool, anyhow::Error>(
                found || contains_error_context(resolve, field.ty, seen, depth + 1)?,
            )
        })?,
        TypeDefKind::Variant(variant) => {
            variant
                .cases
                .iter()
                .filter_map(|case| case.ty)
                .try_fold(false, |found, ty| {
                    Ok::<bool, anyhow::Error>(
                        found || contains_error_context(resolve, ty, seen, depth + 1)?,
                    )
                })?
        }
        TypeDefKind::Tuple(tuple) => tuple.types.iter().try_fold(false, |found, ty| {
            Ok::<bool, anyhow::Error>(
                found || contains_error_context(resolve, *ty, seen, depth + 1)?,
            )
        })?,
        TypeDefKind::Result(result) => {
            result
                .ok
                .iter()
                .chain(result.err.iter())
                .try_fold(false, |found, ty| {
                    Ok::<bool, anyhow::Error>(
                        found || contains_error_context(resolve, *ty, seen, depth + 1)?,
                    )
                })?
        }
        TypeDefKind::Future(Some(ty)) | TypeDefKind::Stream(Some(ty)) => {
            contains_error_context(resolve, *ty, seen, depth + 1)?
        }
        _ => false,
    };
    seen.remove(&id);
    Ok(result)
}
fn reader_type(
    resolve: &Resolve,
    ty: Type,
    depth: usize,
) -> Result<Option<(ReaderKind, Option<Type>)>> {
    ensure!(depth < 64, "Go async type depth limit");
    let Type::Id(id) = ty else { return Ok(None) };
    match resolve.types[id].kind {
        TypeDefKind::Future(payload) => Ok(Some((ReaderKind::Future, payload))),
        TypeDefKind::Stream(payload) => Ok(Some((ReaderKind::Stream, payload))),
        TypeDefKind::Type(inner) => reader_type(resolve, inner, depth + 1),
        _ => Ok(None),
    }
}
fn validate_type_expansion(resolve: &Resolve, world: WorldId) -> Result<()> {
    ensure!(resolve.types.len() <= 4096, "library contract size limit");
    fn visit(
        resolve: &Resolve,
        ty: Type,
        cache: &mut BTreeMap<TypeId, (usize, usize)>,
        active: &mut BTreeSet<TypeId>,
        height: usize,
    ) -> Result<(usize, usize)> {
        ensure!(height <= 64, "library type depth limit");
        let Type::Id(id) = ty else { return Ok((1, 1)) };
        if let Some(value) = cache.get(&id) {
            ensure!(
                height.saturating_add(value.1).saturating_sub(1) <= 64,
                "library type depth limit"
            );
            return Ok(*value);
        }
        ensure!(active.insert(id), "library recursive type graph");
        let children: Vec<Type> = match &resolve.types[id].kind {
            TypeDefKind::Type(ty)
            | TypeDefKind::List(ty)
            | TypeDefKind::Option(ty)
            | TypeDefKind::Future(Some(ty))
            | TypeDefKind::Stream(Some(ty)) => vec![*ty],
            TypeDefKind::Record(record) => record.fields.iter().map(|field| field.ty).collect(),
            TypeDefKind::Variant(variant) => {
                variant.cases.iter().filter_map(|case| case.ty).collect()
            }
            TypeDefKind::Tuple(tuple) => tuple.types.clone(),
            TypeDefKind::Result(result) => {
                result.ok.iter().chain(result.err.iter()).copied().collect()
            }
            _ => Vec::new(),
        };
        let (mut cost, mut depth) = (1usize, 1usize);
        for child in children {
            let (child_cost, child_depth) = visit(resolve, child, cache, active, height + 1)?;
            cost = cost.saturating_add(child_cost);
            depth = depth.max(child_depth.saturating_add(1));
            ensure!(cost <= 4096, "library type expanded value limit");
            ensure!(depth <= 64, "library type depth limit");
        }
        active.remove(&id);
        cache.insert(id, (cost, depth));
        Ok((cost, depth))
    }
    let (mut cache, mut active) = (BTreeMap::new(), BTreeSet::new());
    for item in resolve.worlds[world]
        .imports
        .values()
        .chain(resolve.worlds[world].exports.values())
    {
        let WorldItem::Interface { id, .. } = item else {
            continue;
        };
        for id in resolve.interfaces[*id].types.values() {
            visit(resolve, Type::Id(*id), &mut cache, &mut active, 1)?;
        }
        for function in resolve.interfaces[*id].functions.values() {
            let mut expanded = 0usize;
            for ty in function
                .params
                .iter()
                .map(|p| p.ty)
                .chain(function.result.iter().copied())
            {
                let (cost, _) = visit(resolve, ty, &mut cache, &mut active, 1)?;
                expanded = expanded.saturating_add(cost);
                ensure!(
                    expanded <= 4096,
                    "library function expanded value limit: {}",
                    function.name
                );
            }
        }
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn ident(name: &str) -> String {
    let name = name.to_snake_case();
    if matches!(name.as_str(), "self" | "super" | "crate") {
        format!("{name}_")
    } else if matches!(
        name.as_str(),
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
    ) {
        format!("r#{name}")
    } else {
        name
    }
}
fn rust_case(name: &str) -> String {
    let name = name.to_upper_camel_case();
    if name == "Self" { "Self_".into() } else { name }
}

pub struct Sources {
    pub rust: String,
    pub go: String,
    pub values: String,
    pub header: String,
    pub prefix: String,
}
struct Generator<'a> {
    resolve: &'a Resolve,
    seen: BTreeSet<TypeId>,
    active: BTreeSet<TypeId>,
    names: BTreeSet<String>,
    tuples: BTreeSet<usize>,
    types: String,
    codecs: String,
}
impl Generator<'_> {
    fn named(&self, id: TypeId) -> Result<(String, String)> {
        let ty = &self.resolve.types[id];
        let name = ty
            .name
            .as_deref()
            .context("Go record/variant types must be named")?;
        let TypeOwner::Interface(interface) = ty.owner else {
            bail!("Go named types must belong to interfaces")
        };
        let interface = &self.resolve.interfaces[interface];
        let interface_name = interface.name.as_deref().context("Go interface name")?;
        let package =
            &self.resolve.packages[interface.package.context("Go interface package")?].name;
        Ok((
            format!(
                "{}{}",
                interface_name.to_upper_camel_case(),
                name.to_upper_camel_case()
            ),
            format!(
                "bindings::exports::{}::{}::{}::{}",
                ident(&package.namespace),
                ident(&package.name),
                ident(interface_name),
                rust_case(name)
            ),
        ))
    }
    fn resource_id(&self, mut id: TypeId) -> Result<TypeId> {
        for _ in 0..64 {
            match self.resolve.types[id].kind {
                TypeDefKind::Resource => return Ok(id),
                TypeDefKind::Type(Type::Id(next)) => id = next,
                _ => bail!("invalid Go resource alias"),
            }
        }
        bail!("Go resource alias depth limit")
    }
    fn rust_type(&self, ty: &Type, depth: usize) -> Result<String> {
        ensure!(depth < 64, "Go type depth limit");
        Ok(match ty {
            Type::Bool => "bool".into(),
            Type::U8 => "u8".into(),
            Type::U16 => "u16".into(),
            Type::U32 => "u32".into(),
            Type::U64 => "u64".into(),
            Type::S8 => "i8".into(),
            Type::S16 => "i16".into(),
            Type::S32 => "i32".into(),
            Type::S64 => "i64".into(),
            Type::F32 => "f32".into(),
            Type::F64 => "f64".into(),
            Type::Char => "char".into(),
            Type::String => "String".into(),
            Type::ErrorContext => "wasmtime::component::ErrorContext".into(),
            Type::Id(id) => {
                let def = &self.resolve.types[*id];
                if matches!(def.kind, TypeDefKind::Resource | TypeDefKind::Handle(_)) {
                    "wasmtime::component::ResourceAny".into()
                } else if let TypeDefKind::Type(inner) = &def.kind {
                    self.rust_type(inner, depth + 1)?
                } else if def.name.is_some() {
                    self.named(*id)?.1
                } else {
                    match &def.kind {
                        TypeDefKind::Type(ty) => self.rust_type(ty, depth + 1)?,
                        TypeDefKind::List(ty) => format!("Vec<{}>", self.rust_type(ty, depth + 1)?),
                        TypeDefKind::Option(ty) => {
                            format!("Option<{}>", self.rust_type(ty, depth + 1)?)
                        }
                        TypeDefKind::Result(result) => format!(
                            "Result<{},{}>",
                            self.rust_optional(result.ok.as_ref())?,
                            self.rust_optional(result.err.as_ref())?
                        ),
                        TypeDefKind::Tuple(tuple) => format!(
                            "({})",
                            tuple
                                .types
                                .iter()
                                .map(|ty| Ok(format!("{},", self.rust_type(ty, depth + 1)?)))
                                .collect::<Result<Vec<_>>>()?
                                .join("")
                        ),
                        _ => bail!("unsupported-go-type: {:?}", def.kind),
                    }
                }
            }
        })
    }
    fn rust_optional(&self, ty: Option<&Type>) -> Result<String> {
        ty.map(|ty| self.rust_type(ty, 0))
            .unwrap_or_else(|| Ok("()".into()))
    }
    fn go_optional(&mut self, ty: Option<&Type>) -> Result<String> {
        ty.map(|ty| self.go_type(ty))
            .unwrap_or_else(|| Ok("Unit".into()))
    }
    fn go_type(&mut self, ty: &Type) -> Result<String> {
        Ok(match ty {
            Type::Bool => "bool".into(),
            Type::U8 => "uint8".into(),
            Type::U16 => "uint16".into(),
            Type::U32 => "uint32".into(),
            Type::U64 => "uint64".into(),
            Type::S8 => "int8".into(),
            Type::S16 => "int16".into(),
            Type::S32 => "int32".into(),
            Type::S64 => "int64".into(),
            Type::F32 => "Float32".into(),
            Type::F64 => "Float64".into(),
            Type::Char => "Char".into(),
            Type::String => "string".into(),
            Type::ErrorContext => "*ErrorContext".into(),
            Type::Id(id) => {
                self.declare(*id)?;
                if self.resolve.types[*id].name.is_some() {
                    self.named(*id)?.0
                } else {
                    self.go_definition(&self.resolve.types[*id].kind.clone())?
                }
            }
        })
    }
    fn go_definition(&mut self, kind: &TypeDefKind) -> Result<String> {
        Ok(match kind {
            TypeDefKind::Type(ty) => self.go_type(ty)?,
            TypeDefKind::Handle(Handle::Own(id) | Handle::Borrow(id)) => {
                let id = self.resource_id(*id)?;
                self.declare(id)?;
                format!("*{}", self.named(id)?.0)
            }
            TypeDefKind::List(ty) => format!("List[{}]", self.go_type(ty)?),
            TypeDefKind::Option(ty) => format!("Option[{}]", self.go_type(ty)?),
            TypeDefKind::Result(result) => format!(
                "Result[{},{}]",
                self.go_optional(result.ok.as_ref())?,
                self.go_optional(result.err.as_ref())?
            ),
            TypeDefKind::Tuple(tuple) => {
                ensure!(tuple.types.len() <= 16, "Go tuple length limit");
                if tuple.types.is_empty() {
                    "Unit".into()
                } else {
                    self.tuples.insert(tuple.types.len());
                    format!(
                        "Tuple{}[{}]",
                        tuple.types.len(),
                        tuple
                            .types
                            .iter()
                            .map(|ty| self.go_type(ty))
                            .collect::<Result<Vec<_>>>()?
                            .join(",")
                    )
                }
            }
            _ => bail!("Go aggregate type must be named"),
        })
    }
    fn decode(&self, ty: &Type, value: &str) -> Result<String> {
        Ok(match ty {
            Type::Id(id) => format!("decode_{}(client,{value})?", id.index()),
            Type::ErrorContext => format!("client.decode_error_context({value})?"),
            Type::F32 => format!("float({value})? as f32"),
            Type::F64 => format!("float({value})?"),
            _ => format!("scalar::<{}>({value})?", self.rust_type(ty, 0)?),
        })
    }
    fn encode(&self, ty: &Type, value: &str) -> Result<String> {
        Ok(match ty {
            Type::Id(id) => format!("encode_{}(client,{value})?", id.index()),
            Type::ErrorContext => bail!("unsupported-go-nested-error-context"),
            Type::F32 | Type::F64 => format!("float_value(*({value}) as f64)"),
            _ => format!("json!({value})"),
        })
    }
    fn decode_optional(&self, ty: Option<&Type>, value: &str) -> Result<String> {
        ty.map(|ty| self.decode(ty, value))
            .unwrap_or_else(|| Ok(format!("scalar::<()>({value})?")))
    }
    fn encode_optional(&self, ty: Option<&Type>, value: &str) -> Result<String> {
        ty.map(|ty| self.encode(ty, value))
            .unwrap_or_else(|| Ok("Value::Null".into()))
    }
    fn declare(&mut self, id: TypeId) -> Result<()> {
        if self.seen.contains(&id) {
            return Ok(());
        }
        ensure!(
            self.active.len() < 64 && self.active.insert(id),
            "Go recursive or excessive type graph"
        );
        let def = self.resolve.types[id].clone();
        let named = if def.name.is_some() {
            Some(self.named(id)?)
        } else {
            None
        };
        if let Some((name, _)) = &named {
            ensure!(
                self.names.insert(name.clone()),
                "Go generated type name collision: {name}"
            );
        }
        let mut declaration = String::new();
        let (decode, encode) = match &def.kind {
            TypeDefKind::Resource => {
                let (go, _) = named.as_ref().context("Go resource must be named")?;
                let kind = id.index() + 1;
                declaration.push_str(&format!("type {go} struct {{state *resourceState}}\nfunc(value *{go}) wispResource()*resourceState{{if value==nil{{return nil}};return value.state}}\nfunc(value *{go}) MarshalJSON()([]byte,error){{if value==nil||value.state==nil{{return nil,fmt.Errorf(\"resource is closed\")}};return value.state.marshal()}}\nfunc(value *{go}) UnmarshalJSON(data []byte)error{{state,err:=decodeResource(data,{kind});if err==nil{{value.state=state}};return err}}\nfunc(value *{go}) Close()error{{if value==nil||value.state==nil{{return fmt.Errorf(\"resource is closed\")}};return value.state.close()}}\nfunc(value *{go}) IsClosed()bool{{return value==nil||value.state==nil||value.state.isClosed()}}\n"));
                (
                    format!("client.decode_resource(value,{kind},true)?"),
                    format!("client.encode_resource(*value,{kind})?"),
                )
            }
            TypeDefKind::Handle(handle) => {
                let (resource, own) = match handle {
                    Handle::Own(id) => (*id, true),
                    Handle::Borrow(id) => (*id, false),
                };
                let resource = self.resource_id(resource)?;
                self.declare(resource)?;
                let kind = resource.index() + 1;
                (
                    format!("client.decode_resource(value,{kind},{own})?"),
                    format!("client.encode_resource(*value,{kind})?"),
                )
            }
            TypeDefKind::Record(record) => {
                let (go, rust) = named.as_ref().context("Go record must be named")?;
                declaration.push_str(&format!("type {go} struct {{\n"));
                let mut fields = Vec::new();
                let mut encoded = Vec::new();
                let mut go_fields = BTreeSet::new();
                for field in &record.fields {
                    let name = field.name.to_upper_camel_case();
                    ensure!(go_fields.insert(name.clone()), "Go field name collision");
                    declaration.push_str(&format!(
                        "{name} {} `json:{:?}`\n",
                        self.go_type(&field.ty)?,
                        field.name
                    ));
                    fields.push(format!(
                        "{}: {},",
                        ident(&field.name),
                        self.decode(&field.ty, &format!("field(&mut fields,{:?})?", field.name))?
                    ));
                    encoded.push(format!(
                        "{:?}: {}",
                        field.name,
                        self.encode(&field.ty, &format!("&value.{}", ident(&field.name)))?
                    ));
                }
                declaration.push_str("}\n");
                (
                    format!(
                        "{{let mut fields=object(value)?;let result={rust}{{{}}};finished(fields)?;result}}",
                        fields.join("")
                    ),
                    format!("json!({{{}}})", encoded.join(",")),
                )
            }
            TypeDefKind::Enum(enumeration) => {
                let (go, rust) = named.as_ref().context("Go enum must be named")?;
                declaration.push_str(&format!("type {go} string\nconst (\n"));
                let mut decoded = Vec::new();
                let mut encoded = Vec::new();
                let mut cases = BTreeSet::new();
                for case in &enumeration.cases {
                    let name = case.name.to_upper_camel_case();
                    ensure!(cases.insert(name.clone()), "Go enum case collision");
                    declaration.push_str(&format!("{go}{name} {go} = {:?}\n", case.name));
                    decoded.push(format!(
                        "{:?}=>{rust}::{},",
                        case.name,
                        rust_case(&case.name)
                    ));
                    encoded.push(format!(
                        "{rust}::{}=>json!({:?}),",
                        rust_case(&case.name),
                        case.name
                    ));
                }
                declaration.push_str(")\n");
                (
                    format!(
                        "match scalar::<String>(value)?.as_str(){{{} _=>return Err(\"invalid enum case\".into())}}",
                        decoded.join("")
                    ),
                    format!("match value{{{}}}", encoded.join("")),
                )
            }
            TypeDefKind::Flags(flags) => {
                let (go, rust) = named.as_ref().context("Go flags must be named")?;
                ensure!(flags.flags.len() <= 64, "Go flags support at most 64 names");
                declaration.push_str(&format!("type {go} uint64\nconst (\n"));
                let mut decoded = String::new();
                let mut encoded = String::new();
                let mut cases = BTreeSet::new();
                for (index, flag) in flags.flags.iter().enumerate() {
                    let name = flag.name.to_upper_camel_case();
                    ensure!(cases.insert(name.clone()), "Go flag name collision");
                    declaration.push_str(&format!("{go}{name} {go} = 1 << {index}\n"));
                    let constant = flag.name.to_shouty_snake_case();
                    decoded.push_str(&format!(
                        "if bits & (1u64<<{index})!=0{{result|={rust}::{constant};}}"
                    ));
                    encoded.push_str(&format!(
                        "if value.contains({rust}::{constant}){{bits|=1u64<<{index};}}"
                    ));
                }
                declaration.push_str(")\n");
                let mask = if flags.flags.len() == 64 {
                    u64::MAX
                } else {
                    (1u64 << flags.flags.len()) - 1
                };
                (
                    format!(
                        "{{let bits=scalar::<u64>(value)?;if bits & !{mask}u64 != 0{{return Err(\"unknown flag bits\".into())}}let mut result={rust}::empty();{decoded}result}}"
                    ),
                    format!("{{let mut bits=0u64;{encoded}json!(bits)}}"),
                )
            }
            TypeDefKind::Variant(variant) => {
                let (go, rust) = named.as_ref().context("Go variant must be named")?;
                declaration.push_str(&format!("type {go}Tag string\nconst (\n"));
                let mut fields = String::new();
                let mut methods = String::new();
                let mut marshals = String::new();
                let mut unmarshals = String::new();
                let mut visitors = String::new();
                let mut decoded = String::new();
                let mut encoded = String::new();
                let mut cases = BTreeSet::new();
                for case in &variant.cases {
                    let name = case.name.to_upper_camel_case();
                    ensure!(
                        cases.insert(name.clone()) && name != "Tag",
                        "Go variant case collision"
                    );
                    declaration.push_str(&format!("{go}{name}Tag {go}Tag = {:?}\n", case.name));
                    let rc = rust_case(&case.name);
                    if let Some(ty) = &case.ty {
                        let gt = self.go_type(ty)?;
                        let field = format!("payload{}", name);
                        visitors.push_str(&format!("case {go}{name}Tag:if value.{field}==nil{{return fmt.Errorf(\"missing variant payload\")}};return visitValue(*value.{field},visit)\n"));
                        fields.push_str(&format!("{field} *{gt}\n"));
                        methods.push_str(&format!("func New{go}{name}(value {gt}) {go}{{return {go}{{tag:{go}{name}Tag,{field}:&value}}}}\nfunc(value {go}) {name}()({gt},bool){{var zero {gt};if value.tag!={go}{name}Tag || value.{field}==nil{{return zero,false}};return *value.{field},true}}\n"));
                        marshals.push_str(&format!("case {go}{name}Tag:if value.{field}==nil{{return nil,fmt.Errorf(\"missing variant payload\")}};return json.Marshal(map[string]any{{\"tag\":value.tag,\"value\":*value.{field}}})\n"));
                        unmarshals.push_str(&format!("case {go}{name}Tag:var decoded {gt};if err:=json.Unmarshal(payload,&decoded);err!=nil{{return err}};*value=New{go}{name}(decoded);return nil\n"));
                        decoded.push_str(&format!(
                            "{:?}=>{rust}::{rc}({}),",
                            case.name,
                            self.decode(ty, "payload")?
                        ));
                        encoded.push_str(&format!(
                            "{rust}::{rc}(payload)=>json!({{\"tag\":{:?},\"value\":{}}}),",
                            case.name,
                            self.encode(ty, "payload")?
                        ));
                    } else {
                        visitors.push_str(&format!("case {go}{name}Tag:return nil\n"));
                        methods.push_str(&format!(
                            "func New{go}{name}() {go}{{return {go}{{tag:{go}{name}Tag}}}}\n"
                        ));
                        marshals.push_str(&format!("case {go}{name}Tag:return json.Marshal(map[string]any{{\"tag\":value.tag,\"value\":nil}})\n"));
                        unmarshals.push_str(&format!("case {go}{name}Tag:var unit Unit;if err:=json.Unmarshal(payload,&unit);err!=nil{{return err}};*value=New{go}{name}();return nil\n"));
                        decoded.push_str(&format!(
                            "{:?}=>{{scalar::<()>(payload)?;{rust}::{rc}}},",
                            case.name
                        ));
                        encoded.push_str(&format!(
                            "{rust}::{rc}=>json!({{\"tag\":{:?},\"value\":null}}),",
                            case.name
                        ));
                    }
                }
                declaration.push_str(&format!(")\ntype {go} struct {{tag {go}Tag\n{fields}}}\nfunc(value {go}) Tag(){go}Tag{{return value.tag}}\n{methods}\nfunc(value {go}) MarshalJSON()([]byte,error){{switch value.tag{{{marshals}}};return nil,fmt.Errorf(\"invalid variant tag\")}}\nfunc(value *{go}) UnmarshalJSON(data []byte)error{{var fields map[string]json.RawMessage;if err:=json.Unmarshal(data,&fields);err!=nil{{return err}};if len(fields)!=2{{return fmt.Errorf(\"invalid variant\")}};var tag {go}Tag;if err:=json.Unmarshal(fields[\"tag\"],&tag);err!=nil{{return err}};payload,ok:=fields[\"value\"];if !ok{{return fmt.Errorf(\"missing variant value\")}};switch tag{{{unmarshals}}};return fmt.Errorf(\"invalid variant tag\")}}\n"));
                declaration.push_str(&format!("func(value {go}) visitWisp(visit func(*resourceState)error)error{{switch value.tag{{{visitors}}};return fmt.Errorf(\"invalid variant tag\")}}\n"));
                (
                    format!(
                        "{{let mut fields=object(value)?;let tag=scalar::<String>(field(&mut fields,\"tag\")?)?;let payload=field(&mut fields,\"value\")?;finished(fields)?;match tag.as_str(){{{decoded}_=>return Err(\"invalid variant tag\".into())}}}}"
                    ),
                    format!("match value{{{encoded}}}"),
                )
            }
            TypeDefKind::Type(inner) => {
                self.go_type(inner)?;
                (self.decode(inner, "value")?, self.encode(inner, "value")?)
            }
            TypeDefKind::List(inner) => {
                self.go_type(inner)?;
                (
                    format!(
                        "array(value)?.into_iter().map(|item|Ok({})).collect::<WireResult<Vec<_>>>()?",
                        self.decode(inner, "item")?
                    ),
                    format!(
                        "Value::Array(value.iter().map(|item|Ok({})).collect::<WireResult<Vec<_>>>()?)",
                        self.encode(inner, "item")?
                    ),
                )
            }
            TypeDefKind::Option(inner) => {
                self.go_type(inner)?;
                (
                    format!(
                        "match value{{Value::Null=>None,other=>{{let mut fields=object(other)?;let item=field(&mut fields,\"some\")?;finished(fields)?;Some({})}}}}",
                        self.decode(inner, "item")?
                    ),
                    format!(
                        "match value{{None=>Value::Null,Some(item)=>json!({{\"some\":{}}})}}",
                        self.encode(inner, "item")?
                    ),
                )
            }
            TypeDefKind::Result(result) => {
                self.go_optional(result.ok.as_ref())?;
                self.go_optional(result.err.as_ref())?;
                (
                    format!(
                        "{{let mut fields=object(value)?;if fields.len()!=1{{return Err(\"invalid result\".into())}}if let Some(item)=fields.remove(\"ok\"){{Ok({})}}else if let Some(item)=fields.remove(\"err\"){{Err({})}}else{{return Err(\"invalid result branch\".into())}}}}",
                        self.decode_optional(result.ok.as_ref(), "item")?,
                        self.decode_optional(result.err.as_ref(), "item")?
                    ),
                    format!(
                        "match value{{Ok(item)=>json!({{\"ok\":{}}}),Err(item)=>json!({{\"err\":{}}})}}",
                        self.encode_optional(result.ok.as_ref(), "item")?,
                        self.encode_optional(result.err.as_ref(), "item")?
                    ),
                )
            }
            TypeDefKind::Tuple(tuple) => {
                self.go_definition(&def.kind)?;
                if tuple.types.is_empty() {
                    ("scalar::<()>(value)?".into(), "Value::Null".into())
                } else {
                    let decoded = tuple
                        .types
                        .iter()
                        .map(|ty| Ok(format!("{},", self.decode(ty, "items.next().unwrap()")?)))
                        .collect::<Result<Vec<_>>>()?
                        .join("");
                    let encoded = tuple
                        .types
                        .iter()
                        .enumerate()
                        .map(|(index, ty)| self.encode(ty, &format!("&value.{index}")))
                        .collect::<Result<Vec<_>>>()?
                        .join(",");
                    (
                        format!(
                            "{{let items=array(value)?;if items.len()!={}{{return Err(\"tuple length mismatch\".into())}}let mut items=items.into_iter();({decoded})}}",
                            tuple.types.len()
                        ),
                        format!("Value::Array(vec![{encoded}])"),
                    )
                }
            }
            _ => bail!("unsupported-go-type: {:?}", def.kind),
        };
        if declaration.is_empty() {
            if let Some((go, _)) = &named {
                declaration = format!("type {go} = {}\n", self.go_definition(&def.kind)?);
            }
        }
        self.types.push_str(&declaration);
        let rust = self.rust_type(&Type::Id(id), 0)?;
        self.codecs.push_str(&format!("fn decode_{}(client:&mut Client,value:Value)->WireResult<{rust}>{{Ok({decode})}}\nfn encode_{}(client:&mut Client,value:&{rust})->WireResult<Value>{{Ok({encode})}}\n",id.index(),id.index()));
        self.active.remove(&id);
        self.seen.insert(id);
        Ok(())
    }
}

fn tuple_source(count: usize) -> String {
    let types = (0..count)
        .map(|i| format!("T{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let fields = (0..count)
        .map(|i| format!("V{i} T{i}\n"))
        .collect::<Vec<_>>()
        .join("");
    let values = (0..count)
        .map(|i| format!("value.V{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let decode = (0..count)
        .map(|i| format!("if err:=json.Unmarshal(items[{i}],&value.V{i});err!=nil{{return err}}; "))
        .collect::<Vec<_>>()
        .join("");
    format!(
        "type Tuple{count}[{types} any] struct {{{fields}}}\nfunc(value Tuple{count}[{types}])MarshalJSON()([]byte,error){{return json.Marshal([]any{{{values}}})}}\nfunc(value *Tuple{count}[{types}])UnmarshalJSON(data []byte)error{{var items []json.RawMessage;if err:=json.Unmarshal(data,&items);err!=nil{{return err}};if len(items)!={count}{{return fmt.Errorf(\"tuple length mismatch\")}};{decode}return nil}}\n"
    )
}
fn result_type<'a>(
    resolve: &'a Resolve,
    ty: &Type,
    depth: usize,
) -> Result<Option<&'a wit_parser::Result_>> {
    ensure!(depth < 64, "Go result type depth limit");
    match ty {
        Type::Id(id) => match &resolve.types[*id].kind {
            TypeDefKind::Result(result) => Ok(Some(result)),
            TypeDefKind::Type(inner) => result_type(resolve, inner, depth + 1),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

pub fn sources(resolve: &Resolve, world: WorldId, component_hash: &str) -> Result<Sources> {
    validate_type_expansion(resolve, world)?;
    ensure!(
        component_hash.len() == 64 && component_hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid Go Component hash"
    );
    let prefix = format!("wisp_go_{}", &component_hash[..16]);
    let target_world = &resolve.worlds[world];
    let wasi = crate::library_wasi::enabled(resolve, world)?;
    let pure = crate::library_wasi::pure_browser_host(resolve, world)?;
    let reactor = crate::library_wasi::reactor_host(resolve, world)?;
    let asynchronous = target_world
        .exports
        .values()
        .filter_map(|item| match item {
            WorldItem::Interface { id, .. } => Some(&resolve.interfaces[*id]),
            _ => None,
        })
        .flat_map(|interface| interface.functions.values())
        .any(|function| {
            matches!(
                function.kind,
                FunctionKind::AsyncFreestanding
                    | FunctionKind::AsyncMethod(_)
                    | FunctionKind::AsyncStatic(_)
            ) || function
                .result
                .is_some_and(|ty| reader_type(resolve, ty, 0).ok().flatten().is_some())
        });
    let error_context = target_world
        .exports
        .values()
        .filter_map(|item| match item {
            WorldItem::Interface { id, .. } => Some(&resolve.interfaces[*id]),
            _ => None,
        })
        .flat_map(|interface| interface.functions.values())
        .any(|function| {
            function
                .params
                .iter()
                .map(|p| p.ty)
                .chain(function.result)
                .any(|ty| is_error_context(resolve, ty, 0).unwrap_or(false))
        });
    ensure!(
        target_world.exports.len() <= 64 && resolve.types.len() <= 4096,
        "Go contract size limit"
    );
    let mut generator = Generator {
        resolve,
        seen: BTreeSet::new(),
        active: BTreeSet::new(),
        names: BTreeSet::new(),
        tuples: BTreeSet::new(),
        types: String::new(),
        codecs: String::new(),
    };
    let binding = target_world.name.to_upper_camel_case();
    let mut rust = format!(
        "#![allow(dead_code,unused_variables)]\nmod bindings {{wasmtime::component::bindgen!({{path:\"wit/host.wit\",world:{:?}}});}}\n{}\n{}\n{}\nstruct Client {{store:wasmtime::Store<{store_type}>,bindings:bindings::{binding},poisoned:bool,owner:u64,next_resource:u64,resources:std::collections::BTreeMap<u64,ResourceEntry>,resource_uses:std::collections::BTreeMap<u64,bool>,consumed:Vec<u64>,next_reader:u64,readers:std::collections::BTreeMap<u64,NativeReader>,next_context:u64,contexts:std::collections::BTreeMap<u64,wasmtime::component::ErrorContext>,consumed_contexts:Vec<u64>,error_context_lifts:usize}}\n",
        target_world.name,
        include_str!("../assets/templates/library-go-shim.rs"),
        include_str!("../assets/templates/library-go-async.rs"),
        include_str!("../assets/templates/library-go-error-context.rs"),
        store_type = if wasi { "WasiState" } else { "()" }
    );
    if wasi {
        rust.push_str(crate::library_wasi::STATE);
    }
    if pure {
        rust.push_str(crate::library_wasi::PURE_LINKER);
    }
    rust.push_str(&format!(r#"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_open(error:*mut Buffer)->usize {{
    let result=catch_unwind(AssertUnwindSafe(||->WireResult<Client>{{
        let engine={engine_init};
        let component=wasmtime::component::Component::new(&engine,include_bytes!("../component.wasm")).map_err(|e|format!("{{e:#}}"))?;
        let mut store=wasmtime::Store::new(&engine,{store_value});
        let mut linker=wasmtime::component::Linker::new(&engine);
        {linker_init}
        let bindings=bindings::{binding}::instantiate(&mut store,&component,&linker).map_err(|e|format!("{{e:#}}"))?;
        Ok(Client{{store,bindings,poisoned:false,owner:NEXT_OWNER.fetch_add(1,std::sync::atomic::Ordering::Relaxed),next_resource:1,resources:Default::default(),resource_uses:Default::default(),consumed:Vec::new(),next_reader:1,readers:Default::default(),next_context:1,contexts:Default::default(),consumed_contexts:Vec::new(),error_context_lifts:0}})
    }}));
    match result{{Ok(Ok(client))=>Box::into_raw(Box::new(client)) as usize,other=>{{
        let message=match other{{Ok(Err(error))=>error,_=>"native initialization panic".into()}};
        if !error.is_null(){{unsafe{{error.write(buffer(json!({{"runtime":message}})))}}}};0
    }}}}
}}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_close(handle:usize){{if handle!=0{{let _=catch_unwind(AssertUnwindSafe(||unsafe{{drop(Box::from_raw(handle as *mut Client))}}));}}}}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_free(value:Buffer){{if !value.data.is_null(){{unsafe{{drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(value.data,value.len)))}}}}}}
"#, store_value = if pure || reactor { crate::library_wasi::PURE_STORE } else if wasi { crate::library_wasi::STORE } else { "()" },
        engine_init = if asynchronous || error_context { "{let mut config=wasmtime::Config::new();config.wasm_component_model(true).wasm_component_model_async(true).wasm_component_model_error_context(true).wasm_component_model_more_async_builtins(true).wasm_component_model_async_stackful(true).concurrency_support(true);wasmtime::Engine::new(&config).map_err(|e|format!(\"{e:#}\"))?}" } else { "wasmtime::Engine::default()" },
        linker_init = if pure { "pure_wasi_linker(&mut linker).map_err(|e|format!(\"{e:#}\"))?;" } else if wasi { "(|| -> wasmtime::Result<()> { wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?; wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker)?; Ok(()) })().map_err(|e|format!(\"{e:#}\"))?;" } else { "" }));
    let mut header = format!(
        "#include <stdint.h>\n#include <stddef.h>\ntypedef struct {{uint8_t *data;size_t len;}} WispBuffer;\nuintptr_t {prefix}_open(WispBuffer *error);\nvoid {prefix}_close(uintptr_t handle);\nvoid {prefix}_free(WispBuffer value);\n"
    );
    let mut functions = String::new();
    let mut rust_functions = String::new();
    let mut interfaces = BTreeSet::from(["Close".to_owned()]);
    let mut function_count = 0;
    for item in target_world.exports.values() {
        let (view, name, accessor, exported, direct) = match item {
            WorldItem::Interface { id, .. } => {
                let interface = &resolve.interfaces[*id];
                let name = interface.name.as_deref().context("Go interface name")?;
                let view = name.to_upper_camel_case();
                ensure!(
                    interfaces.insert(view.clone()),
                    "Go interface name collision: {name}"
                );
                let package = &resolve.packages[interface.package.context("Go package")?].name;
                let accessor = format!(
                    "{}_{}_{}",
                    package.namespace.to_snake_case(),
                    package.name.to_snake_case(),
                    name.to_snake_case()
                );
                for id in interface.types.values() {
                    generator.declare(*id)?;
                }
                functions.push_str(&format!("type {view} struct {{client *Client}}\nfunc(client *Client) {view}()*{view}{{return &{view}{{client:client}}}}\n"));
                (
                    view,
                    name.to_owned(),
                    accessor,
                    interface.functions.values().collect::<Vec<_>>(),
                    false,
                )
            }
            WorldItem::Function(function) => (
                "Client".to_owned(),
                "world".to_owned(),
                String::new(),
                vec![function],
                true,
            ),
            _ => bail!("unsupported-go-export: named interfaces or functions are required"),
        };
        let mut methods = BTreeSet::new();
        for function in exported {
            for ty in function.params.iter().map(|p| p.ty).chain(function.result) {
                if contains_error_context(resolve, ty, &mut BTreeSet::new(), 0)?
                    && !matches!(ty, Type::ErrorContext)
                {
                    bail!("unsupported-go-nested-error-context: {}", function.name)
                }
            }
            function_count += 1;
            ensure!(function_count <= 1024, "Go function count limit");
            let member = function.name.rsplit('.').next().context("Go method name")?;
            let (receiver, method, client_expression, skip, callee) = match function.kind {
                FunctionKind::Freestanding | FunctionKind::AsyncFreestanding => (
                    view.clone(),
                    function.name.to_upper_camel_case(),
                    if direct { "view" } else { "view.client" }.to_owned(),
                    0,
                    if direct {
                        format!("client.bindings.call_{}", function.name.to_snake_case())
                    } else {
                        format!(
                            "client.bindings.{accessor}().func_{}()",
                            function.name.to_snake_case()
                        )
                    },
                ),
                FunctionKind::Constructor(resource) => {
                    let resource_name = resolve.types[resource]
                        .name
                        .as_deref()
                        .context("Go resource name")?;
                    (
                        view.clone(),
                        format!("New{}", resource_name.to_upper_camel_case()),
                        "view.client".to_owned(),
                        0,
                        format!(
                            "client.bindings.{accessor}().{}().func_constructor()",
                            ident(resource_name)
                        ),
                    )
                }
                FunctionKind::Static(resource) => {
                    let resource_name = resolve.types[resource]
                        .name
                        .as_deref()
                        .context("Go resource name")?;
                    (
                        view.clone(),
                        format!(
                            "{}{}",
                            resource_name.to_upper_camel_case(),
                            member.to_upper_camel_case()
                        ),
                        "view.client".to_owned(),
                        0,
                        format!(
                            "client.bindings.{accessor}().{}().func_{}()",
                            ident(resource_name),
                            member.to_snake_case()
                        ),
                    )
                }
                FunctionKind::Method(resource) => {
                    let resource_name = resolve.types[resource]
                        .name
                        .as_deref()
                        .context("Go resource name")?;
                    ensure!(
                        !matches!(member.to_upper_camel_case().as_str(), "Close" | "IsClosed"),
                        "Go resource lifecycle method collision"
                    );
                    (
                        generator.named(generator.resource_id(resource)?)?.0,
                        member.to_upper_camel_case(),
                        "view.state.client".to_owned(),
                        1,
                        format!(
                            "client.bindings.{accessor}().{}().func_{}()",
                            ident(resource_name),
                            member.to_snake_case()
                        ),
                    )
                }
                _ => bail!("unsupported-go-async-function"),
            };
            ensure!(
                methods.insert(format!("{receiver}::{method}")),
                "Go function name collision"
            );
            if direct {
                ensure!(
                    interfaces.insert(method.clone()),
                    "Go client method name collision"
                );
            }
            let symbol = format!(
                "{prefix}_{}_{}",
                name.to_snake_case(),
                function.name.to_snake_case()
            );
            let mut go_params = Vec::new();
            let mut go_values = Vec::new();
            let mut decode = String::new();
            let mut rust_types = Vec::new();
            let mut rust_args = Vec::new();
            let mut context_checks = String::new();
            for (index, param) in function.params.iter().enumerate() {
                if is_error_context(resolve, param.ty, 0)? {
                    ensure!(
                        matches!(param.ty, Type::ErrorContext),
                        "unsupported-go-named-error-context-alias"
                    );
                    context_checks.push_str(&format!("if err:=validateContextArgument(client,arg{index});err!=nil{{__CONTEXT_ERROR__}};"));
                }
                let go_type = generator.go_type(&param.ty)?;
                if index >= skip {
                    go_params.push(format!("arg{index} {go_type}"));
                }
                go_values.push(format!(
                    "{:?}:{}",
                    param.name,
                    if index < skip {
                        "view".into()
                    } else {
                        format!("arg{index}")
                    }
                ));
                decode.push_str(&format!(
                    "let arg{index}={};",
                    generator
                        .decode(&param.ty, &format!("field(&mut fields,{:?})?", param.name))?
                ));
                rust_types.push(format!("{},", generator.rust_type(&param.ty, 0)?));
                rust_args.push(format!("arg{index},"));
            }
            let reader = function
                .result
                .map(|ty| reader_type(resolve, ty, 0))
                .transpose()?
                .flatten();
            if let Some(result) = &function.result {
                if reader.is_none() {
                    generator.go_type(result)?;
                }
            }
            if let Some((kind, payload)) = reader {
                ensure!(skip == 0, "unsupported-go-async-resource-method");
                let payload_rust = generator.rust_optional(payload.as_ref())?;
                let payload_go = generator.go_optional(payload.as_ref())?;
                let (native_kind, read_kind, go_kind) = match kind {
                    ReaderKind::Future => ("FutureReader", "future", "Future"),
                    ReaderKind::Stream => ("StreamReader", "stream", "Stream"),
                };
                let call = if matches!(function.kind, FunctionKind::AsyncFreestanding) {
                    format!(
                        "block_on(client.store.run_concurrent(async move |accessor|callee.call_concurrent(&accessor,({})).await)).and_then(|result|result.map_err(|error|format!(\"{{error:#}}\"))).and_then(|result|result.map_err(|error|format!(\"{{error:#}}\")))",
                        rust_args.join("")
                    )
                } else {
                    format!(
                        "{callee}.func().typed::<({}),({native_kind}<{payload_rust}>,)>(&client.store).and_then(|callee|callee.call(&mut client.store,({}))).map_err(|error|format!(\"{{error:#}}\"))",
                        rust_types.join(""),
                        rust_args.join("")
                    )
                };
                let typed_call = if matches!(function.kind, FunctionKind::AsyncFreestanding) {
                    format!(
                        "let callee={callee}.func().typed::<({}),({native_kind}<{payload_rust}>,)>(&client.store).map_err(|error|format!(\"{{error:#}}\"))?;",
                        rust_types.join("")
                    )
                } else {
                    String::new()
                };
                rust_functions.push_str(&format!("#[unsafe(no_mangle)]\npub extern \"C\" fn {symbol}(handle:usize,data:*const u8,len:usize)->Buffer{{invoke(handle,data,len,|client,input|{{let mut fields=object(input)?;{decode}finished(fields)?;client.commit_resources();{typed_call}let returned={call};let (reader,)=match returned{{Ok(value)=>value,Err(error)=>{{client.poisoned=true;return Err(error)}}}};let id=client.add_{read_kind}(reader)?;Ok(json!({{\"value\":id}}))}})}}\n"));
                header.push_str(&format!(
                    "WispBuffer {symbol}(uintptr_t handle,const uint8_t *data,size_t len);\n"
                ));
                let read_symbol = format!("{symbol}_read");
                let payload_encode = generator.encode_optional(payload.as_ref(), "&value")?;
                rust_functions.push_str(&format!("#[unsafe(no_mangle)]\npub extern \"C\" fn {read_symbol}(handle:usize,data:*const u8,len:usize)->Buffer{{invoke(handle,data,len,|client,input|{{let mut fields=object(input)?;let id=scalar::<u64>(field(&mut fields,\"id\")?)?;finished(fields)?;let values=client.read_{read_kind}::<{payload_rust}>(id)?;let mut encoded=Vec::with_capacity(values.len());for value in values{{encoded.push({payload_encode});}}Ok(json!({{\"value\":encoded}}))}})}}\n"));
                header.push_str(&format!(
                    "WispBuffer {read_symbol}(uintptr_t handle,const uint8_t *data,size_t len);\n"
                ));
                let go_read = format!("read{}{}", view, method);
                functions.push_str(&format!("func {go_read}(client *Client,id uint64)([]{payload_go},error){{data,err:=json.Marshal(map[string]uint64{{\"id\":id}});if err!=nil{{return nil,err}};reply:=takeBuffer(C.{read_symbol}(client.handle,(*C.uint8_t)(unsafe.Pointer(&data[0])),C.size_t(len(data))));runtime.KeepAlive(data);values,err:=finishReply[List[{payload_go}],Unit](client,reply,nil);return []{payload_go}(values),err}}\n"));
                functions.push_str(&format!("func(view *{receiver}) {method}({})(*{go_kind}[{payload_go}],error){{if view==nil||view.client==nil{{return nil,ErrClosed}};client:=view.client;client.mu.Lock();defer client.mu.Unlock();if client.handle==0{{return nil,ErrClosed}};arguments:=map[string]any{{{}}};if err:=validateText(arguments);err!=nil{{return nil,err}};if err:=validateArguments(client,arguments);err!=nil{{return nil,err}};data,err:=json.Marshal(arguments);if err!=nil{{return nil,err}};reply:=takeBuffer(C.{symbol}(client.handle,(*C.uint8_t)(unsafe.Pointer(&data[0])),C.size_t(len(data))));runtime.KeepAlive(data);id,err:=finishReply[uint64,Unit](client,reply,arguments);if err!=nil{{return nil,err}};value:=&{go_kind}[{payload_go}]{{client:client,id:id,read:{go_read}}};runtime.SetFinalizer(value,func(value *{go_kind}[{payload_go}]){{_=value.Close()}});return value,nil}}\n",go_params.join(","),go_values.join(",")));
                continue;
            }
            ensure!(
                !matches!(function.kind, FunctionKind::AsyncFreestanding),
                "unsupported-go-async-return"
            );
            let result = function
                .result
                .as_ref()
                .map(|ty| result_type(resolve, ty, 0))
                .transpose()?
                .flatten();
            let (ok, err) = match result {
                Some(result) => (result.ok.as_ref(), result.err.as_ref()),
                None => (function.result.as_ref(), None),
            };
            let go_ok = generator.go_optional(ok)?;
            let go_err = generator.go_optional(err)?;
            let rust_result = generator.rust_optional(function.result.as_ref())?;
            let returns_context = function
                .result
                .is_some_and(|ty| is_error_context(resolve, ty, 0).unwrap_or(false));
            ensure!(
                !returns_context || matches!(function.result, Some(Type::ErrorContext)),
                "unsupported-go-named-error-context-alias"
            );
            let encoded = if let Some(result) = result {
                format!(
                    "match returned{{Ok(value)=>Ok(json!({{\"value\":{}}})),Err(value)=>Ok(json!({{\"guest\":{}}}))}}",
                    generator.encode_optional(result.ok.as_ref(), "&value")?,
                    generator.encode_optional(result.err.as_ref(), "&value")?
                )
            } else {
                if returns_context {
                    "Ok(json!({\"value\":client.encode_error_context(returned)?}))".into()
                } else {
                    format!(
                        "Ok(json!({{\"value\":{}}}))",
                        generator.encode_optional(function.result.as_ref(), "&returned")?
                    )
                }
            };
            let callee_result = if function.result.is_some() {
                format!("({rust_result},)")
            } else {
                "()".into()
            };
            let returned = if function.result.is_some() {
                "let (returned,)=returned;"
            } else {
                "let returned=();"
            };
            let native_call = if direct {
                let args = function
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, param)| {
                        if matches!(param.ty, Type::String) {
                            format!("&arg{index}")
                        } else {
                            format!("arg{index}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{callee}(&mut client.store{comma}{args})",
                    comma = if args.is_empty() { "" } else { "," }
                )
            } else {
                format!(
                    "{callee}.func().typed::<({}),{callee_result}>(&client.store).and_then(|callee|callee.call(&mut client.store,({})))",
                    rust_types.join(""),
                    rust_args.join("")
                )
            };
            rust_functions.push_str(&format!("#[unsafe(no_mangle)]\npub extern \"C\" fn {symbol}(handle:usize,data:*const u8,len:usize)->Buffer{{invoke(handle,data,len,|client,input|{{{capacity}let mut fields=object(input)?;{decode}finished(fields)?;client.commit_resources();let returned={native_call};let returned=match returned{{Ok(value)=>value,Err(error)=>{{client.poisoned=true;return Err(format!(\"{{error:#}}\"))}}}};{returned}{encoded}}})}}\n",returned=if direct { "" } else { returned },capacity=if returns_context{"if client.error_context_lifts>=127{return Err(\"error-context Store lifetime capacity reached (127 lifts)\".into())};"}else{""}));
            header.push_str(&format!(
                "WispBuffer {symbol}(uintptr_t handle,const uint8_t *data,size_t len);\n"
            ));
            let signature = if ok.is_some() {
                format!("({go_ok},error)")
            } else {
                "error".into()
            };
            let error_return = if ok.is_some() {
                "return zero,err"
            } else {
                "return err"
            };
            let closed_return = if ok.is_some() {
                "return zero,ErrClosed"
            } else {
                "return ErrClosed"
            };
            let last = if ok.is_some() {
                format!("return finishReply[{go_ok},{go_err}](client,reply,arguments)")
            } else {
                format!("_,err=finishReply[Unit,{go_err}](client,reply,arguments);return err")
            };
            let receiver_check = if skip == 1 {
                format!("if view==nil||view.state==nil{{{closed_return}}};")
            } else {
                format!("if view==nil{{{closed_return}}};")
            };
            let context_checks = context_checks.replace("__CONTEXT_ERROR__", error_return);
            functions.push_str(&format!("func(view *{receiver}) {method}({}){signature}{{\n{}{receiver_check}client:={client_expression};if client==nil{{{closed_return}}};client.mu.Lock();defer client.mu.Unlock();if client.handle==0{{{closed_return}}};{context_checks}arguments:=map[string]any{{{}}};if err:=validateText(arguments);err!=nil{{{error_return}}};if err:=validateArguments(client,arguments);err!=nil{{{error_return}}};data,err:=json.Marshal(arguments);if err!=nil{{{error_return}}};native:=C.{symbol}(client.handle,(*C.uint8_t)(unsafe.Pointer(&data[0])),C.size_t(len(data)));reply:=takeBuffer(native);runtime.KeepAlive(data);{last}\n}}\n",go_params.join(","),if ok.is_some(){format!("var zero {go_ok};")}else{String::new()},go_values.join(",")));
        }
    }
    rust.push_str(&format!(r#"
#[unsafe(no_mangle)]
pub extern "C" fn {prefix}_resource_drop(handle:usize,data:*const u8,len:usize)->Buffer {{
    invoke(handle,data,len,|client,input|{{
        let mut fields=object(input)?;let id=scalar::<u64>(field(&mut fields,"id")?)?;finished(fields)?;
        let entry=client.resources.remove(&id).ok_or("resource is closed")?;
        entry.handle.resource_drop(&mut client.store).map_err(|error|{{client.poisoned=true;format!("{{error:#}}")}})?;
        Ok(json!({{"value":null}}))
    }})
}}
"#));
    rust.push_str(&format!(r#"
#[unsafe(no_mangle)]
pub extern "C" fn {prefix}_reader_close(handle:usize,data:*const u8,len:usize)->Buffer {{
    invoke(handle,data,len,|client,input|{{let mut fields=object(input)?;let id=scalar::<u64>(field(&mut fields,"id")?)?;finished(fields)?;client.close_reader(id)?;Ok(json!({{"value":null}}))}})
}}
"#));
    rust.push_str(&format!(r#"
#[unsafe(no_mangle)]
pub extern "C" fn {prefix}_context_close(handle:usize,data:*const u8,len:usize)->Buffer {{
    invoke(handle,data,len,|client,input|{{let mut fields=object(input)?;let id=scalar::<u64>(field(&mut fields,"id")?)?;finished(fields)?;client.close_context(id)?;Ok(json!({{"value":null}}))}})
}}
"#));
    header.push_str(&format!(
        "WispBuffer {prefix}_resource_drop(uintptr_t handle,const uint8_t *data,size_t len);\n"
    ));
    header.push_str(&format!(
        "WispBuffer {prefix}_reader_close(uintptr_t handle,const uint8_t *data,size_t len);\n"
    ));
    header.push_str(&format!(
        "WispBuffer {prefix}_context_close(uintptr_t handle,const uint8_t *data,size_t len);\n"
    ));
    rust.push_str(&generator.codecs);
    rust.push_str(&rust_functions);
    let go = format!(
        r#"//go:build cgo && ((linux && amd64) || (darwin && arm64))
package component
/*
#cgo linux,amd64 LDFLAGS: ${{SRCDIR}}/native/linux_amd64/libwisp_go.a -lm -ldl -lpthread
#cgo darwin,arm64 LDFLAGS: ${{SRCDIR}}/native/darwin_arm64/libwisp_go.a -lm -lpthread -framework Security -framework SystemConfiguration -liconv
#include "bridge.h"
*/
import "C"
import ("encoding/json";"errors";"runtime";"sync";"unsafe")
var ErrClosed=errors.New("Component client is closed")
type Client struct{{mu sync.Mutex;handle C.uintptr_t}}
func takeBuffer(value C.WispBuffer)[]byte{{defer C.{prefix}_free(value);return C.GoBytes(unsafe.Pointer(value.data),C.int(value.len))}}
func NewClient()(*Client,error){{var failure C.WispBuffer;handle:=C.{prefix}_open(&failure);if handle==0{{_,err:=decodeReply[Unit,Unit](takeBuffer(failure));return nil,err}};client:=&Client{{handle:handle}};runtime.SetFinalizer(client,func(c *Client){{_ = c.Close()}});return client,nil}}
func(client *Client)Close()error{{if client==nil{{return nil}};client.mu.Lock();defer client.mu.Unlock();if client.handle!=0{{C.{prefix}_close(client.handle);client.handle=0}};runtime.SetFinalizer(client,nil);return nil}}
func(client *Client)dropResource(id uint64)error{{if client.handle==0{{return ErrClosed}};data,err:=json.Marshal(map[string]uint64{{"id":id}});if err!=nil{{return err}};reply:=takeBuffer(C.{prefix}_resource_drop(client.handle,(*C.uint8_t)(unsafe.Pointer(&data[0])),C.size_t(len(data))));runtime.KeepAlive(data);_,err=finishReply[Unit,Unit](client,reply,nil);return err}}
func(client *Client)closeReader(id uint64)error{{if client.handle==0{{return ErrClosed}};data,err:=json.Marshal(map[string]uint64{{"id":id}});if err!=nil{{return err}};reply:=takeBuffer(C.{prefix}_reader_close(client.handle,(*C.uint8_t)(unsafe.Pointer(&data[0])),C.size_t(len(data))));runtime.KeepAlive(data);_,err=finishReply[Unit,Unit](client,reply,nil);return err}}
func(client *Client)closeContext(id uint64)error{{if client.handle==0{{return ErrClosed}};data,err:=json.Marshal(map[string]uint64{{"id":id}});if err!=nil{{return err}};reply:=takeBuffer(C.{prefix}_context_close(client.handle,(*C.uint8_t)(unsafe.Pointer(&data[0])),C.size_t(len(data))));runtime.KeepAlive(data);_,err=finishReply[Unit,Unit](client,reply,nil);return err}}
{async_values}
{functions}
"#,
        async_values = include_str!("../assets/templates/library-go-async.go")
    );
    let mut values = include_str!("../assets/templates/library-go-values.go").to_owned();
    values.push_str(include_str!("../assets/templates/library-go-resources.go"));
    values.push_str(include_str!(
        "../assets/templates/library-go-error-context.go"
    ));
    values.push_str(&generator.types);
    for count in generator.tuples {
        values.push_str(&tuple_source(count));
    }
    Ok(Sources {
        rust,
        go,
        values,
        header,
        prefix,
    })
}

fn run(command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("running {command:?}"))?;
    ensure!(
        output.status.success(),
        "{command:?} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[allow(dead_code)]
pub fn generate(component: &[u8], output: &Path, module: &str, version: &str) -> Result<Value> {
    generate_with_notices(component, output, module, version, &BTreeMap::new())
}

pub fn generate_with_notices(
    component: &[u8],
    output: &Path,
    module: &str,
    version: &str,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<Value> {
    ensure!(!output.try_exists()?, "Go output already exists");
    validate_module(module, version)?;
    let (resolve, world) = decode_component(component)?;
    let hash = digest(component);
    let sources = sources(&resolve, world, &hash)?;
    let rust_version = run(Command::new("rustc")
        .arg("--version")
        .env("RUSTUP_TOOLCHAIN", "1.98.1")
        .env("RUSTUP_AUTO_INSTALL", "0"))?;
    ensure!(
        rust_version.split_whitespace().nth(1) == Some("1.98.1"),
        "Rust 1.98.1 is required"
    );
    let go_version = run(Command::new("go").arg("version"))?;
    ensure!(
        go_version.split_whitespace().nth(2) == Some("go1.27.1"),
        "Go 1.27.1 is required"
    );
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin_arm64",
        ("linux", "x86_64") => "linux_amd64",
        _ => bail!("Go libraries support Linux x86_64 and macOS ARM"),
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    let package = stage.path();
    let shim = package.join("build-shim");
    fs::create_dir_all(shim.join("src"))?;
    fs::create_dir_all(shim.join("wit"))?;
    for (path, bytes) in source_files(component, module, version, notices)? {
        let target = package.join(path);
        fs::create_dir_all(target.parent().context("Go source parent")?)?;
        fs::write(target, bytes)?;
    }
    let name = format!("wisp-go-shim-{}", &hash[..16]);
    let build = match std::env::var_os("WISP_GO_CARGO_TARGET_DIR") {
        Some(path) => std::path::PathBuf::from(path),
        None => std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
            })
            .context("set WISP_GO_CARGO_TARGET_DIR or a user cache directory")?
            .join("wisp/library-go/native-target"),
    };
    let build = if build.is_absolute() {
        build
    } else {
        std::env::current_dir()?.join(build)
    };
    let build = build.join("reproducible-v1");
    fs::create_dir_all(&build)?;
    let build = build.canonicalize()?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cargo"))
        })
        .context("Cargo home is required")?
        .canonicalize()?;
    let wrapper = shim.join("rustc-wrapper.sh");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    let configure = |command: &mut Command| {
        command
            .env("RUSTUP_TOOLCHAIN", "1.98.1")
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("CARGO_NET_OFFLINE", "true")
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .env("CARGO_INCREMENTAL", "0")
            .env("SOURCE_DATE_EPOCH", "0")
            .env("ZERO_AR_DATE", "1")
            .env("CARGO_PROFILE_DEV_OPT_LEVEL", "0")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTC")
            .env("RUSTC_WRAPPER", &wrapper)
            .env("WISP_GO_BUILD_TARGET", &build)
            .env("WISP_GO_CARGO_HOME", &cargo_home)
            .env("WISP_GO_BUILD_SHIM", &shim)
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .current_dir(&shim);
    };
    let mut cargo = Command::new("cargo");
    configure(&mut cargo);
    cargo
        .args(["rustc", "--locked", "--offline", "--target-dir"])
        .arg(&build)
        .arg("--")
        .arg("-Cmetadata=wisp-go-component-library")
        .arg(format!(
            "--remap-path-prefix={}=/wisp/go-shim",
            shim.display()
        ));
    run(&mut cargo)?;
    let native = package.join("native").join(platform);
    fs::create_dir_all(&native)?;
    fs::copy(
        build
            .join("debug")
            .join(format!("lib{}.a", name.replace('-', "_"))),
        native.join("libwisp_go.a"),
    )?;
    let report = json!({"schema":1,"kind":"typed-go-component-library","module":module,"version":version,"platform":platform,"componentSha256":hash,"abiPrefix":sources.prefix,"nativeSha256":digest(&fs::read(native.join("libwisp_go.a"))?),"cargoLockSha256":digest(&fs::read(shim.join("Cargo.lock"))?),"sourceSha256":source_files(component,module,version,notices)?.iter().map(|(path,bytes)|(path.clone(),digest(bytes))).collect::<BTreeMap<_,_>>(),"tools":{"rust":"1.98.1","wasmtime":"50.0.0","go":"1.27.1"}});
    fs::write(
        package.join("go-library.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::create_dir(output)?;
    if let Err(error) = fs::rename(stage.path(), output) {
        let _ = fs::remove_dir(output);
        return Err(error.into());
    };
    Ok(report)
}

pub(crate) fn validate_module_path(module: &str) -> Result<()> {
    ensure!(
        module.len() <= 200
            && module.contains('/')
            && module.split('/').all(|part| !part.is_empty()
                && !matches!(part, "." | "..")
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-._".contains(&b))),
        "invalid Go module path (use lowercase ASCII)"
    );
    ensure!(
        module.split('/').next().unwrap().contains('.'),
        "Go module path requires a domain"
    );
    Ok(())
}
pub(crate) fn validate_module(module: &str, version: &str) -> Result<()> {
    validate_module_path(module)?;
    let parsed = semver::Version::parse(version)?;
    ensure!(
        parsed.to_string() == version && parsed.build.is_empty(),
        "invalid Go module version"
    );
    let suffix = module.rsplit('/').next().unwrap();
    if parsed.major >= 2 {
        ensure!(
            suffix == format!("v{}", parsed.major),
            "Go module major version requires /vN suffix"
        );
    } else {
        ensure!(
            !(suffix.starts_with('v') && suffix[1..].parse::<u64>().is_ok()),
            "Go v0/v1 module must not have /vN suffix"
        );
    }
    Ok(())
}

fn decode_component(component: &[u8]) -> Result<(Resolve, WorldId)> {
    ensure!(
        component.len() <= 16 * 1024 * 1024,
        "Go Component size limit"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(component)?;
    let decoded = std::panic::catch_unwind(|| wit_parser::decoding::decode(component))
        .map_err(|_| anyhow::anyhow!("Go WIT decoder rejected the type graph"))??;
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) = decoded else {
        bail!("expected executable Component")
    };
    Ok((resolve, world))
}

fn format_go(source: &str) -> Result<Vec<u8>> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("gofmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().context("gofmt stdin")?;
    let source = source.as_bytes().to_vec();
    let writer = std::thread::spawn(move || input.write_all(&source));
    let output = child.wait_with_output()?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("gofmt writer panic"))??;
    ensure!(
        output.status.success(),
        "gofmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}

fn source_files(
    component: &[u8],
    module: &str,
    version: &str,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    validate_module(module, version)?;
    let (mut resolve, world) = decode_component(component)?;
    let hash = digest(component);
    let sources = sources(&resolve, world, &hash)?;
    resolve.worlds[world].imports.clear();
    let owner = resolve.worlds[world].package.context("Go WIT owner")?;
    let others = resolve
        .packages
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != owner)
        .collect::<Vec<_>>();
    let mut printer = wit_component::WitPrinter::default();
    printer.print(&resolve, owner, &others)?;
    let name = format!("wisp-go-shim-{}", &hash[..16]);
    let mut files = BTreeMap::from([
        ("build-shim/rustc-wrapper.sh".into(), include_bytes!("../assets/templates/library-go-rustc-wrapper.sh").to_vec()),
        ("WISP-GO-LICENSES.json".into(), licenses::bundle()?),
        ("build-shim/Cargo.lock".into(), include_str!("../assets/library-go-Cargo.lock")
            .replace("name = \"wisp-go-shim-seed\"", &format!("name = {name:?}")).into_bytes()),
        ("go.mod".into(), format!("module {module}\n\ngo 1.27.0\n").into_bytes()),
        ("bridge.go".into(), format_go(&sources.go)?),
        ("values.go".into(), format_go(&sources.values)?),
        ("bridge.h".into(), sources.header.into_bytes()),
        ("build-shim/src/lib.rs".into(), sources.rust.into_bytes()),
        ("build-shim/wit/host.wit".into(), printer.output.to_string().into_bytes()),
        ("build-shim/component.wasm".into(), component.to_vec()),
        ("build-shim/Cargo.toml".into(), format!(r#"[package]
name = {name:?}
version = "0.1.0"
edition = "2024"
[workspace]
[lib]
crate-type = ["staticlib"]
[dependencies]
wasmtime = {{version="=50.0.0",default-features=false,features=["component-model","component-model-async","cranelift","runtime","std"]}}
serde = "=1.0.229"
serde_json = "=1.0.151"
wasmtime-wasi = {{version="=50.0.0",default-features=false,features=["p2"]}}
wasmtime-wasi-http = {{version="=50.0.0",default-features=false,features=["p2","default-send-request"]}}
[profile.dev]
debug = 0
"#).into_bytes()),
    ]);
    ensure!(notices.len() <= 17, "Go notice count limit");
    let mut size = 0;
    for (path, bytes) in notices {
        let leaf = path.strip_prefix("notices/").unwrap_or(path);
        ensure!(
            (path == "WISP-NOTICES.txt" || path.starts_with("notices/"))
                && !leaf.is_empty()
                && leaf != "."
                && leaf != ".."
                && leaf.len() <= 128
                && leaf
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && !leaf.ends_with(".go"),
            "unsafe Go notice path"
        );
        size += bytes.len();
        ensure!(
            bytes.len() <= 256 * 1024 && size <= 1024 * 1024,
            "Go notice size limit"
        );
        ensure!(
            files.insert(path.clone(), bytes.clone()).is_none(),
            "Go notice path collision"
        );
    }
    Ok(files)
}

/// Regenerate contract sources, then rebuild and compare the native archive.
/// Verification requires the archive platform and the pinned local compiler/cache.
#[allow(dead_code)]
pub fn verify_output(
    component: &[u8],
    module: &str,
    version: &str,
    output: &Path,
) -> Result<Value> {
    verify_output_with_notices(component, module, version, output, &BTreeMap::new())
}

pub fn verify_output_with_notices(
    component: &[u8],
    module: &str,
    version: &str,
    output: &Path,
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<Value> {
    let expected = source_files(component, module, version, notices)?;
    let mut files = BTreeMap::new();
    package_files(output, output, &mut files, &mut (500 * 1024 * 1024))?;
    let read = |path: &str| {
        files
            .get(path)
            .map(Vec::as_slice)
            .with_context(|| format!("Go package file missing: {path}"))
    };
    let report: Value = serde_json::from_slice(read("go-library.json")?)?;
    ensure!(
        report["schema"] == 1
            && report["kind"] == "typed-go-component-library"
            && report["module"] == module
            && report["version"] == version
            && report["componentSha256"] == digest(component),
        "Go package provenance mismatch"
    );
    for (path, bytes) in &expected {
        ensure!(read(path)? == bytes, "Go generated source mismatch: {path}");
    }
    let source_hashes = expected
        .iter()
        .map(|(path, bytes)| (path.clone(), digest(bytes)))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        report["sourceSha256"] == serde_json::to_value(source_hashes)?,
        "Go generated source hashes mismatch"
    );
    let platform = report["platform"].as_str().context("Go platform missing")?;
    ensure!(
        matches!(platform, "darwin_arm64" | "linux_amd64"),
        "unsupported Go native platform"
    );
    ensure!(
        report["nativeSha256"] == digest(read(&format!("native/{platform}/libwisp_go.a"))?),
        "Go native artifact hash mismatch"
    );
    ensure!(
        report["cargoLockSha256"] == digest(read("build-shim/Cargo.lock")?),
        "Go Cargo lock hash mismatch"
    );
    let current_platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin_arm64",
        ("linux", "x86_64") => "linux_amd64",
        _ => bail!("unsupported-go-verification-platform"),
    };
    ensure!(
        platform == current_platform,
        "unsupported-go-verification-platform: native archive requires its build platform"
    );
    let mut allowed = expected.keys().cloned().collect::<BTreeSet<_>>();
    allowed.extend([
        "go-library.json".to_owned(),
        format!("native/{platform}/libwisp_go.a"),
    ]);
    ensure!(
        files.keys().cloned().collect::<BTreeSet<_>>() == allowed,
        "unexpected Go package files"
    );
    drop(files);
    let temp = tempfile::tempdir()?;
    let rebuilt = generate_with_notices(
        component,
        &temp.path().join("package"),
        module,
        version,
        notices,
    )?;
    ensure!(
        rebuilt["nativeSha256"] == report["nativeSha256"],
        "Go regenerated native archive mismatch"
    );
    ensure!(rebuilt == report, "Go regenerated provenance mismatch");
    Ok(report)
}

pub struct ModuleArchive {
    pub zip: Vec<u8>,
    pub mod_file: Vec<u8>,
    pub info: Vec<u8>,
}

fn package_files(
    root: &Path,
    directory: &Path,
    result: &mut BTreeMap<String, Vec<u8>>,
    budget: &mut u64,
) -> Result<()> {
    use std::io::Read;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(!kind.is_symlink(), "Go package contains symlink");
        if kind.is_dir() {
            package_files(root, &entry.path(), result, budget)?;
        } else {
            ensure!(
                kind.is_file() && result.len() < 128,
                "unsupported or excessive Go package files"
            );
            let path = entry.path();
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("non-UTF8 Go path")?
                .replace('\\', "/");
            ensure!(relative.len() < 512, "Go package path limit");
            let mut bytes = Vec::new();
            fs::File::open(path)?
                .take(*budget + 1)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() as u64 <= *budget, "Go package size limit");
            *budget -= bytes.len() as u64;
            result.insert(relative, bytes);
        }
    }
    Ok(())
}

/// Build the ordinary Go module proxy's versioned ZIP, .mod and .info payloads.
pub fn module_archive(package: &Path, module: &str, version: &str) -> Result<ModuleArchive> {
    use std::io::{Cursor, Write};
    validate_module(module, version)?;
    let mut files = BTreeMap::new();
    package_files(package, package, &mut files, &mut (500 * 1024 * 1024))?;
    let mod_file = files
        .get("go.mod")
        .context("Go module file missing")?
        .clone();
    ensure!(
        mod_file == module_metadata(module, version)?.0
            || mod_file == portable::module_file(module)
            || mod_file == guest::module_file(module),
        "Go module path mismatch"
    );
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644)
        .last_modified_time(zip::DateTime::default());
    for (path, bytes) in files {
        writer.start_file(format!("{module}@v{version}/{path}"), options)?;
        writer.write_all(&bytes)?;
    }
    let zip = writer.finish()?.into_inner();
    let info = serde_json::to_vec(
        &json!({"Version":format!("v{version}"),"Time":"1970-01-01T00:00:00Z"}),
    )?;
    Ok(ModuleArchive {
        zip,
        mod_file,
        info,
    })
}

pub fn module_metadata(module: &str, version: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    validate_module(module, version)?;
    Ok((
        format!("module {module}\n\ngo 1.27.0\n").into_bytes(),
        serde_json::to_vec(
            &json!({"Version":format!("v{version}"),"Time":"1970-01-01T00:00:00Z"}),
        )?,
    ))
}

#[allow(dead_code)]
pub fn verify_archive(
    component: &[u8],
    module: &str,
    version: &str,
    bytes: &[u8],
) -> Result<Value> {
    verify_archive_with_notices(component, module, version, bytes, &BTreeMap::new())
}

pub fn verify_archive_with_notices(
    component: &[u8],
    module: &str,
    version: &str,
    bytes: &[u8],
    notices: &BTreeMap<String, Vec<u8>>,
) -> Result<Value> {
    let temp = unpack_module_archive(bytes, module, version)?;
    let report = verify_output_with_notices(component, module, version, temp.path(), notices)?;
    let regenerated = module_archive(temp.path(), module, version)?;
    ensure!(
        regenerated.zip == bytes,
        "Go module archive is not canonical"
    );
    Ok(report)
}

pub(super) fn unpack_module_archive(
    bytes: &[u8],
    module: &str,
    version: &str,
) -> Result<tempfile::TempDir> {
    use std::io::{Cursor, Read};
    use std::path::Component;
    validate_module(module, version)?;
    ensure!(
        bytes.len() <= 500 * 1024 * 1024,
        "Go module archive size limit"
    );
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(
        archive.len() <= 128 && archive.comment().is_empty(),
        "Go module ZIP entry count or comment"
    );
    let temp = tempfile::tempdir()?;
    let prefix = format!("{module}@v{version}/");
    let mut paths = BTreeSet::new();
    let mut budget = 500 * 1024 * 1024u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        ensure!(
            !file.is_dir(),
            "Go module ZIP directory entries are unsupported"
        );
        let name = file
            .name()
            .strip_prefix(&prefix)
            .context("Go module ZIP prefix mismatch")?
            .to_owned();
        ensure!(
            !name.is_empty()
                && name.len() < 512
                && !name.contains('\\')
                && !name.contains(':')
                && name.split('/').all(|part| !part.is_empty())
                && Path::new(&name)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
                && paths.insert(name.clone()),
            "Go module ZIP unsafe or duplicate path"
        );
        if let Some(mode) = file.unix_mode() {
            ensure!(
                mode & 0o170000 == 0 || mode & 0o170000 == 0o100000,
                "Go module ZIP non-regular file"
            );
        }
        ensure!(file.size() <= budget, "Go module expanded size limit");
        let mut value = Vec::new();
        (&mut file).take(budget + 1).read_to_end(&mut value)?;
        ensure!(
            value.len() as u64 == file.size() && value.len() as u64 <= budget,
            "Go module ZIP size mismatch"
        );
        budget -= value.len() as u64;
        let target = temp.path().join(name);
        fs::create_dir_all(target.parent().context("Go ZIP parent")?)?;
        fs::write(target, value)?;
    }
    Ok(temp)
}
