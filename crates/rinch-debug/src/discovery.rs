//! The legacy discovery files, `~/.rinch/debug/{pid}.json`: how a client
//! finds an app's TCP port.
//!
//! An app writes one while it still listens on TCP. A client reads them to
//! reach apps built on rinch revisions before the socket transport (see
//! [`crate::client`]). On Unix an app is found by its socket instead.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One app's legacy discovery file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryEntry {
    pub pid: u32,
    pub port: u16,
    pub app_name: String,
    pub protocol_version: u32,
    pub started_at: String,
}

/// The directory discovery files are written to and read from.
///
/// Public so clients resolve it exactly the way the writer does — a client that
/// derives the home directory by some other route (`dirs::home_dir()`, say)
/// silently looks in the wrong place whenever `HOME` is unset or differs.
pub fn discovery_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".rinch").join("debug")
}

/// Writes this process's discovery file for `port` into `dir`, and returns
/// its path.
///
/// # Errors
///
/// When the directory or the file cannot be written.
pub fn write_discovery_file(dir: &Path, app_name: &str, port: u16) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let entry = DiscoveryEntry {
        pid: std::process::id(),
        port,
        app_name: app_name.to_string(),
        protocol_version: 1,
        started_at: format!("{:?}", std::time::SystemTime::now()),
    };
    let data = serde_json::to_string_pretty(&entry).map_err(std::io::Error::other)?;
    let path = dir.join(format!("{}.json", std::process::id()));
    std::fs::write(&path, data)?;
    Ok(path)
}

/// Every discovery file in `dir` that parses, whatever its process's state.
#[must_use]
pub fn read_discovery_entries(dir: &Path) -> Vec<DiscoveryEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                return None;
            }
            let data = std::fs::read_to_string(path).ok()?;
            serde_json::from_str(&data).ok()
        })
        .collect()
}
