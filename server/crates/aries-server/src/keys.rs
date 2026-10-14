//! session keys on a background thread: the handler scan can take a while (or
//! fail until the client builds its handler), and the tick must never wait.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aries_oracle::keys::{fingerprint, read_handler_keys};
use aries_range::{KeySource, SessionKeys};

pub struct BackgroundKeys {
    found: Arc<Mutex<Option<SessionKeys>>>,
}

impl BackgroundKeys {
    pub fn start(pid: u32, image_base: u64) -> Self {
        let found = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&found);
        std::thread::Builder::new()
            .name("key-scan".into())
            .spawn(move || {
                loop {
                    match read_handler_keys(pid, image_base, 30.0) {
                        Ok(keys) => {
                            log::info!(target: "keys", "ares handler {:#x}, server-send key fingerprint {}", keys.handler, fingerprint(&keys.server_send));
                            *slot.lock().expect("key slot") = Some(SessionKeys { client_send: keys.client_send, server_send: keys.server_send });
                            return;
                        }
                        Err(e) => {
                            log::warn!(target: "keys", "handler not ready ({e}); retrying");
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
            })
            .expect("key thread");
        Self { found }
    }
}

impl KeySource for BackgroundKeys {
    fn read_keys(&mut self) -> Result<SessionKeys, String> {
        self.found.lock().expect("key slot").clone().ok_or_else(|| "ares handler keys not read yet".into())
    }
}
