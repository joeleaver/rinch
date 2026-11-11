//! The font watch's kernel notices (#2537), on directories of the test's own
//! under the temporary directory: never the user's font directories, and no
//! fontconfig. Each test arms a [`Watch`] on a [`SystemFontPaths`] made here,
//! changes something, and reads one wait.

use super::*;
use std::time::Instant;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// A wait long enough for an event already queued, short enough to keep a
/// "nothing happened" test quick.
const SETTLE: Duration = Duration::from_millis(300);

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rinch-font-watch-2537-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn join(&self, path: &str) -> PathBuf {
        self.0.join(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn paths(fonts: &[&PathBuf], config_dirs: &[&PathBuf], files: &[&PathBuf]) -> SystemFontPaths {
    let owned = |list: &[&PathBuf]| list.iter().map(|p| (*p).clone()).collect();
    let mut paths = SystemFontPaths::default();
    paths.font_dirs = owned(fonts);
    paths.config_dirs = owned(config_dirs);
    paths.config_files = owned(files);
    paths
}

fn armed(paths: &SystemFontPaths) -> Watch {
    let mut watch = Watch::new().expect("inotify and an eventfd");
    watch.arm(paths);
    watch
}

/// The waits until one is not quiet, or `SETTLE` has passed: a burst of
/// events can arrive over more than one read.
fn next(watch: &mut Watch) -> Wait {
    let deadline = Instant::now() + SETTLE;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Wait::Quiet;
        }
        match watch.wait(Some(deadline - now)) {
            Wait::Quiet => continue,
            seen => return seen,
        }
    }
}

#[test]
fn a_font_installed_in_a_font_directory_is_a_change_and_nothing_is_none() {
    let dir = Dir::new("install");
    let fonts = dir.join("fonts");
    std::fs::create_dir(&fonts).unwrap();
    let mut watch = armed(&paths(&[&fonts], &[], &[]));

    // Nothing changing: the wait sleeps out its time and sees nothing.
    let start = Instant::now();
    assert_eq!(watch.wait(Some(SETTLE)), Wait::Quiet);
    assert!(start.elapsed() >= SETTLE, "it slept in the kernel");

    std::fs::write(fonts.join("probe.ttf"), FACE).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed);
    // The rest of that write's events, then nothing again.
    while next(&mut watch) != Wait::Quiet {}
    assert_eq!(watch.wait(Some(SETTLE)), Wait::Quiet);
}

#[test]
fn a_new_subdirectory_is_a_change_and_is_watched_once_armed_again() {
    let dir = Dir::new("subdir");
    let fonts = dir.join("fonts");
    std::fs::create_dir(&fonts).unwrap();
    let mut watch = armed(&paths(&[&fonts], &[], &[]));
    let sub = fonts.join("sub");
    std::fs::create_dir(&sub).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed);

    // The rescan lists the new directory; armed with it, a font there is seen.
    watch.arm(&paths(&[&fonts, &sub], &[], &[]));
    std::fs::write(sub.join("probe.ttf"), FACE).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed);
}

#[test]
fn fontconfig_s_own_uuid_file_is_no_change() {
    let dir = Dir::new("uuid");
    let fonts = dir.join("fonts");
    std::fs::create_dir(&fonts).unwrap();
    let mut watch = armed(&paths(&[&fonts], &[], &[]));
    std::fs::write(fonts.join(".uuid"), b"0f5b3c8e-0000-4000-8000-000000000000").unwrap();
    assert_eq!(next(&mut watch), Wait::Quiet);
}

#[test]
fn a_configured_directory_that_does_not_exist_is_seen_when_it_is_made() {
    let dir = Dir::new("missing");
    // Two levels missing: the nearest existing parent is watched for the
    // first, and once that is made the watch, armed again, waits for the next.
    let fonts = dir.join("share/fonts");
    let mut watch = armed(&paths(&[&fonts], &[], &[]));
    std::fs::create_dir(dir.join("share")).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed);
    watch.arm(&paths(&[&fonts], &[], &[]));
    std::fs::write(dir.join("share/unrelated"), b"x").unwrap();
    assert_eq!(
        next(&mut watch),
        Wait::Quiet,
        "only the awaited name counts"
    );
    std::fs::create_dir(&fonts).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed);
}

#[test]
fn a_font_directory_removed_and_made_again_is_followed() {
    let dir = Dir::new("again");
    let fonts = dir.join("fonts");
    std::fs::create_dir(&fonts).unwrap();
    let listed = paths(&[&fonts], &[], &[]);
    let mut watch = armed(&listed);

    std::fs::remove_dir(&fonts).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed, "its removal");
    // Armed again from the same list: now missing, it is awaited.
    watch.arm(&listed);
    std::fs::create_dir(&fonts).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed, "its making again");
    watch.arm(&listed);
    std::fs::write(fonts.join("probe.ttf"), FACE).unwrap();
    assert_eq!(
        next(&mut watch),
        Wait::Changed,
        "a font in the new directory"
    );
}

