use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryEntry {
    pub pid: u32,
    pub port: u16,
    pub app_name: String,
    pub protocol_version: u32,
    pub started_at: String,
    /// The app's session token, sent in the handshake (#2481). Absent from
    /// a file written by a rinch from before #2481, whose server takes none
    /// and which any local user can reach.
    #[serde(default)]
    pub token: Option<String>,
}

/// Same rule as `rinch_debug::discovery::discovery_dir` (#2516): an absolute
/// `HOME`/`USERPROFILE` only, never the current directory.
fn discovery_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(|k| std::env::var_os(k))
        .map(PathBuf::from)
        .find(|p| p.is_absolute())
        .map(|h| h.join(".rinch").join("debug"))
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
    let Some(dir) = discovery_dir() else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };

    let mut result = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
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
