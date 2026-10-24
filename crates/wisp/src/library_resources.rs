//! Resource ownership wrappers shared by native and Component-guest libraries.
use anyhow::{Context, Result, bail, ensure};
use heck::ToUpperCamelCase;
use wit_parser::{Function, FunctionKind, Handle, InterfaceId, Resolve, Type, TypeDefKind, TypeId};

use crate::library::{guest_argument, ident, rust_type};

pub fn is_resource(resolve: &Resolve, id: TypeId) -> bool {
    matches!(resolve.types[id].kind, TypeDefKind::Resource)
}

fn canonical_resource(resolve: &Resolve, mut id: TypeId) -> Result<TypeId> {
    for _ in 0..64 {
        match resolve.types[id].kind {
            TypeDefKind::Resource => return Ok(id),
            TypeDefKind::Type(Type::Id(next)) => id = next,
            _ => bail!("invalid library resource alias"),
        }
    }
    bail!("library resource type depth limit")
}

fn handle(resolve: &Resolve, ty: &Type) -> Option<(TypeId, bool)> {
    let Type::Id(id) = ty else { return None };
    match resolve.types[*id].kind {
        TypeDefKind::Handle(Handle::Own(resource)) => Some((resource, true)),
        TypeDefKind::Handle(Handle::Borrow(resource)) => Some((resource, false)),
        _ => None,
    }
}

fn children(resolve: &Resolve, ty: &Type) -> Vec<Type> {
    let Type::Id(id) = ty else { return Vec::new() };
    match &resolve.types[*id].kind {
        TypeDefKind::Type(t) | TypeDefKind::Option(t) | TypeDefKind::List(t) => vec![*t],
        TypeDefKind::Record(r) => r.fields.iter().map(|f| f.ty).collect(),
        TypeDefKind::Variant(v) => v.cases.iter().filter_map(|c| c.ty).collect(),
        TypeDefKind::Tuple(t) => t.types.clone(),
        TypeDefKind::Result(r) => r.ok.iter().chain(r.err.iter()).copied().collect(),
        TypeDefKind::Future(t) | TypeDefKind::Stream(t) => t.iter().copied().collect(),
        _ => Vec::new(),
    }
}

fn has_handle(resolve: &Resolve, ty: &Type, own: Option<bool>, depth: usize) -> Result<bool> {
    handle_presence(
        resolve,
        ty,
        own,
        depth,
        &mut std::collections::BTreeMap::new(),
    )
}

fn handle_presence(
    resolve: &Resolve,
    ty: &Type,
    own: Option<bool>,
    depth: usize,
    cache: &mut std::collections::BTreeMap<TypeId, bool>,
) -> Result<bool> {
    ensure!(depth < 64, "library resource type depth limit");
    if let Some((_, is_owned)) = handle(resolve, ty) {
        return Ok(own.is_none_or(|own| own == is_owned));
    }
    if let Type::Id(id) = ty {
        if is_resource(resolve, *id) {
            return Ok(own != Some(false));
        }
        if let Some(found) = cache.get(id) {
            return Ok(*found);
        }
    }
    let mut found = false;
    for child in children(resolve, ty) {
        if handle_presence(resolve, &child, own, depth + 1, cache)? {
            found = true;
            break;
        }
    }
    if let Type::Id(id) = ty {
        cache.insert(*id, found);
    }
    Ok(found)
}

fn expanded_value_cost(
    resolve: &Resolve,
    ty: &Type,
    depth: usize,
    cache: &mut std::collections::BTreeMap<TypeId, usize>,
) -> Result<usize> {
    ensure!(depth < 64, "library resource type depth limit");
    if let Type::Id(id) = ty {
        if let Some(cost) = cache.get(id) {
            return Ok(*cost);
        }
    }
    let mut cost = 1usize;
    for child in children(resolve, ty) {
        cost = cost
            .saturating_add(expanded_value_cost(resolve, &child, depth + 1, cache)?)
            .min(4097);
    }
    if let Type::Id(id) = ty {
        cache.insert(*id, cost);
    }
    Ok(cost)
}

