package component

import (
    "bytes"
    "encoding/json"
    "fmt"
    "math"
    "reflect"
    "runtime"
    "sync/atomic"
    "unicode/utf8"
)

type Unit struct{}
func (Unit) MarshalJSON() ([]byte,error) { return []byte("null"),nil }
func (*Unit) UnmarshalJSON(data []byte) error {
    if !bytes.Equal(data,[]byte("null")) { return fmt.Errorf("expected unit") }; return nil
}

// Option preserves Some(None) when options are nested.
type Option[T any] struct { value T; present bool }
func Some[T any](value T) Option[T] { return Option[T]{value:value,present:true} }
func None[T any]() Option[T] { return Option[T]{} }
func (value Option[T]) Value() (T,bool) { return value.value,value.present }
func (value Option[T]) MarshalJSON() ([]byte,error) {
    if !value.present { return []byte("null"),nil }
    return json.Marshal(map[string]any{"some":value.value})
}
func (value *Option[T]) UnmarshalJSON(data []byte) error {
    if bytes.Equal(data,[]byte("null")) { *value=None[T](); return nil }
    var fields map[string]json.RawMessage
    if err:=json.Unmarshal(data,&fields);err!=nil {return err}
    item,exists:=fields["some"];if !exists || len(fields)!=1 {return fmt.Errorf("expected option")}
    var decoded T;if err:=json.Unmarshal(item,&decoded);err!=nil {return err}
    *value=Some(decoded);return nil
}

type Result[T,E any] struct { value T; failure E; state uint8 }
func OK[T,E any](value T) Result[T,E] { return Result[T,E]{value:value,state:1} }
func Failed[T,E any](failure E) Result[T,E] { return Result[T,E]{failure:failure,state:2} }
func (value Result[T,E]) Value() (T,bool) { return value.value,value.state==1 }
func (value Result[T,E]) Failure() (E,bool) { return value.failure,value.state==2 }
func (value Result[T,E]) MarshalJSON() ([]byte,error) {
    switch value.state {
    case 1:return json.Marshal(map[string]any{"ok":value.value})
    case 2:return json.Marshal(map[string]any{"err":value.failure})
    default:return nil,fmt.Errorf("result is uninitialized")
    }
}
func (value *Result[T,E]) UnmarshalJSON(data []byte) error {
    var fields map[string]json.RawMessage
    if err:=json.Unmarshal(data,&fields);err!=nil {return err}
    if len(fields)!=1 {return fmt.Errorf("expected result")}
    if item,ok:=fields["ok"];ok {var v T;if err:=json.Unmarshal(item,&v);err!=nil{return err};*value=OK[T,E](v);return nil}
    if item,ok:=fields["err"];ok {var e E;if err:=json.Unmarshal(item,&e);err!=nil{return err};*value=Failed[T,E](e);return nil}
    return fmt.Errorf("expected result branch")
}

// List encodes list<u8> as numbers and preserves empty lists.
type List[T any] []T
func (value List[T]) MarshalJSON() ([]byte,error) {
    items:=make([]any,len(value));for i,item:=range value {items[i]=item};return json.Marshal(items)
}
func (value *List[T]) UnmarshalJSON(data []byte) error {
    var raw []json.RawMessage;if err:=json.Unmarshal(data,&raw);err!=nil{return err}
    if raw==nil {return fmt.Errorf("expected list")}
    items:=make(List[T],len(raw));for i,item:=range raw {if err:=json.Unmarshal(item,&items[i]);err!=nil{return err}}
    *value=items;return nil
}

type Char rune
func (value Char) MarshalJSON() ([]byte,error) {
    if !utf8.ValidRune(rune(value)) {return nil,fmt.Errorf("invalid Unicode scalar")};return json.Marshal(string(rune(value)))
}
func (value *Char) UnmarshalJSON(data []byte) error {
    var text string;if err:=json.Unmarshal(data,&text);err!=nil{return err}
    if !utf8.ValidString(text) || utf8.RuneCountInString(text)!=1 {return fmt.Errorf("expected Unicode scalar")}
    scalar,_:=utf8.DecodeRuneInString(text);*value=Char(scalar);return nil
}

