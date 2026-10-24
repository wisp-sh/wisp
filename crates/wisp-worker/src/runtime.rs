use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use wasmtime::component::{Component, Instance, Val};
use wasmtime::{Engine, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wisp_worker_protocol::{Request, Response, VERSION};

pub struct Host {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    pub p1: wasmtime_wasi::p1::WasiP1Ctx,
    pub limits: StoreLimits,
}
impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
pub struct Session {
    pub owner: u64,
    pub store: Store<Host>,
    pub instance: Instance,
    pub handles: HashMap<u64, Val>,
    pub next_handle: u64,
    pub stdout: MemoryOutputPipe,
    pub stderr: MemoryOutputPipe,
    stdout_offset: usize,
    stderr_offset: usize,
}
pub struct Worker {
    pub engine: Engine,
    allow_precompiled: bool,
    modules: HashMap<String, Module>,
    components: HashMap<String, Component>,
    pub sessions: HashMap<u64, Session>,
    next_session: u64,
    stop: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Worker {
    pub fn new() -> Result<Self> {
        Self::with_precompiled(false)
    }
    pub fn with_precompiled(allow_precompiled: bool) -> Result<Self> {
        let mut config = wasmtime::Config::new();
        config
            .wasm_component_model(true)
            .wasm_component_model_async(true)
            .wasm_component_model_more_async_builtins(true)
            .wasm_component_model_error_context(true)
            .wasm_component_model_async_stackful(true)
            .concurrency_support(true)
            .wasm_exceptions(true)
            .epoch_interruption(true)
            .consume_fuel(true)
            .max_wasm_stack(32 * 1024 * 1024)
            .async_stack_size(64 * 1024 * 1024)
            .parallel_compilation(false)
            .cranelift_opt_level(wasmtime::OptLevel::None);
        if std::env::var_os("XDG_CACHE_HOME").is_some() {
            config.cache(Some(wasmtime::Cache::from_file(None)?));
        }
        let engine = Engine::new(&config)?;
        let stop = Arc::new(AtomicBool::new(false));
        let clock = engine.clone();
        let done = stop.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                clock.increment_epoch();
            }
        });
        Ok(Self {
            engine,
            allow_precompiled,
            modules: HashMap::new(),
            components: HashMap::new(),
            sessions: HashMap::new(),
            next_session: 1,
            stop,
        })
    }
    pub fn precompile(&self, path: &str) -> Result<Vec<u8>> {
        let (bytes, _) = Self::bytes(path, None)?;
        match bytes.get(..8) {
            Some(b"\0asm\x01\0\0\0") => Ok(self.engine.precompile_module(&bytes)?),
            Some(b"\0asm\x0d\0\x01\0") => Ok(self.engine.precompile_component(&bytes)?),
            _ => bail!("precompile requires a binary Wasm module or Component"),
        }
    }
    fn bytes(path: &str, expected: Option<&str>) -> Result<(Vec<u8>, String)> {
        let file = fs::File::open(path).with_context(|| format!("reading Wasm artifact {path}"))?;
        ensure!(
            file.metadata()?.len() <= 512 * 1024 * 1024,
            "Wasm artifact size limit"
        );
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(512 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 512 * 1024 * 1024, "Wasm artifact size limit");
        let hash = format!("{:x}", Sha256::digest(&bytes));
        if let Some(expected) = expected {
            ensure!(hash == expected, "Wasm artifact checksum mismatch");
        }
        Ok((bytes, hash))
    }
    fn core(&mut self, path: &str) -> Result<Module> {
        let (bytes, hash) = Self::bytes(path, None)?;
        self.core_bytes(&bytes, hash)
    }
    fn core_bytes(&mut self, bytes: &[u8], hash: String) -> Result<Module> {
        if let Some(module) = self.modules.get(&hash) {
            return Ok(module.clone());
        }
        let module = if Engine::detect_precompiled(bytes).is_some() {
            ensure!(
                self.allow_precompiled,
                "precompiled artifacts require --allow-precompiled and must come from a trusted compiler"
            );
            // The launcher explicitly opts into trusted native artifacts. Use
            // the checked bytes rather than reopening a mutable path.
            unsafe { Module::deserialize(&self.engine, bytes)? }
        } else {
            Module::new(&self.engine, bytes)?
        };
        if self.modules.len() >= 64 {
            self.modules.clear();
        }
        self.modules.insert(hash, module.clone());
        Ok(module)
    }
    fn component(&mut self, bytes: &[u8], hash: String) -> Result<Component> {
        if let Some(component) = self.components.get(&hash) {
            return Ok(component.clone());
        }
        let component = if Engine::detect_precompiled(bytes).is_some() {
            ensure!(
                self.allow_precompiled,
                "precompiled artifacts require --allow-precompiled and must come from a trusted compiler"
            );
            // The process owner opted into loading trusted native code.
            unsafe { Component::deserialize(&self.engine, bytes)? }
        } else {
            Component::new(&self.engine, bytes)?
        };
        if self.components.len() >= 32 {
            self.components.clear();
        }
        self.components.insert(hash, component.clone());
        Ok(component)
    }
    fn context(
        request: &Request,
        stdout: MemoryOutputPipe,
        stderr: MemoryOutputPipe,
    ) -> Result<WasiCtxBuilder> {
        let mut builder = WasiCtxBuilder::new();
        builder
            .arg(&request.module)
            .args(&request.args)
            .envs(&request.env.iter().collect::<Vec<_>>())
            .stdin(MemoryInputPipe::new(STANDARD.decode(&request.stdin)?))
            .stdout(stdout)
            .stderr(stderr)
            .allow_tcp(request.network)
            .allow_udp(request.network)
            .allow_ip_name_lookup(request.network);
        if request.network {
            builder.inherit_network();
        }
        for mount in &request.mounts {
            let dir = fs::canonicalize(&mount.host)?;
            let perms = if mount.read_only {
                wasmtime_wasi::FsPerms::ReadOnly
            } else {
                wasmtime_wasi::FsPerms::ReadWrite
            };
            builder.preopened_dir(dir, &mount.guest, perms)?;
        }
        Ok(builder)
    }
    fn store(
        &self,
        request: &Request,
    ) -> Result<(Store<Host>, MemoryOutputPipe, MemoryOutputPipe)> {
        ensure!(
            request.limits.output_bytes <= 32 * 1024 * 1024,
            "output limit exceeds 32 MiB"
        );
        ensure!(request.limits.timeout_ms > 0, "timeout must be positive");
        let stdout = MemoryOutputPipe::new(request.limits.output_bytes);
        let stderr = MemoryOutputPipe::new(request.limits.output_bytes);
        let wasi = Self::context(request, stdout.clone(), stderr.clone())?.build();
        let p1 = Self::context(request, stdout.clone(), stderr.clone())?.build_p1();
        let mut store = Store::new(
            &self.engine,
            Host {
                wasi,
                p1,
                table: ResourceTable::new(),
                limits: StoreLimitsBuilder::new()
                    .memory_size(request.limits.memory_bytes)
                    .memories(32)
                    .instances(256)
                    .tables(256)
                    .build(),
            },
        );
        store.limiter(|host| &mut host.limits);
        store.set_epoch_deadline(request.limits.timeout_ms.div_ceil(10));
        store.set_fuel(request.limits.fuel)?;
        Ok((store, stdout, stderr))
    }
    async fn open(&mut self, request: &Request, bytes: &[u8], hash: String) -> Result<Session> {
        let component = self.component(bytes, hash)?;
        let (mut store, stdout, stderr) = self.store(request)?;
        let mut linker = wasmtime::component::Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        if request.wasi_p3 {
            wasmtime_wasi::p3::add_to_linker(&mut linker)?;
        }
        for (name, path) in &request.modules {
            linker.root().module(name, &self.core(path)?)?;
        }
        let instance = linker.instantiate_async(&mut store, &component).await?;
        Ok(Session {
            owner: self.next_session,
            store,
            instance,
            handles: HashMap::new(),
            next_handle: 1,
            stdout,
            stderr,
            stdout_offset: 0,
            stderr_offset: 0,
        })
    }
    pub async fn execute(&mut self, request: Request) -> Response {
        let mut response = Response::new(request.id);
        let result = self.execute_inner(&request, &mut response).await;
        if let Err(error) = result {
            if let Some(exit) = error.downcast_ref::<wasmtime_wasi::I32Exit>() {
                response.exit_code = exit.0;
                response.ok = exit.0 == 0;
            } else {
                response.ok = false;
                response.exit_code = 1;
                response.error = Some(format!("{error:#}"));
            }
        }
        response
    }
    async fn execute_inner(&mut self, request: &Request, response: &mut Response) -> Result<()> {
        ensure!(
            request.version == VERSION,
            "unsupported worker protocol version"
        );
        ensure!(request.limits.timeout_ms > 0, "timeout must be positive");
        match request.operation.as_str() {
            "ping" => {
                response.results.push(serde_json::json!({"worker":"0.2.0","wasmtime":wisp_worker_protocol::WASMTIME_VERSION,"protocol":VERSION,"exceptions":true,"componentModuleImports":true}));
                return Ok(());
            }
            "close" => {
                ensure!(
                    self.sessions.remove(&request.instance).is_some(),
                    "unknown instance"
                );
                return Ok(());
            }
            "call" | "drop" => {
                let mut session = self
                    .sessions
                    .remove(&request.instance)
                    .context("unknown or terminated instance")?;
                session
                    .store
                    .set_epoch_deadline(request.limits.timeout_ms.div_ceil(10));
                session.store.set_fuel(request.limits.fuel)?;
                let result = if request.operation == "drop" {
                    session.drop_handle(request.handle).await
                } else {
                    session.call(request, response).await
                };
                session.output(response);
                // A trapped instance is never reused. Successful calls retain handles and state.
                if result.is_ok() {
                    self.sessions.insert(request.instance, session);
                }
                return result;
            }
            "open" | "run" => {}
            _ => bail!("unknown worker operation"),
        }
        let (bytes, hash) = Self::bytes(&request.module, request.sha256.as_deref())?;
        if bytes.get(..8) == Some(b"\0asm\x0d\0\x01\0")
            || Engine::detect_precompiled(&bytes) == Some(wasmtime::Precompiled::Component)
        {
            let mut session = self.open(request, &bytes, hash).await?;
            if request.operation == "open" {
                ensure!(self.sessions.len() < 128, "open instance limit");
                response.instance = self.next_session;
                self.next_session += 1;
                session.output(response);
                self.sessions.insert(response.instance, session);
                return Ok(());
            }
            let result = if !request.function.is_empty() {
                session.call(request, response).await
            } else if request.wasi_p3 {
                match wasmtime_wasi::p3::bindings::Command::new(
                    &mut session.store,
                    &session.instance,
                ) {
                    Ok(command) => {
                        let result = session
                            .store
                            .run_concurrent(async |store| {
                                command.wasi_cli_run().call_run(store).await
                            })
                            .await??;
                        result.map_err(|()| wasmtime_wasi::I32Exit(1).into())
                    }
                    Err(_) => session.command().await,
                }
            } else {
                session.command().await
            };
            response.stdout = STANDARD.encode(session.stdout.contents());
            response.stderr = STANDARD.encode(session.stderr.contents());
            result
        } else {
            ensure!(request.operation == "run", "open requires a Component");
            let module = self.core_bytes(&bytes, hash)?;
            let (mut store, stdout, stderr) = self.store(request)?;
            let mut linker = wasmtime::Linker::new(&self.engine);
            wasmtime_wasi::p1::add_to_linker_async(&mut linker, |host: &mut Host| &mut host.p1)?;
            for (name, path) in &request.modules {
                linker
                    .module_async(&mut store, name, &self.core(path)?)
                    .await?;
            }
            let result: Result<()> = async {
                let instance = linker.instantiate_async(&mut store, &module).await?;
                if let Some(initialize) = instance
                    .get_typed_func::<(), ()>(&mut store, "_initialize")
                    .ok()
                {
                    initialize.call_async(&mut store, ()).await?;
                }
                let name = if request.function.is_empty() {
                    "_start"
                } else {
                    &request.function
                };
                let function = instance
                    .get_func(&mut store, name)
                    .with_context(|| format!("missing core export {name}"))?;
                let ty = function.ty(&store);
                let params = crate::values::core_params(ty.params().collect(), &request.params)?;
                let mut results = vec![wasmtime::Val::null_func_ref(); ty.results().len()];
                function
                    .call_async(&mut store, &params, &mut results)
                    .await?;
                response.results = results
                    .iter()
                    .map(crate::values::core_json)
                    .collect::<Result<_>>()?;
                Ok(())
            }
            .await;
            response.stdout = STANDARD.encode(stdout.contents());
            response.stderr = STANDARD.encode(stderr.contents());
            result
        }
    }
}
impl Session {
    fn output(&mut self, response: &mut Response) {
        let stdout = self.stdout.contents();
        let stderr = self.stderr.contents();
        response.stdout = STANDARD.encode(&stdout[self.stdout_offset..]);
        response.stderr = STANDARD.encode(&stderr[self.stderr_offset..]);
        self.stdout_offset = stdout.len();
        self.stderr_offset = stderr.len();
    }
    async fn command(&mut self) -> Result<()> {
        wasmtime_wasi::p2::bindings::Command::new(&mut self.store, &self.instance)?
            .wasi_cli_run()
            .call_run(&mut self.store)
            .await?
            .map_err(|()| wasmtime_wasi::I32Exit(1).into())
    }
    async fn call(&mut self, request: &Request, response: &mut Response) -> Result<()> {
        let parent = if request.interface.is_empty() {
            None
        } else {
            Some(
                self.instance
                    .get_export(&mut self.store, None, &request.interface)
                    .context("missing exported interface")?
                    .1,
            )
        };
        let index = self
            .instance
            .get_export(&mut self.store, parent.as_ref(), &request.function)
            .context("missing exported function")?
            .1;
        let function = self
            .instance
            .get_func(&mut self.store, &index)
            .context("export is not a function")?;
        let types = function.ty(&self.store);
        // Validate results too, before decoding consumes any owned handles or
        // guest execution creates an async reader that IPC cannot manage.
        for ty in types.params().map(|(_, ty)| ty).chain(types.results()) {
            crate::values::ensure_transfer_supported(&ty)?;
        }
        ensure!(
            types.params().len() == request.params.len(),
            "incorrect parameter count"
        );
        let params = types
            .params()
            .zip(&request.params)
            .map(|((_, ty), json)| crate::values::decode(self, &ty, json))
            .collect::<Result<Vec<_>>>()?;
        let mut results = vec![Val::Bool(false); types.results().len()];
        self.store
            .run_concurrent(async |store| {
                function.call_concurrent(store, &params, &mut results).await
            })
            .await??;
        response.results = results
            .into_iter()
            .map(|value| crate::values::encode(self, value))
            .collect::<Result<_>>()?;
        Ok(())
    }
    async fn drop_handle(&mut self, id: u64) -> Result<()> {
        match self
            .handles
            .remove(&id)
            .context("unknown or closed handle")?
        {
            Val::Resource(resource) => resource.resource_drop_async(&mut self.store).await?,
            Val::ErrorContext(_) => {}
            _ => bail!("handle does not support resource drop"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::{decode, encode, ensure_transfer_supported};
    use serde_json::{Value, json};
    use wasmtime::component::{Resource, ResourceAny, ResourceType, Type, types::ComponentItem};

    async fn session() -> Session {
        let mut worker = Worker::new().unwrap();
        worker
            .open(&Request::default(), b"(component)", "empty-test".into())
            .await
            .unwrap()
    }

    fn parameter_type(session: &Session, definition: &str) -> Type {
        let engine = session.store.engine();
        let component = Component::new(
            engine,
            // Records and variants need an imported type identity before they
            // can appear in an imported function's signature.
            format!(
                "(component {definition}
                    (import \"value-type\" (type $param (eq $t)))
                    (import \"f\" (func (param \"value\" $param))))"
            ),
        )
        .unwrap();
        let ty = component.component_type();
        let function = ty
            .imports(engine)
            .find_map(|(_, import)| match import.ty {
                ComponentItem::ComponentFunc(function) => Some(function),
                _ => None,
            })
            .expect("expected function type");
        let (_, ty) = function.params().next().unwrap();
        ty
    }

    #[tokio::test]
    async fn nested_options_roundtrip_every_discriminant() {
        let mut session = session().await;
        let ty = parameter_type(
            &session,
            "(type $inner (option u32)) (type $middle (option $inner)) (type $t (option $middle))",
        );
        let mut value = Val::U32(42);
        let mut wire = json!(42);
        let mut cases = vec![];
        for _ in 0..3 {
            value = Val::Option(Some(Box::new(value)));
            wire = json!({"some": wire});
        }
        cases.push((value, wire));
        for depth in 0..3 {
            let mut value = Val::Option(None);
            let mut wire = json!({"none": null});
            for _ in 0..depth {
                value = Val::Option(Some(Box::new(value)));
                wire = json!({"some": wire});
            }
            cases.push((value, wire));
        }
        for (value, wire) in cases {
            assert_eq!(encode(&mut session, value.clone()).unwrap(), wire);
            assert_eq!(decode(&mut session, &ty, &wire).unwrap(), value);
        }
    }

    #[tokio::test]
    async fn options_reject_ambiguous_or_malformed_tags_and_payloads() {
        let mut session = session().await;
        let ty = parameter_type(
            &session,
            "(type $inner (option u32)) (type $t (option $inner))",
        );
        for wire in [
            Value::Null,
            json!(42),
            json!([]),
            json!({}),
            json!({"none": false}),
            json!({"none": null, "extra": null}),
            json!({"none": null, "some": {"none": null}}),
            json!({"other": null}),
            json!({"some": null}),
            json!({"some": {"none": 0}}),
            json!({"some": {"some": "wrong type"}}),
            json!({"some": {"some": 4_294_967_296_u64}}),
        ] {
            assert!(decode(&mut session, &ty, &wire).is_err(), "accepted {wire}");
        }
    }

    #[tokio::test]
    async fn options_in_containers_keep_their_tags() {
        let mut session = session().await;
        let ty = parameter_type(
            &session,
            r#"(type $o (option u32))
                (type $l (list $o))
                (type $r (result $o (error $o)))
                (type $variant (variant (case "empty") (case "value" $o)))
                (import "variant" (type $v (eq $variant)))
                (type $t (tuple $l $r $v))"#,
        );
        let value = Val::Tuple(vec![
            Val::List(vec![
                Val::Option(None),
                Val::Option(Some(Box::new(Val::U32(7)))),
            ]),
            Val::Result(Ok(Some(Box::new(Val::Option(None))))),
            Val::Variant("value".into(), Some(Box::new(Val::Option(None)))),
        ]);
        let wire = encode(&mut session, value.clone()).unwrap();
        assert_eq!(decode(&mut session, &ty, &wire).unwrap(), value);
    }

    #[tokio::test]
    async fn future_and_stream_types_are_rejected_through_containers() {
        let session = session().await;
        for kind in ["future", "stream"] {
            for wrapper in [
                "(type $t (option $async))",
                "(type $t (list $async))",
                "(type $t (tuple u32 $async))",
                "(type $t (record (field \"value\" $async)))",
                "(type $t (result $async))",
                "(type $t (result (error $async)))",
                "(type $t (variant (case \"empty\") (case \"value\" $async)))",
            ] {
                let ty = parameter_type(&session, &format!("(type $async ({kind} u32)) {wrapper}"));
                let error = ensure_transfer_supported(&ty).unwrap_err().to_string();
                assert_eq!(
                    error,
                    format!("{kind} transfer is not supported by the worker protocol")
                );
            }
        }
    }

    struct Counter;
    struct OtherResource;

    fn resource_handle(session: &mut Session) -> (Value, ResourceAny) {
        let resource =
            ResourceAny::try_from_resource(Resource::<Counter>::new_own(7), &mut session.store)
                .unwrap();
        (encode(session, Val::Resource(resource)).unwrap(), resource)
    }

    #[tokio::test]
    async fn resource_borrows_retain_handles_and_owned_transfers_consume_them() {
        let mut session = session().await;
        let (wire, resource) = resource_handle(&mut session);
        let id = wire["handle"].as_u64().unwrap();
        let borrowed = Type::Borrow(resource.ty());
        for _ in 0..2 {
            assert_eq!(
                decode(&mut session, &borrowed, &wire).unwrap(),
                Val::Resource(resource)
            );
            assert!(session.handles.contains_key(&id));
        }
        let owned = Type::Own(resource.ty());
        assert_eq!(
            decode(&mut session, &owned, &wire).unwrap(),
            Val::Resource(resource)
        );
        assert!(!session.handles.contains_key(&id));
        assert!(decode(&mut session, &owned, &wire).is_err());
        assert!(session.drop_handle(id).await.is_err());
        resource
            .resource_drop_async(&mut session.store)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn invalid_resource_handles_do_not_consume_the_original() {
        let mut session = session().await;
        let (wire, resource) = resource_handle(&mut session);
        let id = wire["handle"].as_u64().unwrap();
        let own = Type::Own(resource.ty());
        let foreign = json!({"instance": session.owner + 1, "handle": id});
        let closed = json!({"instance": session.owner, "handle": id + 1});
        assert!(decode(&mut session, &own, &foreign).is_err());
        assert!(decode(&mut session, &own, &closed).is_err());
        assert!(
            decode(
                &mut session,
                &Type::Own(ResourceType::host::<OtherResource>()),
                &wire
            )
            .is_err()
        );
        assert!(decode(&mut session, &Type::ErrorContext, &wire).is_err());
        assert!(session.handles.contains_key(&id));
        session.drop_handle(id).await.unwrap();
        assert!(decode(&mut session, &own, &wire).is_err());
        assert!(session.drop_handle(id).await.is_err());
    }

    #[tokio::test]
    async fn borrowed_resource_cannot_be_transferred_as_owned() {
        let session = session().await;
        let engine = session.store.engine().clone();
        let component = Component::new(
            &engine,
            r#"(component
                (import "check" (func $check (result bool)))
                (core func $check (canon lower (func $check)))
                (core module $m
                    (import "host" "check" (func $check (result i32)))
                    (func (export "run") (result i32) call $check))
                (core instance $i (instantiate $m
                    (with "host" (instance (export "check" (func $check))))))
                (func (export "run") (result bool)
                    (canon lift (core func $i "run"))))"#,
        )
        .unwrap();
        let mut linker = wasmtime::component::Linker::<Session>::new(&engine);
        linker
            .root()
            .func_new_async("check", |mut store, _, _, results| {
                Box::new(async move {
                    // The call store supplies the active borrow scope. Its Session
                    // data supplies the worker handle table used by the codecs.
                    let resource = ResourceAny::try_from_resource(
                        Resource::<Counter>::new_borrow(7),
                        &mut store,
                    )?;
                    assert!(!resource.owned());
                    let session = store.data_mut();
                    let wire = encode(session, Val::Resource(resource)).unwrap();
                    let id = wire["handle"].as_u64().unwrap();
                    let error = decode(session, &Type::Own(resource.ty()), &wire).unwrap_err();
                    assert!(error.to_string().contains("borrowed resource"));
                    assert!(session.handles.contains_key(&id));
                    assert_eq!(
                        decode(session, &Type::Borrow(resource.ty()), &wire).unwrap(),
                        Val::Resource(resource)
                    );
                    assert_eq!(session.handles.remove(&id), Some(Val::Resource(resource)));
                    // Wasmtime 50 rejects blocking calls inside an async call.
                    resource.resource_drop_async(&mut store).await?;
                    results[0] = Val::Bool(true);
                    Ok(())
                })
            })
            .unwrap();
        let mut store = Store::new(&engine, session);
        store.set_fuel(10_000).unwrap();
        store.set_epoch_deadline(100);
        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .unwrap();
        let run = instance
            .get_typed_func::<(), (bool,)>(&mut store, "run")
            .unwrap();
        assert_eq!(run.call_async(&mut store, ()).await.unwrap(), (true,));
    }
}
