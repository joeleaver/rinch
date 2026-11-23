//! `take_launch_activation_token` (#2817) takes `XDG_ACTIVATION_TOKEN` out of
//! the process's environment, so a program the app starts does not inherit
//! it. Its own binary, with one test: the test changes the environment,
//! which no other thread may touch meanwhile.
#![cfg(feature = "windowing")]

#[test]
fn the_launch_token_is_taken_once_and_removed_from_the_environment_2817() {
    // SAFETY: the only test in this binary; nothing else reads the
    // environment while it runs.
    unsafe { std::env::set_var("XDG_ACTIVATION_TOKEN", "launcher-31") };
    let token = unsafe { rinch::windows::take_launch_activation_token() };
    assert_eq!(token.as_deref(), Some("launcher-31"));
    assert_eq!(
        std::env::var_os("XDG_ACTIVATION_TOKEN"),
        None,
        "a child would inherit it"
    );
    let again = unsafe { rinch::windows::take_launch_activation_token() };
    assert_eq!(again, None, "taken once");
    // An empty one is no token, and goes too.
    unsafe { std::env::set_var("XDG_ACTIVATION_TOKEN", "") };
    assert_eq!(
        unsafe { rinch::windows::take_launch_activation_token() },
        None
    );
    assert_eq!(std::env::var_os("XDG_ACTIVATION_TOKEN"), None);
}
