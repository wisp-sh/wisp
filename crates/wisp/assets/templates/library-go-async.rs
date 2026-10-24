use std::{any::Any,collections::VecDeque,future::poll_fn,pin::Pin,sync::{Arc,Mutex,Condvar,atomic::{AtomicBool,Ordering}},task::{Context,Poll,Waker,Wake},time::{Duration,Instant}};
use wasmtime::component::{ComponentType,Lift,Source,FutureConsumer,FutureReader,FutureAny,StreamConsumer,StreamReader,StreamAny,StreamResult};

enum NativeReader { Future(FutureAny), Stream(StreamAny), Queued {state:Box<dyn Any+Send>,cancelled:Arc<AtomicBool>} }
struct Collected<T> { values: Mutex<VecDeque<T>>, done: Mutex<bool>, waker: Mutex<Option<Waker>> }
impl<T> Default for Collected<T> {fn default()->Self {Self{values:Mutex::new(VecDeque::new()),done:Mutex::new(false),waker:Mutex::new(None)}}}
struct Collector<T>(Arc<Collected<T>>,Arc<AtomicBool>);
impl<T> Collector<T> {fn wake(&self){if let Some(waker)=self.0.waker.lock().unwrap().take(){waker.wake()}}}
impl<T:Lift+Send+'static,D:'static> FutureConsumer<D> for Collector<T> {
    type Item=T;
    fn poll_consume(self:Pin<&mut Self>,_:&mut Context<'_>,mut store:wasmtime::StoreContextMut<'_,D>,mut source:Source<'_,T>,finish:bool)->Poll<wasmtime::Result<()>> {
        if finish {*self.0.done.lock().unwrap()=true;} else {let mut values=Vec::with_capacity(1);source.read(&mut store,&mut values)?;self.0.values.lock().unwrap().extend(values);}
        self.wake();Poll::Ready(Ok(()))
    }
}
impl<T:Lift+Send+'static,D:'static> StreamConsumer<D> for Collector<T> {
    type Item=T;
    fn poll_consume(self:Pin<&mut Self>,_:&mut Context<'_>,mut store:wasmtime::StoreContextMut<'_,D>,mut source:Source<'_,T>,finish:bool)->Poll<wasmtime::Result<StreamResult>> {
        if finish||self.1.load(Ordering::SeqCst) {*self.0.done.lock().unwrap()=true;self.wake();return Poll::Ready(Ok(if finish{StreamResult::Cancelled}else{StreamResult::Dropped}))}
        let count=source.remaining(&mut store);
        if count>65536 {return Poll::Ready(Err(wasmtime::Error::msg("stream batch limit")))}
        let mut values=Vec::with_capacity(count);source.read(&mut store,&mut values)?;
        let mut queued=self.0.values.lock().unwrap();if queued.len()+values.len()>65536 {return Poll::Ready(Err(wasmtime::Error::msg("stream item limit")))}
        queued.extend(values);drop(queued);self.wake();Poll::Ready(Ok(StreamResult::Completed))
    }
}
impl<T> Drop for Collector<T> {fn drop(&mut self){*self.0.done.lock().unwrap()=true;self.wake()}}
async fn wait_done<T>(state:Arc<Collected<T>>){poll_fn(move|cx|{if *state.done.lock().unwrap(){Poll::Ready(())}else{*state.waker.lock().unwrap()=Some(cx.waker().clone());Poll::Pending}}).await}
async fn wait_item<T>(state:Arc<Collected<T>>){poll_fn(move|cx|{if *state.done.lock().unwrap()||!state.values.lock().unwrap().is_empty(){Poll::Ready(())}else{*state.waker.lock().unwrap()=Some(cx.waker().clone());Poll::Pending}}).await}
struct ThreadWake{ready:Mutex<bool>,cv:Condvar}
impl Wake for ThreadWake{fn wake(self:Arc<Self>){self.wake_by_ref()}fn wake_by_ref(self:&Arc<Self>){*self.ready.lock().unwrap()=true;self.cv.notify_one()}}
fn block_on<F:std::future::Future>(future:F)->WireResult<F::Output>{
    let signal=Arc::new(ThreadWake{ready:Mutex::new(false),cv:Condvar::new()});let waker=Waker::from(signal.clone());let mut cx=Context::from_waker(&waker);let mut future=std::pin::pin!(future);let deadline=Instant::now()+Duration::from_secs(30);
    loop {if let Poll::Ready(value)=future.as_mut().poll(&mut cx){return Ok(value)};let now=Instant::now();if now>=deadline{return Err("async operation timed out".into())};let ready=signal.ready.lock().unwrap();let(mut ready,timed)=signal.cv.wait_timeout_while(ready,deadline-now,|v|!*v).unwrap();if timed.timed_out()&&!*ready{return Err("async operation timed out".into())};*ready=false}
}
impl Client {
    fn add_future<T:ComponentType+'static>(&mut self,reader:FutureReader<T>)->WireResult<u64>{let any=reader.try_into_future_any(&mut self.store).map_err(|e|format!("{e:#}"))?;self.add_reader(NativeReader::Future(any))}
    fn add_stream<T:ComponentType+'static>(&mut self,reader:StreamReader<T>)->WireResult<u64>{let any=reader.try_into_stream_any(&mut self.store).map_err(|e|format!("{e:#}"))?;self.add_reader(NativeReader::Stream(any))}
    fn add_reader(&mut self,mut reader:NativeReader)->WireResult<u64>{
        if self.readers.len()>=65536||self.next_reader==u64::MAX {
            match &mut reader {NativeReader::Future(reader)=>{let _=reader.close(&mut self.store);},NativeReader::Stream(reader)=>{let _=reader.close(&mut self.store);},NativeReader::Queued{..}=>{}}
            return Err("async reader limit".into())
        }
        let id=self.next_reader;self.next_reader+=1;self.readers.insert(id,reader);Ok(id)
    }
    fn close_reader(&mut self,id:u64)->WireResult<()> {if let Some(mut reader)=self.readers.remove(&id){match &mut reader {NativeReader::Future(reader)=>reader.close(&mut self.store).map_err(|e|format!("{e:#}"))?,NativeReader::Stream(reader)=>reader.close(&mut self.store).map_err(|e|format!("{e:#}"))?,NativeReader::Queued{cancelled,..}=>cancelled.store(true,Ordering::SeqCst)};block_on(self.store.run_concurrent(async |accessor|poll_fn(|cx|accessor.poll_no_interesting_tasks(cx)).await)).and_then(|r|r.map_err(|e|format!("{e:#}")))?};Ok(())}
    fn read_future<T:ComponentType+Lift+Send+'static>(&mut self,id:u64)->WireResult<Vec<T>> {
        let Some(NativeReader::Future(reader))=self.readers.remove(&id) else{return Err("future is closed".into())};
        let reader=reader.try_into_future_reader::<T>().map_err(|e|format!("{e:#}"))?;
        let state=Arc::new(Collected::<T>::default());reader.pipe(&mut self.store,Collector(state.clone(),Arc::new(AtomicBool::new(false)))).map_err(|e|format!("{e:#}"))?;
        block_on(self.store.run_concurrent(async |_|wait_done(state.clone()).await)).and_then(|r|r.map_err(|e|format!("{e:#}")))?;
        Ok(std::mem::take(&mut *state.values.lock().unwrap()).into_iter().collect())
    }
    fn read_stream<T:ComponentType+Lift+Send+'static>(&mut self,id:u64)->WireResult<Vec<T>> {
        let (state,cancelled)=match self.readers.remove(&id) {
            Some(NativeReader::Stream(reader))=>{let reader=reader.try_into_stream_reader::<T>().map_err(|e|format!("{e:#}"))?;let state=Arc::new(Collected::<T>::default());let cancelled=Arc::new(AtomicBool::new(false));reader.pipe(&mut self.store,Collector(state.clone(),cancelled.clone())).map_err(|e|format!("{e:#}"))?;(state,cancelled)},
            Some(NativeReader::Queued{state,cancelled})=>( *state.downcast::<Arc<Collected<T>>>().map_err(|_|"stream payload type mismatch")?,cancelled),
            _=>return Err("stream is closed".into()),
        };
        let needs_wait=state.values.lock().unwrap().is_empty()&&!*state.done.lock().unwrap();
        if needs_wait{block_on(self.store.run_concurrent(async |_|wait_item(state.clone()).await)).and_then(|r|r.map_err(|e|format!("{e:#}")))?;}
        let value=state.values.lock().unwrap().pop_front();
        if value.is_some()||!*state.done.lock().unwrap(){self.readers.insert(id,NativeReader::Queued{state:Box::new(state),cancelled});}
        Ok(value.into_iter().collect())
    }
}
