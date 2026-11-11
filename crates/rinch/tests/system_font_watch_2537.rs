//! The font watch sees a font installed while the app runs, by the kernel's
//! notice, from the paths of rinch's own scan (#2537).
//!
//! fontconfig runs on a configuration of its own: `FONTCONFIG_FILE` names one
//! whose only font directory is a temporary one, so nothing of the host's is
//! read or watched. It is its own test binary so the variable is set before
//! fontconfig first loads. Linux only: elsewhere nothing watches.

#![cfg(all(feature = "desktop", target_os = "linux"))]

use std::time::{Duration, Instant};

const INSTALLED: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Until `done`, or panic after `limit`.
fn until(limit: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_font_installed_while_the_watch_runs_is_scanned_once() {
    let dir = std::env::temp_dir().join(format!("rinch-fc-2537-{}", std::process::id()));
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    let conf = dir.join("fonts.conf");
    std::fs::write(
        &conf,
        format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n\
             <fontconfig><dir>{}</dir><cachedir>{}</cachedir></fontconfig>\n",
            fonts.display(),
            dir.join("cache").display()
        ),
    )
    .unwrap();
    // SAFETY: the only test in this binary, before any thread or fontconfig
    // call reads the environment.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &conf) };

    // The watch's paths are the scan's: this configuration's file and its
    // one font directory.
    let paths = rinch_dom::fonts::system_font_paths();
    if !paths.config_files.contains(&conf) {
        // No libfontconfig on this host: nothing to watch.
        eprintln!("libfontconfig not found; skipped");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(paths.font_dirs.contains(&fonts), "{paths:?}");
    let scans = rinch_dom::fonts::system_font_scans();
    assert_eq!(scans, 1, "the paths came from the one scan");

    rinch::font::watch_system_fonts(Some(Duration::from_millis(50)));
    // Nothing changing: no rescan.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rinch_dom::fonts::system_font_scans(), scans);

    let generation = rinch_dom::fonts::system_fonts_generation();
    // Installed as a package manager does: written beside the directory and
    // renamed in, one event. (Written in place, its close can come after a
    // first rescan under load, and count as a second change.)
    let staged = dir.join("Inter-Regular.ttf.partial");
    std::fs::write(&staged, INSTALLED).unwrap();
    std::fs::rename(&staged, fonts.join("Inter-Regular.ttf")).unwrap();
    until(
        Duration::from_secs(30),
        "the install was never seen",
        || rinch_dom::fonts::system_fonts_generation() > generation,
    );
    // The burst of events from one install is one rescan.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rinch_dom::fonts::system_font_scans(), scans + 1);
    let mut collection = rinch_dom::fonts::new_font_context().collection;
    assert!(
        collection.family_by_name("Inter").is_some(),
        "the new scan has the installed face"
    );

    // Stopped, the watch rescans no more.
    rinch::font::watch_system_fonts(None);
    std::thread::sleep(Duration::from_millis(200));
    std::fs::remove_file(fonts.join("Inter-Regular.ttf")).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rinch_dom::fonts::system_font_scans(), scans + 1);

    let _ = std::fs::remove_dir_all(&dir);
}
