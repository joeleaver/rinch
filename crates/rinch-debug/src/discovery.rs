use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct DiscoveryEntry {
    pub pid: u32,
    pub port: u16,
    pub app_name: String,
    pub protocol_version: u32,
    pub started_at: String,
    /// The per-session secret a client sends in its handshake (#2481). The
    /// file is readable by the app's user only, so holding the token is what
    /// "the same user" means over loopback TCP. `None` in a file written by a
    /// rinch from before #2481, whose server takes no token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
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

fn discovery_file_path() -> PathBuf {
    discovery_dir().join(format!("{}.json", std::process::id()))
}

/// Create `dir` (and missing parents) so that only this user can enter it.
///
/// On Unix the directory must end up ours, a real directory (not a symbolic
/// link) and mode 0700: a directory an older rinch made 0755 is tightened, and
/// one owned by someone else is an error, so the server does not start.
/// Elsewhere it is created with the defaults (a Windows profile directory is
/// already private to its user).
pub(crate) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        if !meta.is_dir() {
            return Err(std::io::Error::other(format!(
                "{} is not a directory",
                dir.display()
            )));
        }
        // SAFETY: geteuid has no preconditions and cannot fail.
        let uid = unsafe { libc::geteuid() };
        if meta.uid() != uid {
            return Err(std::io::Error::other(format!(
                "{} belongs to uid {}, not to this user ({uid})",
                dir.display(),
                meta.uid()
            )));
        }
        if meta.mode() & 0o777 != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Write `data` to `path` readable by this user only (0600 on Unix, set when
/// the file is created, so there is no moment it is open to others). Written
/// beside it first and renamed into place, so a client never reads half a
/// file.
fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

pub fn write_discovery_file(app_name: &str, port: u16, token: &str) -> std::io::Result<()> {
    let dir = discovery_dir();
    create_private_dir(&dir)?;

    let entry = DiscoveryEntry {
        pid: std::process::id(),
        port,
        app_name: app_name.to_string(),
        protocol_version: 1,
        started_at: format!("{:?}", std::time::SystemTime::now()),
        token: Some(token.to_string()),
    };

    let data = serde_json::to_string_pretty(&entry).map_err(std::io::Error::other)?;
    write_private_file(&discovery_file_path(), data.as_bytes())
}

/// The path of this process's discovery file.
pub fn own_discovery_file() -> PathBuf {
    discovery_file_path()
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

/// A fresh session token: 32 random bytes, hex-encoded.
pub(crate) fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compare a client's token with ours in time independent of where they
/// first differ.
pub(crate) fn token_matches(given: Option<&str>, ours: &str) -> bool {
    let Some(given) = given else {
        return false;
    };
    let (a, b) = (given.as_bytes(), ours.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_hex_and_differ() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn only_the_same_token_matches() {
        assert!(token_matches(Some("abcd"), "abcd"));
        assert!(!token_matches(Some("abce"), "abcd"));
        assert!(!token_matches(Some("abc"), "abcd"));
        assert!(!token_matches(Some(""), "abcd"));
        assert!(!token_matches(None, "abcd"));
    }

    /// A directory an older rinch made 0755 is tightened to 0700.
    #[cfg(unix)]
    #[test]
    fn an_existing_open_directory_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rinch-2481-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mode, 0o700);
    }

    /// A symbolic link where the directory should be is refused, not
    /// followed.
    #[cfg(unix)]
    #[test]
    fn a_symlink_in_place_of_the_directory_is_refused() {
        let base = std::env::temp_dir().join(format!("rinch-2481-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("real")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
        let r = create_private_dir(&base.join("link"));
        let _ = std::fs::remove_dir_all(&base);
        assert!(r.is_err(), "a symlinked directory was accepted");
    }
}
