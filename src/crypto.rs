use aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::XChaCha20Poly1305;
use generic_array::GenericArray;

pub struct Crypto {
    cipher: XChaCha20Poly1305,
}

impl Crypto {
    pub fn new(key: &[u8; 32]) -> Self {
        let key = GenericArray::from_slice(key);
        Crypto {
            cipher: XChaCha20Poly1305::new(key),
        }
    }

    pub fn generate_nonce() -> [u8; 24] {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        nonce.into()
    }

    pub fn encrypt(&self, nonce: &[u8; 24], plaintext: &[u8]) -> Result<Vec<u8>, crate::error::Error> {
        let nonce = GenericArray::from_slice(nonce);
        self.cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| crate::error::Error::Crypto("AEAD encryption failed".into()))
    }

    pub fn decrypt(&self, nonce: &[u8; 24], ciphertext: &[u8]) -> Result<Vec<u8>, crate::error::Error> {
        let nonce = GenericArray::from_slice(nonce);
        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| crate::error::Error::Crypto("AEAD decryption failed: data may be corrupted".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [0xABu8; 32];
        let crypto = Crypto::new(&key);
        let nonce = Crypto::generate_nonce();
        let plaintext = b"Hello, VPN!";

        let ciphertext = crypto.encrypt(&nonce, plaintext).unwrap();
        let decrypted = crypto.decrypt(&nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_nonce_fails() {
        let key = [0xABu8; 32];
        let crypto = Crypto::new(&key);
        let nonce1 = Crypto::generate_nonce();
        let nonce2 = Crypto::generate_nonce();
        let plaintext = b"test data";

        let ciphertext = crypto.encrypt(&nonce1, plaintext).unwrap();
        let result = crypto.decrypt(&nonce2, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = [0xABu8; 32];
        let key2 = [0xBAu8; 32];
        let crypto1 = Crypto::new(&key1);
        let crypto2 = Crypto::new(&key2);
        let nonce = Crypto::generate_nonce();
        let plaintext = b"test data";

        let ciphertext = crypto1.encrypt(&nonce, plaintext).unwrap();
        let result = crypto2.decrypt(&nonce, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_corrupted_ciphertext_fails() {
        let key = [0xABu8; 32];
        let crypto = Crypto::new(&key);
        let nonce = Crypto::generate_nonce();
        let plaintext = b"test data";

        let mut ciphertext = crypto.encrypt(&nonce, plaintext).unwrap();
        if !ciphertext.is_empty() {
            ciphertext[0] ^= 0x01;
        }
        let result = crypto.decrypt(&nonce, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_plaintext() {
        let key = [0xABu8; 32];
        let crypto = Crypto::new(&key);
        let nonce = Crypto::generate_nonce();
        let plaintext = b"";

        let ciphertext = crypto.encrypt(&nonce, plaintext).unwrap();
        let decrypted = crypto.decrypt(&nonce, &ciphertext).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_nonce_uniqueness() {
        let mut nonces = std::collections::HashSet::new();
        for _ in 0..100_000 {
            let nonce = Crypto::generate_nonce();
            assert!(nonces.insert(nonce), "nonce collision detected");
        }
    }

    #[test]
    fn test_various_sizes() {
        let key = [0xABu8; 32];
        let crypto = Crypto::new(&key);
        let nonce = Crypto::generate_nonce();

        for size in &[1, 16, 64, 1400, 1500, 65535] {
            let plaintext = vec![0x42u8; *size];
            let ciphertext = crypto.encrypt(&nonce, &plaintext).unwrap();
            let decrypted = crypto.decrypt(&nonce, &ciphertext).unwrap();
            assert_eq!(decrypted, plaintext, "size {size} failed");
        }
    }
}
