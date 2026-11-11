//! The kernel's notice that something a font scan read changed (#2537):
//! inotify watches on what fontconfig's configuration lists, and an eventfd
//! that wakes the thread for a new setting. Both through `rustix`, safely.
//!
//! What is watched, for a scan's [`SystemFontPaths`]:
//! - each font directory that exists: a file or directory made, removed,
//!   moved in or out, or written in it is a change, except fontconfig's own
//!   `.uuid` file (written by a scan, so it would set off the next);
//! - each included configuration directory (`conf.d`): any entry changed;
//! - the directory of each configuration file, for that file's name only
//!   (editors replace a file by renaming over it, which a watch on the file
//!   itself would not survive);
//! - a font or configuration directory that does not exist (a configured
//!   `~/.local/share/fonts` on a new account), through its nearest existing
//!   parent, for the making of the next part of its path;
//! - and on each watched directory itself, its removal or move: the watch
//!   is then armed again from the rescan, so a directory removed and made
//!   again is followed.
//!
//! A change is reported, not what changed: the caller rescans, and arms the
//! watch again from what the new scan read. A queue overflow is a change. An
//! error from `poll` or `read` is [`Wait::Failed`], which ends the watch: it
//! would fail again at once, so neither spinning nor rescanning would help.

use parley::fontique::SystemFontPaths;
use rustix::event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll};
use rustix::fd::OwnedFd;
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// What one wait saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Wait {
    /// Something the scan read changed.
    Changed,
    /// [`Wake::notify`] was called: read the setting again.
    Woken,
    /// The time ran out, or only events that are no change arrived.
    Quiet,
    /// `poll` or `read` failed: the watch cannot go on.
    Failed(rustix::io::Errno),
}

/// What one [`Watch::arm`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Armed {
    /// Directories watched now that were not watched before: a font written
    /// into one between the rescan that listed it and this arm is in neither,
    /// so the caller scans once more.
    pub(super) new: usize,
    /// Directories not watched because the user's inotify watch limit
    /// (`fs.inotify.max_user_watches`) was reached.
    pub(super) over_limit: usize,
}

/// Wakes a [`Watch`]'s wait from another thread.
#[derive(Debug)]
pub(super) struct Wake(OwnedFd);

impl Wake {
    pub(super) fn notify(&self) {
        // An eventfd counter: a write never blocks short of 2^64 - 1 pending
        // notices, and a failed one only leaves the thread to its next event.
        let _ = rustix::io::write(&self.0, &1u64.to_ne_bytes());
    }
}

/// Why a watched directory is watched.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Role {
    /// A font directory: any entry.
    Fonts,
    /// An included configuration directory: any entry.
    Config,
    /// The directory of configuration files: these names only.
    ConfigFiles(HashSet<OsString>),
    /// The nearest existing parent of a missing directory: the making of
    /// this name, the next part of its path.
    Awaited(OsString),
}

/// Events on a directory's entries.
const ENTRIES: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::CLOSE_WRITE);
/// Events on the watched directory itself.
const ITSELF: WatchFlags = WatchFlags::DELETE_SELF
    .union(WatchFlags::MOVE_SELF)
    .union(WatchFlags::ONLYDIR);
/// The awaited name appearing.
const APPEARING: WatchFlags = WatchFlags::CREATE.union(WatchFlags::MOVED_TO);

/// fontconfig's per-directory id file, written by a scan.
const UUID: &[u8] = b".uuid";

pub(super) struct Watch {
    inotify: OwnedFd,
    wake: Arc<Wake>,
    /// The directory and roles of each watch descriptor armed.
    roles: HashMap<i32, Watched>,
    /// Descriptors this watch removed whose `IGNORED` is still to come.
    removed: HashSet<i32>,
    buf: Vec<MaybeUninit<u8>>,
}

impl Watch {
    pub(super) fn new() -> rustix::io::Result<Self> {
        let inotify = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
        let wake = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        Ok(Self {
            inotify,
            wake: Arc::new(Wake(wake)),
            roles: HashMap::new(),
            removed: HashSet::new(),
            buf: vec![MaybeUninit::uninit(); 16 * 1024],
        })
    }

    pub(super) fn waker(&self) -> Arc<Wake> {
        Arc::clone(&self.wake)
    }