#[test]
fn a_configuration_file_edited_or_replaced_is_a_change_its_neighbours_are_not() {
    let dir = Dir::new("conf");
    let conf = dir.join("fonts.conf");
    std::fs::write(&conf, b"<fontconfig/>").unwrap();
    let mut watch = armed(&paths(&[], &[], &[&conf]));

    std::fs::write(dir.join("notes.txt"), b"x").unwrap();
    assert_eq!(next(&mut watch), Wait::Quiet, "another file beside it");

    std::fs::write(&conf, b"<fontconfig><dir>/x</dir></fontconfig>").unwrap();
    assert_eq!(next(&mut watch), Wait::Changed, "written in place");

    // An editor's save: a new file renamed over the old one.
    let tmp = dir.join("fonts.conf.tmp");
    std::fs::write(&tmp, b"<fontconfig/>").unwrap();
    assert_eq!(next(&mut watch), Wait::Quiet, "the temporary file");
    std::fs::rename(&tmp, &conf).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed, "renamed over it");
}

#[test]
fn a_file_added_to_an_included_configuration_directory_is_a_change() {
    let dir = Dir::new("confd");
    let conf_d = dir.join("conf.d");
    std::fs::create_dir(&conf_d).unwrap();
    let mut watch = armed(&paths(&[], &[&conf_d], &[]));
    std::fs::write(conf_d.join("50-user.conf"), b"<fontconfig/>").unwrap();
    assert_eq!(next(&mut watch), Wait::Changed);
}

#[test]
fn a_directory_no_longer_listed_is_no_longer_watched() {
    let dir = Dir::new("unlisted");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut watch = armed(&paths(&[&a, &b], &[], &[]));
    watch.arm(&paths(&[&b], &[], &[]));
    // The removed watch's own notice is no change.
    assert_eq!(next(&mut watch), Wait::Quiet);
    std::fs::write(a.join("probe.ttf"), FACE).unwrap();
    assert_eq!(next(&mut watch), Wait::Quiet);
    std::fs::write(b.join("probe.ttf"), FACE).unwrap();
    assert_eq!(next(&mut watch), Wait::Changed);
}

#[test]
fn a_notice_wakes_the_wait_and_says_so() {
    let dir = Dir::new("wake");
    let mut watch = armed(&paths(&[&dir.0], &[], &[]));
    let wake = watch.waker();
    let notifier = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        wake.notify();
    });
    let start = Instant::now();
    assert_eq!(watch.wait(None), Wait::Woken);
    assert!(start.elapsed() < Duration::from_secs(5));
    notifier.join().unwrap();
    assert_eq!(
        watch.wait(Some(Duration::from_millis(50))),
        Wait::Quiet,
        "read once"
    );
}

/// A directory wanted under two paths (a link) is one descriptor. Whichever
/// path is armed last, the descriptor keeps every event either wanted: as a
/// font directory under one and the awaited parent of a missing directory
/// under the other, a file removed in it is still seen.
#[test]
fn a_directory_wanted_through_a_link_keeps_every_event_in_either_order() {
    for fonts_first in [true, false] {
        let dir = Dir::new(if fonts_first { "link-a" } else { "link-b" });
        let real = dir.join("real");
        let link = dir.join("link");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        std::fs::write(real.join("old.ttf"), FACE).unwrap();
        let as_fonts = (real.clone(), (ENTRIES | ITSELF, vec![Role::Fonts]));
        let as_parent = (
            link.clone(),
            (APPEARING | ITSELF, vec![Role::Awaited("missing".into())]),
        );
        let order = if fonts_first {
            vec![as_fonts, as_parent]
        } else {
            vec![as_parent, as_fonts]
        };
        let mut watch = Watch::new().expect("inotify and an eventfd");
        assert_eq!(watch.arm_dirs(order).new, 1, "one descriptor");
        std::fs::remove_file(real.join("old.ttf")).unwrap();
        assert_eq!(
            next(&mut watch),
            Wait::Changed,
            "a removal, fonts first: {fonts_first}"
        );
    }
}

#[test]
fn a_directory_watched_for_the_first_time_is_scanned_once_more_after_its_watch() {
    let dir = Dir::new("rescan");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut watch = Watch::new().expect("inotify and an eventfd");
    let both = paths(&[&a, &b], &[], &[]);
    let mut rescans = 0;
    let mut rescan = || {
        rescans += 1;
        both.clone()
    };

    // The start-up arm: new directories, but no second scan of every start.
    let first = paths(&[&a], &[], &[]);
    assert_eq!(arm_after_scan(&mut watch, &first, true, &mut rescan), 0);
    // A rescan listed `b`, new: once more, after its watch began.
    assert_eq!(arm_after_scan(&mut watch, &both, false, &mut rescan), 1);
    // Nothing new: no extra scan.
    assert_eq!(arm_after_scan(&mut watch, &both, false, &mut rescan), 0);
    assert_eq!(rescans, 1);
}
