//! aries range game server. owns the udp socket, runs the 128 hz tick, and
//! drives the range game mode for the pinned 13.06 client.

mod console;
mod tick;

use std::time::{Duration, Instant};

use log::{LevelFilter, info};

fn main() {
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("logs").join(format!("aries_server_{}.log", std::process::id()))));
    console::init("aries server", LevelFilter::Info, log_path.as_deref());
    info!("aries server {} starting, {} hz", env!("CARGO_PKG_VERSION"), tick::TICK_RATE);

    let mut clock = tick::TickClock::new(tick::TICK_RATE);
    let mut last_report = Instant::now();
    loop {
        let started = Instant::now();
        // the game loop lands here as the range crate comes online
        clock.wait(started);
        if last_report.elapsed() >= Duration::from_secs(10) {
            last_report = Instant::now();
            let stats = clock.take_stats();
            let average = stats.busy_total.as_secs_f64() * 1000.0 / stats.ticks.max(1) as f64;
            info!("tick {} | {:.1} hz | avg {average:.3} ms max {:.3} ms | overruns {}", clock.tick, stats.ticks as f64 / 10.0, stats.busy_max.as_secs_f64() * 1000.0, stats.overruns);
        }
    }
}
