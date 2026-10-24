wit_bindgen::generate!({ path: "wit", world: "library" });

use exports::wisp::resource_library::counters::{Counter, CounterBorrow, Guest, GuestCounter};
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

static DROPPED: AtomicU32 = AtomicU32::new(0);
struct Component;
struct CounterValue(Cell<u32>);

impl Guest for Component {
    type Counter = CounterValue;
    fn make(start: u32) -> Counter { Counter::new(CounterValue(Cell::new(start))) }
    fn checked(start: u32) -> Result<Counter, String> {
        if start == 0 { Err("zero".into()) } else { Ok(Self::make(start)) }
    }
    fn dropped() -> u32 { DROPPED.load(Ordering::Relaxed) }
}

impl GuestCounter for CounterValue {
    fn new(start: u32) -> Self { Self(Cell::new(start)) }
    fn value(&self) -> u32 { self.0.get() }
    fn add(&self, delta: u32) -> u32 { self.0.set(self.0.get() + delta); self.0.get() }
    fn clear(&self) { self.0.set(0); }
    fn combine(left: CounterBorrow<'_>, right: Counter) -> Counter {
        Counter::new(Self(Cell::new(left.get::<Self>().0.get() + right.get::<Self>().0.get())))
    }
    fn explode(&self) { panic!("intentional resource trap") }
}

impl exports::wisp::resource_library::warehouse::Guest for Component {
    fn make_bundle(start: u32) -> exports::wisp::resource_library::warehouse::Bundle {
        exports::wisp::resource_library::warehouse::Bundle {
            primary: <Self as Guest>::make(start),
            others: vec![<Self as Guest>::make(start + 1), <Self as Guest>::make(start + 2)],
            optional: Some(<Self as Guest>::make(start + 3)),
        }
    }
    fn echo_bundle(value: exports::wisp::resource_library::warehouse::Bundle) -> exports::wisp::resource_library::warehouse::Bundle { value }
    fn echo_list(value: Vec<Counter>) -> Vec<Counter> { value }
    fn echo_option(value: Option<Counter>) -> Option<Counter> { value }
    fn echo_choice(value: exports::wisp::resource_library::warehouse::Choice) -> exports::wisp::resource_library::warehouse::Choice { value }
    fn sum(value: exports::wisp::resource_library::warehouse::Selection<'_>) -> u32 {
        value.first.get::<CounterValue>().0.get() + value.rest.iter().map(|v|v.get::<CounterValue>().0.get()).sum::<u32>()
    }
    fn sum_pair(value: exports::wisp::resource_library::warehouse::Pair<'_>) -> u32 {
        value.0.get::<CounterValue>().0.get() + value.1.map(|v|v.get::<CounterValue>().0.get()).unwrap_or(0)
    }
    fn sum_option(value: Option<CounterBorrow<'_>>) -> u32 { value.map(|v|v.get::<CounterValue>().0.get()).unwrap_or(0) }
    fn mix(value: exports::wisp::resource_library::warehouse::Mixed<'_>) -> Counter {
        <Self as Guest>::make(value.base.get::<CounterValue>().0.get() + value.added.get::<CounterValue>().0.get())
    }
}

impl Drop for CounterValue {
    fn drop(&mut self) { DROPPED.fetch_add(1, Ordering::Relaxed); }
}

export!(Component);
