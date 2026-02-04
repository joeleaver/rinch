use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct DiscoveryEntry {
    pub pid: u32,
    pub port: u16,
    pub app_name: String,
    pub protocol_version: u32,
    pub started_at: String,
}

fn discovery_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".rinch").join("debug")
}

fn discovery_file_path() -> PathBuf {
    discovery_dir().join(format!("{}.json", std::process::id()))
}

pub fn write_discovery_file(app_name: &str, port: u16) -> std::io::Result<()> {
    let dir = discovery_dir();
    std::fs::create_dir_all(&dir)?;

    let entry = DiscoveryEntry {
        pid: std::process::id(),
        port,
        app_name: app_name.to_string(),
        protocol_version: 1,
        started_at: format!("{:?}", std::time::SystemTime::now()),
    };

    let data = serde_json::to_string_pretty(&entry).map_err(std::io::Error::other)?;
    std::fs::write(discovery_file_path(), data)
}

pub fn remove_discovery_file() {
    let _ = std::fs::remove_file(discovery_file_path());
}

/// List all discovery entries (used by MCP server for scanning).
pub fn list_discovery_entries() -> Vec<DiscoveryEntry> {
    let dir = discovery_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let data = std::fs::read_to_string(entry.path()).ok()?;
            serde_json::from_str(&data).ok()
        })
        .collect()
}
