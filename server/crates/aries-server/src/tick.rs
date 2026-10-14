//! fixed-rate tick clock. 128 hz (7.8125 ms) like retail valorant servers.
//! waits on a high-resolution waitable timer, then spins the last stretch.

use std::time::{Duration, Instant};

pub const TICK_RATE: u32 = 128;
/// wake this early and spin the rest; covers timer wake-up jitter.
const SPIN_MARGIN: Duration = Duration::from_micros(600);

pub struct TickClock {
    period: Duration,
    next: Instant,
    pub tick: u64,
    stats: TickStats,
    timer: Timer,
}

/// per-window frame timing, reported once per stats window.
#[derive(Default, Clone, Copy)]
pub struct TickStats {
    pub ticks: u32,
    pub busy_total: Duration,
    pub busy_max: Duration,
    pub overruns: u32,
}

impl TickClock {
    pub fn new(rate: u32) -> Self {
        let period = Duration::from_nanos(1_000_000_000 / rate as u64);
        Self { period, next: Instant::now() + period, tick: 0, stats: TickStats::default(), timer: Timer::new() }
    }

    /// seconds per tick, for simulation code.
    #[allow(dead_code)]
    pub fn delta(&self) -> f64 {
        self.period.as_secs_f64()
    }

    /// call at the end of a tick with when the work started; waits for the next boundary.
    pub fn wait(&mut self, work_started: Instant) {
        let busy = work_started.elapsed();
        self.stats.ticks += 1;
        self.stats.busy_total += busy;
        self.stats.busy_max = self.stats.busy_max.max(busy);
        let now = Instant::now();
        if now >= self.next {
            // overran: count it and resync instead of bursting to catch up
            self.stats.overruns += 1;
            if now - self.next > self.period * 4 {
                self.next = now;
            }
        } else {
            let remaining = self.next - now;
            if remaining > SPIN_MARGIN {
                self.timer.sleep(remaining - SPIN_MARGIN);
            }
            while Instant::now() < self.next {
                std::hint::spin_loop();
            }
        }
        self.next += self.period;
        self.tick += 1;
    }

    /// takes and resets the stats window.
    pub fn take_stats(&mut self) -> TickStats {
        std::mem::take(&mut self.stats)
    }
}

#[cfg(windows)]
struct Timer(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Timer {
    fn new() -> Self {
        use windows_sys::Win32::System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
            TIMER_ALL_ACCESS,
        };
        unsafe {
            windows_sys::Win32::Media::timeBeginPeriod(1);
            // the tick thread must win the scheduler when the box is busy
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            Self(CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS))
        }
    }

    fn sleep(&self, duration: Duration) {
        use windows_sys::Win32::System::Threading::{INFINITE, SetWaitableTimer, WaitForSingleObject};
        if self.0.is_null() {
            std::thread::sleep(duration);
            return;
        }
        // negative = relative, in 100 ns units
        let due = -((duration.as_nanos() / 100) as i64);
        unsafe {
            if SetWaitableTimer(self.0, &due, 0, None, std::ptr::null(), 0) != 0 {
                WaitForSingleObject(self.0, INFINITE);
            } else {
                std::thread::sleep(duration);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Timer {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
            windows_sys::Win32::Media::timeEndPeriod(1);
        }
    }
}

#[cfg(not(windows))]
struct Timer;

#[cfg(not(windows))]
impl Timer {
    fn new() -> Self {
        Self
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
