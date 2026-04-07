/// Compute the next exponential backoff delay in milliseconds.
///
/// `attempt` is zero-based (0 = first retry).
pub fn next_backoff_ms(base_ms: u64, attempt: u32, max_ms: u64) -> u64 {
    let factor = 2u64.saturating_pow(attempt.min(20));
    base_ms.saturating_mul(factor).min(max_ms)
}

#[cfg(test)]
mod tests {
    use super::next_backoff_ms;

    #[test]
    fn grows_exponentially_until_cap() {
        assert_eq!(next_backoff_ms(100, 0, 5_000), 100);
        assert_eq!(next_backoff_ms(100, 1, 5_000), 200);
        assert_eq!(next_backoff_ms(100, 2, 5_000), 400);
        assert_eq!(next_backoff_ms(100, 10, 5_000), 5_000);
    }
}
