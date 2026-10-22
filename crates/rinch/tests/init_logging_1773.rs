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
