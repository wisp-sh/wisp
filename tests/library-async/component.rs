wit_bindgen::generate!({path:"wit",world:"consumer",with:{"wisp:async-values/operations@0.1.0":wisp_wisp__async_values::operations}});
use wisp_wisp__async_values::{Library, operations::Payload};
struct Consumer;
impl exports::wisp::async_consumer::check::Guest for Consumer {
    async fn run() -> u32 {
        let library = Library::new().unwrap();
        let mut operations = library.operations();
        let future = operations
            .delayed(Payload {
                label: "guest".into(),
                values: vec![13, 29],
            })
            .await
            .unwrap();
        let value = future.receive().await.unwrap();
        assert_eq!(value.label, "guest");
        assert_eq!(value.values, vec![13, 29]);
        let mut stream = operations
            .words(vec!["one".into(), "two".into()])
            .await
            .unwrap();
        assert_eq!(stream.next().await.unwrap(), Some("one".into()));
        assert_eq!(stream.next().await.unwrap(), Some("two".into()));
        assert_eq!(stream.next().await.unwrap(), None);
        let mut cancelled = operations.words(vec!["cancel".into()]).await.unwrap();
        cancelled.close().unwrap();
        let mut future = operations
            .delayed(Payload {
                label: "cancel".into(),
                values: vec![],
            })
            .await
            .unwrap();
        future.close().unwrap();
        library.drain().await.unwrap();
        42
    }
}
export!(Consumer);
