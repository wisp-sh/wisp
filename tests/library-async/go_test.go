package async_test

import (
    "testing"
    component "component.local/async-values"
)

func TestAsyncReaders(t *testing.T) {
    client,err:=component.NewClient();if err!=nil {t.Fatal(err)};defer client.Close()
    operations:=client.Operations()
    payload:=component.OperationsPayload{Label:"nested",Values:component.List[uint32]{3,5}}
    future,err:=operations.Delayed(payload);if err!=nil {t.Fatal(err)}
    value,ok,err:=future.Receive();if err!=nil||!ok||value.Label!="nested"||len(value.Values)!=2||value.Values[1]!=5 {t.Fatalf("future: %#v %v %v",value,ok,err)}
    if err:=future.Close();err!=nil {t.Fatal(err)}
    words,err:=operations.Words(component.List[string]{"hello","βeta"});if err!=nil {t.Fatal(err)}
    for _,want:=range []string{"hello","βeta"}{value,ok,err:=words.Next();if err!=nil||!ok||value!=want {t.Fatalf("stream: %q %v %v",value,ok,err)}}
    if _,ok,err:=words.Next();err!=nil||ok {t.Fatalf("stream end: %v %v",ok,err)}
    if err:=words.Close();err!=nil {t.Fatal(err)}
    many:=make(component.List[string],100)
    for i:=range many {many[i]="more"}
    partial,err:=operations.Words(many);if err!=nil {t.Fatal(err)}
    if value,ok,err:=partial.Next();err!=nil||!ok||value!="more" {t.Fatalf("partial stream: %q %v %v",value,ok,err)}
    if err:=partial.Close();err!=nil {t.Fatal(err)}
    cancelled,err:=operations.Delayed(component.OperationsPayload{Label:"cancel"});if err!=nil {t.Fatal(err)}
    if err:=cancelled.Close();err!=nil {t.Fatal(err)}
    if _,_,err:=cancelled.Receive();err==nil {t.Fatal("closed future remained usable")}
    active,err:=operations.Active();if err!=nil||active!=0 {t.Fatalf("active after close: %d %v",active,err)}
}
