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

/// The user's home directory as an absolute path, from `HOME` (or
/// `USERPROFILE`). `None` when neither is set, is empty, or is relative: a
/// relative one would put the directory under whatever the current directory
/// happens to be, which may be shared (#2516).
pub fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|p| p.is_absolute())
}

/// The directory discovery files are written to and read from
/// (`<home>/.rinch/debug`), or `None` when there is no usable home directory
/// (#2516: the server then does not start, and a client finds nothing).
///
/// Public so clients resolve it exactly the way the writer does — a client that
/// derives the home directory by some other route (`dirs::home_dir()`, say)
/// silently looks in the wrong place whenever `HOME` is unset or differs.
pub fn discovery_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".rinch").join("debug"))
}

fn no_home() -> std::io::Error {
    std::io::Error::other("no absolute HOME/USERPROFILE: not starting the debug interface")
}

fn discovery_file_path() -> Option<PathBuf> {
    discovery_dir().map(|d| d.join(format!("{}.json", std::process::id())))
}

/// Create `dir` (and missing parents) below `root` so that only this user can
/// enter it, checking every component from `root` down (#2516).
///
/// On Unix `root` (the home directory) must be ours and not writable by
/// others, and each component below it must end up ours, a real directory (not
/// a symbolic link) and mode 0700: one an older rinch made 0755 is tightened,
/// and one owned by someone else is an error, so the server does not start.
/// `dir` must lie under `root`. Elsewhere it is created with the defaults (a
/// Windows profile directory is already private to its user).
pub(crate) fn create_private_dir(root: &Path, dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        // SAFETY: geteuid has no preconditions and cannot fail.
        let uid = unsafe { libc::geteuid() };
        let rel = dir.strip_prefix(root).map_err(|_| {
            std::io::Error::other(format!("{} is not under {}", dir.display(), root.display()))
        })?;
        // The root may itself be a link (a relocated home); what matters is
        // whose it is and who can write to it.
        let meta = std::fs::metadata(root)?;
        if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o002 != 0 {
            return Err(std::io::Error::other(format!(
                "{} is not a directory of this user's, or is writable by everyone",
                root.display()
            )));
        }
        let mut cur = root.to_path_buf();
        for part in rel.components() {
            cur.push(part);
            match std::fs::DirBuilder::new().mode(0o700).create(&cur) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            let meta = std::fs::symlink_metadata(&cur)?;
            if !meta.is_dir() {
                return Err(std::io::Error::other(format!(
                    "{} is not a directory",
                    cur.display()
                )));
            }
            if meta.uid() != uid {
                return Err(std::io::Error::other(format!(
                    "{} belongs to uid {}, not to this user ({uid})",
                    cur.display(),
                    meta.uid()
                )));
            }
            if meta.mode() & 0o777 != 0o700 {
                std::fs::set_permissions(&cur, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = root;
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
    let root = home_dir().ok_or_else(no_home)?;
    let dir = root.join(".rinch").join("debug");
    create_private_dir(&root, &dir)?;

    let entry = DiscoveryEntry {
        pid: std::process::id(),
        port,
        app_name: app_name.to_string(),
        protocol_version: 1,
        started_at: format!("{:?}", std::time::SystemTime::now()),
        token: Some(token.to_string()),
    };

    let data = serde_json::to_string_pretty(&entry).map_err(std::io::Error::other)?;
    write_private_file(
        &dir.join(format!("{}.json", std::process::id())),
        data.as_bytes(),
    )
}

/// The path of this process's discovery file.
pub fn own_discovery_file() -> Option<PathBuf> {
    discovery_file_path()
}

pub fn remove_discovery_file() {
    if let Some(p) = discovery_file_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// List all discovery entries (used by MCP server for scanning).
pub fn list_discovery_entries() -> Vec<DiscoveryEntry> {
    let Some(dir) = discovery_dir() else {
        return vec![];
    };
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

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rinch-2516-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A directory an older rinch made 0755 is tightened to 0700, at every
    /// level below the root.
    #[cfg(unix)]
    #[test]
    fn existing_open_directories_are_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("open");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let rinch = root.join(".rinch");
        let dir = rinch.join("debug");
        std::fs::create_dir_all(&dir).unwrap();
        for d in [&rinch, &dir] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        create_private_dir(&root, &dir).unwrap();
        let (a, b) = (mode(&rinch), mode(&dir));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!((a, b), (0o700, 0o700));
    }

    /// A symbolic link in place of the directory, or of a parent, is refused.
    #[cfg(unix)]
    #[test]
    fn a_symlink_in_place_of_a_component_is_refused() {
        let root = scratch("link");
        std::fs::create_dir_all(root.join("real/debug")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join(".rinch")).unwrap();
        let parent_link = create_private_dir(&root, &root.join(".rinch/debug"));
        std::fs::remove_file(root.join(".rinch")).unwrap();
        std::fs::create_dir_all(root.join(".rinch")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join(".rinch/debug")).unwrap();
        let leaf_link = create_private_dir(&root, &root.join(".rinch/debug"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(parent_link.is_err(), "a symlinked parent was accepted");
        assert!(leaf_link.is_err(), "a symlinked directory was accepted");
    }

    /// A world-writable root (a shared directory such as /tmp) is refused, and
    /// nothing is created in it.
    #[cfg(unix)]
    #[test]
    fn a_world_writable_root_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("ww");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
        let r = create_private_dir(&root, &root.join(".rinch/debug"));
        let created = root.join(".rinch").exists();
        let _ = std::fs::remove_dir_all(&root);
        assert!(r.is_err());
        assert!(!created, "created under a world-writable root");
    }

    /// A directory outside the root is refused.
    #[cfg(unix)]
    #[test]
    fn a_directory_outside_the_root_is_refused() {
        let root = scratch("outside");
        let other = scratch("outside-other");
        let r = create_private_dir(&root, &other.join("x"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&other);
        assert!(r.is_err());
    }
}
