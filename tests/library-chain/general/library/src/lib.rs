wit_bindgen::generate!({ path: "wit", world: "library" });

use exports::wisp::catalog::types::{Failure, Request, Response};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Catalog;

impl exports::wisp::catalog::catalog::Guest for Catalog {
    fn increment() -> u32 { COUNTER.fetch_add(1, Ordering::Relaxed) + 1 }
    fn process(mut value: Request) -> Result<Response, Failure> {
        if value.items.is_empty() {
            return Err(Failure::Empty);
        }
        if let Some(item) = value.items.iter().find(|item| item.amount == 0) {
            return Err(Failure::Invalid(item.name.clone()));
        }
        Ok(Response {
            summary: format!("{}:{}", value.label.unwrap_or_else(|| "default".into()), value.items[0].name),
            counts: value.items.iter().map(|item| item.amount).collect(),
            first: Some(value.items.remove(0)),
            policy: value.policy,
            metadata: value.metadata,
            detail: value.detail,
        })
    }
}

impl exports::wisp::catalog::formatting::Guest for Catalog {
    fn echo(value: Response) -> Response { value }
    fn greet(value: String) -> String { format!("hello {value}") }
    fn count(values: Vec<String>) -> u32 { values.len() as u32 }
    fn current() -> u32 { COUNTER.load(Ordering::Relaxed) }
}

export!(Catalog);
