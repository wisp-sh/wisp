//! Owned error-context values with the pinned Wasmtime Store-lifetime limit.
use crate::library::{guest_argument, ident, rust_type};
use anyhow::{Result, ensure};
use wit_parser::{Function, FunctionKind, Resolve, Type, TypeDefKind, WorldId, WorldItem};

pub fn is_context(resolve: &Resolve, ty: &Type) -> bool {
    match ty {
        Type::ErrorContext => true,
        Type::Id(id) => match &resolve.types[*id].kind {
            TypeDefKind::Type(ty) => is_context(resolve, ty),
            _ => false,
        },
        _ => false,
    }
}
pub fn enabled(resolve: &Resolve, world: WorldId) -> bool {
    resolve.worlds[world].exports.values().any(|item| {
        let WorldItem::Interface { id, .. } = item else {
            return false;
        };
        resolve.interfaces[*id].functions.values().any(|function| {
            function
                .params
                .iter()
                .map(|p| &p.ty)
                .chain(function.result.iter())
                .any(|ty| is_context(resolve, ty))
        })
    })
}
pub fn runtime_source() -> &'static str {
    include_str!("../assets/templates/library-error-context.rs")
}

pub fn client_method(
    resolve: &Resolve,
    function: &Function,
    path: &str,
    accessor: &str,
) -> Result<Option<String>> {
    let returns_context = function
        .result
        .as_ref()
        .is_some_and(|ty| is_context(resolve, ty));
    if !returns_context && !function.params.iter().any(|p| is_context(resolve, &p.ty)) {
        return Ok(None);
    }
    ensure!(
        matches!(function.kind, FunctionKind::Freestanding),
        "unsupported-library-async-error-context-function"
    );
    let optional = |ty: Option<&Type>| {
        ty.map(|ty| rust_type(resolve, ty, 0))
            .unwrap_or_else(|| Ok("()".into()))
    };
    let method = ident(&function.name);
    let mut params = Vec::new();
    let mut checks = String::new();
    let mut native_args = Vec::new();
    let mut native_types = Vec::new();
    let mut guest_args = Vec::new();
    for (index, p) in function.params.iter().enumerate() {
        let arg = format!("__wisp_arg_{index}");
        if is_context(resolve, &p.ty) {
            params.push(format!("mut {arg}: super::ErrorContext"));
            checks.push_str(&format!("if !self.library.same_instance(&{arg}.library) {{return Err(CallError::Runtime(\"error context belongs to another Component instance\".into()))}}\nif {arg}.handle.is_none(){{return Err(CallError::Runtime(\"error context is closed\".into()))}}\n"));
            native_args.push(format!("{arg}.handle.take().expect(\"checked context\")"));
            native_types.push("wasmtime::component::ErrorContext,".into());
            guest_args.push(format!("{arg}.handle.take().expect(\"checked context\")"));
        } else {
            let ty = rust_type(resolve, &p.ty, 0)?;
            params.push(format!("{arg}: {ty}"));
            native_types.push(format!("{ty},"));
            native_args.push(arg.clone());
            guest_args.push(guest_argument(resolve, &p.ty, &arg, false, 0)?);
        }
    }
    let native_result = if returns_context {
        "wasmtime::component::ErrorContext".into()
    } else {
        optional(function.result.as_ref())?
    };
    let mut public_result = if returns_context {
        "super::ErrorContext".into()
    } else {
        native_result.clone()
    };
    let mut error = "std::convert::Infallible".into();
    let mut wrap = if returns_context {
        "Ok(super::ErrorContext { library:self.library.clone(), handle:Some(value) })".to_owned()
    } else {
        "Ok(value)".into()
    };
    if let Some(Type::Id(id)) = function.result {
        if let TypeDefKind::Result(result) = &resolve.types[id].kind {
            public_result = optional(result.ok.as_ref())?;
            error = optional(result.err.as_ref())?;
            wrap = "value.map_err(CallError::Guest)".into();
        }
    }
    let capacity = if returns_context {
        "if state.error_context_lifts >= 127 {return Err(CallError::Runtime(\"error-context Store lifetime capacity reached (127 lifts)\".into()))}"
    } else {
        ""
    };
    let count = if returns_context {
        "state.error_context_lifts += 1;"
    } else {
        ""
    };
    let comma = if params.is_empty() { "" } else { ", " };
    let trailing = if native_args.is_empty() { "" } else { "," };
    Ok(Some(format!(
        r#"pub fn {method}(&mut self{comma}{params})->Result<{public_result},CallError<{error}>> {{
{checks}
#[cfg(target_arch="wasm32")] {{ let value=super::guest::{path}::{method}({guest_args}); {wrap} }}
#[cfg(not(target_arch="wasm32"))] {{
let mut state=self.library.access().map_err(CallError::Runtime)?;
if state.poisoned {{return Err(CallError::Runtime("Component instance is poisoned".into()))}}
{capacity}
let result={{let super::State{{store,bindings,..}}=&mut *state;bindings.{accessor}().func_{method}().func().typed::<({native_types}),({native_result},)>(&*store).and_then(|callee|callee.call(store,({native_args}{trailing}))).map(|(value,)|value)}};
let value=result.map_err(|error|{{state.poisoned=true;CallError::Runtime(format!("{{error:#}}"))}})?;
{count}
{wrap}
}}
}}
"#,
        params = params.join(", "),
        native_types = native_types.join(" "),
        native_args = native_args.join(", "),
        guest_args = guest_args.join(", ")
    )))
}
