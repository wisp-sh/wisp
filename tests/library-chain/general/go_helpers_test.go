package component

import (
    "errors"
    "sync"
    "testing"
)

var ErrClosed=errors.New("closed")
type Client struct { mu sync.Mutex; handle uintptr; drops int; contextDrops int }
func(client *Client)dropResource(id uint64)error { client.drops++;return nil }
func(client *Client)closeContext(id uint64)error {client.contextDrops++;return nil}
type mockResource struct { state *resourceState }
func(value *mockResource)wispResource()*resourceState { if value==nil{return nil};return value.state }
func(value *mockResource)UnmarshalJSON(data []byte)error {
    state,err:=decodeResource(data,1);value.state=state;return err
}
func(value *mockResource)MarshalJSON()([]byte,error){return value.state.marshal()}

func TestResourceReceiptsAndLifetimes(t *testing.T) {
    client:=&Client{handle:1}
    reference:=[]byte(`{"value":{"owner":4,"id":7,"kind":1}}`)
    value,err:=finishReply[*mockResource,Unit](client,reference,nil)
    if err!=nil || value.state.client!=client || value.state.id.Load()!=7 { t.Fatalf("bind: %v",err) }
    arguments:=struct{ Values List[Option[*mockResource]] }{List[Option[*mockResource]]{Some(value)}}
    if err:=validateArguments(client,arguments);err!=nil { t.Fatal(err) }
    if err:=validateArguments(&Client{handle:1},arguments);err==nil { t.Fatal("foreign client accepted") }
    _,err=finishReply[Unit,string](client,[]byte(`{"guest":"declared","consumed":[7]}`),arguments)
    var failure *CallError[string]
    if !errors.As(err,&failure) || failure.Guest==nil || *failure.Guest!="declared" { t.Fatalf("guest: %v",err) }
    if value.state.id.Load()!=0 { t.Fatal("owned resource remained live after guest error") }
    if err:=value.state.close();err!=nil || client.drops!=0 { t.Fatalf("consumed close: %v",err) }
    if err:=validateArguments(client,arguments);!errors.Is(err,ErrClosed) { t.Fatalf("consumed accepted: %v",err) }
    value,err=finishReply[*mockResource,Unit](client,reference,nil)
    if err!=nil { t.Fatal(err) }
    if value.state.isClosed() { t.Fatal("new resource closed") }
    if err:=value.state.close();err!=nil { t.Fatal(err) }
    if err:=value.state.close();err!=nil || client.drops!=1 { t.Fatalf("double drop: %d %v",client.drops,err) }
    if !value.state.isClosed() { t.Fatal("closed resource open") }
    value,err=finishReply[*mockResource,Unit](client,reference,nil)
    if err!=nil { t.Fatal(err) }
    client.handle=0
    if !value.state.isClosed() { t.Fatal("resource survives closed client") }
    if err:=value.state.close();err!=nil || client.drops!=1 { t.Fatalf("drop after client close: %v",err) }
}

func TestNestedGuestResourceAndInvalidReferences(t *testing.T) {
    client:=&Client{handle:1}
    _,err:=finishReply[Unit,Option[*mockResource]](client,[]byte(`{"guest":{"some":{"owner":4,"id":8,"kind":1}}}`),nil)
    var failure *CallError[Option[*mockResource]]
    if !errors.As(err,&failure) || failure.Guest==nil { t.Fatalf("guest result: %v",err) }
    resource,present:=failure.Guest.Value()
    if !present || resource.state.client!=client { t.Fatal("nested error resource was not bound") }
    defer resource.state.close()
    for _,data:=range []string{`{"owner":4,"id":8,"kind":2}`,`{"owner":0,"id":8,"kind":1}`,`{"owner":4,"id":0,"kind":1}`,`{"owner":4,"id":8,"kind":1,"extra":0}`} {
        if _,err:=decodeResource([]byte(data),1);err==nil { t.Fatalf("invalid resource accepted: %s",data) }
    }
    if _,err:=finishReply[Unit,Unit](client,[]byte(`{"value":null,"consumed":[99]}`),nil);err==nil { t.Fatal("unknown receipt accepted") }
    if _,err:=finishReply[Unit,Unit](client,[]byte(`{"value":null,"consumed":[8,8]}`),resource);err==nil { t.Fatal("duplicate receipt accepted") }
}

func TestTextValidationPrivateValuesAndCycles(t *testing.T) {
    invalid:=string([]byte{0xff})
    for _,value:=range []any{invalid,Some(invalid),OK[string,Unit](invalid),Failed[Unit,string](invalid),List[Option[string]]{Some(invalid)}} {
        if err:=validateText(value);err==nil { t.Fatalf("invalid UTF-8 accepted: %T",value) }
    }
    type node struct { Next *node; Text string }
    cyclic:=&node{Text:invalid};cyclic.Next=cyclic
    if err:=validateText(cyclic);err==nil { t.Fatal("cycle hid invalid string") }
    client:=&Client{handle:1}
    resource,err:=finishReply[*mockResource,Unit](client,[]byte(`{"value":{"owner":4,"id":8,"kind":1}}`),nil)
    if err!=nil { t.Fatal(err) }
    defer resource.state.close()
    if err:=validateText(Some(resource));err!=nil { t.Fatal(err) }
    if err:=validateArguments(client,Some(resource));err!=nil { t.Fatal(err) }
    if err:=validateArguments(client,Some[*mockResource](nil));!errors.Is(err,ErrClosed) { t.Fatalf("nil resource accepted: %v",err) }
}

func TestErrorContextReceiptsAndLifetimes(t *testing.T) {
    client:=&Client{handle:1}
    reference:=[]byte(`{"value":{"owner":4,"id":7}}`)
    context,err:=finishReply[*ErrorContext,Unit](client,reference,nil)
    if err!=nil||context.state.client!=client||context.IsClosed(){t.Fatalf("bind: %v",err)}
    if err:=validateContextArgument(&Client{handle:1},context);err==nil{t.Fatal("foreign context accepted")}
    args:=map[string]any{"value":context}
    _,err=finishReply[Unit,string](client,[]byte(`{"guest":"declared","consumedContexts":[7]}`),args)
    var failure *CallError[string]
    if !errors.As(err,&failure)||failure.Guest==nil||*failure.Guest!="declared"{t.Fatalf("guest: %v",err)}
    if !context.IsClosed(){t.Fatal("consumed context remained open")}
    if err:=context.Close();err!=nil||client.contextDrops!=0{t.Fatalf("consumed close: %v",err)}
    if err:=validateContextArgument(client,context);!errors.Is(err,ErrClosed){t.Fatalf("consumed accepted: %v",err)}
    context,err=finishReply[*ErrorContext,Unit](client,reference,nil)
    if err!=nil{t.Fatal(err)}
    if err:=context.Close();err!=nil{t.Fatal(err)}
    if err:=context.Close();err!=nil||client.contextDrops!=1{t.Fatalf("double close: %d %v",client.contextDrops,err)}
    if _,err:=finishReply[Unit,Unit](client,[]byte(`{"value":null,"consumedContexts":[99]}`),nil);err==nil{t.Fatal("unknown context receipt accepted")}
    if _,err:=finishReply[Unit,Unit](client,[]byte(`{"value":null,"consumedContexts":[8,8]}`),nil);err==nil{t.Fatal("duplicate context receipt accepted")}
}
