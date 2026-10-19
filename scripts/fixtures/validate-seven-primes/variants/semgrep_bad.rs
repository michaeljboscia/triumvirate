//! Validator self-test fixture (Seven Primes T-000): trips p/rust `reqwest-accept-invalid` (WARNING, amendment A-1).
pub fn client() {
    let _c = reqwest::Client::builder().danger_accept_invalid_certs(true).build();
}
