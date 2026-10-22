//! One call that turns on logging on every target (issue #1773).
//!
//! rinch logs through `tracing`. Each app used to wire its own output: an
//! `env_logger` or `tracing-subscriber` natively, `console_log` and
//! `console_error_panic_hook` in the browser. [`init_logging`] does both, and
//! adds no crate to a web build: the browser's output is the small subscriber
//! in this file, not `tracing-subscriber`.

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
/// idempotent: a second call changes nothing.
///
/// The `tracing` subscriber and the `log` logger are two global slots, each
/// set only if empty. If the app installed a `log` logger first
/// (`env_logger`, `console_log`), `init_logging` still installs its `tracing`
/// subscriber, but cannot route `log` records to it: they keep going to the
/// app's logger, and `log`'s max level stays what the app set. If something
/// installed a `tracing` subscriber first, that one keeps the `tracing` events
/// (and, if it bridged `log`, the `log` records); `init_logging` adds nothing.
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

/// [`init_logging`] with another default filter: a level and `target=level`
/// directives, `tracing`'s `EnvFilter` syntax (`"debug"`,
/// `"warn,my_app=debug"`). Natively `RUST_LOG` still wins when it is set; the
/// browser has no environment, so there it is the filter. The desktop and
/// Android builds read the whole `EnvFilter` syntax; the browser (and a native
/// build without the desktop shell) reads levels and targets only, the
/// longest matching target winning. On Android its most verbose level is the
/// logcat level.
pub fn init_logging_with_filter(filter: &str) {
    imp::init(filter);
}

// Native desktop/Android builds already carry tracing-subscriber (the desktop
// shell installs one); the full `EnvFilter` syntax comes with it.
#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "desktop", feature = "android"),
    not(all(target_os = "android", feature = "android"))
))]
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
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(super::simple::Filter::parse(filter).max_log_level())
                .with_tag("rinch"),
        );
    }
}

// The browser, and native builds without the desktop shell: the small
// subscriber below, so a web app's dependency tree gains nothing.
#[cfg(any(
    target_arch = "wasm32",
    not(any(feature = "desktop", feature = "android"))
))]
mod imp {
    pub(super) fn init(filter: &str) {
        #[cfg(target_arch = "wasm32")]
        super::simple::install_panic_hook();
        let filter = if cfg!(target_arch = "wasm32") {
            filter.to_string()
        } else {
            std::env::var("RUST_LOG").unwrap_or_else(|_| filter.to_string())
        };
        super::simple::install(super::simple::Filter::parse(&filter));
    }
}

