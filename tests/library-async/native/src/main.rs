use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use wasmtime::component::{
    Component, ComponentType, FutureReader, Lift, Linker, Lower, StreamReader, TypedFunc,
};
use wasmtime::{Config, Engine, Store};
#[derive(Debug, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
struct Payload {
    label: String,
    values: Vec<u32>,
}
use std::sync::Condvar;
use std::task::Wake;
use std::time::{Duration, Instant};
struct ThreadWake {
    ready: Mutex<bool>,
    cv: Condvar,
}
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref()
    }
    fn wake_by_ref(self: &Arc<Self>) {
        *self.ready.lock().unwrap() = true;
        self.cv.notify_one()
    }
}
fn block_on<F: std::future::Future>(future: F) -> Result<F::Output, ()> {
    let signal = Arc::new(ThreadWake {
        ready: Mutex::new(false),
        cv: Condvar::new(),
    });
    let waker = Waker::from(signal.clone());
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Poll::Ready(v) = future.as_mut().poll(&mut cx) {
            return Ok(v);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(());
        }
        let ready = signal.ready.lock().unwrap();
        let (mut ready, timed) = signal
            .cv
            .wait_timeout_while(ready, deadline - now, |v| !*v)
            .unwrap();
        if timed.timed_out() && !*ready {
            return Err(());
        }
        *ready = false
    }
}

