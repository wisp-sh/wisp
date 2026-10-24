use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wit_parser::{FunctionKind, Handle, Resolve, Type, TypeDefKind, TypeId, WorldId, WorldItem};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AbiType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    S8,
    S16,
    S32,
    S64,
    F32,
    F64,
    Char,
    String,
    ErrorContext,
    Named(String),
    List(Box<AbiType>),
    FixedLengthList {
        element: Box<AbiType>,
        length: u32,
    },
    Map {
        key: Box<AbiType>,
        value: Box<AbiType>,
    },
    Tuple(Vec<AbiType>),
    Option(Box<AbiType>),
    Result {
        ok: Option<Box<AbiType>>,
        err: Option<Box<AbiType>>,
    },
    Record(Vec<(String, AbiType)>),
    Variant(Vec<(String, Option<Box<AbiType>>)>),
    Enum(Vec<String>),
    Flags(Vec<String>),
    Resource,
    Own(String),
    Borrow(String),
    Future(Option<Box<AbiType>>),
    Stream(Option<Box<AbiType>>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AbiFunctionKind {
    Freestanding,
    Constructor,
    Method,
    Static,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AbiAsyncCategory {
    Sync,
    Async,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbiNamedType {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: AbiType,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbiFunction {
    pub name: String,
    #[serde(rename = "itemName")]
    pub item_name: String,
    pub kind: AbiFunctionKind,
    pub params: Vec<(String, AbiType)>,
    pub result: Option<AbiType>,
    #[serde(rename = "ownerResource", skip_serializing_if = "Option::is_none")]
    pub owner_resource: Option<String>,
    #[serde(rename = "async")]
    pub asynchronous: AbiAsyncCategory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbiInterface {
    pub package: String,
    pub name: String,
    pub imported: bool,
    pub types: Vec<AbiNamedType>,
    pub functions: Vec<AbiFunction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbiModel {
    pub schema: u32,
    pub package: String,
    pub world: String,
    pub imports: Vec<AbiInterface>,
    pub exports: Vec<AbiInterface>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSupportReason {
    pub code: String,
    pub wit: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSupport {
    pub target: String,
    pub supported: bool,
    pub reasons: Vec<TargetSupportReason>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSupportMatrix {
    pub schema: u32,
    pub targets: Vec<TargetSupport>,
}

impl TargetSupportMatrix {
    fn require_supported(&self) -> Result<()> {
        let diagnostics = self
            .targets
            .iter()
            .filter(|target| {
                !(target.target == "browser"
                    && !target.reasons.is_empty()
                    && target.reasons.iter().all(|reason| {
                        reason.code == "unsupported-browser-future-lifecycle"
                            || reason.code == "unsupported-browser-stream-lifecycle"
                            || reason.code == "unsupported-browser-error-context-lifecycle"
                            || reason.code == "unsupported-browser-wasi-network"
                    }))
            })
            .flat_map(|target| {
                target.reasons.iter().map(move |reason| {
                    format!(
                        "[{}] target={} wit={} {}",
                        reason.code, target.target, reason.wit, reason.detail
                    )
                })
            })
            .collect::<Vec<_>>();
        ensure!(
            diagnostics.is_empty(),
            "target support matrix rejected WIT contract:\n{}",
            diagnostics.join("\n")
        );
        Ok(())
    }
}

fn resolve_alias(ty: &AbiType, types: &[AbiNamedType]) -> Result<AbiType> {
    fn visit(
        ty: &AbiType,
        types: &[AbiNamedType],
        pending: &mut std::collections::BTreeSet<String>,
    ) -> Result<AbiType> {
        let AbiType::Named(name) = ty else {
            return Ok(ty.clone());
        };
        ensure!(pending.insert(name.clone()), "recursive WIT alias: {name}");
        let definition = types
            .iter()
            .find(|candidate| candidate.name == *name)
            .with_context(|| format!("unknown WIT alias: {name}"))?;
        let resolved = visit(&definition.ty, types, pending)?;
        pending.remove(name);
        Ok(resolved)
    }
    visit(ty, types, &mut std::collections::BTreeSet::new())
}

fn supported_scalar(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingScalar>> {
    Ok(match resolve_alias(ty, types)? {
        AbiType::String => Some(BindingScalar::String),
        AbiType::U8 => Some(BindingScalar::U8),
        AbiType::U16 => Some(BindingScalar::U16),
        AbiType::U32 => Some(BindingScalar::U32),
        AbiType::U64 => Some(BindingScalar::U64),
        AbiType::S64 => Some(BindingScalar::I64),
        AbiType::Bool => Some(BindingScalar::Bool),
        AbiType::S8 => Some(BindingScalar::I8),
        AbiType::S16 => Some(BindingScalar::I16),
        AbiType::S32 => Some(BindingScalar::I32),
        AbiType::Char => Some(BindingScalar::Char),
        AbiType::F32 => Some(BindingScalar::F32),
        AbiType::F64 => Some(BindingScalar::F64),
        _ => None,
    })
}

fn supported_record(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingRecord>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::Record(fields) = &definition.ty else {
        return Ok(None);
    };
    let fields = fields
        .iter()
        .map(|(field_name, field_type)| {
            Ok(BindingRecordField {
                wit_name: field_name.clone(),
                go_name: upper_camel(field_name),
                rust_name: rust_identifier(field_name),
                js_name: lower_camel(field_name),
                scalar: supported_scalar(field_type, types)?
                    .with_context(|| format!("unsupported record field {name}.{field_name}"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(BindingRecord {
        target_name: upper_camel(name),
        fields,
    }))
}

fn supported_enum(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingEnum>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::Enum(cases) = &definition.ty else {
        return Ok(None);
    };
    Ok(Some(BindingEnum {
        target_name: upper_camel(name),
        cases: cases
            .iter()
            .map(|case| BindingEnumCase {
                wit_name: case.clone(),
                target_name: upper_camel(case),
            })
            .collect(),
    }))
}

fn supported_flags(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingFlags>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::Flags(names) = &definition.ty else {
        return Ok(None);
    };
    Ok(Some(BindingFlags {
        target_name: upper_camel(name),
        names: names
            .iter()
            .map(|flag| BindingFlag {
                wit_name: flag.clone(),
                go_name: upper_camel(flag),
                rust_name: rust_identifier(flag),
                js_name: lower_camel(flag),
            })
            .collect(),
    }))
}

fn supported_variant(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingVariant>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::Variant(cases) = &definition.ty else {
        return Ok(None);
    };
    let cases = cases
        .iter()
        .map(|(case_name, payload)| {
            Ok(BindingVariantCase {
                wit_name: case_name.clone(),
                target_name: upper_camel(case_name),
                payload: payload
                    .as_deref()
                    .map(|payload| {
                        supported_scalar(payload, types)?.with_context(|| {
                            format!("unsupported variant payload {name}.{case_name}")
                        })
                    })
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(BindingVariant {
        target_name: upper_camel(name),
        cases,
    }))
}

fn supported_option(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingOption>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::Option(payload) = &definition.ty else {
        return Ok(None);
    };
    let payload = supported_scalar(payload, types)?
        .with_context(|| format!("unsupported option payload {name}"))?;
    Ok(Some(BindingOption {
        target_name: upper_camel(name),
        payload,
    }))
}

fn supported_result(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingResult>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::Result { ok, err } = &definition.ty else {
        return Ok(None);
    };
    let lower = |branch: &Option<Box<AbiType>>, label: &str| -> Result<Option<BindingScalar>> {
        branch
            .as_deref()
            .map(|payload| {
                supported_scalar(payload, types)?
                    .with_context(|| format!("unsupported result payload {name}.{label}"))
            })
            .transpose()
    };
    Ok(Some(BindingResult {
        target_name: upper_camel(name),
        ok: lower(ok, "ok")?,
        err: lower(err, "err")?,
    }))
}

fn supported_list(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingList>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::List(element) = &definition.ty else {
        return Ok(None);
    };
    let element = supported_scalar(element, types)?
        .with_context(|| format!("unsupported list element {name}"))?;
    Ok(Some(BindingList {
        target_name: upper_camel(name),
        element,
    }))
}

fn supported_tuple(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingTuple>> {
    let AbiType::Named(name) = ty else {
        return Ok(None);
    };
    let definition = types
        .iter()
        .find(|candidate| candidate.name == *name)
        .with_context(|| format!("unknown WIT type: {name}"))?;
    let AbiType::Tuple(elements) = &definition.ty else {
        return Ok(None);
    };
    ensure!(!elements.is_empty(), "empty tuple {name} is unsupported");
    let elements = elements
        .iter()
        .enumerate()
        .map(|(index, element)| {
            supported_scalar(element, types)?
                .with_context(|| format!("unsupported tuple element {name}.{index}"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(BindingTuple {
        target_name: upper_camel(name),
        elements,
    }))
}

fn supported_value(ty: &AbiType, types: &[AbiNamedType]) -> Result<Option<BindingValue>> {
    if let Some(scalar) = supported_scalar(ty, types)? {
        return Ok(Some(BindingValue::Scalar(scalar)));
    }
    if let Some(record) = supported_record(ty, types)? {
        return Ok(Some(BindingValue::Record(record)));
    }
    if let Some(enum_) = supported_enum(ty, types)? {
        return Ok(Some(BindingValue::Enum(enum_)));
    }
    if let Some(flags) = supported_flags(ty, types)? {
        return Ok(Some(BindingValue::Flags(flags)));
    }
    if let Some(variant) = supported_variant(ty, types)? {
        return Ok(Some(BindingValue::Variant(variant)));
    }
    if let Some(option) = supported_option(ty, types)? {
        return Ok(Some(BindingValue::Option(option)));
    }
    if let Some(result) = supported_result(ty, types)? {
        return Ok(Some(BindingValue::Result(result)));
    }
    if let Some(list) = supported_list(ty, types)? {
        return Ok(Some(BindingValue::List(list)));
    }
    Ok(supported_tuple(ty, types)?.map(BindingValue::Tuple))
}

fn type_name(resolve: &Resolve, id: TypeId) -> Result<String> {
    resolve.types[id]
        .name
        .clone()
        .with_context(|| format!("ABI resource/type {id:?} has no stable name"))
}

fn lower_type(resolve: &Resolve, ty: Type, stack: &mut Vec<TypeId>) -> Result<AbiType> {
    Ok(match ty {
        Type::Bool => AbiType::Bool,
        Type::U8 => AbiType::U8,
        Type::U16 => AbiType::U16,
        Type::U32 => AbiType::U32,
        Type::U64 => AbiType::U64,
        Type::S8 => AbiType::S8,
        Type::S16 => AbiType::S16,
        Type::S32 => AbiType::S32,
        Type::S64 => AbiType::S64,
        Type::F32 => AbiType::F32,
        Type::F64 => AbiType::F64,
        Type::Char => AbiType::Char,
        Type::String => AbiType::String,
        Type::ErrorContext => AbiType::ErrorContext,
        Type::Id(id) => {
            let definition = &resolve.types[id];
            if let Some(name) = &definition.name {
                AbiType::Named(name.clone())
            } else {
                if stack.contains(&id) {
                    bail!("anonymous recursive WIT type graph")
                }
                stack.push(id);
                let lowered = lower_definition(resolve, &definition.kind, stack)?;
                stack.pop();
                lowered
            }
        }
    })
}

fn lower_definition(
    resolve: &Resolve,
    kind: &TypeDefKind,
    stack: &mut Vec<TypeId>,
) -> Result<AbiType> {
    let mut lower = |ty| lower_type(resolve, ty, stack);
    Ok(match kind {
        TypeDefKind::Record(record) => AbiType::Record(
            record
                .fields
                .iter()
                .map(|field| Ok((field.name.clone(), lower(field.ty)?)))
                .collect::<Result<_>>()?,
        ),
        TypeDefKind::Resource => AbiType::Resource,
        TypeDefKind::Handle(Handle::Own(id)) => AbiType::Own(type_name(resolve, *id)?),
        TypeDefKind::Handle(Handle::Borrow(id)) => AbiType::Borrow(type_name(resolve, *id)?),
        TypeDefKind::Flags(flags) => {
            AbiType::Flags(flags.flags.iter().map(|flag| flag.name.clone()).collect())
        }
        TypeDefKind::Tuple(tuple) => AbiType::Tuple(
            tuple
                .types
                .iter()
                .map(|ty| lower(*ty))
                .collect::<Result<_>>()?,
        ),
        TypeDefKind::Variant(variant) => AbiType::Variant(
            variant
                .cases
                .iter()
                .map(|case| {
                    Ok((
                        case.name.clone(),
                        case.ty.map(&mut lower).transpose()?.map(Box::new),
                    ))
                })
                .collect::<Result<_>>()?,
        ),
        TypeDefKind::Enum(enum_) => {
            AbiType::Enum(enum_.cases.iter().map(|case| case.name.clone()).collect())
        }
        TypeDefKind::Option(ty) => AbiType::Option(Box::new(lower(*ty)?)),
        TypeDefKind::Result(result) => AbiType::Result {
            ok: result.ok.map(&mut lower).transpose()?.map(Box::new),
            err: result.err.map(&mut lower).transpose()?.map(Box::new),
        },
        TypeDefKind::List(ty) => AbiType::List(Box::new(lower(*ty)?)),
        TypeDefKind::Map(key, value) => AbiType::Map {
            key: Box::new(lower(*key)?),
            value: Box::new(lower(*value)?),
        },
        TypeDefKind::FixedLengthList(ty, length) => AbiType::FixedLengthList {
            element: Box::new(lower(*ty)?),
            length: *length,
        },
        TypeDefKind::Future(ty) => AbiType::Future(ty.map(&mut lower).transpose()?.map(Box::new)),
        TypeDefKind::Stream(ty) => AbiType::Stream(ty.map(&mut lower).transpose()?.map(Box::new)),
        TypeDefKind::Type(ty) => lower(*ty)?,
        TypeDefKind::Unknown => bail!("unresolved WIT type reached canonical ABI model"),
    })
}

fn lower_function(resolve: &Resolve, function: &wit_parser::Function) -> Result<AbiFunction> {
    let (kind, owner_resource) = match function.kind {
        FunctionKind::Freestanding | FunctionKind::AsyncFreestanding => {
            (AbiFunctionKind::Freestanding, None)
        }
        FunctionKind::Constructor(id) => {
            (AbiFunctionKind::Constructor, Some(type_name(resolve, id)?))
        }
        FunctionKind::Method(id) | FunctionKind::AsyncMethod(id) => {
            (AbiFunctionKind::Method, Some(type_name(resolve, id)?))
        }
        FunctionKind::Static(id) | FunctionKind::AsyncStatic(id) => {
            (AbiFunctionKind::Static, Some(type_name(resolve, id)?))
        }
    };
    let asynchronous = if function.kind.is_async() {
        AbiAsyncCategory::Async
    } else {
        AbiAsyncCategory::Sync
    };
    Ok(AbiFunction {
        name: function.name.clone(),
        item_name: function.item_name().to_owned(),
        kind,
        params: function
            .params
            .iter()
            .map(|param| {
                Ok((
                    param.name.clone(),
                    lower_type(resolve, param.ty, &mut Vec::new())?,
                ))
            })
            .collect::<Result<_>>()?,
        result: function
            .result
            .map(|ty| lower_type(resolve, ty, &mut Vec::new()))
            .transpose()?,
        owner_resource,
        asynchronous,
    })
}

fn lower_interface(
    resolve: &Resolve,
    id: wit_parser::InterfaceId,
    imported: bool,
) -> Result<AbiInterface> {
    let interface = &resolve.interfaces[id];
    let package = interface
        .package
        .context("ABI interface has no package owner")?;
    Ok(AbiInterface {
        package: resolve.packages[package].name.to_string(),
        name: interface
            .name
            .clone()
            .context("ABI interface has no stable name")?,
        imported,
        types: interface
            .types
            .iter()
            .map(|(name, id)| {
                Ok(AbiNamedType {
                    name: name.clone(),
                    ty: lower_definition(resolve, &resolve.types[*id].kind, &mut vec![*id])?,
                })
            })
            .collect::<Result<_>>()?,
        functions: interface
            .functions
            .values()
            .map(|function| lower_function(resolve, function))
            .collect::<Result<_>>()?,
    })
}

impl AbiModel {
    fn uses_standard_wasi_network(&self) -> bool {
        self.imports.iter().any(|interface| {
            interface.package.starts_with("wasi:http@")
                || interface.package.starts_with("wasi:sockets@")
        })
    }

    fn supports_standard_wasi_network_profile(&self) -> bool {
        let imports = self
            .imports
            .iter()
            .map(|interface| (interface.package.as_str(), interface.name.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        let authored = std::collections::BTreeSet::from([
            ("wasi:clocks@0.2.12", "monotonic-clock"),
            ("wasi:http@0.2.12", "outgoing-handler"),
            ("wasi:http@0.2.12", "types"),
            ("wasi:io@0.2.12", "error"),
            ("wasi:io@0.2.12", "poll"),
            ("wasi:io@0.2.12", "streams"),
            ("wasi:sockets@0.2.12", "instance-network"),
            ("wasi:sockets@0.2.12", "network"),
        ]);
        let executable = std::collections::BTreeSet::from([
            ("wasi:http@0.2.12", "outgoing-handler"),
            ("wasi:http@0.2.12", "types"),
            ("wasi:io@0.2.12", "error"),
            ("wasi:io@0.2.12", "poll"),
            ("wasi:io@0.2.12", "streams"),
            ("wasi:sockets@0.2.12", "instance-network"),
            ("wasi:sockets@0.2.12", "network"),
        ]);
        imports == authored || imports == executable
    }

    fn has_special_lifecycle_binding_surface(&self) -> bool {
        let mut without_imports = self.clone();
        without_imports.imports.clear();
        without_imports.future_u32_surface().is_some()
            || without_imports.stream_u32_surface().is_some()
            || without_imports.error_context_surface().is_some()
            || without_imports.exports.iter().any(|interface| {
                interface
                    .types
                    .iter()
                    .any(|ty| matches!(ty.ty, AbiType::Resource))
            })
    }

    pub fn from_resolve(resolve: &Resolve, world_id: WorldId) -> Result<Self> {
        let world = &resolve.worlds[world_id];
        let package_id = world.package.context("selected world has no package")?;
        let package = resolve.packages[package_id].name.to_string();
        let mut imports = Vec::new();
        let mut exports = Vec::new();
        let mut world_import_functions = Vec::new();
        let mut world_export_functions = Vec::new();
        let mut world_types = Vec::new();
        let mut world_type_ids = std::collections::BTreeSet::new();

        for item in world.imports.values() {
            match item {
                WorldItem::Interface { id, .. } => {
                    imports.push(lower_interface(resolve, *id, true)?)
                }
                WorldItem::Function(function) => {
                    world_import_functions.push(lower_function(resolve, function)?)
                }
                WorldItem::Type { id, .. } => {
                    if world_type_ids.insert(*id) {
                        world_types.push(AbiNamedType {
                            name: type_name(resolve, *id)?,
                            ty: lower_definition(
                                resolve,
                                &resolve.types[*id].kind,
                                &mut vec![*id],
                            )?,
                        });
                    }
                }
            }
        }
        for item in world.exports.values() {
            match item {
                WorldItem::Interface { id, .. } => {
                    exports.push(lower_interface(resolve, *id, false)?)
                }
                WorldItem::Function(function) => {
                    world_export_functions.push(lower_function(resolve, function)?)
                }
                WorldItem::Type { id, .. } => {
                    if world_type_ids.insert(*id) {
                        world_types.push(AbiNamedType {
                            name: type_name(resolve, *id)?,
                            ty: lower_definition(
                                resolve,
                                &resolve.types[*id].kind,
                                &mut vec![*id],
                            )?,
                        });
                    }
                }
            }
        }
        if !world_import_functions.is_empty() {
            imports.push(AbiInterface {
                package: package.clone(),
                name: format!("{}-world", world.name),
                imported: true,
                types: if world_export_functions.is_empty() {
                    world_types.clone()
                } else {
                    Vec::new()
                },
                functions: world_import_functions,
            });
        }
        if !world_export_functions.is_empty() {
            exports.push(AbiInterface {
                package: package.clone(),
                name: format!("{}-world", world.name),
                imported: false,
                types: world_types,
                functions: world_export_functions,
            });
        }
        Ok(Self {
            schema: 1,
            package,
            world: world.name.clone(),
            imports,
            exports,
        })
    }

    pub fn canonical_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(self).context("serialize canonical WIT model")
    }

    pub fn canonical_contract(&self) -> Result<serde_json::Value> {
        Ok(serde_json::json!({
            "schema": 2,
            "package": self.package,
            "world": self.world,
            "imports": self.imports,
            "exports": self.exports,
            "targetSupport": self.target_support_matrix()?,
        }))
    }

    pub fn canonical_contract_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.canonical_contract()?).context("serialize canonical WIT contract")
    }

    pub fn contract_fingerprint(&self) -> Result<String> {
        let mut input = b"wisp-wit-contract\0".to_vec();
        input.extend_from_slice(b"2\0");
        input.extend_from_slice(&self.canonical_contract_bytes()?);
        Ok(format!("sha256:{:x}", Sha256::digest(input)))
    }

    pub fn emit_binding_inventory(&self) -> serde_json::Value {
        let functions = self
            .exports
            .iter()
            .flat_map(|interface| {
                interface.functions.iter().map(move |function| {
                    serde_json::json!({
                        "interface": interface.name,
                        "name": function.name,
                        "kind": function.kind,
                        "params": function.params,
                        "result": function.result,
                        "ownerResource": function.owner_resource,
                        "async": function.asynchronous,
                    })
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({"schema": 1, "functions": functions})
    }

    pub fn target_support_matrix(&self) -> Result<TargetSupportMatrix> {
        if self.uses_standard_wasi_network() && self.has_special_lifecycle_binding_surface() {
            return Ok(TargetSupportMatrix {
                schema: 1,
                targets: ["browser", "go", "node", "rust"]
                    .into_iter()
                    .map(|target| TargetSupport {
                        target: target.into(),
                        supported: false,
                        reasons: vec![TargetSupportReason {
                            code: "unsupported-standard-wasi-network-lifecycle".into(),
                            wit: format!("{}/{}/wasi-network", self.package, self.world),
                            detail: "standard WASI networking cannot be combined with special resource, future, stream, or error-context lifecycle bindings"
                                .into(),
                        }],
                    })
                    .collect(),
            });
        }
        if self.error_context_surface().is_some() {
            return Ok(TargetSupportMatrix {
                schema: 1,
                targets: ["browser", "go", "node", "rust"]
                    .into_iter()
                    .map(|target| {
                        let reasons = if target == "browser" {
                            vec![TargetSupportReason {
                                code: "unsupported-browser-error-context-lifecycle".into(),
                                wit: format!("{}/{}/error-context", self.package, self.world),
                                detail: "the pinned browser backend has no proven Store teardown boundary for retained lifted error contexts".into(),
                            }]
                        } else {
                            Vec::new()
                        };
                        TargetSupport {
                            target: target.into(),
                            supported: reasons.is_empty(),
                            reasons,
                        }
                    })
                    .collect(),
            });
        }
        if self.stream_u32_surface().is_some() {
            return Ok(TargetSupportMatrix {
                schema: 1,
                targets: ["browser", "go", "node", "rust"]
                    .into_iter()
                    .map(|target| {
                        let reasons = if target == "browser" {
                            vec![TargetSupportReason {
                                code: "unsupported-browser-stream-lifecycle".into(),
                                wit: format!("{}/{}/stream<u32>", self.package, self.world),
                                detail: "the pinned browser backend has no proven explicit cancellation/close lifecycle for stream values".into(),
                            }]
                        } else {
                            Vec::new()
                        };
                        TargetSupport {
                            target: target.into(),
                            supported: reasons.is_empty(),
                            reasons,
                        }
                    })
                    .collect(),
            });
        }
        if self.future_u32_surface().is_some() {
            return Ok(TargetSupportMatrix {
                schema: 1,
                targets: ["browser", "go", "node", "rust"]
                    .into_iter()
                    .map(|target| {
                        let reasons = if target == "browser" {
                            vec![TargetSupportReason {
                                code: "unsupported-browser-future-lifecycle".into(),
                                wit: format!("{}/{}/future<u32>", self.package, self.world),
                                detail: "the pinned browser backend has no proven explicit cancellation/close lifecycle for future values".into(),
                            }]
                        } else {
                            Vec::new()
                        };
                        TargetSupport {
                            target: target.into(),
                            supported: reasons.is_empty(),
                            reasons,
                        }
                    })
                    .collect(),
            });
        }
        let expected_result = AbiType::Result {
            ok: Some(Box::new(AbiType::List(Box::new(AbiType::String)))),
            err: Some(Box::new(AbiType::String)),
        };
        let mut targets = Vec::new();
        for target in ["browser", "go", "node", "rust"] {
            let mut reasons = Vec::new();
            let mut reject = |code: &str, wit: String, detail: String| {
                reasons.push(TargetSupportReason {
                    code: code.to_owned(),
                    wit,
                    detail,
                });
            };
            let world_identity = format!("{}/{}", self.package, self.world);

            // Admit only the first complete operational resource surface. The runtime
            // registry currently has one generated constructor shape (u32), one
            // borrowing instance method, and one static borrow/own transfer. Reject
            // every extension as one stable structural reason until its lifecycle is
            // implemented coherently across all four targets.
            let resource = self.exports.iter().find_map(|interface| {
                interface
                    .types
                    .iter()
                    .find_map(|ty| matches!(ty.ty, AbiType::Resource).then_some((interface, ty)))
            });
            if let Some((interface, resource)) = resource {
                let owner_functions = interface
                    .functions
                    .iter()
                    .filter(|function| function.owner_resource.as_deref() == Some(&resource.name))
                    .collect::<Vec<_>>();
                let constructors = owner_functions
                    .iter()
                    .filter(|function| function.kind == AbiFunctionKind::Constructor)
                    .copied()
                    .collect::<Vec<_>>();
                let methods = owner_functions
                    .iter()
                    .filter(|function| function.kind == AbiFunctionKind::Method)
                    .copied()
                    .collect::<Vec<_>>();
                let statics = owner_functions
                    .iter()
                    .filter(|function| function.kind == AbiFunctionKind::Static)
                    .copied()
                    .collect::<Vec<_>>();
                let complete_surface = self.exports.len() == 1
                    && interface.types.len() == 1
                    && interface.functions.len() == 3
                    && owner_functions.len() == 3
                    && constructors.len() == 1
                    && constructors[0].params.len() == 1
                    && constructors[0].params[0].1 == AbiType::U32
                    && constructors[0].result == Some(AbiType::Own(resource.name.clone()))
                    && methods.len() == 1
                    && methods[0].params.len() == 1
                    && methods[0].params[0].1 == AbiType::Borrow(resource.name.clone())
                    && methods[0].result.as_ref() == Some(&expected_result)
                    && statics.len() == 1
                    && statics[0].params.len() == 2
                    && statics[0].params[0].1 == AbiType::Borrow(resource.name.clone())
                    && statics[0].params[1].1 == AbiType::Own(resource.name.clone())
                    && statics[0].result.as_ref() == Some(&expected_result)
                    && owner_functions
                        .iter()
                        .all(|function| function.asynchronous == AbiAsyncCategory::Sync);
                let identity =
                    format!("{world_identity}/{}.type.{}", interface.name, resource.name);
                if !complete_surface {
                    reject(
                        "unsupported-resource-surface",
                        identity,
                        "the first resource slice requires constructor, method, static, own, and borrow semantics together".into(),
                    );
                }
                targets.push(TargetSupport {
                    target: target.into(),
                    supported: reasons.is_empty(),
                    reasons,
                });
                continue;
            }
            if self.exports.len() != 1 {
                reject(
                    "unsupported-export-group",
                    world_identity.clone(),
                    "the fixed binding profile requires exactly one world-level export group"
                        .into(),
                );
            }
            if let Some(interface) = self.exports.first() {
                let interface_identity = format!("{world_identity}/{}", interface.name);
                if interface.package != self.package
                    || interface.name != format!("{}-world", self.world)
                {
                    reject(
                        "unsupported-export-group",
                        interface_identity.clone(),
                        "the fixed binding profile only supports world-level exports".into(),
                    );
                }
                if !interface.types.is_empty() {
                    for ty in &interface.types {
                        let type_identity = format!("{interface_identity}.type.{}", ty.name);
                        match supported_value(&AbiType::Named(ty.name.clone()), &interface.types) {
                            Ok(Some(_)) => continue,
                            Err(_) if matches!(ty.ty, AbiType::Record(_)) => {
                                if let AbiType::Record(fields) = &ty.ty {
                                    for (field_name, field_type) in fields {
                                        if !supported_scalar(field_type, &interface.types)
                                            .is_ok_and(|scalar| scalar.is_some())
                                        {
                                            reject(
                                                "unsupported-record-field-type",
                                                format!("{type_identity}.field.{field_name}"),
                                                format!(
                                                    "flat records require scalar fields, found {field_type:?}"
                                                ),
                                            );
                                        }
                                    }
                                }
                                continue;
                            }
                            Err(_) if matches!(ty.ty, AbiType::Variant(_)) => {
                                if let AbiType::Variant(cases) = &ty.ty {
                                    for (case_name, payload) in cases {
                                        if payload.as_deref().is_some_and(|payload| {
                                            !supported_scalar(payload, &interface.types)
                                                .is_ok_and(|scalar| scalar.is_some())
                                        }) {
                                            reject(
                                                "unsupported-variant-payload-type",
                                                format!("{type_identity}.case.{case_name}"),
                                                format!(
                                                    "flat variants require scalar payloads, found {payload:?}"
                                                ),
                                            );
                                        }
                                    }
                                }
                                continue;
                            }
                            Err(_) if matches!(ty.ty, AbiType::Option(_)) => {
                                if let AbiType::Option(payload) = &ty.ty
                                    && !supported_scalar(payload, &interface.types)
                                        .is_ok_and(|scalar| scalar.is_some())
                                {
                                    reject(
                                        "unsupported-option-payload-type",
                                        format!("{type_identity}.payload"),
                                        format!(
                                            "options require scalar payloads, found {payload:?}"
                                        ),
                                    );
                                }
                                continue;
                            }
                            Err(_) if matches!(ty.ty, AbiType::Result { .. }) => {
                                if let AbiType::Result { ok, err } = &ty.ty {
                                    for (branch, payload) in [("ok", ok), ("err", err)] {
                                        if payload.as_deref().is_some_and(|payload| {
                                            !supported_scalar(payload, &interface.types)
                                                .is_ok_and(|scalar| scalar.is_some())
                                        }) {
                                            reject(
                                                "unsupported-result-payload-type",
                                                format!("{type_identity}.{branch}"),
                                                format!(
                                                    "results require scalar payloads, found {payload:?}"
                                                ),
                                            );
                                        }
                                    }
                                }
                                continue;
                            }
                            Err(_) if matches!(ty.ty, AbiType::List(_)) => {
                                if let AbiType::List(element) = &ty.ty
                                    && !supported_scalar(element, &interface.types)
                                        .is_ok_and(|scalar| scalar.is_some())
                                {
                                    reject(
                                        "unsupported-list-element-type",
                                        format!("{type_identity}.element"),
                                        format!("lists require scalar elements, found {element:?}"),
                                    );
                                }
                                continue;
                            }
                            Err(_) if matches!(ty.ty, AbiType::Tuple(_)) => {
                                if let AbiType::Tuple(elements) = &ty.ty {
                                    if elements.is_empty() {
                                        reject(
                                            "empty-tuple",
                                            type_identity.clone(),
                                            "named tuple requires at least one element".into(),
                                        );
                                    }
                                    for (index, element) in elements.iter().enumerate() {
                                        if !supported_scalar(element, &interface.types)
                                            .is_ok_and(|scalar| scalar.is_some())
                                        {
                                            reject(
                                                "unsupported-tuple-element-type",
                                                format!("{type_identity}.element.{index}"),
                                                format!(
                                                    "tuples require scalar elements, found {element:?}"
                                                ),
                                            );
                                        }
                                    }
                                }
                                continue;
                            }
                            _ => {}
                        }
                        reject(
                            "unsupported-exported-type",
                            type_identity,
                            "the fixed binding profile only supports scalar aliases and named flat records, enums, flags, variants, scalar options, scalar results, or scalar lists".into(),
                        );
                    }
                }
                if interface.functions.is_empty() {
                    reject(
                        "missing-export",
                        interface_identity.clone(),
                        "the fixed binding profile requires at least one exported function".into(),
                    );
                }

                let mut target_names = std::collections::BTreeMap::<String, String>::new();
                if target == "go" {
                    for ty in &interface.types {
                        let identity = format!("{interface_identity}.type.{}", ty.name);
                        let emitted = upper_camel(&ty.name);
                        if emitted.is_empty() {
                            reject(
                                "empty-target-name",
                                identity,
                                "target type name is empty".into(),
                            );
                        } else if let Some(previous) =
                            target_names.insert(emitted.clone(), identity.clone())
                        {
                            reject(
                                "target-name-collision",
                                identity,
                                format!("target name {emitted} also identifies {previous}"),
                            );
                        }
                    }
                }
                for ty in &interface.types {
                    if let AbiType::Enum(cases) = &ty.ty {
                        let mut case_names = std::collections::BTreeMap::<String, String>::new();
                        if target == "rust" && upper_camel(&ty.name) == "Self" {
                            reject(
                                "target-name-reserved",
                                format!("{interface_identity}.type.{}", ty.name),
                                "reserved Rust enum type name: Self".into(),
                            );
                        }
                        if cases.is_empty() {
                            reject(
                                "empty-enum",
                                format!("{interface_identity}.type.{}", ty.name),
                                "named enum requires at least one case".into(),
                            );
                        }
                        if matches!(target, "go" | "rust") {
                            for case in cases {
                                let emitted = upper_camel(case);
                                let identity =
                                    format!("{interface_identity}.type.{}.case.{case}", ty.name);
                                if emitted.is_empty() {
                                    reject(
                                        "empty-target-name",
                                        identity,
                                        "target enum case name is empty".into(),
                                    );
                                } else if target == "rust" && emitted == "Self" {
                                    reject(
                                        "target-name-reserved",
                                        identity,
                                        format!("reserved Rust enum case name: {emitted}"),
                                    );
                                } else if let Some(previous) =
                                    case_names.insert(emitted.clone(), identity.clone())
                                {
                                    reject(
                                        "target-name-collision",
                                        identity,
                                        format!(
                                            "target enum case name {emitted} also identifies {previous}"
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                for ty in &interface.types {
                    if let AbiType::Flags(flags) = &ty.ty {
                        let mut flag_names = std::collections::BTreeMap::<String, String>::new();
                        for flag in flags {
                            let emitted = match target {
                                "go" => upper_camel(flag),
                                "rust" => rust_identifier(flag),
                                "browser" | "node" => lower_camel(flag),
                                _ => unreachable!(),
                            };
                            let identity =
                                format!("{interface_identity}.type.{}.flag.{flag}", ty.name);
                            if emitted.is_empty() {
                                reject(
                                    "empty-target-name",
                                    identity,
                                    "target flag name is empty".into(),
                                );
                            } else if target == "rust" && emitted.starts_with("r#") {
                                reject(
                                    "target-name-reserved",
                                    identity,
                                    format!("reserved Rust flag field name: {flag}"),
                                );
                            } else if matches!(target, "browser" | "node") && js_reserved(&emitted)
                            {
                                reject(
                                    "target-name-reserved",
                                    identity,
                                    format!("reserved JavaScript flag field name: {emitted}"),
                                );
                            } else if let Some(previous) =
                                flag_names.insert(emitted.clone(), identity.clone())
                            {
                                reject(
                                    "target-name-collision",
                                    identity,
                                    format!(
                                        "target flag name {emitted} also identifies {previous}"
                                    ),
                                );
                            }
                        }
                    }
                }
                for ty in &interface.types {
                    if let AbiType::Variant(cases) = &ty.ty {
                        let mut case_names = std::collections::BTreeMap::<String, String>::new();
                        let type_identity = format!("{interface_identity}.type.{}", ty.name);
                        if target == "rust" && upper_camel(&ty.name) == "Self" {
                            reject(
                                "target-name-reserved",
                                type_identity.clone(),
                                "reserved Rust variant type name: Self".into(),
                            );
                        }
                        if cases.is_empty() {
                            reject(
                                "empty-variant",
                                type_identity,
                                "named variant requires at least one case".into(),
                            );
                        }
                        for (case, _) in cases {
                            let emitted = upper_camel(case);
                            let identity =
                                format!("{interface_identity}.type.{}.case.{case}", ty.name);
                            if emitted.is_empty() {
                                reject(
                                    "empty-target-name",
                                    identity,
                                    "target variant case name is empty".into(),
                                );
                            } else if target == "rust" && emitted == "Self" {
                                reject(
                                    "target-name-reserved",
                                    identity,
                                    "reserved Rust variant case name: Self".into(),
                                );
                            } else if let Some(previous) =
                                case_names.insert(emitted.clone(), identity.clone())
                            {
                                reject(
                                    "target-name-collision",
                                    identity,
                                    format!(
                                        "target variant case name {emitted} also identifies {previous}"
                                    ),
                                );
                            }
                        }
                    }
                }
                for function in &interface.functions {
                    let function_identity = format!("{interface_identity}.{}", function.name);
                    if function.kind != AbiFunctionKind::Freestanding
                        || function.owner_resource.is_some()
                    {
                        reject(
                            "unsupported-function-kind",
                            function_identity.clone(),
                            "the fixed binding profile only supports freestanding exports".into(),
                        );
                    }
                    if function.asynchronous != AbiAsyncCategory::Sync {
                        reject(
                            "unsupported-async-function",
                            function_identity.clone(),
                            "the fixed binding profile only supports synchronous exports".into(),
                        );
                    }
                    for (name, ty) in &function.params {
                        if !supported_value(ty, &interface.types).is_ok_and(|value| value.is_some())
                        {
                            reject(
                                "unsupported-parameter-type",
                                format!("{function_identity}.param.{name}"),
                                format!(
                                    "expected a supported scalar or named flat record, enum, flags, variant, scalar option, or scalar result, found {ty:?}"
                                ),
                            );
                        }
                    }
                    if function
                        .result
                        .as_ref()
                        .map(|ty| resolve_alias(ty, &interface.types))
                        .transpose()?
                        .as_ref()
                        != Some(&expected_result)
                    {
                        reject(
                            "unsupported-result-type",
                            format!("{function_identity}.result"),
                            "expected result<list<string>, string>".into(),
                        );
                    }

                    let target_name = match target {
                        "go" => upper_camel(&function.item_name),
                        "rust" => rust_identifier(&function.item_name),
                        "browser" | "node" => lower_camel(&function.item_name),
                        _ => unreachable!(),
                    };
                    if target_name.is_empty() {
                        reject(
                            "empty-target-name",
                            function_identity.clone(),
                            "target function name is empty".into(),
                        );
                    }
                    if matches!(target, "browser" | "node") && js_reserved(&target_name) {
                        reject(
                            "target-name-reserved",
                            function_identity.clone(),
                            format!("reserved JavaScript export name: {target_name}"),
                        );
                    }
                    if let Some(previous) =
                        target_names.insert(target_name.clone(), function_identity.clone())
                    {
                        reject(
                            "target-name-collision",
                            function_identity.clone(),
                            format!("target name {target_name} also identifies {previous}"),
                        );
                    }

                    if matches!(target, "go" | "rust") {
                        let mut parameter_names =
                            std::collections::BTreeMap::<String, String>::new();
                        for (name, _) in &function.params {
                            let emitted = if target == "go" {
                                lower_camel(name)
                            } else {
                                rust_identifier(name)
                            };
                            let identity = format!("{function_identity}.param.{name}");
                            if emitted.is_empty() {
                                reject(
                                    "empty-target-name",
                                    identity,
                                    "target parameter name is empty".into(),
                                );
                            } else if target == "go" && go_reserved(&emitted) {
                                reject(
                                    "target-name-reserved",
                                    identity,
                                    format!("reserved Go parameter name: {emitted}"),
                                );
                            } else if let Some(previous) =
                                parameter_names.insert(emitted.clone(), identity.clone())
                            {
                                reject(
                                    "target-name-collision",
                                    identity,
                                    format!(
                                        "target parameter name {emitted} also identifies {previous}"
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            if target == "browser" && self.uses_standard_wasi_network() {
                reasons.clear();
                reasons.push(TargetSupportReason {
                    code: "unsupported-browser-wasi-network".into(),
                    wit: format!("{world_identity}/wasi-network"),
                    detail: "the browser target has no standard WASI HTTP and sockets host implementation"
                        .into(),
                });
            }

            if target != "browser"
                && reasons.is_empty()
                && self.uses_standard_wasi_network()
                && !self.supports_standard_wasi_network_profile()
            {
                reasons.push(TargetSupportReason {
                    code: "unsupported-standard-wasi-network-profile".into(),
                    wit: format!("{world_identity}/wasi-network"),
                    detail: "standard WASI networking requires the exact pinned HTTP outgoing-handler and sockets instance-network/network profile"
                        .into(),
                });
            }
            targets.push(TargetSupport {
                target: target.into(),
                supported: reasons.is_empty(),
                reasons,
            });
        }
        Ok(TargetSupportMatrix { schema: 1, targets })
    }

    pub fn binding_plan(&self) -> Result<BindingPlan> {
        self.target_support_matrix()?.require_supported()?;
        if let Some(error_context) = self.error_context_surface() {
            return Ok(BindingPlan {
                aliases: Vec::new(),
                functions: Vec::new(),
                resource: None,
                future: None,
                stream: None,
                error_context: Some(error_context),
                network: self.supports_standard_wasi_network_profile(),
            });
        }
        if let Some(stream) = self.stream_u32_surface() {
            return Ok(BindingPlan {
                aliases: Vec::new(),
                functions: Vec::new(),
                resource: None,
                future: None,
                stream: Some(stream),
                error_context: None,
                network: self.supports_standard_wasi_network_profile(),
            });
        }
        if let Some(future) = self.future_u32_surface() {
            return Ok(BindingPlan {
                aliases: Vec::new(),
                functions: Vec::new(),
                resource: None,
                future: Some(future),
                stream: None,
                error_context: None,
                network: self.supports_standard_wasi_network_profile(),
            });
        }
        if let Some((interface, resource)) = self.exports.iter().find_map(|interface| {
            interface
                .types
                .iter()
                .find_map(|ty| matches!(ty.ty, AbiType::Resource).then_some((interface, ty)))
        }) {
            ensure!(
                self.exports.len() == 1,
                "resource runtime requires one exported interface"
            );
            ensure!(
                interface.types.len() == 1,
                "resource runtime requires one exported resource type"
            );
            let expected_result = AbiType::Result {
                ok: Some(Box::new(AbiType::List(Box::new(AbiType::String)))),
                err: Some(Box::new(AbiType::String)),
            };
            let mut functions = Vec::new();
            for function in interface
                .functions
                .iter()
                .filter(|function| function.owner_resource.as_deref() == Some(&resource.name))
            {
                ensure!(
                    function.asynchronous == AbiAsyncCategory::Sync,
                    "resource runtime only supports synchronous functions"
                );
                let is_constructor = function.kind == AbiFunctionKind::Constructor;
                if is_constructor {
                    ensure!(
                        function.result == Some(AbiType::Own(resource.name.clone())),
                        "resource constructor must return own<{}>",
                        resource.name
                    );
                } else {
                    ensure!(
                        function.result.as_ref() == Some(&expected_result),
                        "resource methods require result<list<string>, string> results"
                    );
                }
                let params = function
                    .params
                    .iter()
                    .map(|(name, ty)| {
                        let value = match ty {
                            AbiType::Own(owner) if owner == &resource.name => {
                                BindingResourceParameterKind::Own
                            }
                            AbiType::Borrow(owner) if owner == &resource.name => {
                                BindingResourceParameterKind::Borrow
                            }
                            _ => BindingResourceParameterKind::Scalar(
                                supported_scalar(ty, &interface.types)?
                                    .context("unsupported resource scalar parameter")?,
                            ),
                        };
                        Ok(BindingResourceParameter {
                            go: lower_camel(name),
                            rust: rust_identifier(name),
                            value,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                functions.push(BindingResourceFunction {
                    wit_name: function.name.clone(),
                    item_name: function.item_name.clone(),
                    kind: function.kind.clone(),
                    go_name: upper_camel(&function.item_name),
                    rust_name: rust_identifier(&function.item_name),
                    js_name: lower_camel(&function.item_name),
                    params,
                });
            }
            return Ok(BindingPlan {
                aliases: Vec::new(),
                functions: Vec::new(),
                resource: Some(BindingResource {
                    interface: interface.package.rsplit_once('@').map_or_else(
                        || format!("{}/{}", interface.package, interface.name),
                        |(package, version)| format!("{}/{}@{}", package, interface.name, version),
                    ),
                    wit_name: resource.name.clone(),
                    target_name: upper_camel(&resource.name),
                    functions,
                }),
                future: None,
                stream: None,
                error_context: None,
                network: self.supports_standard_wasi_network_profile(),
            });
        }
        ensure!(
            self.exports.len() == 1,
            "the fixed binding profile requires exactly one world-level export group"
        );
        let interface = &self.exports[0];
        ensure!(
            interface.package == self.package && interface.name == format!("{}-world", self.world),
            "the fixed binding profile only supports world-level exports"
        );
        ensure!(
            interface.types.iter().all(|ty| {
                supported_value(&AbiType::Named(ty.name.clone()), &interface.types)
                    .is_ok_and(|value| value.is_some())
            }),
            "the fixed binding profile only supports scalar aliases and named flat records, enums, flags, variants, scalar options, or scalar results"
        );
        ensure!(
            !interface.functions.is_empty(),
            "the fixed binding profile requires at least one exported function"
        );

        let expected_result = AbiType::Result {
            ok: Some(Box::new(AbiType::List(Box::new(AbiType::String)))),
            err: Some(Box::new(AbiType::String)),
        };
        let aliases = interface
            .types
            .iter()
            .map(|ty| {
                Ok(BindingAlias {
                    target_name: upper_camel(&ty.name),
                    value: supported_value(&AbiType::Named(ty.name.clone()), &interface.types)?
                        .context("unsupported binding alias")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut functions = Vec::new();
        let mut go_names = std::collections::BTreeSet::new();
        let mut rust_names = std::collections::BTreeSet::new();
        let mut js_names = std::collections::BTreeSet::new();
        for function in &interface.functions {
            ensure!(
                function.kind == AbiFunctionKind::Freestanding && function.owner_resource.is_none(),
                "the fixed binding profile only supports freestanding exports"
            );
            ensure!(
                function.asynchronous == AbiAsyncCategory::Sync,
                "the fixed binding profile only supports synchronous exports"
            );
            ensure!(
                function.params.iter().all(|(_, ty)| {
                    supported_value(ty, &interface.types).is_ok_and(|value| value.is_some())
                }),
                "the fixed binding profile only supports scalar or named flat record, enum, flags, variant, scalar option, or scalar result parameters"
            );
            ensure!(
                function
                    .result
                    .as_ref()
                    .map(|ty| resolve_alias(ty, &interface.types))
                    .transpose()?
                    .as_ref()
                    == Some(&expected_result),
                "the fixed binding profile requires result<list<string>, string> results"
            );
            ensure!(
                !function.name.as_bytes().contains(&0),
                "export name contains NUL"
            );
            let go_name = upper_camel(&function.item_name);
            let rust_name = rust_identifier(&function.item_name);
            let js_name = lower_camel(&function.item_name);
            ensure!(
                !go_name.is_empty() && !rust_name.is_empty() && !js_name.is_empty(),
                "empty target function name"
            );
            ensure!(
                go_names.insert(go_name.clone()),
                "colliding Go export name: {go_name}"
            );
            ensure!(
                rust_names.insert(rust_name.clone()),
                "colliding Rust export name: {rust_name}"
            );
            ensure!(
                !js_reserved(&js_name),
                "reserved JavaScript export name: {js_name}"
            );
            ensure!(
                js_names.insert(js_name.clone()),
                "colliding JavaScript export name: {js_name}"
            );
            let mut go_params = std::collections::BTreeSet::new();
            let mut rust_params = std::collections::BTreeSet::new();
            let params = function
                .params
                .iter()
                .map(|(name, ty)| {
                    let go = lower_camel(name);
                    let rust = rust_identifier(name);
                    ensure!(
                        !go.is_empty() && !rust.is_empty(),
                        "empty target parameter name"
                    );
                    ensure!(
                        go_params.insert(go.clone()),
                        "colliding Go parameter name: {go}"
                    );
                    ensure!(
                        rust_params.insert(rust.clone()),
                        "colliding Rust parameter name: {rust}"
                    );
                    Ok(BindingParameter {
                        go,
                        rust,
                        value: supported_value(ty, &interface.types)?
                            .context("unsupported binding parameter")?,
                        alias: match ty {
                            AbiType::Named(name) => Some(upper_camel(name)),
                            _ => None,
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            functions.push(BindingFunction {
                wit_name: function.name.clone(),
                go_name,
                rust_name,
                js_name,
                params,
            });
        }
        Ok(BindingPlan {
            aliases,
            functions,
            resource: None,
            future: None,
            stream: None,
            error_context: None,
            network: self.supports_standard_wasi_network_profile(),
        })
    }

    fn stream_u32_surface(&self) -> Option<BindingStream> {
        let [interface] = self.exports.as_slice() else {
            return None;
        };
        if !self.imports.is_empty() || interface.types.len() != 2 || interface.functions.len() != 3
        {
            return None;
        }
        let operation = interface
            .types
            .iter()
            .find(|ty| ty.name == "stream-operation")?;
        let error = interface
            .types
            .iter()
            .find(|ty| ty.name == "operation-error")?;
        if operation.ty
            != AbiType::Record(vec![
                (
                    "value".into(),
                    AbiType::Stream(Some(Box::new(AbiType::U32))),
                ),
                ("driver".into(), AbiType::U32),
            ])
            || error.ty != AbiType::Variant(vec![("capacity".into(), None)])
        {
            return None;
        }
        let function = |name: &str| {
            interface
                .functions
                .iter()
                .find(|function| function.item_name == name)
        };
        let create = function("stream-values")?;
        let drive = function("stream-drive")?;
        let cancel = function("stream-cancel")?;
        let driver_shape = |function: &AbiFunction| {
            function.kind == AbiFunctionKind::Freestanding
                && function.owner_resource.is_none()
                && function.asynchronous == AbiAsyncCategory::Async
                && function.params == vec![("driver".into(), AbiType::U32)]
                && function.result == Some(AbiType::U32)
        };
        if create.kind != AbiFunctionKind::Freestanding
            || create.owner_resource.is_some()
            || create.asynchronous != AbiAsyncCategory::Sync
            || create.params
                != vec![
                    ("first".into(), AbiType::U32),
                    ("second".into(), AbiType::U32),
                ]
            || create.result
                != Some(AbiType::Result {
                    ok: Some(Box::new(AbiType::Named("stream-operation".into()))),
                    err: Some(Box::new(AbiType::Named("operation-error".into()))),
                })
            || !driver_shape(drive)
            || !driver_shape(cancel)
        {
            return None;
        }
        Some(BindingStream {
            interface: interface.package.rsplit_once('@').map_or_else(
                || format!("{}/{}", interface.package, interface.name),
                |(package, version)| format!("{package}/{}@{version}", interface.name),
            ),
            create: create.name.clone(),
            drive: drive.name.clone(),
            cancel: cancel.name.clone(),
        })
    }

    fn error_context_surface(&self) -> Option<BindingErrorContext> {
        let [interface] = self.exports.as_slice() else {
            return None;
        };
        if !self.imports.is_empty() || !interface.types.is_empty() || interface.functions.len() != 2
        {
            return None;
        }
        let function = |name: &str| interface.functions.iter().find(|f| f.item_name == name);
        let create = function("error-value")?;
        let debug = function("debug-message")?;
        let freestanding = |f: &AbiFunction| {
            f.kind == AbiFunctionKind::Freestanding
                && f.owner_resource.is_none()
                && f.asynchronous == AbiAsyncCategory::Sync
        };
        if !freestanding(create)
            || !create.params.is_empty()
            || create.result != Some(AbiType::ErrorContext)
            || !freestanding(debug)
            || debug.params != vec![("value".into(), AbiType::ErrorContext)]
            || debug.result != Some(AbiType::String)
        {
            return None;
        }
        Some(BindingErrorContext {
            interface: interface.package.rsplit_once('@').map_or_else(
                || format!("{}/{}", interface.package, interface.name),
                |(package, version)| format!("{package}/{}@{version}", interface.name),
            ),
            create: create.name.clone(),
            debug: debug.name.clone(),
        })
    }

    fn future_u32_surface(&self) -> Option<BindingFuture> {
        let [interface] = self.exports.as_slice() else {
            return None;
        };
        if !self.imports.is_empty() || interface.types.len() != 2 || interface.functions.len() != 3
        {
            return None;
        }
        let operation = interface
            .types
            .iter()
            .find(|ty| ty.name == "future-operation")?;
        let error = interface
            .types
            .iter()
            .find(|ty| ty.name == "operation-error")?;
        if operation.ty
            != AbiType::Record(vec![
                (
                    "value".into(),
                    AbiType::Future(Some(Box::new(AbiType::U32))),
                ),
                ("driver".into(), AbiType::U32),
            ])
            || error.ty != AbiType::Variant(vec![("capacity".into(), None)])
        {
            return None;
        }
        let function = |name: &str| {
            interface
                .functions
                .iter()
                .find(|function| function.item_name == name)
        };
        let create = function("future-value")?;
        let drive = function("future-drive")?;
        let cancel = function("future-cancel")?;
        let driver_shape = |function: &AbiFunction| {
            function.kind == AbiFunctionKind::Freestanding
                && function.owner_resource.is_none()
                && function.asynchronous == AbiAsyncCategory::Async
                && function.params == vec![("driver".into(), AbiType::U32)]
                && function.result == Some(AbiType::U32)
        };
        if create.kind != AbiFunctionKind::Freestanding
            || create.owner_resource.is_some()
            || create.asynchronous != AbiAsyncCategory::Sync
            || create.params != vec![("value".into(), AbiType::U32)]
            || create.result
                != Some(AbiType::Result {
                    ok: Some(Box::new(AbiType::Named("future-operation".into()))),
                    err: Some(Box::new(AbiType::Named("operation-error".into()))),
                })
            || !driver_shape(drive)
            || !driver_shape(cancel)
        {
            return None;
        }
        Some(BindingFuture {
            interface: interface.package.rsplit_once('@').map_or_else(
                || format!("{}/{}", interface.package, interface.name),
                |(package, version)| format!("{package}/{}@{version}", interface.name),
            ),
            create: create.name.clone(),
            drive: drive.name.clone(),
            cancel: cancel.name.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingParameter {
    go: String,
    rust: String,
    value: BindingValue,
    alias: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum BindingValue {
    Scalar(BindingScalar),
    Record(BindingRecord),
    Enum(BindingEnum),
    Flags(BindingFlags),
    Variant(BindingVariant),
    Option(BindingOption),
    Result(BindingResult),
    List(BindingList),
    Tuple(BindingTuple),
}

#[derive(Clone, Debug, PartialEq)]
struct BindingTuple {
    target_name: String,
    elements: Vec<BindingScalar>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingList {
    target_name: String,
    element: BindingScalar,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingResult {
    target_name: String,
    ok: Option<BindingScalar>,
    err: Option<BindingScalar>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingOption {
    target_name: String,
    payload: BindingScalar,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingVariant {
    target_name: String,
    cases: Vec<BindingVariantCase>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingVariantCase {
    wit_name: String,
    target_name: String,
    payload: Option<BindingScalar>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingFlags {
    target_name: String,
    names: Vec<BindingFlag>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingFlag {
    wit_name: String,
    go_name: String,
    rust_name: String,
    js_name: String,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingEnum {
    target_name: String,
    cases: Vec<BindingEnumCase>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingEnumCase {
    wit_name: String,
    target_name: String,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingRecord {
    target_name: String,
    fields: Vec<BindingRecordField>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingRecordField {
    wit_name: String,
    go_name: String,
    rust_name: String,
    js_name: String,
    scalar: BindingScalar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingScalar {
    String,
    U8,
    U16,
    U32,
    U64,
    I64,
    Bool,
    I8,
    I16,
    I32,
    Char,
    F32,
    F64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingAlias {
    target_name: String,
    value: BindingValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingFunction {
    pub wit_name: String,
    go_name: String,
    rust_name: String,
    js_name: String,
    params: Vec<BindingParameter>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingResourceParameter {
    go: String,
    rust: String,
    value: BindingResourceParameterKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingResourceParameterKind {
    Scalar(BindingScalar),
    Own,
    Borrow,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingResourceFunction {
    wit_name: String,
    item_name: String,
    kind: AbiFunctionKind,
    go_name: String,
    rust_name: String,
    js_name: String,
    params: Vec<BindingResourceParameter>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingResource {
    interface: String,
    wit_name: String,
    target_name: String,
    functions: Vec<BindingResourceFunction>,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingFuture {
    interface: String,
    create: String,
    drive: String,
    cancel: String,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingStream {
    interface: String,
    create: String,
    drive: String,
    cancel: String,
}

#[derive(Clone, Debug, PartialEq)]
struct BindingErrorContext {
    interface: String,
    create: String,
    debug: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingPlan {
    aliases: Vec<BindingAlias>,
    pub functions: Vec<BindingFunction>,
    resource: Option<BindingResource>,
    future: Option<BindingFuture>,
    stream: Option<BindingStream>,
    error_context: Option<BindingErrorContext>,
    network: bool,
}

fn scalar_shape(scalar: BindingScalar) -> &'static str {
    match scalar {
        BindingScalar::String => "string",
        BindingScalar::U8 => "u8",
        BindingScalar::U16 => "u16",
        BindingScalar::U32 => "u32",
        BindingScalar::U64 => "u64",
        BindingScalar::I64 => "s64",
        BindingScalar::Bool => "bool",
        BindingScalar::I8 => "s8",
        BindingScalar::I16 => "s16",
        BindingScalar::I32 => "s32",
        BindingScalar::Char => "char",
        BindingScalar::F32 => "f32",
        BindingScalar::F64 => "f64",
    }
}

impl BindingPlan {
    pub fn executable_export_shape(&self) -> serde_json::Value {
        if let Some(stream) = &self.stream {
            return serde_json::json!([{
                "interface": stream.interface,
                "stream": "u32",
                "functions": [
                    {"name": stream.create, "kind": "freestanding", "params": ["u32", "u32"], "result": {"stream":"u32"}},
                    {"name": stream.drive, "kind": "freestanding", "params": ["u32"], "result": "u32", "async": true},
                    {"name": stream.cancel, "kind": "freestanding", "params": ["u32"], "result": "u32", "async": true}
                ]
            }]);
        }
        if let Some(value) = &self.error_context {
            return serde_json::json!([{
                "interface": value.interface,
                "errorContext": true,
                "functions": [
                    {"name": value.create, "kind": "freestanding", "params": [], "result": "error-context"},
                    {"name": value.debug, "kind": "freestanding", "params": ["error-context"], "result": "string"}
                ]
            }]);
        }
        if let Some(future) = &self.future {
            return serde_json::json!([{
                "interface": future.interface,
                "future": "u32",
                "functions": [
                    {"name": future.create, "kind": "freestanding", "params": ["u32"], "result": {"future":"u32"}},
                    {"name": future.drive, "kind": "freestanding", "params": ["u32"], "result": "u32", "async": true},
                    {"name": future.cancel, "kind": "freestanding", "params": ["u32"], "result": "u32", "async": true}
                ]
            }]);
        }
        if let Some(resource) = &self.resource {
            return serde_json::json!([{
                "interface": resource.interface,
                "resource": resource.wit_name,
                "functions": resource.functions.iter().map(|function| {
                    let result = if function.kind == AbiFunctionKind::Constructor {
                        serde_json::json!({"own": resource.wit_name})
                    } else {
                        serde_json::json!({"result":{"ok":{"list":"string"},"err":"string"}})
                    };
                    serde_json::json!({
                        "name": function.wit_name,
                        "kind": function.kind,
                        "params": function.params.iter().map(|parameter| match parameter.value {
                            BindingResourceParameterKind::Scalar(scalar) => serde_json::Value::String(scalar_shape(scalar).into()),
                            BindingResourceParameterKind::Own => serde_json::json!({"own":resource.wit_name}),
                            BindingResourceParameterKind::Borrow => serde_json::json!({"borrow":resource.wit_name}),
                        }).collect::<Vec<_>>(),
                        "result": result,
                    })
                }).collect::<Vec<_>>()
            }]);
        }
        let result = serde_json::json!({"result":{"ok":{"list":"string"},"err":"string"}});
        let mut functions = self
            .functions
            .iter()
            .map(|function| {
                let params = function
                    .params
                    .iter()
                    .map(|parameter| match &parameter.value {
                        BindingValue::Scalar(scalar) => {
                            serde_json::Value::String(scalar_shape(*scalar).into())
                        }
                        BindingValue::Record(record) => serde_json::json!({
                            "record": {
                                "fields": record.fields.iter().map(|field| {
                                    serde_json::json!({
                                        "name": field.wit_name,
                                        "type": scalar_shape(field.scalar),
                                    })
                                }).collect::<Vec<_>>()
                            }
                        }),
                        BindingValue::Enum(enum_) => serde_json::json!({
                            "enum": {"cases": enum_.cases.iter().map(|case| {
                                serde_json::json!({"name": case.wit_name})
                            }).collect::<Vec<_>>()}
                        }),
                        BindingValue::Flags(flags) => serde_json::json!({
                            "flags": {"flags": flags.names.iter().map(|flag| {
                                serde_json::json!({"name": flag.wit_name})
                            }).collect::<Vec<_>>()}
                        }),
                        BindingValue::Variant(variant) => serde_json::json!({
                            "variant": {"cases": variant.cases.iter().map(|case| {
                                match case.payload {
                                    Some(payload) => serde_json::json!({
                                        "name": case.wit_name,
                                        "type": scalar_shape(payload),
                                    }),
                                    None => serde_json::json!({
                                        "name": case.wit_name,
                                        "type": serde_json::Value::Null,
                                    }),
                                }
                            }).collect::<Vec<_>>()}
                        }),
                        BindingValue::Option(option) => serde_json::json!({
                            "option": scalar_shape(option.payload)
                        }),
                        BindingValue::Result(result) => serde_json::json!({
                            "result": {
                                "ok": result.ok.map(|value| scalar_shape(value)),
                                "err": result.err.map(|value| scalar_shape(value)),
                            }
                        }),
                        BindingValue::List(list) => serde_json::json!({
                            "list": scalar_shape(list.element)
                        }),
                        BindingValue::Tuple(tuple) => serde_json::json!({
                            "tuple": {
                                "types": tuple.elements.iter().map(|element| scalar_shape(*element)).collect::<Vec<_>>()
                            }
                        }),
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "name": function.wit_name,
                    "params": params,
                    "result": result,
                })
            })
            .collect::<Vec<_>>();
        functions.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        serde_json::Value::Array(functions)
    }
}

fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split(['-', '_']).filter(|part| !part.is_empty())
}

fn upper_camel(name: &str) -> String {
    words(name)
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|head| head.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn lower_camel(name: &str) -> String {
    let mut parts = words(name);
    let Some(first) = parts.next() else {
        return String::new();
    };
    first.to_owned() + &parts.map(upper_camel).collect::<String>()
}

fn go_reserved(name: &str) -> bool {
    [
        "break",
        "default",
        "func",
        "interface",
        "select",
        "case",
        "defer",
        "go",
        "map",
        "struct",
        "chan",
        "else",
        "goto",
        "package",
        "switch",
        "const",
        "fallthrough",
        "if",
        "range",
        "type",
        "continue",
        "for",
        "import",
        "return",
        "var",
    ]
    .contains(&name)
}

fn js_reserved(name: &str) -> bool {
    [
        "await",
        "arguments",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "eval",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "function",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "null",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ]
    .contains(&name)
}

fn rust_identifier(name: &str) -> String {
    let value = name.replace('-', "_");
    if [
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn",
    ]
    .contains(&value.as_str())
    {
        format!("r#{value}")
    } else {
        value
    }
}
