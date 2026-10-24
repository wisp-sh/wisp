#[test]
fn jq_installed_crate_positive_and_negative_controls() {
    use wisp_wisp__jq::Library;
    let library = Library::new().expect("installed Component instantiates");
    let mut jq = library.world();
    assert_eq!(
        jq.evaluate(".items[] | . * 2".into(), "{\"items\":[1,2,3]}".into())
            .unwrap(),
        vec!["2", "4", "6"]
    );
    assert_eq!(
        jq.evaluate("select(.ok)".into(), "{\"ok\":false}".into())
            .unwrap(),
        Vec::<String>::new()
    );
    let bad_filter = jq.evaluate(".[".into(), "{}".into()).unwrap_err();
    assert!(format!("{bad_filter:?}").contains("Guest"), "{bad_filter:?}");
    let bad_json = jq.evaluate(".".into(), "{".into()).unwrap_err();
    assert!(format!("{bad_json:?}").contains("Guest"), "{bad_json:?}");
    let clock = jq.evaluate("now".into(), "null".into()).unwrap_err();
    assert!(
        format!("{clock:?}").contains("WISP_DENIED: wall clock"),
        "{clock:?}"
    );
    assert!(
        format!("{:?}", jq.evaluate(".".into(), "42".into()).unwrap_err()).contains("poisoned")
    );
    let mut fresh = Library::new().unwrap().world();
    assert_eq!(fresh.evaluate(".".into(), "42".into()).unwrap(), vec!["42"]);
}