#[derive(Debug)]
pub enum CallError<E> {
    Guest(E),
    Runtime(String),
}
impl<E: std::fmt::Debug> std::fmt::Display for CallError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl<E: std::fmt::Debug> std::error::Error for CallError<E> {}
struct State {
    store: Store<()>,
    poisoned: bool,
}
type Deferred = Box<dyn FnOnce(&mut State) + Send>;
struct Access<'a> {
    state: Option<std::sync::MutexGuard<'a, State>>,
    deferred: &'a Mutex<Vec<Deferred>>,
}
impl Access<'_> {
    fn flush(&mut self) {
        loop {
            let pending = std::mem::take(&mut *self.deferred.lock().unwrap());
            if pending.is_empty() { break; }
            for close in pending { close(self.state.as_deref_mut().unwrap()); }
        }
    }
}
impl std::ops::Deref for Access<'_> {
    type Target = State;
    fn deref(&self) -> &State { self.state.as_deref().unwrap() }
}
impl std::ops::DerefMut for Access<'_> {
    fn deref_mut(&mut self) -> &mut State { self.state.as_deref_mut().unwrap() }
}
impl Drop for Access<'_> {
    fn drop(&mut self) {
        loop {
            let mut queue = self.deferred.lock().unwrap();
            if queue.is_empty() {
                self.state.take();
                break;
            }
            let pending = std::mem::take(&mut *queue);
            drop(queue);
            for close in pending { close(self.state.as_deref_mut().unwrap()); }
        }
    }
}
#[derive(Clone)]
pub struct Library {
    state: Arc<Mutex<State>>,
    deferred: Arc<Mutex<Vec<Deferred>>>,
    open_readers: Arc<std::sync::atomic::AtomicUsize>,
}
impl Library {
    fn access(&self) -> Result<Access<'_>, String> {
        let state = self.state.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => "Component instance is busy".to_owned(),
            std::sync::TryLockError::Poisoned(_) => "Component instance lock is poisoned".to_owned(),
        })?;
        let mut access = Access { state: Some(state), deferred: &self.deferred };
        access.flush();
        Ok(access)
    }
    fn defer(&self, close: impl FnOnce(&mut State) + Send + 'static) {
        self.deferred.lock().unwrap().push(Box::new(close));
        let _ = self.access();
    }
}
include!("../../../../crates/wisp/assets/templates/library-async.rs");
fn main() -> wasmtime::Result<()> {
    let mut config = Config::new();
    config
        .wasm_component_model(true)
        .wasm_component_model_async(true)
        .wasm_component_model_more_async_builtins(true)
        .wasm_component_model_async_stackful(true)
        .concurrency_support(true);
    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, ());
    let component =
        Component::from_file(&engine, std::env::args().nth(1).expect("component path"))?;
    let instance = Linker::new(&engine).instantiate(&mut store, &component)?;
    if std::env::args().any(|arg| arg == "--composed") {
        let interface = instance
            .get_export_index(&mut store, None, "wisp:async-consumer/check@0.1.0")
            .unwrap();
        let index = instance
            .get_export_index(&mut store, Some(&interface), "run")
            .unwrap();
        let run: TypedFunc<(), (u32,)> = instance.get_typed_func(&mut store, &index)?;
        let result = block_on(
            store.run_concurrent(async move |accessor| run.call_concurrent(&accessor, ()).await),
        )
        .unwrap()??;
        assert_eq!(result, (42,));
        block_on(store.run_concurrent(async |accessor| {
            std::future::poll_fn(|cx| accessor.poll_no_interesting_tasks(cx)).await
        }))
        .unwrap()?;
        store.assert_concurrent_state_empty();
        println!("composed async guest 42");
        return Ok(());
    }
    let interface = instance
        .get_export_index(&mut store, None, "wisp:async-values/operations@0.1.0")
        .unwrap();
    let idx = instance
        .get_export_index(&mut store, Some(&interface), "delayed")
        .unwrap();
    let delayed: TypedFunc<(Payload,), (FutureReader<Payload>,)> =
        instance.get_typed_func(&mut store, &idx)?;
    let idx = instance
        .get_export_index(&mut store, Some(&interface), "words")
        .unwrap();
    let words: TypedFunc<(Vec<String>,), (StreamReader<String>,)> =
        instance.get_typed_func(&mut store, &idx)?;
    let idx = instance
        .get_export_index(&mut store, Some(&interface), "active")
        .unwrap();
    let active: TypedFunc<(), (u32,)> = instance.get_typed_func(&mut store, &idx)?;
    let library = Library {
        state: Arc::new(Mutex::new(State {
            store,
            poisoned: false,
        })),
        deferred: Arc::new(Mutex::new(Vec::new())),
        open_readers: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let access = library.access().unwrap();
    let cleaned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = cleaned.clone();
    library.defer(move |_| flag.store(true, std::sync::atomic::Ordering::Release));
    assert!(!cleaned.load(std::sync::atomic::Ordering::Acquire));
    assert!(library.access().err().unwrap().contains("busy"));
    drop(access);
    assert!(cleaned.load(std::sync::atomic::Ordering::Acquire));
    for _ in 0..8 {
        let raw = {
            let mut state = library.state.lock().unwrap();
            let payload = Payload {
                label: "nested".into(),
                values: vec![3, 5],
            };
            block_on(state.store.run_concurrent(async move |accessor| {
                delayed.call_concurrent(&accessor, (payload,)).await
            }))
            .unwrap()??
            .0
        };
        let future = Future::from_reader(library.clone(), raw);
        assert_eq!(
            block_on(future.receive()).unwrap()?,
            Payload {
                label: "nested".into(),
                values: vec![3, 5]
            }
        );
        let raw = {
            let mut state = library.state.lock().unwrap();
            block_on(state.store.run_concurrent(async move |accessor| {
                words
                    .call_concurrent(&accessor, (vec!["hello".into(), "βeta".into()],))
                    .await
            }))
            .unwrap()??
            .0
        };
        let mut stream = Stream::from_reader(library.clone(), raw);
        assert_eq!(block_on(stream.next()).unwrap()?, Some("hello".into()));
        assert_eq!(block_on(stream.next()).unwrap()?, Some("βeta".into()));
        assert_eq!(block_on(stream.next()).unwrap()?, None);
        stream.close()?;
        assert!(block_on(stream.next()).unwrap().is_err());
    }
    for future_kind in [true, false] {
        if future_kind {
            let raw = {
                let mut state = library.state.lock().unwrap();
                block_on(state.store.run_concurrent(async move |accessor| {
                    delayed
                        .call_concurrent(
                            &accessor,
                            (Payload {
                                label: "cancel".into(),
                                values: vec![],
                            },),
                        )
                        .await
                }))
                .unwrap()??
                .0
            };
            drop(Future::from_reader(library.clone(), raw));
        } else {
            let raw = {
                let mut state = library.state.lock().unwrap();
                block_on(state.store.run_concurrent(async move |accessor| {
                    words
                        .call_concurrent(&accessor, (vec!["one".into(), "two".into()],))
                        .await
                }))
                .unwrap()??
                .0
            };
            let mut stream = Stream::from_reader(library.clone(), raw);
            assert_eq!(block_on(stream.next()).unwrap()?, Some("one".into()));
            drop(stream);
        }
        block_on(library.drain()).unwrap()?;
        let mut state = library.state.lock().unwrap();
        assert_eq!(active.call(&mut state.store, ())?, (0,));
        state.store.assert_concurrent_state_empty();
    }
    println!(
        "native async wrappers: future record, stream strings, drop cancellation, stale close, empty Store passed"
    );
    Ok(())
}
