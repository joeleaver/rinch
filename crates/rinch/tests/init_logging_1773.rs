//! `rinch::init_logging` natively (issue #1773). Its own test binary: it sets
//! the process's global subscriber, which no other test may see.
#![cfg(not(target_arch = "wasm32"))]

use tracing::level_filters::LevelFilter;

#[test]
fn init_logging_installs_a_subscriber_at_info_and_is_idempotent() {
    // SAFETY: this binary runs one test; nothing else reads the environment.
    unsafe { std::env::remove_var("RUST_LOG") };
    assert!(
        !tracing::dispatcher::has_been_set(),
        "nothing set a subscriber before the call"
    );

    rinch::init_logging_with_filter("debug");
    assert!(tracing::dispatcher::has_been_set());
    assert_eq!(
        LevelFilter::current(),
        LevelFilter::DEBUG,
        "the filter given is the one in force"
    );

    // A second call leaves the first subscriber alone.
    rinch::init_logging();
    assert_eq!(LevelFilter::current(), LevelFilter::DEBUG);
}

/// Run this binary again as `child_<mode>` and return its stderr. The global
/// logger and subscriber can be set once per process, so each case is a
/// process of its own.
fn run_child(mode: &str) -> String {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            &format!("child_{mode}"),
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("RINCH_1773_CHILD", mode)
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert!(out.status.success(), "child {mode} failed: {out:?}");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn emit_both() {
    log::info!("log-record-1773");
    tracing::info!("tracing-event-1773");
}

#[test]
fn child_plain() {
    if std::env::var("RINCH_1773_CHILD").as_deref() != Ok("plain") {
        return;
    }
    rinch::init_logging();
    emit_both();
}

/// An app logger installed before `init_logging`.
struct AppLogger;

impl log::Log for AppLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        eprintln!("APP-LOGGER {}", record.args());
    }
    fn flush(&self) {}
}

#[test]
fn child_preset() {
    if std::env::var("RINCH_1773_CHILD").as_deref() != Ok("preset") {
        return;
    }
    log::set_logger(&AppLogger).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    rinch::init_logging();
    emit_both();
}

/// A `log::info!` record reaches `init_logging`'s output, as a `tracing` event
/// does.
#[test]
fn log_records_and_tracing_events_both_reach_stderr() {
    let err = run_child("plain");
    assert!(err.contains("log-record-1773"), "{err}");
    assert!(err.contains("tracing-event-1773"), "{err}");
}

/// With a `log` logger already installed, `init_logging` still installs its
/// `tracing` subscriber (tracing events print as before) but cannot bridge
/// `log`: the app's logger keeps the `log` records.
#[test]
fn an_app_logger_set_first_keeps_the_log_records() {
    let err = run_child("preset");
    assert!(err.contains("APP-LOGGER log-record-1773"), "{err}");
    assert!(
        err.lines()
            .any(|l| l.contains("tracing-event-1773") && !l.contains("APP-LOGGER")),
        "the tracing event is printed by rinch's subscriber: {err}"
    );
}
