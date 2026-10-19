//! Validator self-test fixture (Seven Primes T-000): the passing variant.

pub fn add(a: u32, b: u32) -> u32 {
    fy::id(a) + b
}

#[cfg(test)]
mod tests {
    #[test]
    fn adds() {
        assert_eq!(super::add(1, 1), 2);
    }
}
