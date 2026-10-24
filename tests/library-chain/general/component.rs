wit_bindgen::generate!({
    path: "wit", world: "consumer",
    with: {
        "wisp:catalog/types@0.1.0": wisp_wisp__catalog::types,
        "wisp:catalog/catalog@0.1.0": wisp_wisp__catalog::catalog,
        "wisp:catalog/formatting@0.1.0": wisp_wisp__catalog::formatting,
    },
});

use wisp_wisp__catalog::{catalog, formatting, types::{Failure, Request, Response}, CallError, Library};
struct Consumer;

impl exports::wisp::catalog_consumer::checkout::Guest for Consumer {
    fn process(value: Request) -> Result<Response, Failure> {
        match catalog::Client::new().unwrap().process(value) {
            Ok(value) => Ok(value),
            Err(CallError::Guest(error)) => Err(error),
            Err(CallError::Runtime(error)) => panic!("{error}"),
        }
    }
    fn echo(value: Response) -> Response { formatting::Client::new().unwrap().echo(value).unwrap() }
    fn greet(value: String) -> String { formatting::Client::new().unwrap().greet(value).unwrap() }
    fn count(values: Vec<String>) -> u32 { formatting::Client::new().unwrap().count(values).unwrap() }
    fn shared_state() -> u32 {
        let library = Library::new().unwrap();
        let mut catalog = library.catalog();
        let mut formatting = library.formatting();
        assert_eq!(formatting.current().unwrap(), 0);
        assert_eq!(catalog.increment().unwrap(), 1);
        assert_eq!(formatting.current().unwrap(), 1);
        assert_eq!(catalog.increment().unwrap(), 2);
        formatting.current().unwrap()
    }
}
export!(Consumer);
