/// A readable Component future. Dropping it closes or cancels its reader.
#[cfg(not(target_arch = "wasm32"))]
pub struct Future<T: wasmtime::component::Lift + Send + 'static> {
    library: Library,
    reader: Option<wasmtime::component::FutureReader<T>>,
    shared: std::sync::Arc<async_runtime::Collected<T>>,
    registered: bool,
}
#[cfg(target_arch = "wasm32")]
pub struct Future<T: 'static> {
    reader: Option<wit_bindgen::rt::async_support::FutureReader<T>>,
}
/// A readable Component stream. Each next call requests one item.
#[cfg(not(target_arch = "wasm32"))]
pub struct Stream<T: wasmtime::component::Lift + Send + 'static> {
    library: Library,
    reader: Option<wasmtime::component::StreamReader<T>>,
    shared: std::sync::Arc<async_runtime::Collected<T>>,
    closed: bool,
    registered: bool,
}
#[cfg(target_arch = "wasm32")]
pub struct Stream<T: 'static> {
    reader: Option<wit_bindgen::rt::async_support::StreamReader<T>>,
}

#[cfg(not(target_arch = "wasm32"))]
mod async_runtime {
    use std::{
        collections::VecDeque,
        future::poll_fn,
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        task::{Context, Poll, Waker},
    };
    use wasmtime::component::{FutureConsumer, Lift, Source, StreamConsumer, StreamResult};
    pub(super) struct Collected<T> {
        pub values: Mutex<VecDeque<T>>,
        pub done: AtomicBool,
        cancelled: AtomicBool,
        permits: AtomicUsize,
        producer: Mutex<Option<Waker>>,
        waiter: Mutex<Option<Waker>>,
    }
    impl<T> Default for Collected<T> {
        fn default() -> Self {
            Self {
                values: Mutex::new(VecDeque::new()),
                done: AtomicBool::new(false),
                cancelled: AtomicBool::new(false),
                permits: AtomicUsize::new(0),
                producer: Mutex::new(None),
                waiter: Mutex::new(None),
            }
        }
    }
    impl<T> Collected<T> {
        fn wake(&self) {
            if let Some(w) = self.waiter.lock().unwrap().take() {
                w.wake()
            }
        }
        pub fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
            if let Some(w) = self.producer.lock().unwrap().take() {
                w.wake()
            }
            self.wake();
        }
        pub fn permit(&self) {
            self.permits.fetch_add(1, Ordering::Release);
            if let Some(w) = self.producer.lock().unwrap().take() {
                w.wake()
            }
        }
    }
    pub(super) struct Collector<T>(pub Arc<Collected<T>>);
    impl<T> Drop for Collector<T> {
        fn drop(&mut self) {
            self.0.done.store(true, Ordering::Release);
            self.0.wake()
        }
    }
    impl<D: Send + 'static, T: Lift + Send + 'static> FutureConsumer<D> for Collector<T> {
        type Item = T;
        fn poll_consume(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            mut store: wasmtime::StoreContextMut<'_, D>,
            mut source: Source<'_, T>,
            finish: bool,
        ) -> Poll<wasmtime::Result<()>> {
            if !finish && !self.0.cancelled.load(Ordering::Acquire) {
                let mut values = Vec::with_capacity(1);
                source.read(&mut store, &mut values)?;
                self.0.values.lock().unwrap().extend(values);
            }
            self.0.wake();
            Poll::Ready(Ok(()))
        }
    }
    impl<D: Send + 'static, T: Lift + Send + 'static> StreamConsumer<D> for Collector<T> {
        type Item = T;
        fn poll_consume(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            mut store: wasmtime::StoreContextMut<'_, D>,
            mut source: Source<'_, T>,
            finish: bool,
        ) -> Poll<wasmtime::Result<StreamResult>> {
            if self.0.cancelled.load(Ordering::Acquire) {
                return Poll::Ready(Ok(StreamResult::Dropped));
            }
            if finish {
                return Poll::Ready(Ok(StreamResult::Cancelled));
            }
            if self
                .0
                .permits
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                .is_err()
            {
                *self.0.producer.lock().unwrap() = Some(cx.waker().clone());
                return Poll::Pending;
            }
            let mut values = Vec::with_capacity(1);
            source.read(&mut store, &mut values)?;
            self.0.values.lock().unwrap().extend(values);
            self.0.wake();
            Poll::Ready(Ok(StreamResult::Completed))
        }
    }
    pub(super) async fn event<T>(shared: Arc<Collected<T>>, complete: bool) {
        poll_fn(move |cx| {
            if shared.done.load(Ordering::Acquire)
                || (!complete && !shared.values.lock().unwrap().is_empty())
            {
                Poll::Ready(())
            } else {
                *shared.waiter.lock().unwrap() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
    pub(super) fn error(
        error: impl std::fmt::Display,
    ) -> super::CallError<std::convert::Infallible> {
        super::CallError::Runtime(error.to_string())
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl<T: wasmtime::component::Lift + Send + 'static> Future<T> {
    fn from_reader(library: Library, reader: wasmtime::component::FutureReader<T>) -> Self {
        library.open_readers.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self {
            library,
            reader: Some(reader),
            shared: Default::default(),
            registered: true,
        }
    }
    pub async fn receive(mut self) -> Result<T, CallError<std::convert::Infallible>> {
        let mut state = self.library.access().map_err(async_runtime::error)?;
        if state.poisoned {
            return Err(async_runtime::error("Component instance is poisoned"));
        }
        self.reader
            .take()
            .ok_or_else(|| async_runtime::error("future is closed"))?
            .pipe(
                &mut state.store,
                async_runtime::Collector(self.shared.clone()),
            )
            .map_err(async_runtime::error)?;
        let result = state
            .store
            .run_concurrent(async |_| async_runtime::event(self.shared.clone(), true).await)
            .await;
        if let Err(error) = result {
            state.poisoned = true;
            return Err(async_runtime::error(error));
        }
        drop(state);
        let value = self.shared
            .values
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| async_runtime::error("future closed without a value"));
        self.unregister();
        value
    }
    fn unregister(&mut self) {
        if self.registered {
            self.registered = false;
            self.library.open_readers.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        }
    }
    pub fn close(&mut self) -> Result<(), CallError<std::convert::Infallible>> {
        self.unregister();
        self.shared.cancel();
        if self.reader.is_some() {
            match self.library.access() {
                Ok(mut state) => {
                    let mut reader = self.reader.take().expect("checked reader");
                    reader.close(&mut state.store).map_err(|error| {
                        state.poisoned = true;
                        async_runtime::error(error)
                    })?;
                }
                Err(message) if message == "Component instance is busy" => {
                    let mut reader = self.reader.take().expect("checked reader");
                    self.library.defer(move |state| {
                        if reader.close(&mut state.store).is_err() { state.poisoned = true; }
                    });
                }
                Err(message) => return Err(async_runtime::error(message)),
            }
        }
        Ok(())
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl<T: wasmtime::component::Lift + Send + 'static> Drop for Future<T> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
#[cfg(target_arch = "wasm32")]
impl<T: 'static> Future<T> {
    fn from_reader(
        _library: Library,
        reader: wit_bindgen::rt::async_support::FutureReader<T>,
    ) -> Self {
        Self {
            reader: Some(reader),
        }
    }
    pub async fn receive(mut self) -> Result<T, CallError<std::convert::Infallible>> {
        Ok(self
            .reader
            .take()
            .ok_or_else(|| CallError::Runtime("future is closed".into()))?
            .await)
    }
    pub fn close(&mut self) -> Result<(), CallError<std::convert::Infallible>> {
        self.reader.take();
        Ok(())
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl<T: wasmtime::component::Lift + Send + 'static> Stream<T> {
    fn from_reader(library: Library, reader: wasmtime::component::StreamReader<T>) -> Self {
        library.open_readers.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self {
            library,
            reader: Some(reader),
            shared: Default::default(),
            closed: false,
            registered: true,
        }
    }
    pub async fn next(&mut self) -> Result<Option<T>, CallError<std::convert::Infallible>> {
        if self.closed {
            return Err(async_runtime::error("stream is closed"));
        }
        let mut state = self.library.access().map_err(async_runtime::error)?;
        if state.poisoned {
            return Err(async_runtime::error("Component instance is poisoned"));
        }
        if let Some(reader) = self.reader.take() {
            reader
                .pipe(
                    &mut state.store,
                    async_runtime::Collector(self.shared.clone()),
                )
                .map_err(async_runtime::error)?
        }
        if self.shared.done.load(std::sync::atomic::Ordering::Acquire) {
            let value = self.shared.values.lock().unwrap().pop_front();
            drop(state);
            if value.is_none() { self.unregister(); }
            return Ok(value);
        }
        self.shared.permit();
        let result = state
            .store
            .run_concurrent(async |_| async_runtime::event(self.shared.clone(), false).await)
            .await;
        if let Err(error) = result {
            state.poisoned = true;
            return Err(async_runtime::error(error));
        }
        drop(state);
        let value = self.shared.values.lock().unwrap().pop_front();
        if value.is_none() && self.shared.done.load(std::sync::atomic::Ordering::Acquire) {
            self.unregister();
        }
        Ok(value)
    }
    fn unregister(&mut self) {
        if self.registered {
            self.registered = false;
            self.library.open_readers.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        }
    }
    pub fn close(&mut self) -> Result<(), CallError<std::convert::Infallible>> {
        self.unregister();
        self.shared.cancel();
        if self.reader.is_some() {
            match self.library.access() {
                Ok(mut state) => {
                    let mut reader = self.reader.take().expect("checked reader");
                    reader.close(&mut state.store).map_err(|error| {
                        state.poisoned = true;
                        async_runtime::error(error)
                    })?;
                }
                Err(message) if message == "Component instance is busy" => {
                    let mut reader = self.reader.take().expect("checked reader");
                    self.library.defer(move |state| {
                        if reader.close(&mut state.store).is_err() { state.poisoned = true; }
                    });
                }
                Err(message) => return Err(async_runtime::error(message)),
            }
        }
        self.closed = true;
        Ok(())
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl<T: wasmtime::component::Lift + Send + 'static> Drop for Stream<T> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
#[cfg(target_arch = "wasm32")]
impl<T: 'static> Stream<T> {
    fn from_reader(
        _library: Library,
        reader: wit_bindgen::rt::async_support::StreamReader<T>,
    ) -> Self {
        Self {
            reader: Some(reader),
        }
    }
    pub async fn next(&mut self) -> Result<Option<T>, CallError<std::convert::Infallible>> {
        Ok(self
            .reader
            .as_mut()
            .ok_or_else(|| CallError::Runtime("stream is closed".into()))?
            .next()
            .await)
    }
    pub fn close(&mut self) -> Result<(), CallError<std::convert::Infallible>> {
        self.reader.take();
        Ok(())
    }
}
impl Library {
    /// Wait for all background Component tasks after consuming or closing their readers.
    pub async fn drain(&self) -> Result<(), CallError<std::convert::Infallible>> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.open_readers.load(std::sync::atomic::Ordering::Acquire) != 0 {
                return Err(async_runtime::error("close Component readers before draining"));
            }
            let mut state = self.access().map_err(async_runtime::error)?;
            if self.open_readers.load(std::sync::atomic::Ordering::Acquire) != 0 {
                return Err(async_runtime::error("close Component readers before draining"));
            }
            state
                .store
                .run_concurrent(async |accessor| {
                    std::future::poll_fn(|cx| accessor.poll_no_interesting_tasks(cx)).await
                })
                .await
                .map_err(async_runtime::error)
        }
    }
}
