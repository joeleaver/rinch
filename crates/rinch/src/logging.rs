//! One call that turns on logging on every target (issue #1773).
//!
//! rinch logs through `tracing`. Each app used to wire its own output: an
//! `env_logger` or `tracing-subscriber` natively, `console_log` and
//! `console_error_panic_hook` in the browser. [`init_logging`] does both.

/// Turn on logging for this target, at level `info` unless told otherwise:
///
/// - **Native:** `tracing` events and `log` records are printed to stderr. The
///   `RUST_LOG` environment variable, when set, picks what is shown
///   (`RUST_LOG=debug`, `RUST_LOG=my_app=trace,rinch=warn`).
/// - **Web:** `tracing` events and `log` records go to the browser console,
///   each at its own level (`console.error`, `.warn`, `.info`, `.log` for
///   debug, `.debug` for trace). A panic is written to `console.error` with
///   its message and the JavaScript stack before the module traps — without
///   this a wasm panic shows only "unreachable executed".
/// - **Android:** `log` records go to logcat under the tag `rinch`, as the
///   Android shell already arranges.
///
/// Call it first thing in `main` (or the wasm start function). It is
/// idempotent: a later call, or a logger set up by anything else first, is
/// left as it is.
///
/// ```ignore
/// fn main() {
///     rinch::init_logging();
///     App::new(app).run();
/// }
/// ```
pub fn init_logging() {
    init_logging_with_filter("info");
}

/// [`init_logging`] with another default filter, in `tracing`'s `EnvFilter`
/// syntax (`"debug"`, `"warn,my_app=debug"`). Natively `RUST_LOG` still wins
/// when it is set; the browser has no environment, so there it is the filter.
/// On Android only its level is used.
pub fn init_logging_with_filter(filter: &str) {
    imp::init(filter);
}

#[cfg(all(not(target_arch = "wasm32"), not(all(target_os = "android", feature = "android"))))]
mod imp {
    use tracing_subscriber::EnvFilter;

    pub(super) fn init(filter: &str) {
        let filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(filter))
            .unwrap_or_else(|_| EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
    }
}

#[cfg(all(target_os = "android", feature = "android"))]
mod imp {
    pub(super) fn init(filter: &str) {
        let level = match filter.trim().to_ascii_lowercase().as_str() {
            "error" => log::LevelFilter::Error,
            "warn" => log::LevelFilter::Warn,
            "debug" => log::LevelFilter::Debug,
            "trace" => log::LevelFilter::Trace,
            "off" => log::LevelFilter::Off,
            _ => log::LevelFilter::Info,
        };
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(level)
                .with_tag("rinch"),
        );
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::io;
    use tracing::{Level, Metadata};
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::MakeWriter;

    pub(super) fn init(filter: &str) {
        install_panic_hook();
        let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            // `std::time::SystemTime::now` panics on wasm32-unknown-unknown,
            // and the console stamps its own time.
            .without_time()
            .with_writer(Console)
            .try_init();
    }

    /// Writes each event to the console method of its level.
    struct Console;

    impl<'a> MakeWriter<'a> for Console {
        type Writer = ConsoleLine;

        fn make_writer(&'a self) -> ConsoleLine {
            ConsoleLine {
                level: Level::INFO,
                buf: Vec::new(),
            }
        }

        fn make_writer_for(&'a self, meta: &Metadata<'_>) -> ConsoleLine {
            ConsoleLine {
                level: *meta.level(),
                buf: Vec::new(),
            }
        }
    }

    /// One formatted event, sent to the console when the formatter drops it.
    pub(super) struct ConsoleLine {
        level: Level,
        buf: Vec<u8>,
    }

    impl io::Write for ConsoleLine {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.buf.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for ConsoleLine {
        fn drop(&mut self) {
            if self.buf.is_empty() {
                return;
            }
            let text = String::from_utf8_lossy(&self.buf);
            let line = wasm_bindgen::JsValue::from_str(text.trim_end());
            match self.level {
                Level::ERROR => web_sys::console::error_1(&line),
                Level::WARN => web_sys::console::warn_1(&line),
                Level::INFO => web_sys::console::info_1(&line),
                Level::DEBUG => web_sys::console::log_1(&line),
                Level::TRACE => web_sys::console::debug_1(&line),
            }
        }
    }

    /// Send a panic's message and the JavaScript stack to `console.error`,
    /// then run whatever hook was there before (a test harness's, say).
    fn install_panic_hook() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let error = js_sys::Error::new("");
                let stack = js_sys::Reflect::get(&error, &"stack".into())
                    .ok()
                    .and_then(|s| s.as_string())
                    .unwrap_or_default();
                web_sys::console::error_1(&format!("{info}\n\nStack:\n\n{stack}").into());
                previous(info);
            }));
        });
    }
}
