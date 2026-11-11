//! Socket-directory resolution inside a temporary directory: never the real
//! runtime directory.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use super::*;

/// This user's uid, which the temporary directories belong to.
fn uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Inputs with no runtime directory and the fallback under `root`.
fn inputs(root: &Path) -> Locations {
    Locations {
        xdg_runtime_dir: None,
        tmpdir: None,
        prefer_tmpdir: false,
        fallback_root: root.to_path_buf(),
        uid: uid(),
    }
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn takes_the_runtime_directory_when_it_is_ours() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("run");
    private_dir(&runtime);
    let locations = Locations {
        xdg_runtime_dir: Some(runtime.clone()),
        ..inputs(root.path())
    };
    let dir = locations.create_socket_dir().unwrap();
    assert_eq!(dir, runtime.join("rinch/debug"));
    // Both directories it made are owner-only.
    assert_eq!(mode(&runtime.join("rinch")), 0o700);
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(locations.existing_socket_dir(), Some(dir));
}

#[test]
fn passes_over_a_runtime_directory_that_is_not_safe() {
    let root = tempfile::tempdir().unwrap();
    let open = root.path().join("open");
    fs::create_dir(&open).unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).unwrap();
    let file = root.path().join("file");
    fs::write(&file, "").unwrap();
    let link = root.path().join("link");
    private_dir(&root.path().join("target"));
    symlink(root.path().join("target"), &link).unwrap();
    for runtime in [open, file, link, "relative".into()] {
        let locations = Locations {
            xdg_runtime_dir: Some(runtime.clone()),
            ..inputs(root.path())
        };
        let fallback = root.path().join(format!("rinch-{}", uid()));
        assert_eq!(locations.runtime_dir().unwrap(), fallback, "{runtime:?}");
    }
}

#[test]
fn uses_tmpdir_only_where_it_is_per_user() {
    let root = tempfile::tempdir().unwrap();
    let tmpdir = root.path().join("T");
    private_dir(&tmpdir);
    let apple = Locations {
        tmpdir: Some(tmpdir.clone()),
        prefer_tmpdir: true,
        ..inputs(root.path())
    };
    assert_eq!(apple.runtime_dir().unwrap(), tmpdir);
    let elsewhere = Locations {
        prefer_tmpdir: false,
        ..apple
    };
    assert_eq!(
        elsewhere.runtime_dir().unwrap(),
        root.path().join(format!("rinch-{}", uid()))
    );
}

#[test]
fn creates_the_fallback_owner_only() {
    let root = tempfile::tempdir().unwrap();
    let dir = inputs(root.path()).create_socket_dir().unwrap();
    let fallback = root.path().join(format!("rinch-{}", uid()));
    assert_eq!(dir, fallback.join("rinch/debug"));
    for made in [&fallback, &fallback.join("rinch"), &dir] {
        assert_eq!(mode(made), 0o700, "{made:?}");
    }
}

#[test]
fn refuses_a_fallback_that_is_not_ours_alone() {
    let root = tempfile::tempdir().unwrap();
    let fallback = root.path().join(format!("rinch-{}", uid()));
    fs::create_dir(&fallback).unwrap();
    fs::set_permissions(&fallback, fs::Permissions::from_mode(0o755)).unwrap();
    let error = inputs(root.path()).create_socket_dir().unwrap_err();
    assert!(error.to_string().ends_with("is not mode 0700"), "{error}");

    // A link where the fallback should be is refused, not followed.
    let other = tempfile::tempdir().unwrap();
    let linked = other.path().join(format!("rinch-{}", uid()));
    private_dir(&other.path().join("elsewhere"));
    symlink(other.path().join("elsewhere"), &linked).unwrap();
    let error = inputs(other.path()).create_socket_dir().unwrap_err();
    assert!(error.to_string().ends_with("is a symbolic link"), "{error}");
}

#[test]
fn refuses_a_debug_directory_someone_opened() {
    let root = tempfile::tempdir().unwrap();
    let locations = inputs(root.path());
    let dir = locations.create_socket_dir().unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(locations.create_socket_dir().is_err());
    // A client lists nothing there rather than trust it.
    assert_eq!(locations.existing_socket_dir(), None);
}

#[test]
fn finds_no_directory_a_server_never_made() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(inputs(root.path()).existing_socket_dir(), None);
    // And looking created nothing.
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn reads_its_inputs_from_the_environment() {
    let locations = Locations::from_vars(
        |name| match name {
            "XDG_RUNTIME_DIR" => Some("/run/user/1000".into()),
            "TMPDIR" => Some("/var/folders/x/T".into()),
            _ => None,
        },
        1000,
    );
    assert_eq!(locations.xdg_runtime_dir, Some("/run/user/1000".into()));
    assert_eq!(locations.tmpdir, Some("/var/folders/x/T".into()));
    assert_eq!(locations.fallback_root, Path::new("/tmp"));
    assert_eq!(locations.prefer_tmpdir, cfg!(target_vendor = "apple"));
}
