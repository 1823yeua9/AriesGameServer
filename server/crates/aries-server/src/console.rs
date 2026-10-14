//! console + logging. the loader may start us hidden, so we allocate our own
//! console when none is attached, then log to it (colored) and to a file.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use log::{Level, LevelFilter, Log, Metadata, Record};

#[cfg(windows)]
fn ensure_console(title: &str) {
    use windows_sys::Win32::System::Console::{
        AllocConsole, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetConsoleWindow, GetStdHandle, STD_OUTPUT_HANDLE,
        SetConsoleMode, SetConsoleTitleW,
    };
    unsafe {
        if GetConsoleWindow().is_null() {
            AllocConsole();
        }
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        SetConsoleTitleW(wide.as_ptr());
        // enable ansi colors
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(out, &mut mode) != 0 {
            SetConsoleMode(out, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

#[cfg(not(windows))]
fn ensure_console(_title: &str) {}

struct Logger {
    start: Instant,
    level: LevelFilter,
    file: Mutex<Option<File>>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let elapsed = self.start.elapsed().as_secs_f64();
        let (color, tag) = match record.level() {
            Level::Error => ("\x1b[31m", "error"),
            Level::Warn => ("\x1b[33m", "warn "),
            Level::Info => ("\x1b[36m", "info "),
            Level::Debug => ("\x1b[90m", "debug"),
            Level::Trace => ("\x1b[90m", "trace"),
        };
        let target = record.target().strip_prefix("aries_").unwrap_or(record.target());
        // stdout may be gone if the console was closed; never panic on log
        let _ = writeln!(std::io::stdout(), "\x1b[90m{elapsed:9.3}\x1b[0m {color}{tag}\x1b[0m \x1b[35m{target:<14}\x1b[0m {}", record.args());
        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = writeln!(file, "{elapsed:9.3} {tag} {target:<14} {}", record.args());
            }
        }
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

/// allocates the console (if needed) and installs the logger. log_path's
/// directory is created; logging still works when the file cannot be opened.
pub fn init(title: &str, level: LevelFilter, log_path: Option<&Path>) {
    ensure_console(title);
    let file = log_path.and_then(|path| {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        File::create(path).ok()
    });
    let logger = Logger { start: Instant::now(), level, file: Mutex::new(file) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    if let Some(path) = log_path {
        log::info!("logging to {}", path.display());
    }
}
