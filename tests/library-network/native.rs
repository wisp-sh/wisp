use wisp_wisp__network::{Library, CallError};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let library = Library::new()?;
    let mut client = library.requests();
    let mut response = client.fetch(args[1].clone())?;
    assert_eq!(response.status()?, 200);
    assert_eq!(response.body()?, "network body");
    response.close()?;
    assert!(response.body().is_err());
    assert!(client.resolve("localhost".into())? > 0);
    assert!(matches!(client.resolve("invalid..name".into()), Err(CallError::Guest(_))));
    client.connect(args[2].parse()?)?;
    assert!(matches!(client.connect(args[3].parse()?), Err(CallError::Guest(_))));
    assert!(matches!(client.fetch(format!("http://127.0.0.1:{}/", args[3])), Err(CallError::Guest(_))));
    println!("network-native-passed");
    Ok(())
}
