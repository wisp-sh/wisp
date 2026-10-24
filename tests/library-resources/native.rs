use wisp_wisp__resource_library::{Library, CallError, counters::{Client, Counter}};

fn main() {
    let library = Library::new().unwrap();
    let mut client = library.counters();
    let mut second = library.counters();
    let mut left = Counter::new(&mut client, 10).unwrap();
    assert_eq!(left.add(2).unwrap(), 12);
    let right = second.make(30).unwrap();
    let mut merged = Counter::combine(&mut second, &left, right).unwrap();
    assert_eq!(merged.value().unwrap(), 42);
    assert_eq!(client.dropped().unwrap(), 1);
    assert!(matches!(client.checked(0), Err(CallError::Guest(message)) if message == "zero"));
    {
        let value = client.checked(8).unwrap();
        assert_eq!(value.value().unwrap(), 8);
    }
    assert_eq!(client.dropped().unwrap(), 2);
    merged.close().unwrap();
    assert!(merged.is_closed());
    assert!(matches!(merged.close(), Err(CallError::Runtime(message)) if message == "resource is closed"));
    assert!(matches!(merged.value(), Err(CallError::Runtime(message)) if message == "resource is closed"));
    let mut unrelated = Client::new().unwrap();
    let foreign = unrelated.make(7).unwrap();
    let local = client.make(1).unwrap();
    assert!(matches!(Counter::combine(&mut client, &foreign, local), Err(CallError::Runtime(message)) if message.contains("another Component instance")));
    assert_eq!(left.value().unwrap(), 12);
    left.clear().unwrap();
    assert_eq!(left.value().unwrap(), 0);
    nested(&library);
    assert!(matches!(left.explode(), Err(CallError::Runtime(message)) if message.contains("unreachable")));
    assert!(matches!(left.value(), Err(CallError::Runtime(message)) if message.contains("poisoned")));
    let _ = left.close();
    println!("resources-native-passed");
}

fn nested(library: &Library) {
    use wisp_wisp__resource_library::warehouse::{Choice, Selection, Mixed};
    let mut warehouse = library.warehouse();
    let bundle = warehouse.make_bundle(20).unwrap();
    let bundle = warehouse.echo_bundle(bundle).unwrap();
    assert_eq!(warehouse.sum(Selection { first: &bundle.primary, rest: bundle.others.iter().collect() }).unwrap(), 63);
    assert_eq!(warehouse.sum_pair((&bundle.primary, Some(&bundle.others[0]))).unwrap(), 41);
    assert_eq!(warehouse.sum_option(Some(&bundle.primary)).unwrap(), 20);
    assert_eq!(warehouse.sum_option(None).unwrap(), 0);
    let mut counters = library.counters();
    let mixed = warehouse.mix(Mixed { base: &bundle.primary, added: counters.make(1).unwrap() }).unwrap();
    assert_eq!(mixed.value().unwrap(),21);
    let options = warehouse.echo_option(bundle.optional).unwrap();
    assert_eq!(options.unwrap().value().unwrap(), 23);
    let values = warehouse.echo_list(bundle.others).unwrap();
    assert_eq!(values.iter().map(|v|v.value().unwrap()).sum::<u32>(), 43);
    match warehouse.echo_choice(Choice::Selected(bundle.primary)).unwrap() {
        Choice::Selected(value) => assert_eq!(value.value().unwrap(),20),
        _ => panic!("lost selected resource"),
    }
    assert!(matches!(warehouse.echo_choice(Choice::Missing).unwrap(), Choice::Missing));
    assert!(warehouse.echo_option(None).unwrap().is_none());
    let mut closed = warehouse.make_bundle(10).unwrap();
    closed.others[1].close().unwrap();
    assert!(matches!(warehouse.echo_bundle(closed), Err(CallError::Runtime(message)) if message == "resource is closed"));
    let mut foreign = Library::new().unwrap().warehouse();
    let bundle = foreign.make_bundle(1).unwrap();
    assert!(matches!(warehouse.echo_bundle(bundle), Err(CallError::Runtime(message)) if message.contains("another Component instance")));
}
