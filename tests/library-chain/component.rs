wit_bindgen::generate!({
    path: "wit",
    world: "consumer",
    with: {
        "wisp:library-chain/arithmetic@0.1.0": wisp_wisp__library_chain::arithmetic,
    },
});

use wisp_wisp__library_chain::{arithmetic::{CalculationError, Client, LineItem}, CallError};

struct Consumer;

impl exports::wisp::library_consumer::checkout::Guest for Consumer {
    fn total(item: LineItem) -> Result<u64, CalculationError> {
        let mut library = Client::new().expect("guest client initialization");
        match library.total(item) {
            Ok(value) => Ok(value),
            Err(CallError::Guest(error)) => Err(error),
            Err(CallError::Runtime(error)) => panic!("{error}"),
        }
    }
}

export!(Consumer);
