//! Watching for fonts installed while the app runs (#2486).
//!
//! The process scans the system fonts once and every font context shares that
//! scan (#2467). On Linux this watch asks fontconfig, every interval, whether
//! anything its scan depends on changed (`FcConfigUptoDate`: the newest mtime
//! of its configuration files and font directories against the time the
//! configuration was loaded), and on a change rescans
//! (`rinch_dom::fonts::refresh_system_fonts`) and wakes the event loop, whose
//! next frame adopts the new scan (`RinchApp::adopt_refreshed_system_fonts`).
//!
//! The watch has its own fontconfig configuration, separate from the one
//! fontique scans with, and loads it one interval after it starts, so the
//! start-up pays nothing for it. One thread, started by the first `set` with
//! an interval; `set(None)` lets it end after its current sleep.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

// Read only by the Linux watch thread; elsewhere the setting is kept and unused.
#[cfg_attr(not(all(target_os = "linux", feature = "desktop")), allow(dead_code))]
struct State {
    /// How often to look; `None` stops the thread at its next wake.
    interval: Option<Duration>,
    /// Whether the app has chosen (`set`), so `start_default` leaves it be.
    chosen: bool,
    /// Whether the thread is alive.
    running: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    interval: None,
    chosen: false,
    running: false,
});

/// `rinch::font::watch_system_fonts`.
pub(crate) fn set(interval: Option<Duration>) {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    state.chosen = true;
    apply(&mut state, interval);
}

/// What `App::run` asks for: the default interval, unless the app chose.
#[cfg(feature = "desktop")]
pub(crate) fn start_default() {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    if !state.chosen {
        apply(&mut state, Some(crate::font::DEFAULT_SYSTEM_FONT_WATCH));
    }
}

fn apply(state: &mut State, interval: Option<Duration>) {
    state.interval = interval;
    #[cfg(all(target_os = "linux", feature = "desktop"))]
    if interval.is_some() && !state.running {
        state.running = std::thread::Builder::new()
            .name("rinch-font-watch".into())
            .spawn(run)
            .is_ok();
    }
}

#[cfg(all(target_os = "linux", feature = "desktop"))]
fn run() {
    let mut watch: Option<fontconfig::Watch> = None;
    loop {
        let interval = {
            let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
            match state.interval {
                Some(interval) => interval,
                None => {
                    state.running = false;
                    return;
                }
            }
        };
        std::thread::sleep(interval);
        match &mut watch {
            None => match fontconfig::Watch::new(None) {
                Some(w) => watch = Some(w),
                // No libfontconfig: nothing to watch, and fontique has no
                // system fonts to refresh either.
                None => {
                    STATE.lock().unwrap_or_else(PoisonError::into_inner).running = false;
                    return;
                }
            },
            Some(w) => {
                if w.changed() {
                    rinch_dom::fonts::refresh_system_fonts();
                    rinch_core::dispatch_main_callback(Box::new(|| {}));
                }
            }
        }
    }
}

#[cfg(all(target_os = "linux", feature = "desktop"))]
pub(crate) mod fontconfig {
    //! libfontconfig through `dlopen`, as `text_rasterization` opens it.
    //!
    //! Not `FcConfigUptoDate`: it compares mtimes in whole seconds against
    //! the wall clock of its previous call, so a font installed later in the
    //! same second as a look is never seen. The watch keeps the mtimes
    //! themselves instead, of every file and directory fontconfig names
    //! (its configuration files and every font directory it scanned,
    //! subdirectories included), and a change is any of them moving.

    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    type Config = c_void;
    type StrList = c_void;

    struct Fns {
        init: unsafe extern "C" fn() -> *mut Config,
        create: unsafe extern "C" fn() -> *mut Config,
        parse_and_load: unsafe extern "C" fn(*mut Config, *const u8, c_int) -> c_int,
        build_fonts: unsafe extern "C" fn(*mut Config) -> c_int,
        font_dirs: unsafe extern "C" fn(*mut Config) -> *mut StrList,
        config_files: unsafe extern "C" fn(*mut Config) -> *mut StrList,
        list_next: unsafe extern "C" fn(*mut StrList) -> *const c_char,
        list_done: unsafe extern "C" fn(*mut StrList),
        destroy: unsafe extern "C" fn(*mut Config),
    }

