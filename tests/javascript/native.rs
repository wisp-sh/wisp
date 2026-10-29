use wisp_wisp__typescript_catalog::{types::{Choice, Failure, Features, Item, Request}, CallError, Library};

fn request(items: Vec<Item>) -> Request {
    Request { items, label: Some("order".into()), policy: Features::GIFT | Features::PRIORITY,
              metadata: (7, "meta".into()), detail: Choice::Numbers(vec![1, 2]) }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = Library::new()?;
    let mut catalog = library.catalog();
    let mut formatting = library.formatting();
    let input = request(vec![Item {name: "book".into(), amount: 2, tags: vec!["a".into(), "b".into()]},
                             Item {name: "pen".into(), amount: 5, tags: vec![]}]);
    let output = catalog.process(input)?;
    assert_eq!(output.summary, "order:book");
    assert_eq!(output.counts, [2, 5]);
    let first = output.first.as_ref().unwrap();
    assert_eq!((&first.name, first.amount, &first.tags), (&"book".to_string(), 2, &vec!["a".to_string(), "b".to_string()]));
    assert!(output.policy.contains(Features::GIFT | Features::PRIORITY));
    assert_eq!(output.metadata, (7, "meta".into()));
    assert!(matches!(&output.detail, Choice::Numbers(values) if values == &[1, 2]));
    let echoed = formatting.echo(output)?;
    assert_eq!(echoed.summary, "order:book");
    assert!(matches!(catalog.process(request(vec![])), Err(CallError::Guest(Failure::Empty))));
    assert!(matches!(catalog.process(request(vec![Item {name: "bad".into(), amount: 0, tags: vec![]}])),
                     Err(CallError::Guest(Failure::Invalid(value))) if value == "bad"));
    assert_eq!(formatting.greet("world".into())?, "hello world");
    assert_eq!(formatting.count(vec!["a".into(), "b".into()])?, 2);
    assert_eq!(formatting.current()?, 0);
    assert_eq!(catalog.increment()?, 1);
    assert_eq!(formatting.current()?, 1);
    assert_eq!(catalog.increment()?, 2);
    assert_eq!(formatting.current()?, 2);
    println!("nested-success\ntyped-empty\ntyped-invalid(bad)\nhello world\ncount(2)\nshared-state(2)");
    Ok(())
}
