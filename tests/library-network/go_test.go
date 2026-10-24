package network_test

import (
    "errors"
    "net"
    "net/http"
    "strconv"
    "testing"
    component "component.local/network"
)

func TestNetwork(t *testing.T) {
    listener,err:=net.Listen("tcp","127.0.0.1:0");if err!=nil {t.Fatal(err)}
    server:=&http.Server{Handler:http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){_,_=w.Write([]byte("network body"))})}
    go func(){_ = server.Serve(listener)}()
    defer server.Close()
    port:=listener.Addr().(*net.TCPAddr).Port
    closed,err:=net.Listen("tcp","127.0.0.1:0");if err!=nil {t.Fatal(err)}
    closedPort:=closed.Addr().(*net.TCPAddr).Port
    _=closed.Close()
    client,err:=component.NewClient();if err!=nil {t.Fatal(err)};defer client.Close()
    requests:=client.Requests()
    response,err:=requests.Fetch("http://127.0.0.1:"+strconv.Itoa(port)+"/");if err!=nil {t.Fatal(err)}
    status,err:=response.Status();if err!=nil||status!=200 {t.Fatalf("status: %d %v",status,err)}
    body,err:=response.Body();if err!=nil||body!="network body" {t.Fatalf("body: %q %v",body,err)}
    if err:=response.Close();err!=nil {t.Fatal(err)}
    if _,err:=response.Body();err==nil {t.Fatal("closed response was usable")}
    if count,err:=requests.Resolve("localhost");err!=nil||count==0 {t.Fatalf("DNS: %d %v",count,err)}
    if _,err:=requests.Resolve("invalid..name");err==nil {t.Fatal("invalid name succeeded")} else { var guest *component.CallError[string];if !errors.As(err,&guest) {t.Fatalf("DNS did not return a guest error: %v",err)} }
    if err:=requests.Connect(uint16(port));err!=nil {t.Fatalf("TCP connect: %v",err)}
    if err:=requests.Connect(uint16(closedPort));err==nil {t.Fatal("closed port connected")}
    if _,err:=requests.Fetch("http://127.0.0.1:"+strconv.Itoa(closedPort)+"/");err==nil {t.Fatal("closed HTTP port fetched")}
}