pub fn custom_type(resolve: &Resolve, id: TypeId) -> Result<bool> {
    has_handle(resolve, &Type::Id(id), None, 0)
}

fn contains_handle(resolve: &Resolve, ty: &Type, depth: usize) -> Result<bool> {
    has_handle(resolve, ty, None, depth)
}

fn name(resolve: &Resolve, id: TypeId) -> Result<String> {
    let id = canonical_resource(resolve, id)?;
    Ok(resolve.types[id]
        .name
        .as_ref()
        .context("unnamed library resource")?
        .to_upper_camel_case())
}

fn type_path(resolve: &Resolve, id: TypeId, backend: Option<bool>) -> Result<String> {
    let def = &resolve.types[id];
    let wit_parser::TypeOwner::Interface(owner) = def.owner else {
        bail!("unsupported library resource type owner")
    };
    let interface = &resolve.interfaces[owner];
    let module = ident(interface.name.as_ref().context("resource interface name")?);
    let name = def
        .name
        .as_ref()
        .context("named resource value")?
        .to_upper_camel_case();
    Ok(if let Some(native) = backend {
        let package = &resolve.packages[interface.package.context("interface package")?].name;
        format!(
            "super::{}::{}::{}::{module}::{name}",
            if native { "host::exports" } else { "guest" },
            ident(&package.namespace),
            ident(&package.name)
        )
    } else {
        format!("super::{module}::{name}")
    })
}