    /// Watch what `paths` names, in place of what was watched before. A
    /// directory watched before and now keeps its descriptor, so an event
    /// already queued on it is still read.
    pub(super) fn arm(&mut self, paths: &SystemFontPaths) -> Armed {
        let mut wanted = Wanted::default();
        for dir in &paths.font_dirs {
            wanted.dir_or_its_parent(dir, Role::Fonts);
        }
        for dir in &paths.config_dirs {
            wanted.dir_or_its_parent(dir, Role::Config);
        }
        for file in &paths.config_files {
            let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
                continue;
            };
            let names = HashSet::from([name.to_os_string()]);
            wanted.dir_or_its_parent(dir, Role::ConfigFiles(names));
        }
        self.arm_dirs(wanted.0)
    }

    /// Watch `dirs`, each for its events and roles, in this order, and
    /// nothing else.
    fn arm_dirs(
        &mut self,
        dirs: impl IntoIterator<Item = (PathBuf, (WatchFlags, Vec<Role>))>,
    ) -> Armed {
        let mut armed = Armed::default();
        let mut roles: HashMap<i32, Watched> = HashMap::new();
        let mut flags_of: HashMap<i32, WatchFlags> = HashMap::new();
        for (dir, (flags, dir_roles)) in dirs {
            let added = inotify::add_watch(&self.inotify, &dir, flags).and_then(|wd| {
                // Two paths to one directory (a link) share a descriptor, and
                // an add replaces its events with the ones just given: if
                // those lack any an earlier path wanted, give them all.
                match flags_of.get(&wd) {
                    Some(&before) if !flags.contains(before) => {
                        inotify::add_watch(&self.inotify, &dir, before | flags)
                    }
                    _ => Ok(wd),
                }
            });
            match added {
                Ok(wd) => {
                    *flags_of.entry(wd).or_insert(WatchFlags::empty()) |= flags;
                    let watched = roles.entry(wd).or_insert_with(|| Watched {
                        dir: dir.clone(),
                        roles: Vec::new(),
                    });
                    watched.roles.extend(dir_roles);
                }
                Err(rustix::io::Errno::NOSPC) => armed.over_limit += 1,
                // Unreadable, or gone since the check: that directory is not
                // watched.
                Err(e) => tracing::debug!("font watch: {}: {e}", dir.display()),
            }
        }
        if armed.over_limit > 0 {
            tracing::warn!(
                "rinch: {} font directories are not watched for installed fonts: the \
                 inotify watch limit is reached (sysctl fs.inotify.max_user_watches)",
                armed.over_limit
            );
        }
        armed.new = roles
            .keys()
            .filter(|wd| !self.roles.contains_key(wd))
            .count();
        for &wd in self.roles.keys() {
            if !roles.contains_key(&wd) && inotify::remove_watch(&self.inotify, wd).is_ok() {
                self.removed.insert(wd);
            }
        }
        self.roles = roles;
        armed
    }

    /// Sleep in the kernel until an event or a notice, or for at most
    /// `timeout`.
    pub(super) fn wait(&mut self, timeout: Option<Duration>) -> Wait {
        let timeout = timeout.map(|t| Timespec {
            tv_sec: t.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: t.subsec_nanos().into(),
        });
        let mut fds = [
            PollFd::new(&self.inotify, PollFlags::IN),
            PollFd::new(&self.wake.0, PollFlags::IN),
        ];
        match poll(&mut fds, timeout.as_ref()) {
            Ok(0) | Err(rustix::io::Errno::INTR) => return Wait::Quiet,
            Ok(_) => {}
            Err(e) => return Wait::Failed(e),
        }
        let (events, woken) = (fds[0].revents(), fds[1].revents());
        if !woken.is_empty() {
            let mut count = [0u8; 8];
            let _ = rustix::io::read(&self.wake.0, &mut count);
            return Wait::Woken;
        }
        if events.is_empty() {
            return Wait::Quiet;
        }
        self.read_events()
    }

    /// Read every queued event; a change if any of them is one.
    fn read_events(&mut self) -> Wait {
        let mut changed = false;
        let mut reader = inotify::Reader::new(&self.inotify, &mut self.buf);
        loop {
            let event = match reader.next() {
                Ok(event) => event,
                Err(rustix::io::Errno::AGAIN) => break,
                Err(e) => return Wait::Failed(e),
            };
            let flags = event.events();
            let wd = event.wd();
            if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                changed = true;
            } else if flags.contains(ReadFlags::IGNORED) {
                // Ours to ignore if this watch removed it; else the kernel
                // dropped it (the directory went, or its file system).
                changed |= !self.removed.remove(&wd) && self.roles.contains_key(&wd);
            } else if let Some(watched) = self.roles.get(&wd) {
                let name = event.file_name().map(|n| n.to_bytes());
                if watched
                    .roles
                    .iter()
                    .any(|role| is_change(role, flags, name))
                {
                    tracing::debug!(
                        "font watch: {:?} in {}: {:?}",
                        name.map(String::from_utf8_lossy),
                        watched.dir.display(),
                        flags
                    );
                    changed = true;
                }
            }
        }
        if changed { Wait::Changed } else { Wait::Quiet }
    }
}

