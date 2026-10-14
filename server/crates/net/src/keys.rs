//! key material. production reads both keys from the live client's ares
//! handler (see the oracle); derive_receive_key is the player-key fallback.

use base64::Engine;
use sha2::{Digest, Sha256};

const SEED_WORDS: [u64; 4] = [0x43D2_C3DA_0210_55BF, 0xC451_FBCE_028C_A152, 0x8B70_42D4_5D1E_31F8, 0x7178_CCA8_6C4A_7250];

/// python derive_client_receive_key: sha256(seed || base64(player_key) or its raw bytes).
pub fn derive_receive_key(player_key: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for word in SEED_WORDS {
        hasher.update(word.to_le_bytes());
    }
    match base64::engine::general_purpose::STANDARD.decode(player_key) {
        Ok(decoded) => hasher.update(decoded),
        Err(_) => hasher.update(player_key.chars().map(|c| (c as u32 & 0xFF) as u8).collect::<Vec<_>>()),
    }
    hasher.finalize().into()
}
