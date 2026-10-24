wit_bindgen::generate!({ path: "../wit", world: "consumer" });

use exports::wisp::library_chain::checkout::Guest;
use wisp::library_chain::arithmetic::{self, CalculationError, LineItem};

struct Consumer;

impl Guest for Consumer {
    fn total(item: LineItem) -> Result<u64, CalculationError> {
        arithmetic::total(item)
    }
}

export!(Consumer);
