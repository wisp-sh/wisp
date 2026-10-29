//! Package-manager-based libraries. This format does not change schema-1/2 releases.
#[path = "library_admission.rs"]
mod admission;
#[path = "library_rust_guest.rs"]
pub(crate) mod rust_guest;
#[path = "library_worker.rs"]
mod worker;
use anyhow::{Context, Result, bail, ensure};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use wit_parser::{Resolve, Type, TypeDefKind, WorldId, WorldItem};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> Result<()> {
    fs::create_dir_all(path.as_ref().parent().context("file parent")?)?;
    fs::write(path, bytes)?;
    Ok(())
}
fn command(program: &str, args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("running {program}"))?;
    ensure!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

pub fn package_names(identity: &str) -> Result<(String, String)> {
    crate::identity::package_names(identity)
}

pub(crate) fn ident(name: &str) -> String {
    let name = name.to_snake_case();
    if matches!(name.as_str(), "self" | "super" | "crate" | "Self") {
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

pub(crate) fn rust_type(resolve: &Resolve, ty: &Type, depth: usize) -> Result<String> {
    ensure!(depth < 64, "library type depth limit");
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
        Type::Id(id) => {
            let def = &resolve.types[*id];
            let representation = match &def.kind {
                TypeDefKind::Type(ty) => rust_type(resolve, ty, depth + 1)?,
                TypeDefKind::Record(record) => {
                    for field in &record.fields {
                        rust_type(resolve, &field.ty, depth + 1)?;
                    }
                    String::new()
                }
                TypeDefKind::Variant(variant) => {
                    for case in &variant.cases {
                        optional_type(resolve, case.ty.as_ref(), depth + 1)?;
                    }
                    String::new()
                }
                TypeDefKind::Enum(_) | TypeDefKind::Flags(_) => String::new(),
                TypeDefKind::List(ty) => format!("Vec<{}>", rust_type(resolve, ty, depth + 1)?),
                TypeDefKind::Option(ty) => {
                    format!("Option<{}>", rust_type(resolve, ty, depth + 1)?)
                }
                TypeDefKind::Tuple(tuple) => format!(
                    "({})",
                    tuple
                        .types
                        .iter()
                        .map(|ty| Ok(format!("{},", rust_type(resolve, ty, depth + 1)?)))
                        .collect::<Result<Vec<_>>>()?
                        .join(" ")
                ),
                TypeDefKind::Result(result) => {
                    format!(
                        "Result<{}, {}>",
                        optional_type(resolve, result.ok.as_ref(), depth + 1)?,
                        optional_type(resolve, result.err.as_ref(), depth + 1)?
                    )
                }
                _ => bail!("unsupported-library-type: {:?}", def.kind),
            };
            def.name
                .as_ref()
                .map(|name| name.to_upper_camel_case())
                .unwrap_or(representation)
        }
        _ => bail!("unsupported library type: {ty:?}"),
    })
}

fn has_list(resolve: &Resolve, ty: &Type, depth: usize) -> Result<bool> {
    ensure!(depth < 64, "library type depth limit");
    Ok(match ty {
        Type::String => true,
        Type::Id(id) => match &resolve.types[*id].kind {
            TypeDefKind::List(_) => true,
            TypeDefKind::Type(ty) | TypeDefKind::Option(ty) => has_list(resolve, ty, depth + 1)?,
            TypeDefKind::Record(record) => record
                .fields
                .iter()
                .map(|f| has_list(resolve, &f.ty, depth + 1))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|v| v),
            TypeDefKind::Variant(variant) => variant
                .cases
                .iter()
                .filter_map(|c| c.ty.as_ref())
                .map(|t| has_list(resolve, t, depth + 1))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|v| v),
            TypeDefKind::Tuple(tuple) => tuple
                .types
                .iter()
                .map(|t| has_list(resolve, t, depth + 1))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|v| v),
            TypeDefKind::Result(result) => result
                .ok
                .iter()
                .chain(result.err.iter())
                .map(|t| has_list(resolve, t, depth + 1))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|v| v),
            _ => false,
        },
        _ => false,
    })
}

pub(crate) fn guest_argument(
    resolve: &Resolve,
    ty: &Type,
    expression: &str,
    reference: bool,
    depth: usize,
) -> Result<String> {
    ensure!(depth < 64, "library type depth limit");
    if !has_list(resolve, ty, 0)? {
        return Ok(if reference {
            format!("*{expression}")
        } else {
            expression.into()
        });
    }
    let borrow = if reference {
        expression.into()
    } else {
        format!("&{expression}")
    };
    Ok(match ty {
        Type::String => borrow,
        Type::Id(id) => {
            let def = &resolve.types[*id];
            if def.name.is_some() {
                return Ok(borrow);
            }
            match &def.kind {
                TypeDefKind::List(_) => borrow,
                TypeDefKind::Option(ty) => format!(
                    "{expression}.as_ref().map(|value| {})",
                    guest_argument(resolve, ty, "value", true, depth + 1)?
                ),
                TypeDefKind::Result(result) => format!(
                    "match &{expression} {{ Ok(value) => Ok({}), Err(value) => Err({}) }}",
                    result
                        .ok
                        .as_ref()
                        .map(|ty| guest_argument(resolve, ty, "value", true, depth + 1))
                        .unwrap_or_else(|| Ok("()".into()))?,
                    result
                        .err
                        .as_ref()
                        .map(|ty| guest_argument(resolve, ty, "value", true, depth + 1))
                        .unwrap_or_else(|| Ok("()".into()))?
                ),
                TypeDefKind::Tuple(tuple) => format!(
                    "({})",
                    tuple
                        .types
                        .iter()
                        .enumerate()
                        .map(|(index, ty)| Ok(format!(
                            "{},",
                            guest_argument(
                                resolve,
                                ty,
                                &format!("{expression}.{index}"),
                                false,
                                depth + 1
                            )?
                        )))
                        .collect::<Result<Vec<_>>>()?
                        .join(" ")
                ),
                _ => bail!("unsupported-library-type: anonymous argument"),
            }
        }
        _ => unreachable!(),
    })
}
fn optional_type(resolve: &Resolve, ty: Option<&Type>, depth: usize) -> Result<String> {
    ty.map(|ty| rust_type(resolve, ty, depth))
        .unwrap_or_else(|| Ok("()".into()))
}

