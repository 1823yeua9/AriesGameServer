//! console + logging. the loader may start us hidden, so we allocate our own
//! console when none is attached, then log to it (colored) and to a file.
//! the tick thread only queues lines: a writer thread owns the console and the
//! file, so a slow or paused console (a text selection) never stalls a tick.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// lines waiting for the writer; past this the newest are dropped and counted.
const QUEUE: usize = 8192;

static DROPPED: AtomicU64 = AtomicU64::new(0);

/// log lines dropped because the writer fell behind (reported with the stats).
pub fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

/// a visible console of our own: the loader may start us with no console, a
/// hidden one, or stdout redirected, and the user wants to watch the log. the
/// sink is CONOUT$ itself, so a redirected stdout cannot swallow it.
#[cfg(windows)]
fn ensure_console(title: &str) -> Option<File> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{
        AllocConsole, ENABLE_EXTENDED_FLAGS, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, FreeConsole, GetConsoleMode, GetConsoleWindow, SetConsoleMode, SetConsoleTitleW,
    };
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
    // a click in the window would start a quick-edit selection, which pauses output
    if let Ok(input) = std::fs::OpenOptions::new().read(true).write(true).open("CONIN$") {
        unsafe {
            let handle = input.as_raw_handle();
            let mut mode = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                SetConsoleMode(handle, (mode | ENABLE_EXTENDED_FLAGS) & !ENABLE_QUICK_EDIT_MODE);
            }
        }
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

enum Line {
    Text { console: String, file: String },
    /// write out everything queued so far, then answer
    Flush(SyncSender<()>),
}

struct Logger {
    start: Instant,
    level: LevelFilter,
    queue: SyncSender<Line>,
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
        let console = format!("\x1b[90m{elapsed:9.3}\x1b[0m {color}{tag}\x1b[0m \x1b[35m{target:<14}\x1b[0m {}", record.args());
        let file = format!("{elapsed:9.3} {tag} {target:<14} {}", record.args());
        if self.queue.try_send(Line::Text { console, file }).is_err() {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self) {
        let (done, wait) = sync_channel(1);
        if self.queue.send(Line::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(2));
        }
    }
}

/// writes queued lines; flushes whenever the queue runs dry.
fn writer(queue: Receiver<Line>, mut console: Option<File>, mut file: Option<BufWriter<File>>) {
    let write = |line: Line, console: &mut Option<File>, file: &mut Option<BufWriter<File>>| match line {
        Line::Text { console: text, file: plain } => {
            // the console may be gone if it was closed; never panic on log
            match console.as_mut() {
                Some(console) => {
                    let _ = writeln!(console, "{text}");
                }
                None => {
                    let _ = writeln!(std::io::stdout(), "{text}");
                }
            }
            if let Some(file) = file.as_mut() {
                let _ = writeln!(file, "{plain}");
            }
            None
        }
        Line::Flush(done) => Some(done),
    };
    while let Ok(line) = queue.recv() {
        let mut answers = Vec::new();
        answers.extend(write(line, &mut console, &mut file));
        loop {
            match queue.try_recv() {
                Ok(line) => answers.extend(write(line, &mut console, &mut file)),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if let Some(file) = file.as_mut() {
            let _ = file.flush();
        }
        let _ = std::io::stdout().flush();
        for done in answers {
            let _ = done.send(());
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
        File::create(path).ok().map(BufWriter::new)
    });
    let (queue, lines) = sync_channel(QUEUE);
    std::thread::Builder::new().name("log-writer".into()).spawn(move || writer(lines, console, file)).expect("log thread");
    let logger = Logger { start: Instant::now(), level, queue };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    if let Some(path) = log_path {
        log::info!("logging to {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// lines reach the file in order, and a flush answers once they are written
    #[test]
    fn writer_drains_in_order_and_answers_flushes() {
        let path = std::env::temp_dir().join(format!("aries_console_{}.log", std::process::id()));
        let (queue, lines) = sync_channel(QUEUE);
        let file = BufWriter::new(File::create(&path).unwrap());
        let thread = std::thread::spawn(move || writer(lines, None, Some(file)));
        for i in 0..100 {
            queue.send(Line::Text { console: String::new(), file: format!("line {i}") }).unwrap();
        }
        let (done, wait) = sync_channel(1);
        queue.send(Line::Flush(done)).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).expect("flush answered");
        let text = std::fs::read_to_string(&path).unwrap();
        let written: Vec<_> = text.lines().collect();
        assert_eq!(written.len(), 100);
        assert_eq!(written[0], "line 0");
        assert_eq!(written[99], "line 99");
        drop(queue);
        thread.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
