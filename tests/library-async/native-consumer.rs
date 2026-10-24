use std::{
    future::Future as _,
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};
use wisp_wisp__async_values::{CallError, Library, operations::Payload};
struct ThreadWake {
    ready: Mutex<bool>,
    cv: Condvar,
}
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref()
    }
    fn wake_by_ref(self: &Arc<Self>) {
        *self.ready.lock().unwrap() = true;
        self.cv.notify_one()
    }
}
fn block_on<F: std::future::Future>(future: F) -> Result<F::Output, ()> {
    let signal = Arc::new(ThreadWake {
        ready: Mutex::new(false),
        cv: Condvar::new(),
    });
    let waker = Waker::from(signal.clone());
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Poll::Ready(v) = future.as_mut().poll(&mut cx) {
            return Ok(v);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(());
        }
        let ready = signal.ready.lock().unwrap();
        let (mut ready, timed) = signal
            .cv
            .wait_timeout_while(ready, deadline - now, |v| !*v)
            .unwrap();
        if timed.timed_out() && !*ready {
            return Err(());
        }
        *ready = false
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = Library::new()?;
    let mut operations = library.operations();
    for _ in 0..8 {
        let future = block_on(operations.delayed(Payload {
            label: "native".into(),
            values: vec![13, 29],
        }))
        .unwrap()?;
        let value = block_on(future.receive()).unwrap()?;
        assert_eq!(value.label, "native");
        assert_eq!(value.values, vec![13, 29]);
        let mut stream = block_on(operations.words(vec!["one".into(), "two".into()])).unwrap()?;
        assert_eq!(block_on(stream.next()).unwrap()?, Some("one".into()));
        assert_eq!(block_on(stream.next()).unwrap()?, Some("two".into()));
        assert_eq!(block_on(stream.next()).unwrap()?, None);
        stream.close()?;
        assert!(block_on(stream.next()).unwrap().is_err());
    }
    let mut cancelled = block_on(operations.words(vec!["cancel".into(), "two".into()])).unwrap()?;
    assert_eq!(block_on(cancelled.next()).unwrap()?, Some("cancel".into()));
    drop(cancelled);
    let cancelled = block_on(operations.delayed(Payload {
        label: "cancel".into(),
        values: vec![],
    }))
    .unwrap()?;
    drop(cancelled);
    block_on(library.drain()).unwrap()?;
    assert_eq!(operations.active()?, 0);
    // An async call holds the Store while pending. Closing a different reader
    // on this executor must enqueue its cleanup and return without blocking.
    let mut held = block_on(operations.words(vec!["held".into()])).unwrap()?;
    let held_for_drop = block_on(operations.words(vec!["drop".into()])).unwrap()?;
    assert!(block_on(library.drain()).unwrap().unwrap_err().to_string().contains("close Component readers"));
    let mut in_flight = Box::pin(operations.delayed(Payload { label: "overlap".into(), values: vec![42] }));
    let signal = Arc::new(ThreadWake { ready: Mutex::new(false), cv: Condvar::new() });
    let waker = Waker::from(signal);
    let mut cx = Context::from_waker(&waker);
    assert!(matches!(in_flight.as_mut().poll(&mut cx), Poll::Pending));
    assert!(matches!(library.operations().active(), Err(CallError::Runtime(message)) if message == "Component instance is busy"));
    held.close()?;
    drop(held_for_drop);
    let result = block_on(in_flight).unwrap()?;
    assert_eq!(block_on(result.receive()).unwrap()?.values, vec![42]);
    block_on(library.drain()).unwrap()?;
    assert_eq!(operations.active()?, 0);
    println!("installed Rust async record + strings + cancellation passed");
    Ok(())
}