/// Bound recursive expansion before any target generator walks the WIT graph.
/// Memoization makes a shared DAG cheap to inspect even when expanding it would
/// produce exponentially many output expressions.
pub fn validate_type_expansion(resolve: &Resolve, world: WorldId) -> Result<()> {
    ensure!(resolve.types.len() <= 4096, "library contract size limit");
    fn visit(
        resolve: &Resolve,
        ty: Type,
        height: usize,
        cache: &mut BTreeMap<wit_parser::TypeId, (usize, usize)>,
        active: &mut BTreeSet<wit_parser::TypeId>,
    ) -> Result<(usize, usize)> {
        ensure!(height <= 64, "library type depth limit");
        let Type::Id(id) = ty else {
            return Ok((1, 1));
        };
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
        let mut cost = 1usize;
        let mut depth = 1usize;
        for child in children {
            let (child_cost, child_depth) = visit(resolve, child, height + 1, cache, active)?;
            cost = cost.saturating_add(child_cost);
            depth = depth.max(child_depth.saturating_add(1));
            ensure!(cost <= 4096, "library type expanded value limit");
            ensure!(depth <= 64, "library type depth limit");
        }
        active.remove(&id);
        cache.insert(id, (cost, depth));
        Ok((cost, depth))
    }
    let mut cache = BTreeMap::new();
    let mut active = BTreeSet::new();
    for item in resolve.worlds[world]
        .imports
        .values()
        .chain(resolve.worlds[world].exports.values())
    {
        let WorldItem::Interface { id, .. } = item else {
            continue;
        };
        for id in resolve.interfaces[*id].types.values() {
            visit(resolve, Type::Id(*id), 1, &mut cache, &mut active)?;
        }
        for function in resolve.interfaces[*id].functions.values() {
            let mut expanded = 0usize;
            for ty in function
                .params
                .iter()
                .map(|p| p.ty)
                .chain(function.result.iter().copied())
            {
                let (cost, _) = visit(resolve, ty, 1, &mut cache, &mut active)?;
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

/// The decoder assigns a synthetic world name to top-level functions. Compare
/// their actual signatures, not the synthetic interface identity.
pub fn exports_match(
    decoded: &crate::wit_model::AbiModel,
    authored: &crate::wit_model::AbiModel,
) -> bool {
    fn parts(
        model: &crate::wit_model::AbiModel,
    ) -> (
        Vec<crate::wit_model::AbiInterface>,
        Vec<crate::wit_model::AbiFunction>,
    ) {
        let mut interfaces = Vec::new();
        let mut functions = Vec::new();
        for interface in &model.exports {
            if interface.package == model.package
                && interface.name == format!("{}-world", model.world)
            {
                functions.extend(interface.functions.iter().cloned());
            } else {
                let mut interface = interface.clone();
                // Printed WIT moves `use` aliases ahead of other declarations
                // and groups methods by resource. Named types and functions have
                // no ABI order; field/case/parameter order and ownership remain.
                interface.types.sort_by(|a, b| a.name.cmp(&b.name));
                interface.functions.sort_by(|a, b| a.name.cmp(&b.name));
                interfaces.push(interface);
            }
        }
        (interfaces, functions)
    }
    parts(decoded) == parts(authored)
}

#[cfg(test)]
mod contract_comparison_tests {
    use super::*;

    #[test]
    fn reconstructs_worker_bindings_from_release_platforms() {
        let mut resolve = Resolve::default();
        let package = resolve
            .push_str(
                "scalar.wit",
                "package test:worker; world root { export answer: func() -> u32; }",
            )
            .unwrap();
        let world = resolve.select_world(&[package], Some("root")).unwrap();
        let bytes = b"foreign release worker".to_vec();
        let files = BTreeMap::from([
            ("worker/darwin-arm64/wisp-worker".into(), bytes.clone()),
            ("worker/manifest.json".into(), serde_json::to_vec_pretty(&json!({
                "schema":1,"protocol":1,"workerVersion":"0.2.0","wasmtimeVersion":wisp_worker_protocol::WASMTIME_VERSION,
                "artifacts":{"darwin-arm64":{"path":"worker/darwin-arm64/wisp-worker","sha256":digest(&bytes)}}
            })).unwrap()),
        ]);
        let source = rust_source_with_worker_files(&resolve, world, &files).unwrap();
        assert!(source.contains("../worker/darwin-arm64/wisp-worker"));
        assert!(!source.contains("../worker/linux-amd64/wisp-worker"));
        syn::parse_file(&source).unwrap();
    }

    #[test]
    fn scalar_exports_with_host_imports_keep_the_existing_native_host() {
        let mut resolve = Resolve::default();
        let package = resolve
            .push_str(
                "host.wit",
                r#"
            package test:worker;
            interface dependency { answer: func() -> u32; }
            world root { import dependency; export answer: func() -> u32; }
        "#,
            )
            .unwrap();
        let world = resolve.select_world(&[package], Some("root")).unwrap();
        assert!(!worker::supports(&resolve, world));
        resolve.worlds[world].imports.clear();
        assert!(worker::supports(&resolve, world));
    }

    const CONTRACT: &str = r#"
        package test:comparison;
        interface values {
            record pair { left: u32, right: u64 }
            variant choice { first(u32), second(u64) }
            enum mode { first, second }
            flags options { first, second }
            resource cursor { current: func() -> u32; }
            resource other;
            inspect: func(value: borrow<cursor>, count: u32) -> pair;
        }
        world root { export values; }
    "#;

    fn model(wit: &str) -> crate::wit_model::AbiModel {
        let mut resolve = Resolve::default();
        let package = resolve.push_str("contract.wit", wit).unwrap();
        let world = resolve.select_world(&[package], Some("root")).unwrap();
        crate::wit_model::AbiModel::from_resolve(&resolve, world).unwrap()
    }

    #[test]
    fn matches_reordered_named_type_definitions() {
        let decoded = model(CONTRACT);
        let reordered = CONTRACT.replace(
            "record pair { left: u32, right: u64 }\n            variant choice { first(u32), second(u64) }",
            "variant choice { first(u32), second(u64) }\n            record pair { left: u32, right: u64 }",
        );
        let authored = model(&reordered);
        assert_ne!(decoded.exports[0].types, authored.exports[0].types);
        assert!(exports_match(&decoded, &authored));
    }

    #[test]
    fn matches_reordered_resource_method_groups() {
        let contract = r#"
            package test:comparison;
            interface streams {
                resource input { subscribe: func() -> u32; }
                resource output { subscribe: func() -> u64; }
            }
            world root { export streams; }
        "#;
        let reordered = contract.replace(
            "resource input { subscribe: func() -> u32; }\n                resource output { subscribe: func() -> u64; }",
            "resource output { subscribe: func() -> u64; }\n                resource input { subscribe: func() -> u32; }",
        );
        let decoded = model(contract);
        let authored = model(&reordered);
        assert_ne!(decoded.exports[0].functions, authored.exports[0].functions);
        assert!(exports_match(&decoded, &authored));
        let changed = reordered.replace("subscribe: func() -> u32", "subscribe: func() -> u64");
        assert!(!exports_match(&decoded, &model(&changed)));
    }

    #[test]
    fn rejects_changed_signatures_and_resource_ownership() {
        let decoded = model(CONTRACT);
        for changed in [
            CONTRACT.replace("current: func() -> u32", "current: func() -> u64"),
            CONTRACT.replace("borrow<cursor>", "own<cursor>"),
            CONTRACT.replace(
                "resource cursor { current: func() -> u32; }\n            resource other;",
                "resource cursor;\n            resource other { current: func() -> u32; }",
            ),
            CONTRACT.replace("test:comparison", "test:other"),
        ] {
            assert!(!exports_match(&decoded, &model(&changed)), "{changed}");
        }
    }

    #[test]
    fn rejects_reordered_abi_positions() {
        let decoded = model(CONTRACT);
        for changed in [
            CONTRACT.replace("left: u32, right: u64", "right: u64, left: u32"),
            CONTRACT.replace("first(u32), second(u64)", "second(u64), first(u32)"),
            CONTRACT.replace("enum mode { first, second }", "enum mode { second, first }"),
            CONTRACT.replace(
                "flags options { first, second }",
                "flags options { second, first }",
            ),
            CONTRACT.replace(
                "value: borrow<cursor>, count: u32",
                "count: u32, value: borrow<cursor>",
            ),
        ] {
            assert!(!exports_match(&decoded, &model(&changed)), "{changed}");
        }
    }
}

/// Derive both target paths directly from the selected Resolve and world.
#[cfg(test)]
#[allow(dead_code)]
pub fn rust_source(resolve: &Resolve, world: WorldId) -> Result<String> {
    let worker_files = if worker::supports(resolve, world) {
        wisp_worker_protocol::bundle::files()?
    } else {
        BTreeMap::new()
    };
    rust_source_with_worker_files(resolve, world, &worker_files)
}

fn rust_source_with_worker_files(
    resolve: &Resolve,
    world: WorldId,
    worker_files: &crate::release::Files,
) -> Result<String> {
    let world_id = world;
    validate_type_expansion(resolve, world)?;
    let wasi = crate::library_wasi::enabled(resolve, world)?;
    let pure = crate::library_wasi::pure_browser_host(resolve, world)?;
    let reactor = crate::library_wasi::reactor_host(resolve, world)?;
    let error_context = crate::library_error_context::enabled(resolve, world);
    let asynchronous = crate::library_async::enabled(resolve, world) || error_context;
    let world = &resolve.worlds[world];
    ensure!(
        world.exports.len() <= 64 && resolve.types.len() <= 4096,
        "library contract size limit"
    );
    ensure!(
        !world.exports.is_empty(),
        "library must export an interface"
    );
    let mut source = format!(
        r#"#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
compile_error!("Wisp Component libraries currently support wasm32-unknown-unknown; this Wasm target has not been verified");
#[cfg(all(not(target_arch = "wasm32"), not(any(all(target_os = "linux", target_arch = "x86_64"), all(target_os = "macos", target_arch = "aarch64")))))]
compile_error!("Wisp native Component libraries currently support Linux x86_64 and macOS ARM64");
#[derive(Debug)]
pub enum CallError<E> {{ Guest(E), Runtime(String) }}
impl<E: std::fmt::Debug> std::fmt::Display for CallError<E> {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ write!(f, "{{self:?}}") }}
}}
impl<E: std::fmt::Debug> std::error::Error for CallError<E> {{}}
#[cfg(not(target_arch = "wasm32"))]
mod host {{ wasmtime::component::bindgen!({{ path: "wit/host.wit", world: {:?} }}); }}
#[cfg(target_arch = "wasm32")]
mod guest {{ wit_bindgen::generate!({{ path: "wit/guest.wit", world: "wisp-imports", generate_all }}); }}
"#,
        world.name
    );
    source.push_str(&format!(r#"
#[cfg(not(target_arch = "wasm32"))]
struct State {{ store: wasmtime::Store<{store_type}>, bindings: host::{binding}, poisoned: bool, error_context_lifts: usize }}
#[cfg(not(target_arch = "wasm32"))]
type Deferred = Box<dyn FnOnce(&mut State) + Send>;
#[cfg(not(target_arch = "wasm32"))]
struct Access<'a> {{
    state: Option<std::sync::MutexGuard<'a, State>>,
    deferred: &'a std::sync::Mutex<Vec<Deferred>>,
}}
#[cfg(not(target_arch = "wasm32"))]
impl Access<'_> {{
    fn flush(&mut self) {{
        loop {{
            let pending = std::mem::take(&mut *self.deferred.lock().unwrap_or_else(|e| e.into_inner()));
            if pending.is_empty() {{ break; }}
            for close in pending {{ close(self.state.as_deref_mut().expect("live Store access")); }}
        }}
    }}
}}
#[cfg(not(target_arch = "wasm32"))]
impl std::ops::Deref for Access<'_> {{
    type Target = State;
    fn deref(&self) -> &State {{ self.state.as_deref().expect("live Store access") }}
}}
#[cfg(not(target_arch = "wasm32"))]
impl std::ops::DerefMut for Access<'_> {{
    fn deref_mut(&mut self) -> &mut State {{ self.state.as_deref_mut().expect("live Store access") }}
}}
#[cfg(not(target_arch = "wasm32"))]
impl Drop for Access<'_> {{
    fn drop(&mut self) {{
        loop {{
            let mut queue = self.deferred.lock().unwrap_or_else(|e| e.into_inner());
            if queue.is_empty() {{
                // Store unlock and the final empty-queue check are atomic to
                // deferred producers, so no cleanup can be stranded.
                self.state.take();
                break;
            }}
            let pending = std::mem::take(&mut *queue);
            drop(queue);
            for close in pending {{ close(self.state.as_deref_mut().expect("live Store access")); }}
        }}
    }}
}}
#[derive(Clone)]
pub struct Library {{
    #[cfg(target_arch = "wasm32")]
    identity: std::rc::Rc<()>,
    #[cfg(not(target_arch = "wasm32"))]
    state: std::sync::Arc<std::sync::Mutex<State>>,
    #[cfg(not(target_arch = "wasm32"))]
    deferred: std::sync::Arc<std::sync::Mutex<Vec<Deferred>>>,
    #[cfg(not(target_arch = "wasm32"))]
    open_readers: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}}
