wit_bindgen::generate!({ path: "wit", world: "library" });
use exports::wisp::async_values::operations::{Guest, Payload};
use std::sync::atomic::{AtomicU32, Ordering};
use wit_bindgen::rt::async_support::{FutureReader, StreamReader, spawn_local};
static ACTIVE: AtomicU32 = AtomicU32::new(0);
struct Running;
impl Running {
    fn new() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Component;
impl Guest for Component {
    async fn delayed(value: Payload) -> FutureReader<Payload> {
        wit_bindgen::rt::async_support::yield_async().await;
        let (writer, reader) = wit_future::new(|| Payload {
            label: String::new(),
            values: Vec::new(),
        });
        let running = Running::new();
        spawn_local(async move {
            let _running = running;
            let _ = writer.write(value).await;
        });
        reader
    }
    async fn words(values: Vec<String>) -> StreamReader<String> {
        let (mut writer, reader) = wit_stream::new();
        let running = Running::new();
        spawn_local(async move {
            let _running = running;
            for value in values {
                if writer.write_one(value).await.is_some() {
                    break;
                }
            }
        });
        reader
    }
    fn active() -> u32 {
        ACTIVE.load(Ordering::SeqCst)
    }
}
export!(Component);
