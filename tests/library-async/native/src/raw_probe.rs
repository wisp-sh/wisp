use std::{future::poll_fn,pin::Pin,sync::{Arc,Mutex},task::{Context,Poll,Waker}};
use wasmtime::{Config,Engine,Store};
use wasmtime::component::{Component,ComponentType,Lift,Lower,Linker,Source,FutureConsumer,FutureReader,StreamConsumer,StreamReader,StreamResult,TypedFunc};
#[derive(Debug,PartialEq,ComponentType,Lift,Lower)]
#[component(record)]
struct Payload { label: String, values: Vec<u32> }
struct Collected<T> { values: Mutex<Vec<T>>, done: Mutex<bool>, waker: Mutex<Option<Waker>> }
impl<T> Default for Collected<T> { fn default()->Self {Self{values:Mutex::new(Vec::new()),done:Mutex::new(false),waker:Mutex::new(None)}} }
struct Collector<T>(Arc<Collected<T>>);
impl<T> Collector<T> { fn wake(&self) { if let Some(w)=self.0.waker.lock().unwrap().take(){w.wake()} } }
impl<T: Lift + Send + 'static> FutureConsumer<()> for Collector<T> {
 type Item=T;
 fn poll_consume(self:Pin<&mut Self>,_:&mut Context<'_>,mut store:wasmtime::StoreContextMut<'_,()>,mut source:Source<'_,T>,finish:bool)->Poll<wasmtime::Result<()>>{
  if finish {*self.0.done.lock().unwrap()=true;} else {let mut values=Vec::with_capacity(1);source.read(&mut store,&mut values)?;self.0.values.lock().unwrap().extend(values);}
  self.wake();Poll::Ready(Ok(()))
 }
}
impl<T: Lift + Send + 'static> StreamConsumer<()> for Collector<T> {
 type Item=T;
 fn poll_consume(self:Pin<&mut Self>,_:&mut Context<'_>,mut store:wasmtime::StoreContextMut<'_,()>,mut source:Source<'_,T>,finish:bool)->Poll<wasmtime::Result<StreamResult>>{
  if finish {*self.0.done.lock().unwrap()=true;self.wake();return Poll::Ready(Ok(StreamResult::Cancelled))}
  let mut values=Vec::with_capacity(source.remaining(&mut store));source.read(&mut store,&mut values)?;self.0.values.lock().unwrap().extend(values);self.wake();Poll::Ready(Ok(StreamResult::Completed))
 }
}
impl<T> Drop for Collector<T> { fn drop(&mut self){*self.0.done.lock().unwrap()=true;self.wake()} }
async fn wait<T>(v:Arc<Collected<T>>,done:bool){poll_fn(move|cx|{if *v.done.lock().unwrap()||(!done&&!v.values.lock().unwrap().is_empty()){Poll::Ready(())}else{*v.waker.lock().unwrap()=Some(cx.waker().clone());Poll::Pending}}).await}
use std::task::Wake;use std::sync::Condvar;use std::time::{Duration,Instant};
struct ThreadWake{ready:Mutex<bool>,cv:Condvar} impl Wake for ThreadWake{fn wake(self:Arc<Self>){self.wake_by_ref()}fn wake_by_ref(self:&Arc<Self>){*self.ready.lock().unwrap()=true;self.cv.notify_one()}}
fn block_on<F:std::future::Future>(future:F)->Result<F::Output,()>{let signal=Arc::new(ThreadWake{ready:Mutex::new(false),cv:Condvar::new()});let waker=Waker::from(signal.clone());let mut cx=Context::from_waker(&waker);let mut future=std::pin::pin!(future);let deadline=Instant::now()+Duration::from_secs(2);loop{if let Poll::Ready(v)=future.as_mut().poll(&mut cx){return Ok(v)}let now=Instant::now();if now>=deadline{return Err(())}let ready=signal.ready.lock().unwrap();let(mut ready,timed)=signal.cv.wait_timeout_while(ready,deadline-now,|v|!*v).unwrap();if timed.timed_out()&&!*ready{return Err(())}*ready=false}}

fn main()->wasmtime::Result<()> {
 let mut config=Config::new();
 config.wasm_component_model(true).wasm_component_model_async(true).wasm_component_model_more_async_builtins(true).wasm_component_model_async_stackful(true).concurrency_support(true);
 let engine=Engine::new(&config)?;let mut store=Store::new(&engine,());
 let component=Component::from_file(&engine,std::env::args().nth(1).expect("component path"))?;
 let instance=Linker::new(&engine).instantiate(&mut store,&component)?;
 let interface=instance.get_export_index(&mut store,None,"wisp:async-values/operations@0.1.0").unwrap();
 let idx=instance.get_export_index(&mut store,Some(&interface),"delayed").unwrap();
 let delayed:TypedFunc<(Payload,),(FutureReader<Payload>,)>=instance.get_typed_func(&mut store,&idx)?;
 let idx=instance.get_export_index(&mut store,Some(&interface),"words").unwrap();
 let words:TypedFunc<(Vec<String>,),(StreamReader<String>,)>=instance.get_typed_func(&mut store,&idx)?;
 let idx=instance.get_export_index(&mut store,Some(&interface),"active").unwrap();
 let active:TypedFunc<(),(u32,)>=instance.get_typed_func(&mut store,&idx)?;
 let payload=Payload{label:"nested".into(),values:vec![3,5]};
 let (reader,)=block_on(store.run_concurrent(async move |accessor|delayed.call_concurrent(&accessor,(payload,)).await)).unwrap()??;
 println!("created future, active={:?}",active.call(&mut store,())?);
 let shared=Arc::new(Collected::<Payload>::default());reader.pipe(&mut store,Collector(shared.clone()))?;
 block_on(store.run_concurrent(async |_|wait(shared.clone(),true).await)).unwrap()?;
 assert_eq!(shared.values.lock().unwrap().remove(0),Payload{label:"nested".into(),values:vec![3,5]});
 println!("future record complete, active={:?}",active.call(&mut store,())?);
 let (reader,)=block_on(store.run_concurrent(async move |accessor|words.call_concurrent(&accessor,(vec!["hello".into(),"βeta".into()],)).await)).unwrap()??;
 let shared=Arc::new(Collected::<String>::default());reader.pipe(&mut store,Collector(shared.clone()))?;
 block_on(store.run_concurrent(async |_|wait(shared.clone(),true).await)).unwrap()?;
 assert_eq!(*shared.values.lock().unwrap(),vec!["hello","βeta"]);
 println!("stream strings complete, active={:?}",active.call(&mut store,())?);
 let (mut reader,)=block_on(store.run_concurrent(async move |accessor|delayed.call_concurrent(&accessor,(Payload{label:"cancel".into(),values:vec![]},)).await)).unwrap()??;
 reader.close(&mut store)?;
 block_on(store.run_concurrent(async |accessor|poll_fn(|cx|accessor.poll_no_interesting_tasks(cx)).await)).unwrap()?;
 println!("future cancelled, active={:?}",active.call(&mut store,())?);
 store.assert_concurrent_state_empty();
 Ok(())
}
