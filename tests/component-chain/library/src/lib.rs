wit_bindgen::generate!({ path: "../wit", world: "library" });

use exports::wisp::library_chain::arithmetic::{CalculationError, Guest, LineItem};

struct Library;

impl Guest for Library {
    fn total(item: LineItem) -> Result<u64, CalculationError> {
        u64::from(item.quantity)
            .checked_mul(item.unit_price)
            .ok_or(CalculationError::Overflow)
    }
}

export!(Library);