    impl Fns {
        fn load() -> Option<Self> {
            // SAFETY: fontconfig's public API with the C signatures above
            // (fontconfig.h). The library is never unloaded (leaked), as in
            // `text_rasterization`: fontique keeps it in the process anyway.
            unsafe {
                let lib: &'static libloading::Library = Box::leak(Box::new(
                    libloading::Library::new("libfontconfig.so.1").ok()?,
                ));
                Some(Self {
                    init: *lib.get(b"FcInitLoadConfigAndFonts\0").ok()?,
                    create: *lib.get(b"FcConfigCreate\0").ok()?,
                    parse_and_load: *lib.get(b"FcConfigParseAndLoad\0").ok()?,
                    build_fonts: *lib.get(b"FcConfigBuildFonts\0").ok()?,
                    font_dirs: *lib.get(b"FcConfigGetFontDirs\0").ok()?,
                    config_files: *lib.get(b"FcConfigGetConfigFiles\0").ok()?,
                    list_next: *lib.get(b"FcStrListNext\0").ok()?,
                    list_done: *lib.get(b"FcStrListDone\0").ok()?,
                    destroy: *lib.get(b"FcConfigDestroy\0").ok()?,
                })
            }
        }
    }

    /// The paths fontconfig's scan depends on, and their mtimes when it
    /// was loaded.
    pub(crate) struct Watch {
        fns: Fns,
        /// `None`: the default configuration (`FONTCONFIG_FILE` and the
        /// rest, as fontique's scan reads it). `Some`: that file (tests).
        file: Option<CString>,
        paths: Vec<PathBuf>,
        mtimes: Vec<Option<SystemTime>>,
    }

    fn mtimes(paths: &[PathBuf]) -> Vec<Option<SystemTime>> {
        paths
            .iter()
            .map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .collect()
    }

    impl Watch {
        pub(crate) fn new(file: Option<&Path>) -> Option<Self> {
            let file = match file {
                Some(path) => Some(CString::new(path.as_os_str().as_encoded_bytes()).ok()?),
                None => None,
            };
            let mut watch = Self {
                fns: Fns::load()?,
                file,
                paths: Vec::new(),
                mtimes: Vec::new(),
            };
            watch.paths = watch.load()?;
            watch.mtimes = mtimes(&watch.paths);
            Some(watch)
        }

        /// Load the configuration and build its fonts (which brings
        /// fontconfig's caches up to date), and list what it read.
        fn load(&self) -> Option<Vec<PathBuf>> {
            // SAFETY: see `Fns::load`; the configuration and the lists made
            // here are released here.
            unsafe {
                let config = match &self.file {
                    None => (self.fns.init)(),
                    Some(file) => {
                        let config = (self.fns.create)();
                        if !config.is_null()
                            && ((self.fns.parse_and_load)(config, file.as_ptr().cast(), 1) == 0
                                || (self.fns.build_fonts)(config) == 0)
                        {
                            (self.fns.destroy)(config);
                            return None;
                        }
                        config
                    }
                };
                if config.is_null() {
                    return None;
                }
                let mut paths = Vec::new();
                for list in [
                    (self.fns.config_files)(config),
                    (self.fns.font_dirs)(config),
                ] {
                    if list.is_null() {
                        continue;
                    }
                    loop {
                        let next = (self.fns.list_next)(list);
                        if next.is_null() {
                            break;
                        }
                        let bytes = CStr::from_ptr(next).to_bytes();
                        paths.push(PathBuf::from(
                            std::ffi::OsStr::from_encoded_bytes_unchecked(bytes),
                        ));
                    }
                    (self.fns.list_done)(list);
                }
                (self.fns.destroy)(config);
                Some(paths)
            }
        }

        /// Whether a configuration file or font directory changed since the
        /// watch was loaded or last reported a change. A change reloads it
        /// (a new directory is watched from then on), so each is reported
        /// once.
        pub(crate) fn changed(&mut self) -> bool {
            if mtimes(&self.paths) == self.mtimes {
                return false;
            }
            if let Some(paths) = self.load() {
                self.paths = paths;
            }
            // Taken before the caller rescans: a change from here on is the
            // next one, and one before it is in the rescan.
            self.mtimes = mtimes(&self.paths);
            true
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

        /// A font copied into a watched directory, or into a new
        /// subdirectory of one, is a change, reported once; nothing
        /// happening is none. Its own configuration file, so the test reads
        /// no host font and sets no environment variable.
        #[test]
        fn a_font_installed_in_a_font_directory_is_a_change() {
            let dir =
                std::env::temp_dir().join(format!("rinch-fc-watch-2486-{}", std::process::id()));
            let fonts = dir.join("fonts");
            std::fs::create_dir_all(fonts.join("sub")).unwrap();
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
            let Some(mut watch) = Watch::new(Some(&conf)) else {
                // No libfontconfig on this host: nothing to watch.
                eprintln!("libfontconfig not found; skipped");
                return;
            };
            assert!(
                watch.paths.contains(&fonts.join("sub")),
                "the scanned subdirectory is watched: {:?}",
                watch.paths
            );
            assert!(!watch.changed(), "nothing changed yet");

            // At once, in the same second as the look before it: whole-second
            // comparisons (`FcConfigUptoDate`) miss this.
            std::fs::write(fonts.join("sub").join("probe.ttf"), FACE).unwrap();
            assert!(
                watch.changed(),
                "a new font in a font directory is a change"
            );
            assert!(!watch.changed(), "a change is reported once");

            std::fs::create_dir(fonts.join("new")).unwrap();
            assert!(watch.changed(), "a new subdirectory is a change");
            std::fs::write(fonts.join("new").join("probe.ttf"), FACE).unwrap();
            assert!(watch.changed(), "and is watched from then on");

            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