/// Whether every directory watched has been scanned since its watch began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Settle {
    /// Yes: wait for a change.
    Settled,
    /// No: a directory first watched by the last arm has had no scan since.
    /// Treat it as a change: debounce, then rescan.
    Unsettled,
}

/// Arm `watch` from `paths`, the scan just made.
///
/// A directory watched for the first time was read by that scan a moment
/// before its watch began, so a font written into it in between would be in
/// neither. So after an arm that watches a new directory the fonts are
/// scanned once more (`rescan`, which returns the new scan's paths) and the
/// watch armed from those. If that arm watches a new directory too (one made
/// in the meantime, an installer's `family/` and then `family/static/`), the
/// answer is [`Settle::Unsettled`]: the caller treats it as a change, and
/// rescans after the debounce, so no directory goes unscanned after its watch
/// began, and a process making directories without pause costs one rescan
/// per interval, not a scanning loop.
///
/// Not after the `first` arm, at start-up: that would be a second scan of
/// every start (254 M instructions on one desktop) for a font installed in
/// the moment between the two; it is seen at the next change.
pub(super) fn arm_after_scan(
    watch: &mut Watch,
    paths: &SystemFontPaths,
    first: bool,
    rescan: &mut dyn FnMut() -> SystemFontPaths,
) -> Settle {
    if watch.arm(paths).new == 0 || first {
        return Settle::Settled;
    }
    let paths = rescan();
    if watch.arm(&paths).new == 0 {
        Settle::Settled
    } else {
        Settle::Unsettled
    }
}

/// A watched directory, and why.
struct Watched {
    dir: PathBuf,
    roles: Vec<Role>,
}

/// The directories to watch, with their events and roles, in path order.
#[derive(Default)]
struct Wanted(BTreeMap<PathBuf, (WatchFlags, Vec<Role>)>);

impl Wanted {
    /// Watch `dir` for `role`, or, while it does not exist, its nearest
    /// existing parent for its making.
    fn dir_or_its_parent(&mut self, dir: &Path, role: Role) {
        if dir.is_dir() {
            self.add(dir, ENTRIES | ITSELF, role);
        } else if let Some((parent, name)) = nearest_existing(dir) {
            self.add(&parent, APPEARING | ITSELF, Role::Awaited(name));
        }
    }

    fn add(&mut self, dir: &Path, flags: WatchFlags, role: Role) {
        let (all, roles) = self.0.entry(dir.to_path_buf()).or_default();
        *all |= flags;
        if let Role::ConfigFiles(names) = &role
            && let Some(Role::ConfigFiles(have)) =
                roles.iter_mut().find(|r| matches!(r, Role::ConfigFiles(_)))
        {
            have.extend(names.iter().cloned());
        } else if !roles.contains(&role) {
            roles.push(role);
        }
    }
}

/// Whether an event on a directory watched for `role` is a change.
fn is_change(role: &Role, flags: ReadFlags, name: Option<&[u8]>) -> bool {
    if flags.intersects(ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF | ReadFlags::UNMOUNT) {
        return true;
    }
    let Some(name) = name else {
        return false;
    };
    match role {
        Role::Fonts => name != UUID,
        Role::Config => true,
        Role::ConfigFiles(names) => names.iter().any(|n| n.as_bytes() == name),
        Role::Awaited(next) => {
            next.as_bytes() == name && flags.intersects(ReadFlags::CREATE | ReadFlags::MOVED_TO)
        }
    }
}

/// The nearest existing directory above `missing`, and the name of the next
/// part of the path below it.
fn nearest_existing(missing: &Path) -> Option<(PathBuf, OsString)> {
    let mut child = missing;
    while let Some(parent) = child.parent() {
        if parent.is_dir() {
            return Some((parent.to_path_buf(), child.file_name()?.to_os_string()));
        }
        child = parent;
    }
    None
}

#[cfg(test)]
#[path = "inotify_tests.rs"]
mod tests;
