//! Component-model async support derived from canonical WIT types.
use anyhow::{Result, bail};
use std::collections::BTreeSet;
use wit_parser::{FunctionKind, Resolve, Type, TypeDefKind, TypeId, WorldId, WorldItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderKind {
    Future,
    Stream,
}

/// Resolve aliases before choosing the public reader wrapper.
pub fn reader(resolve: &Resolve, ty: &Type) -> Option<(ReaderKind, Option<Type>)> {
    let Type::Id(id) = ty else { return None };
    match &resolve.types[*id].kind {
        TypeDefKind::Type(ty) => reader(resolve, ty),
        TypeDefKind::Future(ty) => Some((ReaderKind::Future, *ty)),
        TypeDefKind::Stream(ty) => Some((ReaderKind::Stream, *ty)),
        _ => None,
    }
}

fn async_type(resolve: &Resolve, ty: &Type, visited: &mut BTreeSet<TypeId>) -> bool {
    let Type::Id(id) = ty else { return false };
    if !visited.insert(*id) {
        return false;
    }
    let mut check = |ty: &Type| async_type(resolve, ty, visited);
    match &resolve.types[*id].kind {
        TypeDefKind::Future(_) | TypeDefKind::Stream(_) => true,
        TypeDefKind::Type(ty) | TypeDefKind::List(ty) | TypeDefKind::Option(ty) => check(ty),
        TypeDefKind::Record(record) => record.fields.iter().any(|f| check(&f.ty)),
        TypeDefKind::Variant(variant) => variant
            .cases
            .iter()
            .filter_map(|c| c.ty.as_ref())
            .any(&mut check),
        TypeDefKind::Tuple(tuple) => tuple.types.iter().any(check),
        TypeDefKind::Result(result) => result.ok.iter().chain(result.err.iter()).any(check),
        _ => false,
    }
}

pub fn enabled(resolve: &Resolve, world: WorldId) -> bool {
    resolve.worlds[world]
        .exports
        .values()
        .chain(resolve.worlds[world].imports.values())
        .any(|item| {
            let WorldItem::Interface { id, .. } = item else {
                return false;
            };
            let interface = &resolve.interfaces[*id];
            interface.functions.values().any(|f| {
                matches!(
                    f.kind,
                    FunctionKind::AsyncFreestanding
                        | FunctionKind::AsyncMethod(_)
                        | FunctionKind::AsyncStatic(_)
                ) || f
                    .params
                    .iter()
                    .map(|p| &p.ty)
                    .chain(f.result.iter())
                    .any(|ty| async_type(resolve, ty, &mut BTreeSet::new()))
            })
        })
}

/// The current wrapper owns readers returned directly from exported calls.
/// Refuse nested handles until their generated records have equivalent ownership.
pub fn validate_function(resolve: &Resolve, function: &wit_parser::Function) -> Result<()> {
    for parameter in &function.params {
        if async_type(resolve, &parameter.ty, &mut BTreeSet::new()) {
            bail!(
                "unsupported-library-async-parameter: {}.{}",
                function.name,
                parameter.name
            );
        }
    }
    if let Some(ty) = &function.result {
        if async_type(resolve, ty, &mut BTreeSet::new()) {
            let Some((_, payload)) = reader(resolve, ty) else {
                bail!("unsupported-library-nested-async-result: {}", function.name);
            };
            if payload
                .as_ref()
                .is_some_and(|ty| async_type(resolve, ty, &mut BTreeSet::new()))
            {
                bail!(
                    "unsupported-library-nested-async-payload: {}",
                    function.name
                );
            }
        }
    }
    Ok(())
}

pub fn runtime_source() -> &'static str {
    include_str!("../assets/templates/library-async.rs")
}

pub fn client_method(
    resolve: &Resolve,
    function: &wit_parser::Function,
    path: &str,
    accessor: &str,
) -> Result<Option<String>> {
    use crate::library::{guest_argument, ident, rust_type};
    validate_function(resolve, function)?;
    let reader = function.result.as_ref().and_then(|ty| reader(resolve, ty));
    let asynchronous = matches!(function.kind, FunctionKind::AsyncFreestanding);
    if !asynchronous && reader.is_none() {
        return Ok(None);
    }
    anyhow::ensure!(
        matches!(
            function.kind,
            FunctionKind::Freestanding | FunctionKind::AsyncFreestanding
        ),
        "unsupported-library-async-resource-method"
    );
    let method = ident(&function.name);
    let optional = |ty: Option<&Type>| {
        ty.map(|ty| rust_type(resolve, ty, 0))
            .unwrap_or_else(|| Ok("()".into()))
    };
    let (public_result, native_result, wrap) = if let Some((kind, payload)) = reader {
        let payload = optional(payload.as_ref())?;
        let name = match kind {
            ReaderKind::Future => "Future",
            ReaderKind::Stream => "Stream",
        };
        (
            format!("super::{name}<{payload}>"),
            format!("wasmtime::component::{name}Reader<{payload}>"),
            format!("Ok(super::{name}::from_reader(self.library.clone(), value))"),
        )
    } else {
        let result = optional(function.result.as_ref())?;
        (result.clone(), result, "Ok(value)".into())
    };
    let mut error = "std::convert::Infallible".to_owned();
    let mut public_result = public_result;
    let mut wrap = wrap;
    if let Some(Type::Id(id)) = function.result {
        if let TypeDefKind::Result(result) = &resolve.types[id].kind {
            public_result = optional(result.ok.as_ref())?;
            error = optional(result.err.as_ref())?;
            wrap = "value.map_err(CallError::Guest)".into();
        }
    }
    let params = function
        .params
        .iter()
        .enumerate()
        .map(|(index, p)| {
            Ok(format!(
                "__wisp_arg_{index}: {}",
                rust_type(resolve, &p.ty, 0)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    let args = function
        .params
        .iter()
        .enumerate()
        .map(|(index, _)| format!("__wisp_arg_{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let types = function
        .params
        .iter()
        .map(|p| Ok(format!("{},", rust_type(resolve, &p.ty, 0)?)))
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    let guest_args = if asynchronous {
        args.clone()
    } else {
        function
            .params
            .iter()
            .enumerate()
            .map(|(index, p)| {
                guest_argument(resolve, &p.ty, &format!("__wisp_arg_{index}"), false, 0)
            })
            .collect::<Result<Vec<_>>>()?
            .join(", ")
    };
    let comma = if params.is_empty() { "" } else { ", " };
    let await_guest = if asynchronous { ".await" } else { "" };
    Ok(Some(format!(
        r#"pub async fn {method}(&mut self{comma}{params})->Result<{public_result},CallError<{error}>> {{
#[cfg(target_arch="wasm32")] {{ let value=super::guest::{path}::{method}({guest_args}){await_guest}; {wrap} }}
#[cfg(not(target_arch="wasm32"))] {{
    let mut state=self.library.access().map_err(CallError::Runtime)?;
    if state.poisoned {{return Err(CallError::Runtime("Component instance is poisoned".into()))}}
    let result:wasmtime::Result<{native_result}>=async {{
        let super::State {{store,bindings,..}}=&mut *state;
        let callee=bindings.{accessor}().func_{method}().func().typed::<({types}),({native_result},)>(&*store)?;
        store.run_concurrent(async move |accessor|callee.call_concurrent(&accessor,({args}{trailing})).await).await?.map(|(value,)|value)
    }}.await;
    let value=result.map_err(|error|{{state.poisoned=true;CallError::Runtime(format!("{{error:#}}"))}})?;
    {wrap}
}}
}}
"#,
        trailing = if args.is_empty() { "" } else { "," }
    )))
}