fn type_repr(
    resolve: &Resolve,
    ty: &Type,
    native: bool,
    parameter: bool,
    lifetime: &str,
    expand: bool,
    depth: usize,
) -> Result<String> {
    ensure!(depth < 64, "library resource type depth limit");
    if let Some((id, own)) = handle(resolve, ty) {
        ensure!(
            own || parameter,
            "unsupported-library-resource-borrow-result"
        );
        return Ok(if native {
            "wasmtime::component::ResourceAny".into()
        } else {
            format!(
                "{}{}",
                if own {
                    String::new()
                } else {
                    format!("&{lifetime} ")
                },
                type_path(resolve, canonical_resource(resolve, id)?, None)?
            )
        });
    }
    if !contains_handle(resolve, ty, depth)? {
        return rust_type(resolve, ty, 0);
    }
    let Type::Id(id) = ty else { unreachable!() };
    let def = &resolve.types[*id];
    if def.name.is_some() && !expand {
        let mut result = type_path(resolve, *id, if native { Some(true) } else { None })?;
        if !native && has_handle(resolve, ty, Some(false), 0)? {
            result.push_str(&format!("<{lifetime}>"));
        }
        return Ok(result);
    }
    let recur = |ty: &Type| type_repr(resolve, ty, native, parameter, lifetime, false, depth + 1);
    Ok(match &def.kind {
        TypeDefKind::Resource => type_path(resolve, *id, None)?,
        TypeDefKind::Type(ty) => recur(ty)?,
        TypeDefKind::List(ty) => format!("Vec<{}>", recur(ty)?),
        TypeDefKind::Option(ty) => format!("Option<{}>", recur(ty)?),
        TypeDefKind::Tuple(tuple) => format!(
            "({})",
            tuple
                .types
                .iter()
                .map(|t| Ok(format!("{},", recur(t)?)))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
        TypeDefKind::Result(result) => format!(
            "Result<{}, {}>",
            result
                .ok
                .as_ref()
                .map(&recur)
                .transpose()?
                .unwrap_or("()".into()),
            result
                .err
                .as_ref()
                .map(&recur)
                .transpose()?
                .unwrap_or("()".into())
        ),
        _ => bail!("unsupported-library-resource-value: {:?}", def.kind),
    })
}

fn value_type(resolve: &Resolve, ty: &Type, native: bool, parameter: bool) -> Result<String> {
    if !parameter {
        ensure!(
            !has_handle(resolve, ty, Some(false), 0)?,
            "unsupported-library-resource-borrow-result"
        );
    }
    type_repr(resolve, ty, native, parameter, "'_", false, 0)
}

fn optional_value_type(resolve: &Resolve, ty: Option<&Type>, native: bool) -> Result<String> {
    ty.map(|ty| value_type(resolve, ty, native, false))
        .unwrap_or_else(|| Ok("()".into()))
}

// Validate all nested references before taking any owned handle.
fn check(resolve: &Resolve, ty: &Type, expression: &str, depth: usize) -> Result<String> {
    ensure!(depth < 64, "library resource type depth limit");
    if handle(resolve, ty).is_some() {
        return Ok(format!(
            "if !__wisp_library.same_instance(&({expression}).library) {{ return Err(CallError::Runtime(\"resource belongs to another Component instance\".into())); }} if ({expression}).handle.is_none() {{ return Err(CallError::Runtime(\"resource is closed\".into())); }}\n"
        ));
    }
    if !contains_handle(resolve, ty, depth)? {
        return Ok(String::new());
    }
    let Type::Id(id) = ty else { unreachable!() };
    let recur = |ty: &Type, expr: &str| check(resolve, ty, expr, depth + 1);
    Ok(match &resolve.types[*id].kind {
        TypeDefKind::Type(ty) => recur(ty, expression)?,
        TypeDefKind::List(ty) => format!(
            "for __wisp_item in ({expression}).iter() {{ {} }}",
            recur(ty, "__wisp_item")?
        ),
        TypeDefKind::Option(ty) => format!(
            "if let Some(__wisp_item) = &({expression}) {{ {} }}",
            recur(ty, "__wisp_item")?
        ),
        TypeDefKind::Record(record) => record
            .fields
            .iter()
            .map(|f| recur(&f.ty, &format!("({expression}).{}", ident(&f.name))))
            .collect::<Result<Vec<_>>>()?
            .join("\n"),
        TypeDefKind::Tuple(tuple) => tuple
            .types
            .iter()
            .enumerate()
            .map(|(i, t)| recur(t, &format!("({expression}).{i}")))
            .collect::<Result<Vec<_>>>()?
            .join("\n"),
        TypeDefKind::Result(result) => format!(
            "match &({expression}) {{ Ok(__wisp_item) => {{ {} }}, Err(__wisp_item) => {{ {} }} }}",
            result
                .ok
                .as_ref()
                .map(|t| recur(t, "__wisp_item"))
                .transpose()?
                .unwrap_or_default(),
            result
                .err
                .as_ref()
                .map(|t| recur(t, "__wisp_item"))
                .transpose()?
                .unwrap_or_default()
        ),
        TypeDefKind::Variant(variant) => format!(
            "match &({expression}) {{ {} }}",
            variant
                .cases
                .iter()
                .map(|c| Ok(format!(
                    "{}::{}{} => {{ {} }},",
                    type_path(resolve, *id, None)?,
                    c.name.to_upper_camel_case(),
                    if c.ty.is_some() { "(__wisp_item)" } else { "" },
                    c.ty.as_ref()
                        .map(|t| recur(t, "__wisp_item"))
                        .transpose()?
                        .unwrap_or_default()
                )))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
        _ => bail!(
            "unsupported-library-resource-value: {:?}",
            resolve.types[*id].kind
        ),
    })
}

fn convert(
    resolve: &Resolve,
    ty: &Type,
    expression: &str,
    native: bool,
    lift: bool,
    depth: usize,
) -> Result<String> {
    ensure!(depth < 64, "library resource type depth limit");
    if let Some((id, own)) = handle(resolve, ty) {
        return Ok(if lift {
            ensure!(own, "unsupported-library-resource-borrow-result");
            format!(
                "{} {{ library: __wisp_library.clone(), handle: Some({expression}) }}",
                type_path(resolve, canonical_resource(resolve, id)?, None)?
            )
        } else if own {
            format!("({expression}).handle.take().expect(\"checked resource\")")
        } else {
            format!(
                "{}({expression}).handle.as_ref().expect(\"checked resource\")",
                if native { "*" } else { "" }
            )
        });
    }
    if !contains_handle(resolve, ty, depth)? {
        return Ok(expression.into());
    }
    let Type::Id(id) = ty else { unreachable!() };
    let recur = |ty: &Type, expr: &str| convert(resolve, ty, expr, native, lift, depth + 1);
    let target = if lift { None } else { Some(native) };
    Ok(match &resolve.types[*id].kind {
        TypeDefKind::Type(ty) => recur(ty, expression)?,
        TypeDefKind::List(ty) => format!(
            "({expression}).into_iter().map(|mut __wisp_item| {}).collect::<Vec<_>>()",
            recur(ty, "__wisp_item")?
        ),
        TypeDefKind::Option(ty) => format!(
            "({expression}).map(|mut __wisp_item| {})",
            recur(ty, "__wisp_item")?
        ),
        TypeDefKind::Tuple(tuple) => format!(
            "({})",
            tuple
                .types
                .iter()
                .enumerate()
                .map(|(i, t)| Ok(format!("{},", recur(t, &format!("({expression}).{i}"))?)))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
        TypeDefKind::Record(record) => format!(
            "{} {{ {} }}",
            type_path(resolve, *id, target)?,
            record
                .fields
                .iter()
                .map(|f| Ok(format!(
                    "{}: {},",
                    ident(&f.name),
                    recur(&f.ty, &format!("({expression}).{}", ident(&f.name)))?
                )))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
        TypeDefKind::Result(result) => format!(
            "match {expression} {{ Ok(mut __wisp_item) => Ok({}), Err(mut __wisp_item) => Err({}) }}",
            result
                .ok
                .as_ref()
                .map(|t| recur(t, "__wisp_item"))
                .transpose()?
                .unwrap_or("()".into()),
            result
                .err
                .as_ref()
                .map(|t| recur(t, "__wisp_item"))
                .transpose()?
                .unwrap_or("()".into())
        ),
        TypeDefKind::Variant(variant) => format!(
            "match {expression} {{ {} }}",
            variant
                .cases
                .iter()
                .map(|c| Ok(format!(
                    "{}::{}{} => {}::{}{},",
                    type_path(resolve, *id, if lift { Some(native) } else { None })?,
                    c.name.to_upper_camel_case(),
                    if c.ty.is_some() {
                        "(mut __wisp_item)"
                    } else {
                        ""
                    },
                    type_path(resolve, *id, target)?,
                    c.name.to_upper_camel_case(),
                    c.ty.as_ref()
                        .map(|t| -> Result<String> {
                            Ok(format!("({})", recur(t, "__wisp_item")?))
                        })
                        .transpose()?
                        .unwrap_or_default()
                )))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
        _ => bail!(
            "unsupported-library-resource-value: {:?}",
            resolve.types[*id].kind
        ),
    })
}

fn wrap(resolve: &Resolve, ty: Option<&Type>, expression: &str, native: bool) -> Result<String> {
    ty.map(|ty| convert(resolve, ty, expression, native, true, 0))
        .unwrap_or_else(|| Ok(expression.into()))
}

/// Returns None when the ordinary value emitter should handle this function.
/// Resource members are emitted by definitions, so return an empty fragment here.
pub fn client_method(
    resolve: &Resolve,
    function: &Function,
    path: &str,
    accessor: &str,
) -> Result<Option<String>> {
    match function.kind {
        FunctionKind::Constructor(_) | FunctionKind::Method(_) | FunctionKind::Static(_) => {
            return Ok(Some(String::new()));
        }
        FunctionKind::Freestanding => (),
        _ => return Ok(None),
    }
    let mut resource = function
        .result
        .as_ref()
        .map(|ty| contains_handle(resolve, ty, 0))
        .transpose()?
        .unwrap_or(false);
    for p in &function.params {
        resource |= contains_handle(resolve, &p.ty, 0)?;
    }
    if !resource {
        return Ok(None);
    }
    Ok(Some(emit_function(resolve, function, path, accessor)?))
}

fn emit_function(
    resolve: &Resolve,
    function: &Function,
    path: &str,
    accessor: &str,
) -> Result<String> {
    ensure!(function.params.len() <= 64, "library parameter count limit");
    let mut cost = 0usize;
    let mut cache = std::collections::BTreeMap::new();
    for ty in function
        .params
        .iter()
        .map(|p| &p.ty)
        .chain(function.result.iter())
    {
        cost = cost.saturating_add(expanded_value_cost(resolve, ty, 0, &mut cache)?);
    }
    ensure!(cost <= 4096, "library resource expanded value limit");
    let (method, receiver, library, target, guest_target, skip) = match function.kind {
        FunctionKind::Freestanding => (
            ident(&function.name),
            "&mut self",
            "self.library.clone()",
            format!(
                "__wisp_bindings.{accessor}().func_{}()",
                ident(&function.name)
            ),
            format!("super::guest::{path}::{}", ident(&function.name)),
            0,
        ),
        FunctionKind::Constructor(id) => (
            "new".into(),
            "__wisp_client: &mut Client",
            "__wisp_client.library.clone()",
            format!(
                "__wisp_bindings.{accessor}().{}().func_constructor()",
                ident(resolve.types[id].name.as_ref().context("resource name")?)
            ),
            format!("super::guest::{path}::{}::new", name(resolve, id)?),
            0,
        ),
        FunctionKind::Static(id) => {
            let method = ident(
                function
                    .name
                    .rsplit('.')
                    .next()
                    .context("resource static method")?,
            );
            (
                method.clone(),
                "__wisp_client: &mut Client",
                "__wisp_client.library.clone()",
                format!(
                    "__wisp_bindings.{accessor}().{}().func_{method}()",
                    ident(resolve.types[id].name.as_ref().context("resource name")?)
                ),
                format!("super::guest::{path}::{}::{method}", name(resolve, id)?),
                0,
            )
        }
        FunctionKind::Method(id) => {
            let method = ident(
                function
                    .name
                    .rsplit('.')
                    .next()
                    .context("resource method")?,
            );
            (
                method.clone(),
                "&self",
                "self.library.clone()",
                format!(
                    "__wisp_bindings.{accessor}().{}().func_{method}()",
                    ident(resolve.types[id].name.as_ref().context("resource name")?)
                ),
                format!(
                    "self.handle.as_ref().ok_or_else(|| CallError::Runtime(\"resource is closed\".into()))?.{method}"
                ),
                1,
            )
        }
        _ => bail!("unsupported-library-async-resource-method"),
    };
    if skip != 0 {
        ensure!(
            function
                .params
                .first()
                .and_then(|p| handle(resolve, &p.ty))
                .is_some_and(|(_, own)| !own),
            "invalid library resource receiver"
        );
    }
    let mut params = Vec::new();
    let mut checks = String::new();
    let mut native_args = Vec::new();
    let mut native_types = Vec::new();
    let mut guest_args = Vec::new();
    let mut guest_lowering = String::new();
    for (index, p) in function.params.iter().enumerate() {
        let variable = if index < skip {
            "self".into()
        } else {
            format!("__wisp_arg_{index}")
        };
        let ty = value_type(resolve, &p.ty, false, true)?;
        if index >= skip {
            params.push(format!(
                "{}{}: {ty}",
                if has_handle(resolve, &p.ty, Some(true), 0)? {
                    "mut "
                } else {
                    ""
                },
                variable
            ));
        }
        native_types.push(format!("{},", value_type(resolve, &p.ty, true, true)?));
        checks.push_str(&check(resolve, &p.ty, &variable, 0)?);
        native_args.push(convert(resolve, &p.ty, &variable, true, false, 0)?);
        if index >= skip {
            if contains_handle(resolve, &p.ty, 0)? {
                let lowered = format!("__wisp_lowered_{index}");
                guest_lowering.push_str(&format!(
                    "let {lowered} = {};\n",
                    convert(resolve, &p.ty, &variable, false, false, 0)?
                ));
                let own = has_handle(resolve, &p.ty, Some(true), 0)?;
                let named = matches!(p.ty, Type::Id(id) if resolve.types[id].name.is_some());
                guest_args.push(if own {
                    lowered
                } else if named {
                    format!("&{lowered}")
                } else {
                    guest_argument(resolve, &p.ty, &lowered, false, 0)?
                });
            } else {
                guest_args.push(guest_argument(resolve, &p.ty, &variable, false, 0)?);
            }
        }
    }
    let result_def = match function.result {
        Some(Type::Id(id)) => match &resolve.types[id].kind {
            TypeDefKind::Result(result) => Some(result),
            _ => None,
        },
        _ => None,
    };
    let (ok, err) = if let Some(result) = result_def {
        (result.ok.as_ref(), result.err.as_ref())
    } else {
        (function.result.as_ref(), None)
    };
    let result = optional_value_type(resolve, ok, false)?;
    let error = if result_def.is_some() {
        optional_value_type(resolve, err, false)?
    } else {
        "std::convert::Infallible".into()
    };
    let native_result = if result_def.is_some() {
        format!(
            "Result<{}, {}>",
            optional_value_type(resolve, ok, true)?,
            optional_value_type(resolve, err, true)?
        )
    } else {
        optional_value_type(resolve, ok, true)?
    };
    let lift = |native| -> Result<String> {
        Ok(if result_def.is_some() {
            format!(
                "match value {{ Ok(value) => Ok({}), Err(value) => Err(CallError::Guest({})) }}",
                wrap(resolve, ok, "value", native)?,
                wrap(resolve, err, "value", native)?
            )
        } else {
            format!("Ok({})", wrap(resolve, ok, "value", native)?)
        })
    };
    let guest_lift = lift(false)?;
    let native_lift = lift(true)?;
    let native_result_tuple = if function.result.is_some() {
        format!("({native_result},)")
    } else {
        "()".into()
    };
    let native_unwrap = if function.result.is_some() {
        "|(value,)| value"
    } else {
        "|value| value"
    };
    let params = if params.is_empty() {
        String::new()
    } else {
        format!(", {}", params.join(", "))
    };
    Ok(format!(
        r#"
pub fn {method}({receiver}{params}) -> Result<{result}, CallError<{error}>> {{
    let __wisp_library = {library};
    {checks}
    #[cfg(target_arch = "wasm32")] {{
        {guest_lowering}
        let value = {guest_target}({guest_args});
        {guest_lift}
    }}
    #[cfg(not(target_arch = "wasm32"))] {{
        let mut __wisp_state = __wisp_library.access().map_err(CallError::Runtime)?;
        if __wisp_state.poisoned {{ return Err(CallError::Runtime("Component instance is poisoned".into())); }}
        let __wisp_result = {{
            let super::State {{ store: __wisp_store, bindings: __wisp_bindings, .. }} = &mut *__wisp_state;
            {target}.func().typed::<({native_types}), {native_result_tuple}>(&*__wisp_store)
                .and_then(|callee| callee.call(__wisp_store, ({native_args})))
                .map({native_unwrap})
        }};
        let value = __wisp_result.map_err(|error| {{ __wisp_state.poisoned = true; CallError::Runtime(format!("{{error:#}}")) }})?;
        {native_lift}
    }}
}}
"#,
        guest_args = guest_args.join(", "),
        native_types = native_types.join(" "),
        native_args = native_args
            .iter()
            .map(|a| format!("{a},"))
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

/// Emit resource structs beside Client, using the shared Library instance.
pub fn definitions(
    resolve: &Resolve,
    interface: InterfaceId,
    path: &str,
    accessor: &str,
) -> Result<String> {
    let interface = &resolve.interfaces[interface];
    let mut source = String::new();
    for (wit_name, id) in &interface.types {
        if !is_resource(resolve, *id) {
            if custom_type(resolve, *id)? {
                let ty = Type::Id(*id);
                let named = wit_name.to_upper_camel_case();
                let lifetime = if has_handle(resolve, &ty, Some(false), 0)? {
                    "<'a>"
                } else {
                    ""
                };
                let field_type = |ty: &Type| type_repr(resolve, ty, false, true, "'a", false, 0);
                match &resolve.types[*id].kind {
                    TypeDefKind::Record(record) => source.push_str(&format!(
                        "#[derive(Debug)]\npub struct {named}{lifetime} {{ {} }}\n",
                        record
                            .fields
                            .iter()
                            .map(|f| Ok(format!("pub {}: {},", ident(&f.name), field_type(&f.ty)?)))
                            .collect::<Result<Vec<_>>>()?
                            .join(" ")
                    )),
                    TypeDefKind::Variant(variant) => source.push_str(&format!(
                        "#[derive(Debug)]\npub enum {named}{lifetime} {{ {} }}\n",
                        variant
                            .cases
                            .iter()
                            .map(|c| Ok(format!(
                                "{}{},",
                                c.name.to_upper_camel_case(),
                                c.ty.as_ref()
                                    .map(|t| -> Result<String> {
                                        Ok(format!("({})", field_type(t)?))
                                    })
                                    .transpose()?
                                    .unwrap_or_default()
                            )))
                            .collect::<Result<Vec<_>>>()?
                            .join(" ")
                    )),
                    _ => source.push_str(&format!(
                        "pub type {named}{lifetime} = {};\n",
                        type_repr(resolve, &ty, false, true, "'a", true, 0)?
                    )),
                }
            }
            continue;
        }
        let resource = name(resolve, *id)?;
        let mut methods =
            std::collections::BTreeSet::from(["close".to_owned(), "is_closed".to_owned()]);
        source.push_str(&format!(r#"
pub struct {resource} {{
    pub(super) library: super::Library,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) handle: Option<wasmtime::component::ResourceAny>,
    #[cfg(target_arch = "wasm32")]
    pub(super) handle: Option<super::guest::{path}::{resource}>,
}}
impl std::fmt::Debug for {resource} {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.debug_struct({resource:?}).field("closed", &self.is_closed()).finish() }}
}}
impl {resource} {{
    pub fn is_closed(&self) -> bool {{ self.handle.is_none() }}
    pub fn close(&mut self) -> Result<(), CallError<std::convert::Infallible>> {{
        if self.handle.is_none() {{ return Err(CallError::Runtime("resource is closed".into())); }}
        #[cfg(target_arch = "wasm32")] {{ self.handle.take(); Ok(()) }}
        #[cfg(not(target_arch = "wasm32"))] {{
            let mut state = self.library.access().map_err(CallError::Runtime)?;
            let handle = self.handle.take().expect("checked resource");
            handle.resource_drop(&mut state.store).map_err(|error| {{ state.poisoned = true; CallError::Runtime(format!("{{error:#}}")) }})
        }}
    }}
"#));
        for function in interface.functions.values() {
            let owner = match function.kind {
                FunctionKind::Constructor(id)
                | FunctionKind::Method(id)
                | FunctionKind::Static(id) => Some(id),
                FunctionKind::AsyncMethod(owner) | FunctionKind::AsyncStatic(owner) => {
                    ensure!(owner != *id, "unsupported-library-async-resource-method");
                    None
                }
                _ => None,
            };
            if owner != Some(*id) {
                continue;
            }
            let method = if matches!(function.kind, FunctionKind::Constructor(_)) {
                "new".into()
            } else {
                ident(
                    function
                        .name
                        .rsplit('.')
                        .next()
                        .context("resource method name")?,
                )
            };
            ensure!(
                methods.insert(method),
                "library resource method name collision: {wit_name}: {}",
                function.name
            );
            source.push_str(&emit_function(resolve, function, path, accessor)?);
        }
        source.push_str(&format!(
            r#"
}}
impl Drop for {resource} {{
    fn drop(&mut self) {{
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(handle) = self.handle.take() {{
            self.library.defer(move |state| {{
                if handle.resource_drop(&mut state.store).is_err() {{ state.poisoned = true; }}
            }});
        }}
    }}
}}
"#
        ));
    }
    Ok(source)
}