/// A `tracing` subscriber and `log` logger that print one line per event —
/// to the browser console by level on wasm32, to stderr natively. The filter is
/// `EnvFilter`'s common subset: a default level and `target=level` directives
/// (`"warn,my_app=debug"`), the longest matching target winning.
#[cfg_attr(
    all(
        not(target_arch = "wasm32"),
        any(feature = "desktop", feature = "android")
    ),
    allow(dead_code)
)]
mod simple {
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tracing::field::{Field, Visit};
    use tracing::level_filters::LevelFilter;
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Level, Metadata, Subscriber};

    #[derive(Clone, Debug, PartialEq)]
    pub(super) struct Filter {
        default: LevelFilter,
        targets: Vec<(String, LevelFilter)>,
    }

    fn level(word: &str) -> Option<LevelFilter> {
        Some(match word.trim().to_ascii_lowercase().as_str() {
            "off" => LevelFilter::OFF,
            "error" => LevelFilter::ERROR,
            "warn" => LevelFilter::WARN,
            "info" => LevelFilter::INFO,
            "debug" => LevelFilter::DEBUG,
            "trace" => LevelFilter::TRACE,
            _ => return None,
        })
    }

    impl Filter {
        /// Parse `"level,target=level,…"`. A part it cannot read is skipped; with
        /// no default level given the default is `info`.
        pub(super) fn parse(spec: &str) -> Self {
            let mut filter = Filter {
                default: LevelFilter::INFO,
                targets: Vec::new(),
            };
            for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                match part.split_once('=') {
                    Some((target, lvl)) => {
                        if let Some(lvl) = level(lvl) {
                            filter.targets.push((target.trim().to_string(), lvl));
                        }
                    }
                    None => {
                        if let Some(lvl) = level(part) {
                            filter.default = lvl;
                        } else {
                            // A bare target turns everything on for it.
                            filter.targets.push((part.to_string(), LevelFilter::TRACE));
                        }
                    }
                }
            }
            filter
        }

        /// The level in force for `target`: the longest directive naming it or
        /// a module above it, else the default.
        pub(super) fn level_for(&self, target: &str) -> LevelFilter {
            self.targets
                .iter()
                .filter(|(t, _)| {
                    target == t
                        || (target.starts_with(t.as_str()) && target[t.len()..].starts_with("::"))
                })
                .max_by_key(|(t, _)| t.len())
                .map_or(self.default, |(_, l)| *l)
        }

        fn max(&self) -> LevelFilter {
            self.targets
                .iter()
                .map(|(_, l)| *l)
                .fold(self.default, std::cmp::max)
        }

        pub(super) fn max_log_level(&self) -> log::LevelFilter {
            to_log(self.max())
        }
    }

    fn to_log(l: LevelFilter) -> log::LevelFilter {
        match l.into_level() {
            None => log::LevelFilter::Off,
            Some(Level::ERROR) => log::LevelFilter::Error,
            Some(Level::WARN) => log::LevelFilter::Warn,
            Some(Level::INFO) => log::LevelFilter::Info,
            Some(Level::DEBUG) => log::LevelFilter::Debug,
            Some(Level::TRACE) => log::LevelFilter::Trace,
        }
    }

    fn from_log(l: log::Level) -> Level {
        match l {
            log::Level::Error => Level::ERROR,
            log::Level::Warn => Level::WARN,
            log::Level::Info => Level::INFO,
            log::Level::Debug => Level::DEBUG,
            log::Level::Trace => Level::TRACE,
        }
    }

    /// Write one line at `level`.
    fn emit(level: Level, line: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            let line = wasm_bindgen::JsValue::from_str(line);
            match level {
                Level::ERROR => web_sys::console::error_1(&line),
                Level::WARN => web_sys::console::warn_1(&line),
                Level::INFO => web_sys::console::info_1(&line),
                Level::DEBUG => web_sys::console::log_1(&line),
                Level::TRACE => web_sys::console::debug_1(&line),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = level;
            eprintln!("{line}");
        }
    }

    /// `LEVEL target: message key=value …`
    pub(super) fn line(level: Level, target: &str, message: &str, fields: &str) -> String {
        let mut out = format!("{level:>5} {target}: {message}");
        if !fields.is_empty() {
            out.push(' ');
            out.push_str(fields);
        }
        out
    }

    #[derive(Default)]
    struct Fields {
        message: String,
        rest: String,
    }

    impl Visit for Fields {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.message.push_str(value);
            } else {
                self.record_debug(field, &value);
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                let _ = write!(self.message, "{value:?}");
            } else {
                if !self.rest.is_empty() {
                    self.rest.push(' ');
                }
                let _ = write!(self.rest, "{}={value:?}", field.name());
            }
        }
    }

    struct Simple {
        filter: Filter,
        next_span: AtomicU64,
    }

    impl Subscriber for Simple {
        fn enabled(&self, meta: &Metadata<'_>) -> bool {
            *meta.level() <= self.filter.level_for(meta.target())
        }

        fn max_level_hint(&self) -> Option<LevelFilter> {
            Some(self.filter.max())
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed))
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let meta = event.metadata();
            let mut fields = Fields::default();
            event.record(&mut fields);
            emit(
                *meta.level(),
                &line(*meta.level(), meta.target(), &fields.message, &fields.rest),
            );
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    struct LogBridge(Filter);

    impl log::Log for LogBridge {
        fn enabled(&self, meta: &log::Metadata<'_>) -> bool {
            from_log(meta.level()) <= self.0.level_for(meta.target())
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                let level = from_log(record.level());
                emit(
                    level,
                    &line(level, record.target(), &record.args().to_string(), ""),
                );
            }
        }

        fn flush(&self) {}
    }

    /// Install the subscriber and the `log` logger, each unless one is set.
    /// Only the first call does anything.
    pub(super) fn install(filter: Filter) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            // Leaked once: a logger lives for the program (`set_boxed_logger`
            // needs `log`'s `std` feature, which a web build may not have).
            if log::set_logger(Box::leak(Box::new(LogBridge(filter.clone())))).is_ok() {
                log::set_max_level(filter.max_log_level());
            }
            let _ = tracing::subscriber::set_global_default(Simple {
                filter,
                next_span: AtomicU64::new(1),
            });
        });
    }

    /// Send a panic's message and the JavaScript stack to `console.error`,
    /// then run whatever hook was there before (a test harness's, say).
    #[cfg(target_arch = "wasm32")]
    pub(super) fn install_panic_hook() {
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_filter_reads_a_default_and_per_target_levels() {
            let f = Filter::parse("warn, my_app=debug, my_app::net=error");
            assert_eq!(f.level_for("other"), LevelFilter::WARN);
            assert_eq!(f.level_for("my_app"), LevelFilter::DEBUG);
            assert_eq!(f.level_for("my_app::ui"), LevelFilter::DEBUG);
            assert_eq!(
                f.level_for("my_app::net::tcp"),
                LevelFilter::ERROR,
                "the longest matching target wins"
            );
            assert_eq!(
                f.level_for("my_application"),
                LevelFilter::WARN,
                "a target matches at a module boundary, not by prefix"
            );
            assert_eq!(f.max_log_level(), log::LevelFilter::Debug);
        }

        #[test]
        fn an_unreadable_filter_falls_back_to_info() {
            let f = Filter::parse("loud");
            assert_eq!(f.level_for("x"), LevelFilter::INFO);
            assert_eq!(f.level_for("loud::y"), LevelFilter::TRACE);
            assert_eq!(Filter::parse("").level_for("x"), LevelFilter::INFO);
        }

        #[test]
        fn a_line_names_level_target_message_and_fields() {
            assert_eq!(
                line(Level::WARN, "app::disk", "nearly full", "free=12"),
                " WARN app::disk: nearly full free=12"
            );
            assert_eq!(line(Level::ERROR, "a", "b", ""), "ERROR a: b");
        }
    }
}
