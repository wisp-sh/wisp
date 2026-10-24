fn sum<T>(values: impl IntoIterator<Item = T>) -> Result<T, String>
where
    T: std::iter::Sum<T>,
{
    Ok(values.into_iter().sum())
}

pub fn add(a: u32, b: u32) -> u32 {
    let values = vec![a, b];
    sum(values).expect("the arithmetic sum is available")
}

pub fn negate(value: bool) -> bool {
    !value
}

pub fn widen(value: u64) -> u64 {
    value + 1
}
