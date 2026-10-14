//! session keys on a background thread: the handler scan can take a while (or
//! fail until the client builds its handler), and the tick must never wait.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aries_oracle::keys::{fingerprint, read_handler_keys};
use aries_range::{KeySource, SessionKeys};

pub struct BackgroundKeys {
    found: Arc<Mutex<Option<SessionKeys>>>,
    /// the newest failed scan: (its start in server seconds, the error)
    failed: Arc<Mutex<Option<(f64, String)>>>,
}

impl BackgroundKeys {
    /// `clock` is the server's start, so scan times line up with datagram times.
    pub fn start(pid: u32, image_base: u64, clock: Instant) -> Self {
        let found = Arc::new(Mutex::new(None));
        let failed = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&found);
        let failures = Arc::clone(&failed);
        std::thread::Builder::new()
            .name("key-scan".into())
            .spawn(move || {
                loop {
                    let started = clock.elapsed().as_secs_f64();
                    match read_handler_keys(pid, image_base, 30.0) {
                        Ok(keys) => {
                            log::info!(target: "keys", "ares handler {:#x}, server-send key fingerprint {}", keys.handler, fingerprint(&keys.server_send));
                            *slot.lock().expect("key slot") = Some(SessionKeys { client_send: keys.client_send, server_send: keys.server_send });
                            return;
                        }
                        Err(e) => {
                            // the handler appears when the client starts connecting; retry
                            // fast so its first datagrams wait as little as possible
                            log::debug!(target: "keys", "handler not ready ({e}); retrying");
                            *failures.lock().expect("failure slot") = Some((started, e.to_string()));
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            })
            .expect("key thread");
        Self { found, failed }
    }
}

impl KeySource for BackgroundKeys {
    fn read_keys(&mut self) -> Result<SessionKeys, String> {
        self.found.lock().expect("key slot").clone().ok_or_else(|| "ares handler keys not read yet".into())
    }

    fn failed_scan_since(&mut self, since: f64) -> Option<String> {
        self.failed.lock().expect("failure slot").as_ref().filter(|(started, _)| *started >= since).map(|(_, error)| error.clone())
    }
}
