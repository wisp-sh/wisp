use wisp_wisp__library_chain::{arithmetic::{CalculationError, Client, LineItem}, CallError};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arithmetic = Client::new()?;
    let total = arithmetic.total(LineItem { quantity: 3, unit_price: 7 })?;
    assert_eq!(total, 21);
    println!("ok({total})");
    match arithmetic.total(LineItem { quantity: 2, unit_price: u64::MAX }) {
        Err(CallError::Guest(CalculationError::Overflow)) => println!("err(overflow)"),
        other => panic!("expected the named overflow error, received {other:?}"),
    }
    assert_eq!(arithmetic.total(LineItem { quantity: 0, unit_price: u64::MAX })?, 0);
    println!("ok(0)");
    Ok(())
}
