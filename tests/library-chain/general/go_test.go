package consumer

import (
    "errors"
    "testing"
    catalog "component.local/catalog"
)

func request(items catalog.List[catalog.TypesItem]) catalog.TypesRequest {
    return catalog.TypesRequest{
        Items: items,
        Label: catalog.Some("order"),
        Policy: catalog.TypesFeaturesGift | catalog.TypesFeaturesPriority,
        Metadata: catalog.Tuple2[uint32,string]{V0: 7, V1: "meta"},
        Detail: catalog.NewTypesChoiceNumbers(catalog.List[uint32]{1,2}),
    }
}

func TestNestedLibrary(t *testing.T) {
    client, err := catalog.NewClient()
    if err != nil { t.Fatal(err) }
    defer client.Close()
    current, err := client.Formatting().Current()
    if err != nil || current != 0 { t.Fatalf("initial state: %d %v", current, err) }
    for expected := uint32(1); expected <= 2; expected++ {
        incremented, err := client.Catalog().Increment()
        if err != nil || incremented != expected { t.Fatalf("increment: %d %v", incremented, err) }
        current, err = client.Formatting().Current()
        if err != nil || current != expected { t.Fatalf("shared state: %d %v", current, err) }
    }
    output, err := client.Catalog().Process(request(catalog.List[catalog.TypesItem]{
        {Name: "book", Amount: 2, Tags: catalog.List[string]{"a","b"}},
        {Name: "pen", Amount: 5, Tags: catalog.List[string]{}},
    }))
    if err != nil { t.Fatal(err) }
    if output.Summary != "order:book" || len(output.Counts) != 2 || output.Counts[0] != 2 || output.Counts[1] != 5 { t.Fatalf("wrong response: %#v",output) }
    first, present := output.First.Value()
    if !present || first.Name != "book" || first.Amount != 2 || len(first.Tags) != 2 { t.Fatalf("wrong first: %#v",first) }
    if output.Policy != catalog.TypesFeaturesGift | catalog.TypesFeaturesPriority || output.Metadata.V0 != 7 || output.Metadata.V1 != "meta" { t.Fatalf("lost flags or tuple: %#v",output) }
    numbers, present := output.Detail.Numbers()
    if !present || len(numbers) != 2 || numbers[0] != 1 || numbers[1] != 2 { t.Fatal("lost variant payload") }
    echoed,err := client.Formatting().Echo(output)
    if err != nil || echoed.Summary != output.Summary { t.Fatalf("echo: %#v %v",echoed,err) }
    _,err = client.Catalog().Process(request(catalog.List[catalog.TypesItem]{}))
    var failure *catalog.CallError[catalog.TypesFailure]
    if !errors.As(err,&failure) || failure.Guest == nil || failure.Guest.Tag() != catalog.TypesFailureEmptyTag { t.Fatalf("wrong empty error: %v",err) }
    _,err = client.Catalog().Process(request(catalog.List[catalog.TypesItem]{{Name:"bad",Amount:0,Tags:catalog.List[string]{}}}))
    if !errors.As(err,&failure) || failure.Guest == nil { t.Fatalf("wrong invalid error: %v",err) }
    bad,present := failure.Guest.Invalid()
    if !present || bad != "bad" { t.Fatalf("lost typed error payload: %v",err) }
    invalid := request(catalog.List[catalog.TypesItem]{{Name:"book",Amount:1,Tags:catalog.List[string]{}}})
    invalid.Label = catalog.Some(string([]byte{0xff}))
    if _,err := client.Catalog().Process(invalid); err == nil { t.Fatal("invalid nested UTF-8 was accepted") }
    invalid.Label = catalog.None[string]()
    invalid.Detail = catalog.NewTypesChoiceText(string([]byte{0xff}))
    if _,err := client.Catalog().Process(invalid); err == nil { t.Fatal("invalid variant UTF-8 was accepted") }
    greeting,err := client.Formatting().Greet("world")
    if err != nil || greeting != "hello world" { t.Fatalf("greet: %q %v",greeting,err) }
    count,err := client.Formatting().Count(catalog.List[string]{"a","b"})
    if err != nil || count != 2 { t.Fatalf("count: %d %v",count,err) }
    if err := client.Close(); err != nil { t.Fatal(err) }
    if err := client.Close(); err != nil { t.Fatal(err) }
    if _,err := client.Formatting().Greet("closed"); !errors.Is(err,catalog.ErrClosed) { t.Fatalf("call after close: %v",err) }
}
