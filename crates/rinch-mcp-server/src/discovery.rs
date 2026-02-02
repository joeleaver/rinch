use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Check if a PID is still alive
fn pid_alive(pid: u32) -> bool {
    // Check /proc/{pid} on Linux, assume alive elsewhere
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{}", pid)).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true // Assume alive, connection attempt will fail if dead
    }
}

pub fn list_apps() -> Vec<DiscoveryEntry> {
    let dir = discovery_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };

    let mut result = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "json") {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(entry) = serde_json::from_str::<DiscoveryEntry>(&data) else {
            // Invalid file, clean up
            let _ = std::fs::remove_file(&path);
            continue;
        };

        if pid_alive(entry.pid) {
            result.push(entry);
        } else {
            // Stale file, clean up
            tracing::debug!("Removing stale discovery file for PID {}", entry.pid);
            let _ = std::fs::remove_file(&path);
        }
    }
    result
}
