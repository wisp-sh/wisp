/// An owned Component error context tied to its originating library.
/// Native close invalidates this wrapper. Wasmtime retains source handles
/// until the Store is destroyed; the library permits 127 cumulative lifts.
pub struct ErrorContext {
    library: Library,
    #[cfg(not(target_arch = "wasm32"))]
    handle: Option<wasmtime::component::ErrorContext>,
    #[cfg(target_arch = "wasm32")]
    handle: Option<wit_bindgen::rt::async_support::ErrorContext>,
}
impl std::fmt::Debug for ErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErrorContext")
            .field("closed", &self.handle.is_none())
            .finish_non_exhaustive()
    }
}
impl ErrorContext {
    pub fn close(&mut self) {
        self.handle.take();
    }
    pub fn is_closed(&self) -> bool {
        self.handle.is_none()
    }
}