impl Library {{
    #[cfg(not(target_arch = "wasm32"))]
    fn access(&self) -> Result<Access<'_>, String> {{
        let state = self.state.try_lock().map_err(|error| match error {{
            std::sync::TryLockError::WouldBlock => "Component instance is busy".to_owned(),
            std::sync::TryLockError::Poisoned(_) => "Component instance lock is poisoned".to_owned(),
        }})?;
        let mut access = Access {{ state: Some(state), deferred: &self.deferred }};
        access.flush();
        Ok(access)
    }}
    #[cfg(not(target_arch = "wasm32"))]
    fn defer(&self, close: impl FnOnce(&mut State) + Send + 'static) {{
        self.deferred.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(close));
        // The current Access drains this queue if the Store is in use.
        let _ = self.access();
    }}
    fn same_instance(&self, other: &Self) -> bool {{
        #[cfg(target_arch = "wasm32")] {{ std::rc::Rc::ptr_eq(&self.identity, &other.identity) }}
        #[cfg(not(target_arch = "wasm32"))] {{ std::sync::Arc::ptr_eq(&self.state, &other.state) }}
    }}
    pub fn new() -> Result<Self, CallError<std::convert::Infallible>> {{
        #[cfg(target_arch = "wasm32")] {{ Ok(Self {{ identity: std::rc::Rc::new(()) }}) }}
        #[cfg(not(target_arch = "wasm32"))] {{
            let create = || -> wasmtime::Result<Self> {{
                {engine_init}
                let component = wasmtime::component::Component::new(&engine, include_bytes!("../component.wasm"))?;
                let mut store = wasmtime::Store::new(&engine, {store_value});
                let mut linker = wasmtime::component::Linker::new(&engine);
                {linker_init}
                let bindings = host::{binding}::instantiate(&mut store, &component, &linker)?;
                Ok(Self {{ state: std::sync::Arc::new(std::sync::Mutex::new(State {{ store, bindings, poisoned: false, error_context_lifts: 0 }})), deferred: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())), open_readers: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)) }})
            }};
            create().map_err(|e| CallError::Runtime(format!("{{e:#}}")))
        }}
    }}
}}
"#, binding=world.name.to_upper_camel_case(), store_type=if wasi {"WasiState"}else{"()"},
store_value=if pure || reactor {crate::library_wasi::PURE_STORE}else if wasi {crate::library_wasi::STORE}else{"()"}, linker_init=if pure {crate::library_wasi::PURE_LINKER_CALL}else if reactor {crate::library_wasi::REACTOR_LINKER}else if wasi {crate::library_wasi::LINKER}else{""}, engine_init=if asynchronous {"let mut config = wasmtime::Config::new(); config.wasm_component_model(true).wasm_component_model_async(true).wasm_component_model_error_context(true).wasm_component_model_more_async_builtins(true).wasm_component_model_async_stackful(true).concurrency_support(true); let engine = wasmtime::Engine::new(&config)?;"}else if reactor {"let mut config = wasmtime::Config::new(); config.wasm_exceptions(true); let engine = wasmtime::Engine::new(&config)?;"}else{"let engine = wasmtime::Engine::default();"}));
    if wasi {
        source.push_str(crate::library_wasi::STATE);
    }
    if pure {
        source.push_str(crate::library_wasi::PURE_LINKER);
    }
    if asynchronous {
        source.push_str(crate::library_async::runtime_source());
    }
    if error_context {
        source.push_str(crate::library_error_context::runtime_source());
    }
    let mut modules = BTreeSet::from(["host".into(), "guest".into(), "async_runtime".into()]);
    let direct_functions = world
        .exports
        .values()
        .filter_map(|item| match item {
            WorldItem::Function(function) => Some((function.name.clone(), function.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    for id in world
        .exports
        .values()
        .filter_map(|item| match item {
            WorldItem::Interface { id, .. } => Some(Some(*id)),
            _ => None,
        })
        .chain(std::iter::once(None).filter(|_| !direct_functions.is_empty()))
    {
        let direct = id.is_none();
        let mut synthetic = wit_parser::Interface {
            name: Some("world".into()),
            types: Default::default(),
            functions: Default::default(),
            package: world.package,
            docs: Default::default(),
            stability: Default::default(),
            span: Default::default(),
            clone_of: None,
        };
        synthetic.functions.extend(direct_functions.iter().cloned());
        let interface = id.map(|id| &resolve.interfaces[id]).unwrap_or(&synthetic);
        ensure!(
            interface.functions.len() <= 1024,
            "library function count limit"
        );
        let name = interface
            .name
            .as_deref()
            .context("library interface must be named")?;
        let mut type_names = BTreeSet::from([
            "Client".to_owned(),
            "CallError".to_owned(),
            "String".to_owned(),
            "Vec".to_owned(),
            "Option".to_owned(),
            "Result".to_owned(),
        ]);
        for (name, id) in &interface.types {
            ensure!(
                crate::library_async::reader(resolve, &Type::Id(*id)).is_none(),
                "unsupported-library-named-async-alias: {name}"
            );
            ensure!(
                !crate::library_error_context::is_context(resolve, &Type::Id(*id)),
                "unsupported-library-named-error-context-alias: {name}"
            );
            ensure!(
                type_names.insert(name.to_upper_camel_case()),
                "library type name collision: {name}"
            );
        }
        let module = ident(name);
        ensure!(
            modules.insert(module.clone()),
            "library interface name collision: {name}"
        );
        let package = &resolve.packages[interface.package.context("interface package")?].name;
        let path = format!(
            "{}::{}::{}",
            ident(&package.namespace),
            ident(&package.name),
            module
        );
        let guest_path = if direct {
            "super::guest".to_owned()
        } else {
            format!("super::guest::{path}")
        };
        let accessor = format!(
            "{}_{}_{}",
            package.namespace.to_snake_case(),
            package.name.to_snake_case(),
            name.to_snake_case()
        );
        source.push_str(&format!(
            r#"
pub mod {module} {{
use super::CallError;
{type_imports}
pub struct Client {{
    pub(super) library: super::Library,
}}
impl Client {{
    pub fn new() -> Result<Self, CallError<std::convert::Infallible>> {{
        Ok(Self {{ library: super::Library::new()? }})
    }}
"#,
            type_imports = if direct { String::new() } else { format!("#[cfg(not(target_arch = \"wasm32\"))]\npub use super::host::exports::{path}::{{{types}}};\n#[cfg(target_arch = \"wasm32\")]\npub use super::guest::{path}::{{{types}}};", types = interface.types.iter().filter_map(|(name,id)| match crate::library_resources::custom_type(resolve,*id) {
                Ok(true) => None,
                Ok(false) => Some(rust_type(resolve,&Type::Id(*id),0).map(|_| name.to_upper_camel_case())),
                Err(error) => Some(Err(error)),
            })
                .collect::<Result<Vec<_>>>()?
                .join(", ")) },
        ));
        let mut methods = BTreeSet::from(["new".to_owned()]);
        for function in interface.functions.values() {
            ensure!(function.params.len() <= 64, "library parameter count limit");
            if matches!(
                function.kind,
                wit_parser::FunctionKind::Freestanding
                    | wit_parser::FunctionKind::AsyncFreestanding
            ) {
                ensure!(
                    methods.insert(ident(&function.name)),
                    "library method name collision: {}",
                    function.name
                );
            }
            if let Some(resource) =
                crate::library_resources::client_method(resolve, function, &path, &accessor)?
            {
                source.push_str(&resource);
                continue;
            }
            if let Some(method) =
                crate::library_error_context::client_method(resolve, function, &path, &accessor)?
            {
                source.push_str(&method);
                continue;
            }
            if let Some(method) =
                crate::library_async::client_method(resolve, function, &path, &accessor)?
            {
                source.push_str(&method);
                continue;
            }
            ensure!(
                matches!(function.kind, wit_parser::FunctionKind::Freestanding),
                "unsupported library function: {}",
                function.name
            );
            let method = ident(&function.name);
            let params = function
                .params
                .iter()
                .enumerate()
                .map(|(index, p)| {
                    Ok(format!(
                        "{}: {}",
                        format!("__wisp_arg_{index}"),
                        rust_type(resolve, &p.ty, 0)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let args = function
                .params
                .iter()
                .enumerate()
                .map(|(index, _)| format!("__wisp_arg_{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let native_args = function
                .params
                .iter()
                .enumerate()
                .map(|(index, param)| {
                    guest_argument(resolve, &param.ty, &format!("__wisp_arg_{index}"), false, 0)
                })
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            let guest_args = function
                .params
                .iter()
                .enumerate()
                .map(|(index, p)| {
                    guest_argument(resolve, &p.ty, &format!("__wisp_arg_{index}"), false, 0)
                })
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            let (result, guest_error) = match function.result {
                Some(Type::Id(id)) if matches!(resolve.types[id].kind, TypeDefKind::Result(_)) => {
                    let TypeDefKind::Result(result) = &resolve.types[id].kind else {
                        unreachable!()
                    };
                    (
                        optional_type(resolve, result.ok.as_ref(), 0)?,
                        Some(optional_type(resolve, result.err.as_ref(), 0)?),
                    )
                }
                ref ty => (optional_type(resolve, ty.as_ref(), 0)?, None),
            };
            let error = guest_error.as_deref().unwrap_or("std::convert::Infallible");
            source.push_str(&format!("pub fn {method}(&mut self{comma}{params}) -> Result<{result}, CallError<{error}>> {{\n", comma=if params.is_empty(){""}else{", "}, params=params.join(", ")));
            source.push_str(&format!(
                "#[cfg(target_arch = \"wasm32\")]\n{{ {} }}\n",
                if guest_error.is_some() {
                    format!("{guest_path}::{method}({guest_args}).map_err(CallError::Guest)")
                } else {
                    format!("Ok({guest_path}::{method}({guest_args}))")
                }
            ));
            let param_types = function
                .params
                .iter()
                .map(|p| Ok(format!("{},", rust_type(resolve, &p.ty, 0)?)))
                .collect::<Result<Vec<_>>>()?
                .join(" ");
            let return_type = optional_type(resolve, function.result.as_ref(), 0)?;
            if direct {
                let native_method = function.name.to_snake_case();
                source.push_str(&format!("#[cfg(not(target_arch = \"wasm32\"))]\n{{ let mut state = self.library.access().map_err(CallError::Runtime)?;\nif state.poisoned {{ return Err(CallError::Runtime(\"Component instance is poisoned\".into())); }}\nlet result = {{ let super::State {{ store, bindings, .. }} = &mut *state; bindings.call_{native_method}(store{comma}{native_args}) }};\nlet value = result.map_err(|e| {{ state.poisoned = true; CallError::Runtime(format!(\"{{e:#}}\")) }})?;\n{} }}\n}}\n", if guest_error.is_some(){"value.map_err(CallError::Guest)"}else{"Ok(value)"}, comma=if args.is_empty(){""}else{", "}));
            } else {
                source.push_str(&format!("#[cfg(not(target_arch = \"wasm32\"))]\n{{ let mut state = self.library.access().map_err(CallError::Runtime)?;\nif state.poisoned {{ return Err(CallError::Runtime(\"Component instance is poisoned\".into())); }}\nlet result = {{ let super::State {{ store, bindings, .. }} = &mut *state; bindings.{accessor}().func_{}().func().typed::<({param_types}), ({return_type},)>(&*store).and_then(|callee| callee.call(store, ({}))).map(|(value,)| value) }};\nlet value = result.map_err(|e| {{ state.poisoned = true; CallError::Runtime(format!(\"{{e:#}}\")) }})?;\n{} }}\n}}\n", function.name.to_snake_case(), if args.is_empty(){String::new()}else{format!("{args},")}, if guest_error.is_some(){"value.map_err(CallError::Guest)"}else{"Ok(value)"}));
            }
        }
        source.push_str("}\n");
        if let Some(id) = id {
            source.push_str(&crate::library_resources::definitions(
                resolve, id, &path, &accessor,
            )?);
        }
        source.push_str("}\n");
        ensure!(
            !["new", "same_instance", "drain"].contains(&module.as_str()),
            "library interface name collision: {module}"
        );
        source.push_str(&format!("impl Library {{ pub fn {module}(&self) -> {module}::Client {{ {module}::Client {{ library: self.clone() }} }} }}\n"));
    }
    if worker::supports(resolve, world_id) {
        let native = worker::source(resolve, world_id, worker_files)?;
        return Ok(format!(
            "#[cfg(not(target_arch = \"wasm32\"))]\nmod implementation {{\n{native}\n}}\n#[cfg(target_arch = \"wasm32\")]\nmod implementation {{\n{source}\n}}\npub use implementation::*;\n"
        ));
    }
    Ok(source)
}

fn wit(resolve: &Resolve, world: WorldId) -> Result<String> {
    let package = resolve.worlds[world].package.context("world package")?;
    let nested = resolve
        .packages
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != package)
        .collect::<Vec<_>>();
    let mut printer = wit_component::WitPrinter::default();
    printer.print(resolve, package, &nested)?;
    Ok(printer.output.to_string())
}

fn host_wit(resolve: &Resolve, world: WorldId) -> Result<String> {
    let mut host = resolve.clone();
    host.worlds[world].imports.clear();
    wit(&host, world)
}

fn guest_wit(resolve: &Resolve, world: WorldId) -> Result<String> {
    let mut guest = resolve.clone();
    let package = guest.worlds[world].package.context("world package")?;
    guest.packages[package].worlds.clear();
    guest.packages[package]
        .worlds
        .insert("wisp-imports".into(), world);
    guest.worlds[world].name = "wisp-imports".into();
    guest.worlds[world].imports = std::mem::take(&mut guest.worlds[world].exports);
    wit(&guest, world)
}

fn cargo_manifest(
    cargo_name: &str,
    version: &str,
    metadata: &crate::library_metadata::Metadata,
    wasi: bool,
    asynchronous: bool,
    reactor: bool,
    worker: bool,
) -> Result<String> {
    let native_dependencies = if worker {
        "serde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\ntempfile = \"3\"\n".to_owned()
    } else {
        format!(
            "wasmtime = {{ version = \"=50.0.0\", default-features = false, features = [\"component-model\", \"cranelift\", \"runtime\", \"std\"{}{}] }}\n{}",
            if asynchronous {
                ", \"component-model-async\""
            } else {
                ""
            },
            if reactor { ", \"gc\", \"gc-drc\"" } else { "" },
            if wasi {
                crate::library_wasi::DEPENDENCIES
            } else {
                ""
            }
        )
    };
    Ok(format!(
        r#"[package]
name = {cargo_name:?}
version = {version:?}
edition = "2024"
{metadata_fields}include = ["src/**", "wit/**", "worker/**", "component.wasm", "Cargo.toml", "Cargo.lock", "notices/**", "WISP-NOTICES.txt"]
[workspace]
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
{native_dependencies}[target.'cfg(target_arch = "wasm32")'.dependencies]
wit-bindgen = {{ version = "=0.61.1"{guest_async_features} }}
"#,
        metadata_fields = metadata.cargo_fields()?,
        guest_async_features = if asynchronous {
            ", features = [\"async\", \"async-spawn\"]"
        } else {
            ""
        },
    ))
}

#[cfg(test)]
#[path = "library_cargo_seed_tests.rs"]
mod cargo_seed_tests;

fn packaged_cargo_manifest(cargo_name: &str, manifest: &str) -> Result<toml::Value> {
    let mut normalized: toml::Value = toml::from_str(manifest)?;
    normalized
        .as_table_mut()
        .context("manifest table")?
        .remove("workspace");
    for key in [
        "build",
        "autolib",
        "autobins",
        "autoexamples",
        "autotests",
        "autobenches",
        "readme",
    ] {
        normalized["package"]
            .as_table_mut()
            .context("package table")?
            .insert(key.into(), false.into());
    }
    normalized.as_table_mut().unwrap().insert(
        "lib".into(),
        toml::Value::try_from(json!({"name":cargo_name.replace('-', "_"),"path":"src/lib.rs"}))?,
    );
    // Cargo expands dependency version strings to tables in the packaged
    // descriptor. Keep comparing the exact generated requirements afterwards.
    for (_, target) in normalized["target"]
        .as_table_mut()
        .context("target tables")?
        .iter_mut()
    {
        for (_, dependency) in target["dependencies"]
            .as_table_mut()
            .context("target dependencies")?
            .iter_mut()
        {
            if let Some(version) = dependency.as_str() {
                *dependency = toml::Value::Table(toml::Table::from_iter([(
                    "version".into(),
                    version.into(),
                )]));
            }
        }
    }
    Ok(normalized)
}

fn node_manifest(
    name: &str,
    version: &str,
    metadata: &crate::library_metadata::Metadata,
    wasi: bool,
) -> Value {
    let mut manifest = json!({"name":name,"version":version,"type":"module","main":"./index.js","types":"./index.d.ts","exports":{".":{"types":"./index.d.ts","default":"./index.js"}}});
    if wasi {
        manifest["exports"]["."] =
            json!({"browser":"./browser.js","types":"./index.d.ts","default":"./index.js"});
    }
    metadata.node_fields(&mut manifest);
    manifest
}

fn tools() -> Value {
    json!({"witParser":"0.245.1","witBindgen":"0.61.1","wasmtime":"50.0.0","jco":"1.33.0"})
}
fn targets(wasi: bool, asynchronous: bool, go: bool, pure: bool) -> Value {
    let mut targets = vec!["rust-native", "rust-wasm32-unknown-unknown", "node"];
    if (!wasi || pure) && !asynchronous {
        targets.push("browser");
    }
    if go {
        targets.push("go");
    }
    json!(targets)
}
// Callers have already validated the component with all supported features.
// A second validation with exceptions disabled detects this runtime requirement
// without inferring it from the source language or the presence of WASI imports.
fn uses_modern_exceptions(component: &[u8]) -> bool {
    let mut features = wasmparser::WasmFeatures::all();
    features.remove(wasmparser::WasmFeatures::EXCEPTIONS);
    wasmparser::Validator::new_with_features(features)
        .validate_all(component)
        .is_err()
}

#[test]
fn detects_modern_wasm_exceptions() {
    use wasm_encoder::{
        Component, Module, ModuleSection, TagKind, TagSection, TagType, TypeSection,
    };
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], []);
    module.section(&types);
    let mut plain = Component::new();
    plain.section(&ModuleSection(&module));
    assert!(!uses_modern_exceptions(&plain.finish()));
    let mut tags = TagSection::new();
    tags.tag(TagType {
        kind: TagKind::Exception,
        func_type_idx: 0,
    });
    module.section(&tags);
    let mut component = Component::new();
    component.section(&ModuleSection(&module));
    let bytes = component.finish();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .unwrap();
    assert!(uses_modern_exceptions(&bytes));
}

fn requirements(
    wasi: bool,
    asynchronous: bool,
    error_context: bool,
    go_platform: Option<&str>,
    pure: bool,
    reactor: bool,
    exceptions: bool,
) -> Value {
    let mut node_flags = Vec::new();
    if asynchronous {
        node_flags.push("--experimental-wasm-jspi");
    }
    if exceptions {
        node_flags.push("--experimental-wasm-exnref");
    }
    let mut value = json!({"rustNative":{"platforms":["linux-x86_64","macos-aarch64"],"network":if wasi && !pure {"host-network"} else {"none"}},
        "rustComponent":{"target":"wasm32-unknown-unknown","hostImports":"outer-runtime"},
        "node":{"version":"24.20.0","flags":node_flags,
            "futureCancellation":if asynchronous {"unsupported-jco-future-cancellation"} else {"not-applicable"}},
        "browser":{"supported":(!wasi || pure) && !asynchronous,"reason":if wasi && !pure {Some("unsupported-browser-wasi-network")} else if error_context {Some("unsupported-browser-error-context-lifecycle")} else if asynchronous {Some("unsupported-browser-async-lifecycle")} else {None},"initialization":"top-level-await"},
        "errorContext":{"supported":error_context,"nativeCumulativeLiftLimit":if error_context {Some(127)}else{None},"nativeCleanup":if error_context {"Store lifetime"}else{"not-applicable"}},
        "go":{"platform":go_platform,"cgo":go_platform.is_some() && go_platform != Some(crate::library_go::guest::PLATFORM) && go_platform != Some(crate::library_go::portable::PLATFORM)}});
    if go_platform == Some(crate::library_go::guest::PLATFORM)
        || go_platform == Some(crate::library_go::portable::PLATFORM)
    {
        value["go"]["composition"] = json!("wisp-component-imports");
    }
    if go_platform == Some(crate::library_go::portable::PLATFORM) {
        value["go"]["native"] = json!({"runtime":crate::library_go::portable::RUNTIME,"version":crate::library_go::portable::RUNTIME_VERSION,"cgo":false});
        value["go"]["wasm"] = json!({"target":"wasip1/wasm","cgo":false});
    }
    if reactor {
        value["rustNative"]["network"] = json!("none");
        value["browser"]["reason"] = json!("unsupported-browser-wasi-reactor");
        value["wasiReactor"] = json!({"clocks":"host","random":"secure","environment":"empty","arguments":"empty","filesystem":"no-preopens","stdio":"isolated"});
    }
    value
}

fn worker_requirements(mut requirements: Value, files: &crate::release::Files) -> Result<Value> {
    if !files.is_empty() {
        let platforms = wisp_worker_protocol::bundle::validate(files)?;
        requirements["rustNative"]["platforms"] = json!(
            platforms
                .iter()
                .map(|platform| {
                    match platform.as_str() {
                        "linux-amd64" => "linux-x86_64",
                        "darwin-arm64" => "macos-aarch64",
                        _ => unreachable!("validated worker platform"),
                    }
                })
                .collect::<Vec<_>>()
        );
        requirements["rustNative"]["runtime"] = json!({"worker":"0.2.0","protocol":1,"wasmtime":wisp_worker_protocol::WASMTIME_VERSION});
    }
    Ok(requirements)
}

fn jco(dir: &Path) -> Result<String> {
    let jco = std::env::var("WISP_JCO").context("set WISP_JCO to jco 1.33.0")?;
    crate::generator_tools::authenticate_jco(Path::new(&jco))?;
    ensure!(
        command(
            crate::generator_tools::node()?
                .to_str()
                .context("Node path UTF8")?,
            &[&jco, "--version"],
            dir
        )?
        .trim()
            == "1.33.0",
        "library packages require jco 1.33.0"
    );
    let expected: Value =
        serde_json::from_str(include_str!("../assets/installed-tools.lock.json"))?;
    ensure!(
        digest(&fs::read(&jco)?) == expected["js/node_modules/.bin/jco"]["sha256"],
        "jco entry point digest mismatch"
    );
    Ok(jco)
}

fn browser_rejection(resolve: &Resolve, world: WorldId, reason: &str) -> Result<String> {
    let mut names = BTreeSet::from(["_util".to_owned()]);
    for (key, item) in &resolve.worlds[world].exports {
        names.insert(resolve.name_world_key(key));
        if let WorldItem::Interface { id, .. } = item {
            if let Some(name) = &resolve.interfaces[*id].name {
                names.insert(name.to_lower_camel_case());
            }
        }
        if let wit_parser::WorldKey::Name(name) = key {
            names.insert(name.to_lower_camel_case());
        }
    }
    let exports = names
        .iter()
        .map(|name| {
            Ok(format!(
                "__wisp_unsupported as {}",
                serde_json::to_string(name)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    Ok(format!(
        "const __wisp_unsupported = undefined;\nexport {{ {exports} }};\nthrow new Error({});\n",
        serde_json::to_string(reason)?
    ))
}

fn transpile(
    jco: &str,
    component: &Path,
    output: &Path,
    cwd: &Path,
    wasi: bool,
    asynchronous: bool,
    pure: bool,
    resolve: &Resolve,
    world: WorldId,
) -> Result<()> {
    let reactor = crate::library_wasi::reactor_host(resolve, world)?;
    let exceptions = uses_modern_exceptions(&fs::read(component)?);
    if pure {
        // Pure-WASI packages use the same private import object in Node and the browser.
        // The byte-locked JCO output and both entry points are regenerated by admission.
        let mut args = vec![
            jco,
            "transpile",
            component.to_str().context("component path UTF8")?,
            "--out-dir",
            output.to_str().context("output path UTF8")?,
            "--name",
            "guest",
            "--no-wasi-shim",
            "--instantiation",
            "sync",
        ];
        if exceptions {
            args.push("--bindgen-enable-wasm-exnref");
        }
        command(
            crate::generator_tools::node()?
                .to_str()
                .context("Node path UTF8")?,
            &args,
            cwd,
        )?;
        let mut wasm_names = Vec::new();
        for entry in fs::read_dir(output)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 JCO file"))?;
            if name.ends_with(".wasm") {
                wasm_names.push(name);
            }
        }
        for (name, bytes) in crate::library_wasi::browser_files(&wasm_names)? {
            write(output.join(name), bytes)?;
        }
        write(
            output.join("index.d.ts"),
            include_bytes!("../assets/wasi-library/pure-index.d.ts"),
        )?;
        return Ok(());
    }
    let reactor_maps = [
        ("wasi:cli/environment", "environment"),
        ("wasi:cli/exit", "exit"),
        ("wasi:cli/stdin", "stdin"),
        ("wasi:cli/stdout", "stdout"),
        ("wasi:cli/stderr", "stderr"),
        ("wasi:cli/terminal-input", "terminalInput"),
        ("wasi:cli/terminal-output", "terminalOutput"),
        ("wasi:cli/terminal-stdin", "terminalStdin"),
        ("wasi:cli/terminal-stdout", "terminalStdout"),
        ("wasi:cli/terminal-stderr", "terminalStderr"),
        ("wasi:io/error", "error"),
        ("wasi:io/poll", "poll"),
        ("wasi:io/streams", "streams"),
        ("wasi:clocks/monotonic-clock", "monotonicClock"),
        ("wasi:clocks/wall-clock", "wallClock"),
        ("wasi:filesystem/types", "types"),
        ("wasi:filesystem/preopens", "preopens"),
        ("wasi:random/random", "random"),
        ("wasi:random/insecure-seed", "insecureSeed"),
        ("wasi:sockets/network", "network"),
        ("wasi:sockets/instance-network", "instanceNetwork"),
        ("wasi:sockets/tcp", "tcp"),
        ("wasi:sockets/tcp-create-socket", "tcpCreateSocket"),
        ("wasi:sockets/udp", "udp"),
        ("wasi:sockets/udp-create-socket", "udpCreateSocket"),
        ("wasi:sockets/ip-name-lookup", "ipNameLookup"),
    ]
    .map(|(interface, export)| format!("{interface}=./reactor-host.js#{export}"));
    let mut args = vec![
        jco,
        "transpile",
        component.to_str().context("component path UTF8")?,
        "--out-dir",
        output.to_str().context("output path UTF8")?,
        "--name",
        "index",
        "--no-wasi-shim",
    ];
    if asynchronous {
        args.extend(["--async-mode", "jspi"]);
    }
    if exceptions {
        args.push("--bindgen-enable-wasm-exnref");
    }
    if reactor {
        for mapping in &reactor_maps {
            args.extend(["--map", mapping]);
        }
    } else if wasi {
        args.extend([
            "--map",
            "wasi:http/*=./wasi-node/nodejs/http.js#*",
            "wasi:io/*=./wasi-node/nodejs/io.js#*",
            "wasi:sockets/*=./wasi-node/nodejs/sockets.js#*",
            "wasi:clocks/*=./wasi-node/nodejs/clocks.js#*",
        ]);
    }
    command(
        crate::generator_tools::node()?
            .to_str()
            .context("Node path UTF8")?,
        &args,
        cwd,
    )?;
    if reactor {
        write(
            output.join("reactor-host.js"),
            crate::library_wasi::REACTOR_NODE,
        )?;
    } else if wasi {
        for (path, bytes) in crate::library_wasi::node_files() {
            write(output.join(path), bytes)?;
        }
    }
    if wasi || asynchronous {
        let reason = if reactor {
            "unsupported-browser-wasi-reactor"
        } else if wasi {
            "unsupported-browser-wasi-network"
        } else if crate::library_error_context::enabled(resolve, world) {
            "unsupported-browser-error-context-lifecycle"
        } else {
            "unsupported-browser-async-lifecycle"
        };
        write(
            output.join("browser.js"),
            browser_rejection(resolve, world, reason)?,
        )?;
    }
    Ok(())
}

fn tree(root: &Path, prefix: &str, files: &mut crate::release::Files) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = format!(
            "{prefix}{}",
            entry
                .file_name()
                .to_str()
                .context("UTF8 generated filename")?
        );
        if entry.file_type()?.is_dir() {
            tree(&entry.path(), &format!("{path}/"), files)?;
        } else {
            ensure!(entry.file_type()?.is_file(), "generated non-regular file");
            files.insert(path, fs::read(entry.path())?);
        }
    }
    Ok(())
}

/// Admission reconstructs contract-bearing files without executing guest code.
pub fn validate(dir: &Path) -> Result<Value> {
    let dir = dir.canonicalize()?;
    let release = crate::release::strict_json(&admission::read_file(
        &dir.join("library-release.json"),
        admission::MAX_ARCHIVE_BYTES,
    )?)?;
    crate::release::exact(
        &release,
        &[
            "schema",
            "kind",
            "identity",
            "version",
            "componentSha256",
            "contract",
            "packages",
            "targets",
            "tools",
            "metadata",
            "targetRequirements",
        ],
        "library release",
    )?;
    ensure!(
        release["schema"] == 3 && release["kind"] == "component-library",
        "unsupported library release format"
    );
    ensure!(
        release["tools"] == tools(),
        "forged library tool or target support"
    );
    let metadata: crate::library_metadata::Metadata =
        serde_json::from_value(release["metadata"].clone())?;
    metadata.validate()?;
    let version = release["version"].as_str().context("library version")?;
    ensure!(
        semver::Version::parse(version)?.to_string() == version,
        "noncanonical library version"
    );
    let (cargo_name, npm_name) =
        package_names(release["identity"].as_str().context("library identity")?)?;
    let component = admission::read_file(
        &dir.join("component.wasm"),
        crate::limits::MAX_COMPONENT_BYTES,
    )?;
    ensure!(
        component.len() <= crate::limits::MAX_COMPONENT_BYTES as usize
            && release["componentSha256"] == digest(&component),
        "library Component digest mismatch"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&component)?;
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
        std::panic::catch_unwind(|| wit_parser::decoding::decode(&component)).map_err(|_| {
            anyhow::anyhow!("unsupported-component-WIT: pinned WIT decoder rejected type aliases")
        })??
    else {
        bail!("expected executable Component");
    };
    validate_type_expansion(&resolve, world)?;
    let wasi = crate::library_wasi::enabled(&resolve, world)?;
    let pure = crate::library_wasi::pure_browser_host(&resolve, world)?;
    let reactor = crate::library_wasi::reactor_host(&resolve, world)?;
    let error_context = crate::library_error_context::enabled(&resolve, world);
    let asynchronous = crate::library_async::enabled(&resolve, world) || error_context;
    ensure!(
        release["targets"]
            == targets(
                wasi,
                asynchronous,
                release["packages"].get("go").is_some(),
                pure
            ),
        "forged library tool or target support"
    );
    ensure!(
        release["contract"]
            == serde_json::to_value(crate::wit_model::AbiModel::from_resolve(&resolve, world)?)?,
        "library contract mismatch"
    );
    let go = release["packages"].get("go");
    crate::release::exact(
        &release["packages"],
        if go.is_some() {
            &["rust", "node", "go"]
        } else {
            &["rust", "node"]
        },
        "library packages",
    )?;
    if let Some(go) = go {
        crate::release::exact(go, &["name", "sha256", "platform"], "Go package")?;
        let name = go["name"].as_str().context("Go module name")?;
        let module_file =
            admission::read_file(&dir.join("go.mod"), admission::MAX_GO_DESCRIPTOR_BYTES)?;
        let info_file =
            admission::read_file(&dir.join("go.info"), admission::MAX_GO_DESCRIPTOR_BYTES)?;
        let bytes = admission::read_file(&dir.join("go.zip"), admission::MAX_ARCHIVE_BYTES)?;
        ensure!(go["sha256"] == digest(&bytes), "Go archive hash mismatch");
        let report = if go["platform"] == crate::library_go::portable::PLATFORM {
            crate::library_go::portable::verify_archive(
                &component,
                name,
                version,
                &bytes,
                &metadata.go_files(),
            )?
        } else if go["platform"] == crate::library_go::guest::PLATFORM {
            crate::library_go::guest::verify_archive(
                &component,
                name,
                version,
                &bytes,
                &metadata.go_files(),
            )?
        } else {
            crate::library_go::verify_archive_with_notices(
                &component,
                name,
                version,
                &bytes,
                &metadata.go_files(),
            )?
        };
        ensure!(
            go["platform"] == report["platform"],
            "forged Go platform support"
        );
        let (mut module, info) = crate::library_go::module_metadata(name, version)?;
        if go["platform"] == crate::library_go::portable::PLATFORM {
            module = crate::library_go::portable::module_file(name);
        }
        ensure!(
            module_file == module && info_file == info,
            "Go module descriptor mismatch"
        );
    }
    let mut archives = Vec::new();
    for (target, filename, name) in [
        ("rust", "rust.crate", cargo_name.as_str()),
        ("node", "node.tgz", npm_name.as_str()),
    ] {
        crate::release::exact(
            &release["packages"][target],
            &["name", "sha256"],
            "library package",
        )?;
        let archive = admission::read_file(&dir.join(filename), admission::MAX_ARCHIVE_BYTES)?;
        ensure!(
            release["packages"][target]["name"] == name
                && release["packages"][target]["sha256"] == digest(&archive),
            "library package identity or digest mismatch: {target}"
        );
        archives.push(crate::release::archive_files(&archive)?);
    }
    let rust = &archives[0];
    let root = format!("{cargo_name}-{version}/");
    let worker_files = rust
        .iter()
        .filter_map(|(path, bytes)| {
            let relative = path.strip_prefix(&root)?;
            relative
                .starts_with("worker/")
                .then(|| (relative.to_owned(), bytes.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    if worker::supports(&resolve, world) {
        wisp_worker_protocol::bundle::authenticate(&worker_files)?;
    } else {
        ensure!(
            worker_files.is_empty(),
            "unexpected worker bundle for native host contract"
        );
    }
    ensure!(
        release["targetRequirements"]
            == worker_requirements(
                requirements(
                    wasi,
                    asynchronous,
                    error_context,
                    go.and_then(|go| go["platform"].as_str()),
                    pure,
                    reactor,
                    uses_modern_exceptions(&component)
                ),
                &worker_files
            )?,
        "forged library target requirements"
    );
    let mut expected = vec![
        ("component.wasm", component.clone()),
        (
            "src/lib.rs",
            rust_source_with_worker_files(&resolve, world, &worker_files)?.into_bytes(),
        ),
        ("wit/host.wit", host_wit(&resolve, world)?.into_bytes()),
        ("wit/guest.wit", guest_wit(&resolve, world)?.into_bytes()),
        (
            "Cargo.toml.orig",
            cargo_manifest(
                &cargo_name,
                version,
                &metadata,
                wasi,
                asynchronous,
                reactor,
                worker::supports(&resolve, world),
            )?
            .into_bytes(),
        ),
    ];
    expected.extend(
        worker_files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.clone())),
    );
    let notices = metadata.files(false);
    expected.extend(
        notices
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.clone())),
    );
    for (path, bytes) in &expected {
        ensure!(
            rust.get(&format!("{root}{path}")) == Some(bytes),
            "regenerated Rust file differs: {path}"
        );
    }
    let expected_names = expected
        .iter()
        .map(|(name, _)| format!("{root}{name}"))
        .chain([format!("{root}Cargo.toml"), format!("{root}Cargo.lock")])
        .collect::<BTreeSet<_>>();
    ensure!(
        rust.keys().cloned().collect::<BTreeSet<_>>() == expected_names,
        "unexpected Rust package files"
    );
    let normalized = packaged_cargo_manifest(
        &cargo_name,
        &cargo_manifest(
            &cargo_name,
            version,
            &metadata,
            wasi,
            asynchronous,
            reactor,
            worker::supports(&resolve, world),
        )?,
    )?;
    let actual: toml::Value =
        toml::from_str(std::str::from_utf8(&rust[&format!("{root}Cargo.toml")])?)?;
    ensure!(
        actual == normalized,
        "Rust package descriptor differs from generated descriptor"
    );
    let lock: toml::Value =
        toml::from_str(std::str::from_utf8(&rust[&format!("{root}Cargo.lock")])?)?;
    ensure!(
        lock["version"].as_integer() == Some(4),
        "unsupported Cargo lock format"
    );
    for package in lock["package"].as_array().context("Cargo lock packages")? {
        if package["name"].as_str() == Some(cargo_name.as_str()) {
            ensure!(
                package["version"].as_str() == Some(version),
                "Cargo lock root identity mismatch"
            );
        } else {
            ensure!(
                package.get("source").and_then(toml::Value::as_str)
                    == Some("registry+https://github.com/rust-lang/crates.io-index")
                    && package
                        .get("checksum")
                        .and_then(toml::Value::as_str)
                        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
                "unapproved Cargo lock source"
            );
        }
    }
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("component.wasm");
    fs::write(&input, &component)?;
    let output = temp.path().join("node");
    transpile(
        &jco(&dir)?,
        &input,
        &output,
        &dir,
        wasi,
        asynchronous,
        pure,
        &resolve,
        world,
    )?;
    write(
        output.join("package.json"),
        serde_json::to_vec_pretty(&node_manifest(
            &npm_name,
            version,
            &metadata,
            wasi || asynchronous,
        ))?,
    )?;
    for (path, bytes) in metadata.files(true) {
        write(output.join(path), bytes)?;
    }
    let mut node = crate::release::Files::new();
    tree(&output, "package/", &mut node)?;
    ensure!(archives[1] == node, "regenerated Node package differs");
    Ok(
        json!({"valid":true,"schema":3,"identity":release["identity"],"version":version,"componentSha256":release["componentSha256"]}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    package: Identity,
    component: Component,
    #[serde(default)]
    metadata: crate::library_metadata::Authored,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    name: String,
    version: semver::Version,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Component {
    path: PathBuf,
    sha256: String,
}

fn read_rust_guest_release(directory: &Path) -> Result<Vec<u8>> {
    admission::read_file(
        &directory.join("library-release.json"),
        admission::MAX_METADATA_BYTES,
    )
}

pub fn run_cli(args: &[String]) -> Result<Value> {
    if args.len() == 4 && args[0] == "rust-guest" && args[2] == "--out" {
        let directory = Path::new(&args[1]).canonicalize()?;
        let report = validate(&directory)?;
        let release = crate::release::strict_json(&read_rust_guest_release(&directory)?)?;
        ensure!(
            release["identity"] == report["identity"]
                && release["version"] == report["version"]
                && release["componentSha256"] == report["componentSha256"],
            "library changed during Rust guest generation"
        );
        let identity = release["identity"].as_str().context("library identity")?;
        let version = release["version"].as_str().context("library version")?;
        let (crate_name, _) = package_names(identity)?;
        let output = Path::new(&args[3]);
        ensure!(!output.try_exists()?, "Rust guest output already exists");
        let component = admission::read_file(
            &directory.join("component.wasm"),
            crate::limits::MAX_COMPONENT_BYTES,
        )?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let stage = tempfile::Builder::new()
            .prefix(".wisp-rust-guest-")
            .tempdir_in(parent)?;
        let metadata: crate::library_metadata::Metadata =
            serde_json::from_value(release["metadata"].clone())?;
        metadata.validate()?;
        let mut notices = metadata
            .notices
            .iter()
            .map(|(name, text)| (format!("notices/{name}"), text.as_bytes().to_vec()))
            .collect::<BTreeMap<_, _>>();
        notices.insert("WISP-NOTICES.txt".into(), b"This Rust dependency contains an admitted WebAssembly Component and generated scalar import wrappers. Component notices are under notices/. Wisp does not infer a Component license.\n".to_vec());
        for (name, bytes) in
            rust_guest::files_with_notices(&component, &crate_name, version, &notices)?
        {
            write(stage.path().join(name), bytes)?;
        }
        let descriptor = rust_guest::installed(stage.path())?;
        fs::rename(stage.path(), output)?;
        return Ok(descriptor);
    }
    if matches!(
        args.first().map(String::as_str),
        Some("prepare" | "publish" | "fetch")
    ) {
        return crate::library_publish::run_cli(args);
    }
    if args.len() == 2 && args[0] == "validate" {
        return validate(Path::new(&args[1]));
    }
    ensure!(
        matches!(args.len(), 4 | 6)
            && args[0] == "pack"
            && args[2] == "--out"
            && (args.len() == 4 || matches!(args[4].as_str(), "--go-module" | "--go-guest-module")),
        "usage: wisp library pack PACKAGE_DIR --out OUTPUT [--go-module MODULE | --go-guest-module MODULE]"
    );
    let go_module = args.get(5).map(String::as_str);
    let go_guest = args.get(4).is_some_and(|v| v == "--go-guest-module");
    let dir = Path::new(&args[1]).canonicalize()?;
    let out = Path::new(&args[3]);
    ensure!(
        !out.try_exists()?,
        "output already exists; choose a fresh directory"
    );
    let settings: Settings = toml::from_str(&fs::read_to_string(dir.join("wisp.toml"))?)?;
    let metadata = settings.metadata.load(&dir)?;
    let (cargo_name, npm_name) = package_names(&settings.package.name)?;
    ensure!(
        !settings.component.path.as_os_str().is_empty()
            && settings
                .component
                .path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "unsafe component path"
    );
    let component_path = dir.join(&settings.component.path);
    ensure!(
        component_path.canonicalize()?.starts_with(&dir),
        "component path escapes package"
    );
    ensure!(
        fs::metadata(&component_path)?.len() <= crate::limits::MAX_COMPONENT_BYTES,
        "component size limit"
    );
    let bytes = fs::read(component_path)?;
    ensure!(
        digest(&bytes) == settings.component.sha256,
        "component sha256 mismatch"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)?;
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
        std::panic::catch_unwind(|| wit_parser::decoding::decode(&bytes)).map_err(|_| {
            anyhow::anyhow!("unsupported-component-WIT: pinned WIT decoder rejected type aliases")
        })??
    else {
        bail!("expected executable Component");
    };
    let mut authored = Resolve::default();
    let (package, _) = authored.push_dir(dir.join("wit"))?;
    let identity = &authored.packages[package].name;
    ensure!(
        format!("{}:{}", identity.namespace, identity.name) == settings.package.name
            && identity.version.as_ref() == Some(&settings.package.version),
        "WIT identity mismatch"
    );
    ensure!(
        authored.packages[package].worlds.len() == 1,
        "authored library requires exactly one world"
    );
    let authored_world = *authored.packages[package].worlds.values().next().unwrap();
    validate_type_expansion(&resolve, world)?;
    validate_type_expansion(&authored, authored_world)?;
    let model = crate::wit_model::AbiModel::from_resolve(&resolve, world)?;
    let authored_model = crate::wit_model::AbiModel::from_resolve(&authored, authored_world)?;
    ensure!(
        exports_match(&model, &authored_model),
        "WIT contract differs from executable Component"
    );
    let wasi = crate::library_wasi::enabled(&resolve, world)?;
    let pure = crate::library_wasi::pure_browser_host(&resolve, world)?;
    let reactor = crate::library_wasi::reactor_host(&resolve, world)?;
    let error_context = crate::library_error_context::enabled(&resolve, world);
    let asynchronous = crate::library_async::enabled(&resolve, world) || error_context;
    let worker_files = if worker::supports(&resolve, world) {
        wisp_worker_protocol::bundle::files()?
    } else {
        BTreeMap::new()
    };
    let source = rust_source_with_worker_files(&resolve, world, &worker_files)?;
    let contract_wit = wit(&resolve, world)?;
    let host_wit = host_wit(&resolve, world)?;
    let guest_wit = guest_wit(&resolve, world)?;
    let jco = jco(&dir)?;
    fs::create_dir_all(out)?;
    let out = out.canonicalize()?;
    let rust = out.join("rust");
    write(rust.join("src/lib.rs"), source)?;
    write(rust.join("wit/host.wit"), &host_wit)?;
    write(rust.join("wit/guest.wit"), guest_wit)?;
    write(rust.join("component.wasm"), &bytes)?;
    write(out.join("component.wasm"), &bytes)?;
    write(out.join("contract.wit"), contract_wit)?;
    let version = settings.package.version.to_string();
    write(
        rust.join("Cargo.toml"),
        cargo_manifest(
            &cargo_name,
            &version,
            &metadata,
            wasi,
            asynchronous,
            reactor,
            worker::supports(&resolve, world),
        )?,
    )?;
    for (path, bytes) in &worker_files {
        write(rust.join(path), bytes)?;
    }
    for (path, bytes) in metadata.files(false) {
        write(rust.join(path), bytes)?;
    }
    command(
        "cargo",
        &[
            "package",
            "--offline",
            "--no-verify",
            "--allow-dirty",
            "--target-dir",
            "target",
        ],
        &rust,
    )?;
    fs::copy(
        rust.join(format!("target/package/{cargo_name}-{version}.crate")),
        out.join("rust.crate"),
    )?;
    let node = out.join("node");
    fs::create_dir_all(&node)?;
    transpile(
        &jco,
        &out.join("component.wasm"),
        &node,
        &dir,
        wasi,
        asynchronous,
        pure,
        &resolve,
        world,
    )?;
    // JCO's generated entry point uses top-level await for explicit async module initialization.
    write(
        node.join("package.json"),
        serde_json::to_vec_pretty(&node_manifest(
            &npm_name,
            &version,
            &metadata,
            wasi || asynchronous,
        ))?,
    )?;
    for (path, bytes) in metadata.files(true) {
        write(node.join(path), bytes)?;
    }
    let packed: Value = serde_json::from_str(&command(
        "npm",
        &["pack", "--json", "--ignore-scripts"],
        &node,
    )?)?;
    fs::rename(
        node.join(packed[0]["filename"].as_str().context("npm archive")?),
        out.join("node.tgz"),
    )?;
    let mut release = json!({"schema":3,"kind":"component-library","identity":settings.package.name,"version":version,"componentSha256":digest(&bytes),"contract":model,"packages":{"rust":{"name":cargo_name,"sha256":digest(&fs::read(out.join("rust.crate"))?)},"node":{"name":npm_name,"sha256":digest(&fs::read(out.join("node.tgz"))?)}},"targets":targets(wasi, asynchronous, go_module.is_some(), pure),"tools":tools(),"metadata":metadata});
    let mut go_platform = None;
    if let Some(module) = go_module {
        let report = if crate::library_go::portable::supports(&bytes) {
            crate::library_go::portable::generate(
                &bytes,
                &out.join("go"),
                module,
                &version,
                &metadata.go_files(),
            )?
        } else if go_guest {
            ensure!(
                crate::library_go::guest::supports(&bytes),
                "unsupported synchronous Go guest contract"
            );
            crate::library_go::guest::generate(
                &bytes,
                &out.join("go"),
                module,
                &version,
                &metadata.go_files(),
            )?;
            json!({"platform":crate::library_go::guest::PLATFORM})
        } else {
            crate::library_go::generate_with_notices(
                &bytes,
                &out.join("go"),
                module,
                &version,
                &metadata.go_files(),
            )?
        };
        go_platform = Some(
            report["platform"]
                .as_str()
                .context("Go platform")?
                .to_owned(),
        );
        let artifacts = crate::library_go::module_archive(&out.join("go"), module, &version)?;
        release["packages"]["go"] =
            json!({"name":module,"sha256":digest(&artifacts.zip),"platform":go_platform});
        write(out.join("go.zip"), artifacts.zip)?;
        write(out.join("go.mod"), artifacts.mod_file)?;
        write(out.join("go.info"), artifacts.info)?;
    }
    release["targetRequirements"] = worker_requirements(
        requirements(
            wasi,
            asynchronous,
            error_context,
            go_platform.as_deref(),
            pure,
            reactor,
            uses_modern_exceptions(&bytes),
        ),
        &worker_files,
    )?;
    write(
        out.join("library-release.json"),
        serde_json::to_vec_pretty(&release)?,
    )?;
    validate(&out)?;
    Ok(release)
}
