package resources_test

import (
    "errors"
    "runtime"
    "strings"
    "testing"
    "time"
    component "component.local/resources"
)

func TestResources(t *testing.T) {
    client,err:=component.NewClient();if err!=nil {t.Fatal(err)};defer client.Close()
    counters:=client.Counters()
    left,err:=counters.NewCounter(10);if err!=nil {t.Fatal(err)}
    if value,err:=left.Add(2);err!=nil||value!=12 {t.Fatalf("add: %v %v",value,err)}
    right,err:=client.Counters().Make(30);if err!=nil {t.Fatal(err)}
    merged,err:=counters.CounterCombine(left,right);if err!=nil {t.Fatal(err)}
    if value,err:=merged.Value();err!=nil||value!=42 {t.Fatalf("combine: %v %v",value,err)}
    if !right.IsClosed() {t.Fatal("own transfer did not invalidate the original resource")}
    if _,err:=right.Value();err==nil {t.Fatal("transferred resource remained usable")}
    if dropped,err:=counters.Dropped();err!=nil||dropped!=1 {t.Fatalf("owned destructor: %v %v",dropped,err)}
    if _,err:=counters.Checked(0);err==nil {t.Fatal("missing guest error")}else{
        var typed *component.CallError[string];if !errors.As(err,&typed)||typed.Guest==nil||*typed.Guest!="zero" {t.Fatal(err)}
    }
    if err:=merged.Close();err!=nil {t.Fatal(err)}
    if err:=merged.Close();err!=nil {t.Fatalf("idempotent close: %v",err)}
    if _,err:=merged.Value();err==nil {t.Fatal("closed resource usable")}
    if _,err:=counters.CounterCombine(left,left);err==nil||!strings.Contains(err.Error(),"ownership aliases") {t.Fatalf("own/borrow alias: %v",err)}
    if left.IsClosed() {t.Fatal("failed ownership preflight consumed a resource")}

    foreign,err:=component.NewClient();if err!=nil {t.Fatal(err)};defer foreign.Close()
    foreignCounter,err:=foreign.Counters().Make(1);if err!=nil {t.Fatal(err)}
    if _,err:=counters.CounterCombine(foreignCounter,left);err==nil {t.Fatal("cross-client resource accepted")}
    if left.IsClosed() {t.Fatal("cross-client preflight consumed local resource")}

    warehouse:=client.Warehouse()
    bundle,err:=warehouse.MakeBundle(20);if err!=nil {t.Fatal(err)}
    original:=bundle.Primary
    bundle,err=warehouse.EchoBundle(bundle);if err!=nil {t.Fatal(err)}
    if !original.IsClosed() {t.Fatal("nested own transfer did not invalidate original")}
    if value,err:=warehouse.Sum(component.WarehouseSelection{First:bundle.Primary,Rest:bundle.Others});err!=nil||value!=63 {t.Fatalf("borrow record: %v %v",value,err)}
    if value,err:=warehouse.SumPair(component.WarehousePair{V0:bundle.Primary,V1:component.Some(bundle.Others[0])});err!=nil||value!=41 {t.Fatalf("borrow tuple: %v %v",value,err)}
    if value,err:=warehouse.SumOption(component.None[*component.CountersCounter]());err!=nil||value!=0 {t.Fatalf("borrow none: %v %v",value,err)}
    added,err:=counters.Make(1);if err!=nil {t.Fatal(err)}
    mixed,err:=warehouse.Mix(component.WarehouseMixed{Base:bundle.Primary,Added:added});if err!=nil {t.Fatal(err)}
    if value,err:=mixed.Value();err!=nil||value!=21 {t.Fatalf("mixed ownership: %v %v",value,err)}
    if !added.IsClosed()||bundle.Primary.IsClosed() {t.Fatal("mixed ownership invalidation")}
    if err:=mixed.Close();err!=nil {t.Fatal(err)}
    option,err:=warehouse.EchoOption(bundle.Optional);if err!=nil {t.Fatal(err)}
    optional,present:=option.Value();if !present {t.Fatal("lost option")};if err:=optional.Close();err!=nil {t.Fatal(err)}
    values,err:=warehouse.EchoList(bundle.Others);if err!=nil {t.Fatal(err)};for _,value:=range values {if err:=value.Close();err!=nil {t.Fatal(err)}}
    choice,err:=warehouse.EchoChoice(component.NewWarehouseChoiceSelected(bundle.Primary));if err!=nil {t.Fatal(err)}
    selected,ok:=choice.Selected();if !ok {t.Fatal("lost variant")};if err:=selected.Close();err!=nil {t.Fatal(err)}
    closed,err:=warehouse.MakeBundle(2);if err!=nil {t.Fatal(err)}
    if err:=closed.Others[1].Close();err!=nil {t.Fatal(err)}
    if _,err:=warehouse.EchoBundle(closed);err==nil {t.Fatal("nested closed resource accepted")}
    if closed.Primary.IsClosed() {t.Fatal("failed nested preflight consumed a resource")}
    _=closed.Primary.Close();_=closed.Others[0].Close();if value,ok:=closed.Optional.Value();ok {_=value.Close()}

    before,err:=counters.Dropped();if err!=nil {t.Fatal(err)}
    func(){value,err:=counters.Make(5);if err!=nil {t.Fatal(err)};runtime.KeepAlive(value)}()
    deadline:=time.Now().Add(5*time.Second)
    for {runtime.GC();dropped,err:=counters.Dropped();if err!=nil {t.Fatal(err)};if dropped>before {break};if time.Now().After(deadline){t.Fatal("resource finalizer did not close handle")};time.Sleep(10*time.Millisecond)}
    if err:=left.Clear();err!=nil {t.Fatal(err)}
    if value,err:=left.Value();err!=nil||value!=0 {t.Fatalf("no-result method: %v %v",value,err)}
    if err:=left.Explode();err==nil||!strings.Contains(err.Error(),"unreachable") {t.Fatalf("expected guest trap: %v",err)}
    if _,err:=left.Value();err==nil||!strings.Contains(err.Error(),"poisoned") {t.Fatalf("expected poisoned instance: %v",err)}
}
