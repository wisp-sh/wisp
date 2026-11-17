use wisp_example__ocaml_values::{
    CallError, Library,
    api::{Mode, Point, Profile},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = Library::new()?;
    let mut api = library.api();
    assert_eq!(api.add(20, 20)?, 42);
    assert_eq!(api.wide(9_007_199_254_740_993)?, 9_007_199_254_740_994);
    assert_eq!(api.maybe(None)?, None);
    assert_eq!(api.maybe(Some("λ🌍".into()))?, Some("λ🌍!".into()));
    let points = api.points(vec![Point { x: 1.25, y: -2.5 }])?;
    assert_eq!((points[0].x, points[0].y), (1.75, -3.0));
    assert_eq!(api.safe_divide(84, 2)?, 42);
    assert!(matches!(api.safe_divide(1,0),Err(CallError::Guest(e)) if e == "division by zero"));
    api.check(true)?;
    assert!(matches!(api.check(false),Err(CallError::Guest(e)) if e == "false"));
    for i in 1..=200 {
        assert_eq!(api.next()?, i);
        let out = api.echo(Profile {
            name: "λ\0🌍".into(),
            payload: vec![0, 255, 128, 1],
            mode: Mode::Loud,
        })?;
        assert_eq!(out.name, "λ\0🌍!");
        assert_eq!(out.payload, vec![0, 255, 128, 1]);
        assert!(matches!(out.mode, Mode::Loud));
        api.collect()?;
    }
    assert!(api.explode().is_err());
    assert!(api.next().is_err());
    println!("ocaml-installed-rust-ok");
    Ok(())
}
