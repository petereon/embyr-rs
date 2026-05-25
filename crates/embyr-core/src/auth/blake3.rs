pub fn derive_cache_key(api_key: &[u8]) -> [u8; 32] {
    *blake3::hash(api_key).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_cache_key_is_deterministic() {
        let k1 = derive_cache_key(b"mykey");
        let k2 = derive_cache_key(b"mykey");
        assert_eq!(k1, k2);
    }

    #[test]
    fn blake3_different_inputs_produce_different_keys() {
        let k1 = derive_cache_key(b"key-a");
        let k2 = derive_cache_key(b"key-b");
        assert_ne!(k1, k2);
    }
}
