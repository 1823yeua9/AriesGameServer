//! console + logging. the loader may start us hidden, so we allocate our own
//! console when none is attached, then log to it (colored) and to a file.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// a visible console of our own: the loader may start us with no console, a
/// hidden one, or stdout redirected, and the user wants to watch the log. the
/// sink is CONOUT$ itself, so a redirected stdout cannot swallow it.
#[cfg(windows)]
fn ensure_console(title: &str) -> Option<File> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{AllocConsole, ENABLE_VIRTUAL_TERMINAL_PROCESSING, FreeConsole, GetConsoleMode, GetConsoleWindow, SetConsoleMode, SetConsoleTitleW};
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    unsafe {
        let window = GetConsoleWindow();
        if window.is_null() || IsWindowVisible(window) == 0 {
            FreeConsole();
            AllocConsole();
        }
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        SetConsoleTitleW(wide.as_ptr());
    }
    let console = std::fs::OpenOptions::new().read(true).write(true).open("CONOUT$").ok()?;
    unsafe {
        // ansi colors
        let handle = console.as_raw_handle();
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
    Some(console)
}

#[cfg(not(windows))]
fn ensure_console(_title: &str) -> Option<File> {
    None
}

struct Logger {
    start: Instant,
    level: LevelFilter,
    /// CONOUT$ (stdout when there is no console to open)
    console: Mutex<Option<File>>,
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
        // the console may be gone if it was closed; never panic on log
        let line = format!("\x1b[90m{elapsed:9.3}\x1b[0m {color}{tag}\x1b[0m \x1b[35m{target:<14}\x1b[0m {}", record.args());
        match self.console.lock().as_deref_mut() {
            Ok(Some(console)) => {
                let _ = writeln!(console, "{line}");
            }
            _ => {
                let _ = writeln!(std::io::stdout(), "{line}");
            }
        }
        if let Ok(mut file) = self.file.lock() && let Some(file) = file.as_mut() {
            let _ = writeln!(file, "{elapsed:9.3} {tag} {target:<14} {}", record.args());
        }
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
        if let Ok(Some(console)) = self.console.lock().as_deref_mut() {
            let _ = console.flush();
        }
        if let Ok(mut file) = self.file.lock() && let Some(file) = file.as_mut() {
            let _ = file.flush();
        }
    }
}

/// allocates the console (if needed) and installs the logger. log_path's
/// directory is created; logging still works when the file cannot be opened.
pub fn init(title: &str, level: LevelFilter, log_path: Option<&Path>) {
    let console = ensure_console(title);
    let file = log_path.and_then(|path| {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        File::create(path).ok()
    });
    let logger = Logger { start: Instant::now(), level, console: Mutex::new(console), file: Mutex::new(file) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    if let Some(path) = log_path {
        log::info!("logging to {}", path.display());
    }
}
