package context_test

import (
    "strings"
    "testing"
    component "component.local/context-values"
)

func TestErrorContexts(t *testing.T) {
    client,err:=component.NewClient();if err!=nil {t.Fatal(err)};defer client.Close()
    errors:=client.Errors()
    closed,err:=errors.Create();if err!=nil {t.Fatal(err)}
    if err:=closed.Close();err!=nil {t.Fatal(err)}
    if !closed.IsClosed() {t.Fatal("closed context remained open")}
    if _,err:=errors.Describe(closed);err==nil||!strings.Contains(err.Error(),"closed") {t.Fatalf("closed argument: %v",err)}
    other,err:=component.NewClient();if err!=nil {t.Fatal(err)};defer other.Close()
    foreign,err:=other.Errors().Create();if err!=nil {t.Fatal(err)}
    if _,err:=errors.Describe(foreign);err==nil||!strings.Contains(err.Error(),"another Component") {t.Fatalf("foreign context: %v",err)}
    for i:=0;i<126;i++ {context,err:=errors.Create();if err!=nil {t.Fatalf("create %d: %v",i,err)};message,err:=errors.Describe(context);if err!=nil||message!="fixture error" {t.Fatalf("describe %d: %q %v",i,message,err)};if !context.IsClosed() {t.Fatal("owned context was not consumed")}}
    if _,err:=errors.Create();err==nil||!strings.Contains(err.Error(),"capacity") {t.Fatalf("expected Store lifetime limit: %v",err)}
    fresh,err:=component.NewClient();if err!=nil {t.Fatal(err)};defer fresh.Close()
    context,err:=fresh.Errors().Create();if err!=nil {t.Fatal(err)}
    if message,err:=fresh.Errors().Describe(context);err!=nil||message!="fixture error" {t.Fatalf("fresh Store: %q %v",message,err)}
}
