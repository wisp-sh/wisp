use serde_json::{Map, Value, json};
use std::panic::{catch_unwind, AssertUnwindSafe};

type WireResult<T> = Result<T,String>;
const MAX_WIRE: usize = 16 * 1024 * 1024;
static NEXT_OWNER:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
struct ResourceEntry {kind:u32,handle:wasmtime::component::ResourceAny}
impl Client {
    fn commit_resources(&mut self) {
        for (id,owned) in std::mem::take(&mut self.resource_uses) {
            if owned {self.resources.remove(&id);self.consumed.push(id);}
        }
    }
    fn decode_resource(&mut self,value:Value,kind:u32,owned:bool)->WireResult<wasmtime::component::ResourceAny> {
        let mut fields=object(value)?;
        let owner=scalar::<u64>(field(&mut fields,"owner")?)?;
        let id=scalar::<u64>(field(&mut fields,"id")?)?;
        let claimed_kind=scalar::<u32>(field(&mut fields,"kind")?)?;finished(fields)?;
        if owner!=self.owner {return Err("resource belongs to another Component instance".into())}
        let entry=self.resources.get(&id).ok_or("resource is closed")?;
        if kind!=entry.kind || claimed_kind!=kind {return Err("resource type mismatch".into())}
        if self.resource_uses.get(&id).is_some_and(|prior|*prior||owned) {return Err("resource ownership aliases another argument".into())}
        self.resource_uses.insert(id,owned);
        Ok(entry.handle)
    }
    fn encode_resource(&mut self,handle:wasmtime::component::ResourceAny,kind:u32)->WireResult<Value> {
        if self.resources.len()>=65536 {let _=handle.resource_drop(&mut self.store);return Err("resource table limit".into())}
        let id=self.next_resource;
        self.next_resource=self.next_resource.checked_add(1).ok_or("resource identity exhausted")?;
        self.resources.insert(id,ResourceEntry{kind,handle});
        Ok(json!({"owner":self.owner,"id":id,"kind":kind}))
    }
}
impl Drop for Client {
    fn drop(&mut self) {for id in self.readers.keys().copied().collect::<Vec<_>>(){let _=self.close_reader(id);}self.contexts.clear();for entry in std::mem::take(&mut self.resources).into_values(){let _=entry.handle.resource_drop(&mut self.store);}}
}
fn object(value:Value)->WireResult<Map<String,Value>> {
    match value {Value::Object(fields)=>Ok(fields),_=>Err("expected record".into())}
}
fn field(object:&mut Map<String,Value>,name:&str)->WireResult<Value>{object.remove(name).ok_or_else(||format!("missing field {name}"))}
fn finished(object:Map<String,Value>)->WireResult<()> {if object.is_empty(){Ok(())}else{Err("unexpected record fields".into())}}
fn array(value:Value)->WireResult<Vec<Value>>{match value{Value::Array(values)=>Ok(values),_=>Err("expected list or tuple".into())}}
fn scalar<T:serde::de::DeserializeOwned>(value:Value)->WireResult<T>{serde_json::from_value(value).map_err(|e|e.to_string())}
fn float(value:Value)->WireResult<f64>{match value.as_str(){Some("nan")=>Ok(f64::NAN),Some("inf")=>Ok(f64::INFINITY),Some("-inf")=>Ok(f64::NEG_INFINITY),_=>scalar(value)}}
fn float_value(value:f64)->Value{if value.is_nan(){json!("nan")}else if value==f64::INFINITY{json!("inf")}else if value==f64::NEG_INFINITY{json!("-inf")}else{json!(value)}}

#[repr(C)]
pub struct Buffer { data:*mut u8,len:usize }
fn buffer(value:Value)->Buffer {
    let mut bytes=serde_json::to_vec(&value).unwrap_or_else(|_|b"{\"runtime\":\"serialization failed\"}".to_vec());
    if bytes.len()>MAX_WIRE {bytes=b"{\"runtime\":\"native reply size limit\"}".to_vec();}
    let bytes=bytes.into_boxed_slice();let len=bytes.len();let data=Box::into_raw(bytes) as *mut u8;
    Buffer{data,len}
}
fn invoke(handle:usize,data:*const u8,len:usize,call:impl FnOnce(&mut Client,Value)->WireResult<Value>)->Buffer {
    if handle==0 || data.is_null() || len>MAX_WIRE{return buffer(json!({"runtime":"invalid native request"}));}
    let client=unsafe{&mut *(handle as *mut Client)};
    if client.poisoned{return buffer(json!({"runtime":"Component instance is poisoned"}));}
    client.resource_uses.clear();client.consumed.clear();
    let resource_start=client.next_resource;
    client.consumed_contexts.clear();let context_start=client.next_context;
    let result=catch_unwind(AssertUnwindSafe(|| {
        let bytes=unsafe{std::slice::from_raw_parts(data,len)};
        let value=serde_json::from_slice(bytes).map_err(|e|e.to_string())?;
        call(client,value)
    }));
    let mut value=match result {
        Ok(Ok(value))=>value,
        Ok(Err(error))=>json!({"runtime":error}),
        Err(_)=>{client.poisoned=true;json!({"runtime":"native panic contained"})},
    };
    if let Some(fields)=value.as_object_mut(){fields.insert("consumed".into(),json!(client.consumed));fields.insert("consumedContexts".into(),json!(client.consumed_contexts));}
    if serde_json::to_vec(&value).map_or(true,|bytes|bytes.len()>MAX_WIRE) {
        value=json!({"runtime":"native reply size limit","consumed":client.consumed,"consumedContexts":client.consumed_contexts});
    }
    if value.get("runtime").is_some() {
        let undelivered=client.resources.range(resource_start..).map(|(id,_)|*id).collect::<Vec<_>>();
        for id in undelivered {
            if let Some(entry)=client.resources.remove(&id) {
                if entry.handle.resource_drop(&mut client.store).is_err(){client.poisoned=true;}
            }
        }
        let undelivered=client.contexts.range(context_start..).map(|(id,_)|*id).collect::<Vec<_>>();for id in undelivered{client.contexts.remove(&id);}
    }
    buffer(value)
}
