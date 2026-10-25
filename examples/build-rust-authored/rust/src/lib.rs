mod wisp_bindings;
struct Math;
impl wisp_bindings::Guest for Math {
    fn add(a: u32, b: u32) -> u32 { a + b }
    fn negate(value: bool) -> bool { !value }
    fn widen(value: u64) -> u64 { value + 1 }
}
wisp_bindings::export!(Math with_types_in wisp_bindings);
