//! aries range game server. owns the udp socket, runs the 128 hz tick, and
//! drives the range game mode for the pinned 13.06 client.
//!
//! usage: aries-server [--config aries-server.toml] [--port 58100] [--pid N --image-base 0x..]

mod console;
mod keys;
mod settings;
mod tick;

use std::io::ErrorKind;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use aries_range::{Outbox, RangeServer};
use log::{LevelFilter, error, info, warn};

fn main() {
    let settings = match settings::Settings::load() {
        Ok(settings) => settings,
        Err(message) => {
            console::init("aries server", LevelFilter::Info, None);
            error!("{message}");
            std::thread::sleep(Duration::from_secs(5));
            std::process::exit(2);
        }
    };
    let pid_tag = std::process::id();
    console::init("aries server", settings.log_level, Some(&settings.log_dir.join(format!("aries_server_{pid_tag}.log"))));
    info!("aries server {} starting: {} hz, udp {}", env!("CARGO_PKG_VERSION"), tick::TICK_RATE, settings.bind);

    let client = match settings.find_client() {
        Ok(client) => client,
        Err(message) => {
            error!("{message}");
            std::thread::sleep(Duration::from_secs(5));
            std::process::exit(3);
        }
    };
    info!("client pid {} image base {:#x}", client.pid, client.image_base);

    let socket = match UdpSocket::bind(&settings.bind) {
        Ok(socket) => socket,
        Err(e) => {
            error!("cannot bind {}: {e} (is the python responder still running?)", settings.bind);
            std::thread::sleep(Duration::from_secs(5));
            std::process::exit(4);
        }
    };
    socket.set_nonblocking(true).expect("non-blocking socket");

    let key_source = keys::BackgroundKeys::start(client.pid, client.image_base);
    let events = settings.log_dir.join(format!("aries_events_{pid_tag}.jsonl"));
    let mut server = RangeServer::new(settings.range.clone(), Box::new(key_source), Some(&events));
    info!("events -> {}", events.display());

    let mut clock = tick::TickClock::new(tick::TICK_RATE);
    let start = Instant::now();
    let mut last_report = Instant::now();
    let mut buffer = vec![0u8; 4096];
    let mut outbox = Outbox::default();
    let mut received = 0u64;
    let mut sent = 0u64;
    loop {
        let started = Instant::now();
        // drain everything that arrived since the last tick, in arrival order
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((length, source)) => {
                    received += 1;
                    let now = start.elapsed().as_secs_f64();
                    server.handle_datagram(now, source, &buffer[..length], &mut outbox);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // windows reports a closed client port as a receive error; keep going
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => {
                    warn!("recv: {e}");
                    break;
                }
            }
        }
        for (target, datagram) in outbox.datagrams.drain(..) {
            match socket.send_to(&datagram, target) {
                Ok(_) => sent += 1,
                Err(e) => warn!("send to {target}: {e}"),
            }
        }
        clock.wait(started);
        if last_report.elapsed() >= Duration::from_secs(30) {
            last_report = Instant::now();
            server.flush();
            let stats = clock.take_stats();
            let average = stats.busy_total.as_secs_f64() * 1000.0 / stats.ticks.max(1) as f64;
            info!(
                "tick {} | {:.1} hz | avg {average:.3} ms max {:.3} ms | overruns {} | rx {received} tx {sent} | connections {}",
                clock.tick,
                stats.ticks as f64 / 30.0,
                stats.busy_max.as_secs_f64() * 1000.0,
                stats.overruns,
                server.connections.len()
            );
        }
    }
}
