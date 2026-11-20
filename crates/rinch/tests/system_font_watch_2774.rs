//! The font watch never makes the start-up scan: started before any font
//! context, it waits for the first one's scan, then watches what that scan
//! read and still sees a font installed afterwards (#2774).
//!
//! Before, a watch that got there first made the scan itself (a whole
//! `fontique::Collection::new`, about 250 M instructions on a desktop host)
//! while the first document waited for it.
//!
//! fontconfig runs on a configuration of its own, as in
//! `system_font_watch_2537.rs`; its own test binary so `FONTCONFIG_FILE` is
//! set before fontconfig first loads. Linux only: elsewhere nothing watches.

#![cfg(all(feature = "windowing", target_os = "linux"))]

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
fn a_watch_started_before_the_first_font_context_waits_for_its_scan() {
    let dir = std::env::temp_dir().join(format!("rinch-fc-2774-{}", std::process::id()));
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

    // Started first, as `App::run` starts it before the window exists.
    rinch::font::watch_system_fonts(Some(Duration::from_millis(50)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        rinch_dom::fonts::system_font_scans(),
        0,
        "the watch made the start-up scan itself"
    );

    // The first document's font context makes the one scan.
    let _ = rinch_dom::fonts::new_font_context();
    assert_eq!(rinch_dom::fonts::system_font_scans(), 1);
    if !rinch_dom::fonts::system_font_paths()
        .config_files
        .contains(&conf)
    {
        // No libfontconfig on this host: nothing to watch.
        eprintln!("libfontconfig not found; skipped");
        rinch::font::watch_system_fonts(None);
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    // The watch arms on that scan's paths without scanning again. (The scan
    // wakes it at once; its 1 s wait only bounds how long a missed wake-up
    // would delay it.)
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(rinch_dom::fonts::system_font_scans(), 1);

    // And sees a font installed after it armed.
    let generation = rinch_dom::fonts::system_fonts_generation();
    let staged = dir.join("Inter-Regular.ttf.partial");
    std::fs::write(&staged, INSTALLED).unwrap();
    std::fs::rename(&staged, fonts.join("Inter-Regular.ttf")).unwrap();
    until(
        Duration::from_secs(30),
        "the install was never seen",
        || rinch_dom::fonts::system_fonts_generation() > generation,
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rinch_dom::fonts::system_font_scans(), 2);
    let mut collection = rinch_dom::fonts::new_font_context().collection;
    assert!(
        collection.family_by_name("Inter").is_some(),
        "the new scan has the installed face"
    );

    rinch::font::watch_system_fonts(None);
    let _ = std::fs::remove_dir_all(&dir);
}
