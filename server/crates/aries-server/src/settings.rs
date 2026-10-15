//! settings: aries-server.toml (next to the exe, or --config) plus cli
//! overrides. replaces the python responder's 107 flags; production is the default.

use std::path::{Path, PathBuf};

use aries_oracle::{FoundProcess, find_processes};
use aries_range::RangeConfig;
use log::LevelFilter;
use serde::Deserialize;

pub const CLIENT_EXE: &str = "VALORANT-Win64-Shipping.exe";
/// python check_project_client's exe, from the config folder (server/)
pub const PROJECT_CLIENT: &str = "../../game req/VALORANT_GAME/VALORANT/live/ShooterGame/Binaries/Win64/VALORANT-Win64-Shipping.exe";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct FileSettings {
    bind: String,
    log_dir: PathBuf,
    log_level: String,
    /// refuse any client whose exe is not this one (python check_project_client); "" skips the check
    client_path: PathBuf,
    range: RangeConfig,
}

impl Default for FileSettings {
    fn default() -> Self {
        Self { bind: "127.0.0.1:58100".into(), log_dir: PathBuf::from("logs"), log_level: "info".into(), client_path: PathBuf::from(PROJECT_CLIENT), range: RangeConfig::default() }
    }
}

pub struct Settings {
    pub bind: String,
    pub log_dir: PathBuf,
    pub log_level: LevelFilter,
    pub client_path: Option<PathBuf>,
    pub pid: Option<u32>,
    pub image_base: Option<u64>,
    pub range: RangeConfig,
}

fn parse_number(text: &str) -> Option<u64> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

impl Settings {
    pub fn load() -> Result<Self, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
        let config_path = value("--config").map(PathBuf::from).unwrap_or_else(|| exe_dir.join("aries-server.toml"));
        let file: FileSettings = if config_path.exists() {
            let text = std::fs::read_to_string(&config_path).map_err(|e| format!("{}: {e}", config_path.display()))?;
            toml::from_str(&text).map_err(|e| format!("{}: {e}", config_path.display()))?
        } else {
            FileSettings::default()
        };
        let mut bind = file.bind;
        if let Some(port) = value("--port") {
            let port: u16 = port.parse().map_err(|_| format!("bad --port {port}"))?;
            bind = format!("127.0.0.1:{port}");
        }
        if !bind.starts_with("127.0.0.1:") {
            return Err(format!("refusing to bind {bind}: the range server is loopback only"));
        }
        let log_level = match file.log_level.as_str() {
            "trace" => LevelFilter::Trace,
            "debug" => LevelFilter::Debug,
            "warn" => LevelFilter::Warn,
            "error" => LevelFilter::Error,
            _ => LevelFilter::Info,
        };
        let pid = value("--pid").map(|p| parse_number(&p).map(|v| v as u32).ok_or(format!("bad --pid {p}"))).transpose()?;
        let image_base = value("--image-base").map(|b| parse_number(&b).ok_or(format!("bad --image-base {b}"))).transpose()?;
        let log_dir = if file.log_dir.is_relative() { exe_dir.join(file.log_dir) } else { file.log_dir };
        // range paths are relative to the config file, whatever the cwd the loader gives us
        let config_dir = config_path.parent().filter(|_| config_path.exists()).map(Path::to_path_buf).unwrap_or(exe_dir);
        let mut range = file.range;
        let anchor = |path: &mut PathBuf| {
            if path.is_relative() {
                *path = config_dir.join(&*path);
            }
        };
        anchor(&mut range.skin_catalog);
        anchor(&mut range.skin_state);
        // the loader passes the platform's own skin files (cwd-relative, like --config)
        if let Some(path) = value("--skin-catalog") {
            range.skin_catalog = PathBuf::from(path);
        }
        if let Some(path) = value("--skin-state") {
            range.skin_state = PathBuf::from(path);
        }
        if let Some(queue) = range.command_queue.as_mut() {
            anchor(queue);
        }
        if let Some(queue) = value("--command-queue") {
            range.command_queue = Some(PathBuf::from(queue));
        }
        let mut client_path = file.client_path;
        let client_path = if client_path.as_os_str().is_empty() {
            None
        } else {
            anchor(&mut client_path);
            Some(client_path)
        };
        Ok(Self { bind, log_dir, log_level, client_path, pid, image_base, range })
    }

    /// the one running client (or the one given on the command line).
    pub fn find_client(&self) -> Result<FoundProcess, String> {
        let mut found = find_processes(CLIENT_EXE);
        let client = match self.pid {
            Some(pid) => found.into_iter().find(|p| p.pid == pid).ok_or(format!("no {CLIENT_EXE} with pid {pid}"))?,
            None => {
                if found.len() != 1 {
                    return Err(format!("expected one {CLIENT_EXE}, found {}", found.len()));
                }
                found.remove(0)
            }
        };
        // python check_project_client: an unreadable path refuses too
        if let Some(expected) = &self.client_path {
            let actual = client.path.as_ref().ok_or(format!("cannot read the exe path of pid {}; refusing it", client.pid))?;
            let canonical = |path: &Path| path.canonicalize().map(|p| p.to_string_lossy().to_lowercase()).map_err(|e| format!("{}: {e}", path.display()));
            if canonical(expected)? != canonical(actual)? {
                return Err(format!("refusing non-project client {}", actual.display()));
            }
        }
        let image_base = self.image_base.unwrap_or(client.image_base);
        if image_base == 0 {
            return Err(format!("could not read the image base of pid {}", client.pid));
        }
        Ok(FoundProcess { image_base, ..client })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the shipped config parses, with every section the server reads
    #[test]
    fn shipped_config_parses() {
        let file: FileSettings = toml::from_str(include_str!("../../../aries-server.toml")).expect("aries-server.toml");
        assert_eq!(file.bind, "127.0.0.1:58100");
        assert!(file.client_path.ends_with("VALORANT-Win64-Shipping.exe"));
        assert!(file.range.features.retransmit && file.range.lagcomp.enabled);
        assert_eq!(file.range.interp.oracle_interp_ratio, 2.0);
        assert_eq!(file.range.interp.stream(), aries_interp::stream::StreamConfig::default());
        assert_eq!(file.range.bot_count, 5);
        assert!(file.range.pacing.is_fast());
        assert_eq!(file.range.pacing.stage_gap, 0.05);
    }
}
