//! Validator self-test fixture (Seven Primes T-000): the FAILING variant.
//! `return` at the tail trips clippy::needless_return (denied by -D warnings), and the test asserts a wrong sum.

pub fn add(a: u32, b: u32) -> u32 {
    return fy::id(a) + b;
}

#[cfg(test)]
mod tests {
    #[test]
    fn adds() {
        assert_eq!(super::add(1, 1), 3);
    }
}
