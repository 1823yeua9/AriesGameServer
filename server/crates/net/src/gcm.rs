//! aes-256-gcm with the client's 8-byte nonce and 4-byte tag. the stock
//! aes-gcm crate only does 12-16 byte tags, so this is gcm from aes + ghash.

use aes::Aes256;
use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use ghash::GHash;
use ghash::universal_hash::{KeyInit as HashKeyInit, UniversalHash};

/// nonce direction words: (counter, direction) packed little endian.
pub const SERVER_TO_CLIENT: u32 = 0x7908_CBDD;
pub const CLIENT_TO_SERVER: u32 = 0x31ED_27AB;
pub const TAG_BYTES: usize = 4;

pub struct AresGcm {
    cipher: Aes256,
    hash_key: [u8; 16],
}

impl AresGcm {
    pub fn new(key: &[u8; 32]) -> Self {
        let cipher = Aes256::new(GenericArray::from_slice(key));
        let mut hash_key = GenericArray::default();
        cipher.encrypt_block(&mut hash_key);
        Self { cipher, hash_key: hash_key.into() }
    }

    fn ghash(&self, blocks: &[&[u8]]) -> [u8; 16] {
        let mut hash = <GHash as HashKeyInit>::new(GenericArray::from_slice(&self.hash_key));
        for block in blocks {
            hash.update_padded(block);
        }
        hash.finalize().into()
    }

    /// j0 for a non-96-bit iv: ghash(iv || pad || [0]64 || [len(iv) bits]64).
    fn j0(&self, nonce: &[u8; 8]) -> [u8; 16] {
        let mut lengths = [0u8; 16];
        lengths[8..].copy_from_slice(&64u64.to_be_bytes());
        self.ghash(&[nonce, &lengths])
    }

    fn encrypt_block(&self, block: [u8; 16]) -> [u8; 16] {
        let mut block = GenericArray::from(block);
        self.cipher.encrypt_block(&mut block);
        block.into()
    }

    fn ctr(&self, j0: [u8; 16], data: &mut [u8]) {
        let mut counter = j0;
        for chunk in data.chunks_mut(16) {
            let low = u32::from_be_bytes(counter[12..].try_into().unwrap()).wrapping_add(1);
            counter[12..].copy_from_slice(&low.to_be_bytes());
            let stream = self.encrypt_block(counter);
            for (byte, key) in chunk.iter_mut().zip(stream) {
                *byte ^= key;
            }
        }
    }

    fn tag(&self, j0: [u8; 16], ciphertext: &[u8]) -> [u8; 16] {
        let mut lengths = [0u8; 16];
        lengths[8..].copy_from_slice(&((ciphertext.len() as u64) * 8).to_be_bytes());
        let hash = self.ghash(&[ciphertext, &lengths]);
        let mask = self.encrypt_block(j0);
        std::array::from_fn(|i| hash[i] ^ mask[i])
    }

    pub fn nonce(counter: u32, direction: u32) -> [u8; 8] {
        let mut nonce = [0u8; 8];
        nonce[..4].copy_from_slice(&counter.to_le_bytes());
        nonce[4..].copy_from_slice(&direction.to_le_bytes());
        nonce
    }

    /// encrypts in place, returns the 4-byte tag.
    pub fn seal(&self, nonce: &[u8; 8], data: &mut [u8]) -> [u8; TAG_BYTES] {
        let j0 = self.j0(nonce);
        self.ctr(j0, data);
        let tag = self.tag(j0, data);
        [tag[0], tag[1], tag[2], tag[3]]
    }

    /// checks the 4-byte tag, then decrypts in place.
    pub fn open(&self, nonce: &[u8; 8], data: &mut [u8], tag: &[u8; TAG_BYTES]) -> bool {
        let j0 = self.j0(nonce);
        let expected = self.tag(j0, data);
        let diff = expected[..TAG_BYTES].iter().zip(tag).fold(0u8, |acc, (a, b)| acc | (a ^ b));
        if diff != 0 {
            return false;
        }
        self.ctr(j0, data);
        true
    }
}
