wit_bindgen::generate!({ path: "wit", world: "consumer" });
use wisp_wisp__network::{Library, CallError};
struct Component;
impl Guest for Component {
    fn verify(url: String, open_port: u16, closed_port: u16) -> u32 {
        let library = Library::new().unwrap();
        let mut client = library.requests();
        let mut response = client.fetch(url).unwrap();
        assert_eq!(response.status().unwrap(), 200);
        assert_eq!(response.body().unwrap(), "network body");
        response.close().unwrap();
        assert!(response.body().is_err());
        assert!(client.resolve("localhost".into()).unwrap() > 0);
        assert!(matches!(client.resolve("invalid..name".into()), Err(CallError::Guest(_))));
        client.connect(open_port).unwrap();
        assert!(matches!(client.connect(closed_port), Err(CallError::Guest(_))));
        assert!(matches!(client.fetch(format!("http://127.0.0.1:{closed_port}/")), Err(CallError::Guest(_))));
        42
    }
}
export!(Component);
