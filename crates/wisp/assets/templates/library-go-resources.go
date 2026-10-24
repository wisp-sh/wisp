
type resourceState struct {
    client *Client
    id atomic.Uint64
    owner uint64
    kind uint32
}

type resourceLike interface { wispResource() *resourceState }
type resourceVisitor interface { visitWisp(func(*resourceState) error) error }

func (state *resourceState) marshal() ([]byte,error) {
    if state == nil || state.client == nil || state.id.Load() == 0 { return nil,ErrClosed }
    return json.Marshal(map[string]any{"owner":state.owner,"id":state.id.Load(),"kind":state.kind})
}
func decodeResource(data []byte, kind uint32) (*resourceState,error) {
    var fields map[string]json.RawMessage
    if err:=json.Unmarshal(data,&fields);err!=nil { return nil,err }
    if len(fields)!=3 { return nil,fmt.Errorf("invalid resource reference") }
    var owner,id uint64
    var foundKind uint32
    if err:=json.Unmarshal(fields["owner"],&owner);err!=nil { return nil,err }
    if err:=json.Unmarshal(fields["id"],&id);err!=nil { return nil,err }
    if err:=json.Unmarshal(fields["kind"],&foundKind);err!=nil { return nil,err }
    if owner==0 || id==0 || kind==0 || foundKind!=kind { return nil,fmt.Errorf("invalid resource identity") }
    state:=&resourceState{owner:owner,kind:kind}
    state.id.Store(id)
    return state,nil
}
func (state *resourceState) close() error {
    if state==nil { return nil }
    client:=state.client
    if client==nil { state.id.Store(0);return nil }
    client.mu.Lock()
    defer client.mu.Unlock()
    id:=state.id.Swap(0)
    runtime.SetFinalizer(state,nil)
    if id==0 || client.handle==0 { return nil }
    return client.dropResource(id)
}
func (state *resourceState) isClosed() bool {
    if state==nil || state.client==nil || state.id.Load()==0 { return true }
    state.client.mu.Lock()
    defer state.client.mu.Unlock()
    return state.client.handle==0 || state.id.Load()==0
}

func visitValue(value any, visit func(*resourceState) error) error {
    type visitedValue struct { typ reflect.Type; pointer uintptr; length int }
    visited:=make(map[visitedValue]bool)
    var walk func(reflect.Value,int) error
    walk=func(value reflect.Value,depth int) error {
        if !value.IsValid() { return nil }
        if depth>256 { return fmt.Errorf("Component value nesting limit") }
        if value.Type().Implements(reflect.TypeFor[resourceLike]()) {
            if value.Kind()==reflect.Pointer && value.IsNil() { return ErrClosed }
            if !value.CanInterface() { return fmt.Errorf("inaccessible resource value") }
            state:=value.Interface().(resourceLike).wispResource()
            if state==nil { return ErrClosed }
            return visit(state)
        }
        if value.CanInterface() {
            if custom,ok:=value.Interface().(resourceVisitor);ok { return custom.visitWisp(visit) }
        }
        switch value.Kind() {
        case reflect.Interface:
            if !value.IsNil() { return walk(value.Elem(),depth+1) }
        case reflect.Pointer:
            if value.IsNil() { return nil }
            key:=visitedValue{typ:value.Type(),pointer:value.Pointer()}
            if visited[key] { return nil };visited[key]=true
            return walk(value.Elem(),depth+1)
        case reflect.Slice:
            if value.IsNil() { return nil }
            key:=visitedValue{typ:value.Type(),pointer:value.Pointer(),length:value.Len()}
            if visited[key] { return nil };visited[key]=true
            for index:=0;index<value.Len();index++ {
                if err:=walk(value.Index(index),depth+1);err!=nil { return err }
            }
        case reflect.Array:
            for index:=0;index<value.Len();index++ {
                if err:=walk(value.Index(index),depth+1);err!=nil { return err }
            }
        case reflect.Struct:
            for index:=0;index<value.NumField();index++ {
                if value.Type().Field(index).IsExported() {
                    if err:=walk(value.Field(index),depth+1);err!=nil { return err }
                }
            }
        case reflect.Map:
            iterator:=value.MapRange()
            for iterator.Next() {
                if err:=walk(iterator.Value(),depth+1);err!=nil { return err }
            }
        }
        return nil
    }
    return walk(reflect.ValueOf(value),0)
}
func(value Option[T]) visitWisp(visit func(*resourceState)error)error {
    if !value.present { return nil };return visitValue(value.value,visit)
}
func(value Result[T,E]) visitWisp(visit func(*resourceState)error)error {
    if value.state==1 { return visitValue(value.value,visit) }
    if value.state==2 { return visitValue(value.failure,visit) }
    return nil
}
func validateArguments(client *Client, arguments any) error {
    return visitValue(arguments,func(state *resourceState)error {
        if state.client!=client { return fmt.Errorf("resource belongs to another Component client") }
        if state.id.Load()==0 { return ErrClosed }
        return nil
    })
}

