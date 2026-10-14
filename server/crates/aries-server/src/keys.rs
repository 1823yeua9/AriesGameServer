//! session keys on a background thread: the handler scan can take a while (or
//! fail until the client builds its handler), and the tick must never wait.
//! each range entry builds a new handler, so a rescan waits for keys that
//! differ from the last session's.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use aries_oracle::keys::{fingerprint, read_handler_keys};
use aries_range::{KeySource, SessionKeys};

#[derive(Default)]
struct Shared {
    found: Option<SessionKeys>,
    /// the last session's keys: a rescan must not hand them back
    previous: Option<SessionKeys>,
    /// the newest failed scan: (its start in server seconds, the error)
    failed: Option<(f64, String)>,
    wanted: bool,
}

pub struct BackgroundKeys {
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

impl BackgroundKeys {
    /// `clock` is the server's start, so scan times line up with datagram times.
    pub fn start(pid: u32, image_base: u64, clock: Instant) -> Self {
        let shared = Arc::new((Mutex::new(Shared { wanted: true, ..Default::default() }), Condvar::new()));
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("key-scan".into())
            .spawn(move || {
                let (state, wake) = &*worker;
                loop {
                    {
                        let mut guard = state.lock().expect("key state");
                        while !guard.wanted {
                            guard = wake.wait(guard).expect("key state");
                        }
                    }
                    let started = clock.elapsed().as_secs_f64();
                    let result = read_handler_keys(pid, image_base, 30.0);
                    let mut guard = state.lock().expect("key state");
                    match result {
                        Ok(keys) if guard.previous.as_ref().is_some_and(|old| old.server_send == keys.server_send) => {
                            // the old session's handler: the new one is not built yet
                            guard.failed = Some((started, "only the previous session's ares handler was found".into()));
                        }
                        Ok(keys) => {
                            log::info!(target: "keys", "ares handler {:#x}, server-send key fingerprint {}", keys.handler, fingerprint(&keys.server_send));
                            let keys = SessionKeys { client_send: keys.client_send, server_send: keys.server_send };
                            guard.previous = Some(keys.clone());
                            guard.found = Some(keys);
                            guard.wanted = false;
                            continue;
                        }
                        Err(e) => {
                            // the handler appears when the client starts connecting; retry
                            // fast so its first datagrams wait as little as possible
                            log::debug!(target: "keys", "handler not ready ({e}); retrying");
                            guard.failed = Some((started, e.to_string()));
                        }
                    }
                    drop(guard);
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
            .expect("key thread");
        Self { shared }
    }
}

impl KeySource for BackgroundKeys {
    fn read_keys(&mut self) -> Result<SessionKeys, String> {
        self.shared.0.lock().expect("key state").found.clone().ok_or_else(|| "ares handler keys not read yet".into())
    }

    fn failed_scan_since(&mut self, since: f64) -> Option<String> {
        self.shared.0.lock().expect("key state").failed.as_ref().filter(|(started, _)| *started >= since).map(|(_, error)| error.clone())
    }

    fn rescan(&mut self) {
        let (state, wake) = &*self.shared;
        let mut guard = state.lock().expect("key state");
        guard.found = None;
        guard.failed = None;
        guard.wanted = true;
        wake.notify_one();
    }
}
