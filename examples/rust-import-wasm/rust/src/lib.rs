pub fn doubled_sum(a: u32, b: u32) -> u32 {
    go_arithmetic::add(a, b) + cpp_arithmetic::add(a, b)
}

pub fn negate(value: bool) -> bool {
    go_arithmetic::negate(value)
}

pub fn widen(value: u64) -> u64 {
    cpp_arithmetic::widen(value)
}