func finishReply[T,E any](client *Client, reply []byte, arguments any)(T,error) {
    var zero T
    var fields map[string]json.RawMessage
    if err:=json.Unmarshal(reply,&fields);err!=nil { return zero,err }
    if receipt,present:=fields["consumedContexts"];present {
        delete(fields,"consumedContexts")
        var consumed []uint64
        if err:=json.Unmarshal(receipt,&consumed);err!=nil { return zero,err }
        ids:=make(map[uint64]bool,len(consumed))
        for _,id:=range consumed {if id==0||ids[id]{return zero,fmt.Errorf("invalid error context ownership receipt")};ids[id]=true}
        if values,ok:=arguments.(map[string]any);ok {for _,value:=range values {if context,ok:=value.(*ErrorContext);ok&&context!=nil&&context.state!=nil {id:=context.state.id.Load();if ids[id]&&context.state.client==client {context.state.id.CompareAndSwap(id,0);runtime.SetFinalizer(context.state,nil);delete(ids,id)}}}}
        if len(ids)!=0 {return zero,fmt.Errorf("unknown error context ownership receipt")}
    }
    if receipt,present:=fields["consumed"];present {
        delete(fields,"consumed")
        var consumed []uint64
        if err:=json.Unmarshal(receipt,&consumed);err!=nil { return zero,err }
        ids:=make(map[uint64]bool,len(consumed))
        for _,id:=range consumed {
            if id==0 || ids[id] { return zero,fmt.Errorf("invalid ownership receipt") }
            ids[id]=true
        }
        if err:=visitValue(arguments,func(state *resourceState)error {
            id:=state.id.Load()
            if state.client==client && ids[id] {
                state.id.CompareAndSwap(id,0)
                runtime.SetFinalizer(state,nil)
                delete(ids,id)
            }
            return nil
        });err!=nil { return zero,err }
        if len(ids)!=0 { return zero,fmt.Errorf("unknown ownership receipt") }
    }
    stripped,err:=json.Marshal(fields)
    if err!=nil { return zero,err }
    value,callError:=decodeReply[T,E](stripped)
    bind:=func(state *resourceState)error {
        if state.client!=nil && state.client!=client { return fmt.Errorf("resource belongs to another Component client") }
        if state.id.Load()==0 { return ErrClosed }
        if state.client==nil {
            state.client=client
            runtime.SetFinalizer(state,func(value *resourceState){ _=value.close() })
        }
        return nil
    }
    if callError==nil {
        if context,ok:=any(value).(*ErrorContext);ok {if err:=bindContext(client,context);err!=nil{return zero,err}}
        if err:=visitValue(value,bind);err!=nil { return zero,err }
    } else if failure,ok:=callError.(*CallError[E]);ok && failure.Guest!=nil {
        if err:=visitValue(failure.Guest,bind);err!=nil { return zero,err }
    }
    return value,callError
}