type Float32 float32
type Float64 float64
func encodeFloat(value float64) ([]byte,error) {
    if math.IsNaN(value){return []byte(`"nan"`),nil};if math.IsInf(value,1){return []byte(`"inf"`),nil};if math.IsInf(value,-1){return []byte(`"-inf"`),nil}
    return json.Marshal(value)
}
func decodeFloat(data []byte) (float64,error) {
    switch string(data) {case `"nan"`:return math.NaN(),nil;case `"inf"`:return math.Inf(1),nil;case `"-inf"`:return math.Inf(-1),nil}
    var value float64;err:=json.Unmarshal(data,&value);return value,err
}
func(value Float32) MarshalJSON()([]byte,error){return encodeFloat(float64(value))}
func(value Float64) MarshalJSON()([]byte,error){return encodeFloat(float64(value))}
func(value *Float32) UnmarshalJSON(data []byte)error{v,e:=decodeFloat(data);*value=Float32(v);return e}
func(value *Float64) UnmarshalJSON(data []byte)error{v,e:=decodeFloat(data);*value=Float64(v);return e}

type CallError[E any] struct { Guest *E; Runtime string }
func(err *CallError[E]) Error()string {
    if err.Guest!=nil {return fmt.Sprintf("Component guest error: %v",*err.Guest)};return err.Runtime
}
func decodeReply[T,E any](data []byte)(T,error) {
    var zero T
    var fields map[string]json.RawMessage
    if err:=json.Unmarshal(data,&fields);err!=nil{return zero,err}
    if len(fields)!=1{return zero,fmt.Errorf("invalid native reply")}
    if item,ok:=fields["runtime"];ok {var message string;if err:=json.Unmarshal(item,&message);err!=nil{return zero,err};return zero,&CallError[E]{Runtime:message}}
    if item,ok:=fields["guest"];ok {var failure E;if err:=json.Unmarshal(item,&failure);err!=nil{return zero,err};return zero,&CallError[E]{Guest:&failure}}
    if item,ok:=fields["value"];ok {var value T;err:=json.Unmarshal(item,&value);return value,err}
    return zero,fmt.Errorf("invalid native reply branch")
}


// validateText rejects Go byte strings that cannot represent WIT strings.
func validateText(value any) error {
    type visitedValue struct { typ reflect.Type; pointer uintptr; length int }
    visited := make(map[visitedValue]bool)
    var walk func(reflect.Value, int) error
    walk = func(value reflect.Value, depth int) error {
        if !value.IsValid() { return nil }
        if value.Type().Implements(reflect.TypeFor[resourceLike]()) { return nil }
        if depth > 256 { return fmt.Errorf("Component value nesting limit") }
        switch value.Kind() {
        case reflect.String:
            if !utf8.ValidString(value.String()) { return fmt.Errorf("Component string contains invalid UTF-8") }
        case reflect.Interface:
            if !value.IsNil() { return walk(value.Elem(), depth+1) }
        case reflect.Pointer:
            if value.IsNil() { return nil }
            key := visitedValue{typ:value.Type(), pointer:value.Pointer()}
            if visited[key] { return nil }; visited[key] = true
            return walk(value.Elem(), depth+1)
        case reflect.Slice:
            if value.IsNil() { return nil }
            key := visitedValue{typ:value.Type(), pointer:value.Pointer(), length:value.Len()}
            if visited[key] { return nil }; visited[key] = true
            for index:=0; index<value.Len(); index++ {
                if err:=walk(value.Index(index), depth+1); err!=nil { return err }
            }
        case reflect.Array:
            for index:=0; index<value.Len(); index++ {
                if err:=walk(value.Index(index), depth+1); err!=nil { return err }
            }
        case reflect.Struct:
            for index:=0; index<value.NumField(); index++ {
                if err:=walk(value.Field(index), depth+1); err!=nil { return err }
            }
        case reflect.Map:
            iterator:=value.MapRange()
            for iterator.Next() {
                if err:=walk(iterator.Key(), depth+1); err!=nil { return err }
                if err:=walk(iterator.Value(), depth+1); err!=nil { return err }
            }
        }
        return nil
    }
    return walk(reflect.ValueOf(value), 0)
}
