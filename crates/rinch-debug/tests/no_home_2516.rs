//! Issue #2516: with no usable home directory the server does not start and
//! nothing is written to the current directory.

#[test]
fn without_a_home_directory_the_server_does_not_start() {
    let cwd = std::env::temp_dir().join(format!("rinch-2516-cwd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cwd);
    std::fs::create_dir_all(&cwd).unwrap();
    // SAFETY: this binary runs one test; nothing else reads the environment.
    unsafe {
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        std::env::remove_var("RINCH_DEBUG_PORT");
    }
    std::env::set_current_dir(&cwd).unwrap();
    assert!(rinch_debug::discovery::discovery_dir().is_none());
    assert!(rinch_debug::attach("probe-2516", || {}).is_err());

    // A relative HOME is no better: it would land under the current directory.
    unsafe { std::env::set_var("HOME", "relative/home") };
    assert!(rinch_debug::discovery::discovery_dir().is_none());
    assert!(rinch_debug::attach("probe-2516", || {}).is_err());

    let left: Vec<_> = std::fs::read_dir(&cwd).unwrap().collect();
    let _ = std::fs::remove_dir_all(&cwd);
    assert!(left.is_empty(), "files written to the current directory");
}
