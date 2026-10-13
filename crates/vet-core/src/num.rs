//! Conversions between the integer types that counts come in.

/// `n`, or the largest `u32` if it does not fit.
pub fn sat_u32<T: TryInto<u32>>(n: T) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_that_does_not_fit_saturates() {
        assert_eq!(sat_u32(7_usize), 7);
        assert_eq!(sat_u32(u64::from(u32::MAX) + 1), u32::MAX);
        assert_eq!(sat_u32(-1_i64), u32::MAX);
    }
}
