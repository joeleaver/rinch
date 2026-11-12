//! The app's identity on Linux (its `app_id`, icon and desktop entry), and decoding the window icon.

/// Decode a PNG from raw bytes into RGBA pixel data.
///
/// Returns `(rgba_bytes, width, height)`.
pub(crate) fn decode_png_to_rgba(
    png_data: &[u8],
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    let decoder = png::Decoder::new(png_data);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let bytes = &buf[..info.buffer_size()];

    // Convert to RGBA8 if needed
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(bytes.len() / 3 * 4);
            for chunk in bytes.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => {
            return Err(format!("Unsupported PNG color type: {:?}", other).into());
        }
    };

    Ok((rgba, info.width, info.height))
}

/// Decode a PNG icon from raw bytes into a winit Icon.
pub(super) fn load_window_icon(
    png_data: &[u8],
) -> Result<winit::icon::Icon, Box<dyn std::error::Error>> {
    let (rgba, width, height) = decode_png_to_rgba(png_data)?;
    let rgba_icon = winit::icon::RgbaIcon::new(rgba, width, height)?;
    Ok(rgba_icon.into())
}

/// The app id the last window was created with, for a service that has to
/// name the application to the desktop the same way the window does (the
/// global-shortcuts portal, `crate::hotkey`; notifications' `desktop-entry`
/// hint, `crate::notification`).
#[cfg(target_os = "linux")]
pub(super) static WINDOW_APP_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The app id this application presents to the desktop: the window's, once
/// one has been created, else the one a window would get with no
/// `WindowProps::app_id`.
#[cfg(target_os = "linux")]
#[cfg_attr(
    not(any(feature = "global-hotkeys", feature = "notifications")),
    allow(dead_code)
)]
pub(crate) fn current_app_id() -> String {
    WINDOW_APP_ID
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_else(|| resolved_app_id(None))
}

/// The Wayland `app_id` / X11 `WM_CLASS` this window identifies itself by.
///
/// An app that sets `WindowProps::app_id` gets exactly that. Everything else
/// gets its **own** identity, derived from the executable's file stem, rather
/// than a constant shared with every other rinch application.
///
/// A shared default is not merely a cosmetic label. `install_wayland_icon`
/// writes `~/.local/share/applications/{app_id}.desktop` carrying
/// `Name={app_id}`, and compositors match a window to a desktop entry by
/// `app_id` — so under one constant, the first rinch app to ship an icon
/// supplies the dock icon *and* the displayed name for every other rinch app
/// on the machine, and they group together as a single application.
///
/// Falls back to `"rinch-app"` only when the executable path is unavailable,
/// which is where the old behaviour came from.
#[cfg(target_os = "linux")]
pub(crate) fn resolved_app_id(explicit: Option<&str>) -> String {
    if let Some(id) = explicit {
        return id.to_string();
    }
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(app_id_from_exe_path)
        .unwrap_or_else(|| "rinch-app".to_string())
}

/// The `WindowProps::app_id` the running app declared, recorded when its
/// event loop starts. Read by the tray (`rinch::tray`, #1529) so a tray built
/// after that point reports the same identity as the window; a tray built
/// earlier (the usual order: in `main`, before `App::run`) cannot see it and
/// takes `TrayIconBuilder::with_id` or the executable name instead.
#[cfg(all(target_os = "linux", feature = "system-tray"))]
pub(super) static DECLARED_APP_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// See [`DECLARED_APP_ID`].
#[cfg(all(target_os = "linux", feature = "system-tray"))]
pub(crate) fn declared_app_id() -> Option<&'static str> {
    DECLARED_APP_ID.get().map(String::as_str)
}

/// The identity an executable path implies: its **whole** file name.
///
/// Not `file_stem`. A stem truncates at the last dot, so the reverse-DNS
/// binary names this convention is built for — `com.example.notes` — would
/// register as `com.example` and collide with every sibling in that namespace,
/// which is the exact failure the derivation exists to prevent. Linux
/// executables carry no extension for a stem to usefully strip, and this
/// function is Linux-only.
#[cfg(target_os = "linux")]
fn app_id_from_exe_path(path: &std::path::Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

/// Write the icon PNG to a data directory and create a `.desktop` file so Wayland
/// compositors can display the icon in the taskbar via `app_id` matching.
#[cfg(target_os = "linux")]
pub(super) fn install_wayland_icon(app_id: &str, png_data: &[u8]) {
    // If the app already has a system-wide launcher (e.g. it was installed from
    // a package), don't write a user-level NoDisplay stub of the same name: it
    // would shadow that launcher and hide the app from the application menu. The
    // packaged .desktop already supplies the taskbar icon via app_id/StartupWMClass.
    if system_desktop_entry_exists(app_id) {
        return;
    }

    let Some(data_home) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs_icon_fallback)
    else {
        return;
    };
    install_wayland_icon_in(&data_home, app_id, png_data);
}

/// [`install_wayland_icon`]'s writes, against an explicit data directory.
#[cfg(target_os = "linux")]
fn install_wayland_icon_in(data_home: &std::path::Path, app_id: &str, png_data: &[u8]) {
    // Decode and resize icon to 256x256 for desktop integration
    let icon_dir = data_home.join("rinch/icons");
    if std::fs::create_dir_all(&icon_dir).is_err() {
        return;
    }
    let icon_path = icon_dir.join(format!("{app_id}.png"));
    let resized_png = match resize_png_icon(png_data, 256) {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!("Failed to resize icon: {}", e);
            return;
        }
    };
    if std::fs::write(&icon_path, &resized_png).is_err() {
        return;
    }

    // Write .desktop file with absolute icon path (most reliable across compositors)
    let desktop_dir = data_home.join("applications");
    if std::fs::create_dir_all(&desktop_dir).is_err() {
        return;
    }
    let desktop_path = desktop_dir.join(format!("{app_id}.desktop"));
    // An entry there that is not a stub is the app's own launcher (an installer
    // that writes to the user's data directory, as an app that updates itself
    // must): replacing it with a NoDisplay stub took the app out of the
    // application menu on every start. That entry names the icon itself.
    let Some(carried) = icon_stub_carries(&desktop_path) else {
        return;
    };
    let desktop_content = format!(
        "[Desktop Entry]\nType=Application\nName={app_id}\nIcon={}\nNoDisplay=true\n\
         {carried}{RINCH_STUB_MARKER}\n",
        icon_path.display()
    );
    let _ = std::fs::write(&desktop_path, desktop_content);
}

/// The key every `.desktop` stub rinch writes carries (the icon stub and the
/// global-hotkeys stub): an entry with it is rinch's own to rewrite.
#[cfg(target_os = "linux")]
const RINCH_STUB_MARKER: &str = "X-Rinch-Stub=true";

/// Whether [`install_wayland_icon`] may write its stub at `path`, and the
/// lines of the entry there it must keep: `Some("")` when nothing is there,
/// `Some(lines)` for a stub rinch wrote — one carrying [`RINCH_STUB_MARKER`],
/// whose `Exec=` and `X-Rinch-Generated=` lines are kept (the global-hotkeys
/// stub's `Exec=` is what the portal needs), or an icon stub from before the
/// marker (`NoDisplay=true` and no `Exec=`). `None` for anything else: the
/// app's own launcher, or one the user hid through a menu editor, which
/// carries `NoDisplay=true` beside its own `Exec=` and no marker.
#[cfg(target_os = "linux")]
fn icon_stub_carries(path: &std::path::Path) -> Option<String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(String::new()),
        Err(_) => return None,
    };
    let has = |line: &str| text.lines().any(|l| l.trim() == line);
    let marked = has(RINCH_STUB_MARKER);
    let old_icon_stub =
        has("NoDisplay=true") && !text.lines().any(|l| l.trim_start().starts_with("Exec="));
    if !marked && !old_icon_stub {
        return None;
    }
    Some(
        text.lines()
            .map(str::trim)
            .filter(|l| l.starts_with("Exec=") || l.starts_with("X-Rinch-Generated="))
            .map(|l| format!("{l}\n"))
            .collect(),
    )
}

/// Returns true if a `.desktop` entry for `app_id` exists in any system
/// application directory (`$XDG_DATA_DIRS`, default `/usr/local/share:/usr/share`).
#[cfg(target_os = "linux")]
fn system_desktop_entry_exists(app_id: &str) -> bool {
    desktop_entry_in(&system_data_dirs(), app_id)
}

/// `$XDG_DATA_DIRS`, read as an `OsString` so non-UTF8 directory entries are
/// preserved (`var()` would drop the whole value to the default on any
/// non-UTF8 byte).
#[cfg(target_os = "linux")]
fn system_data_dirs() -> std::ffi::OsString {
    std::env::var_os("XDG_DATA_DIRS")
        .unwrap_or_else(|| std::ffi::OsString::from("/usr/local/share:/usr/share"))
}

/// Whether `applications/{app_id}.desktop` exists under any of `dirs`.
#[cfg(target_os = "linux")]
fn desktop_entry_in(dirs: &std::ffi::OsStr, app_id: &str) -> bool {
    let rel = format!("applications/{app_id}.desktop");
    std::env::split_paths(dirs)
        .filter(|p| !p.as_os_str().is_empty())
        .any(|p| p.join(&rel).exists())
}

/// Write a minimal `NoDisplay` desktop entry for `app_id` in the user's
/// applications directory, unless one exists there or in any system
/// directory — never shadowing a packaged launcher, never overwriting a file.
///
/// For a service that the desktop accepts only from an application with an
/// installed entry: the Wayland global-shortcuts portal (`crate::hotkey`)
/// refuses `Registry.Register` with "App info not found" otherwise. Answers
/// the path written, `Ok(None)` when an entry already existed.
#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "global-hotkeys"), allow(dead_code))]
pub(crate) fn ensure_desktop_entry(app_id: &str) -> Result<Option<std::path::PathBuf>, String> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs_icon_fallback)
        .ok_or("neither XDG_DATA_HOME nor HOME is set")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    ensure_desktop_entry_in(&system_data_dirs(), &data_home, app_id, &exe)
}

/// [`ensure_desktop_entry`] against explicit directories.
#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "global-hotkeys"), allow(dead_code))]
fn ensure_desktop_entry_in(
    system_dirs: &std::ffi::OsStr,
    data_home: &std::path::Path,
    app_id: &str,
    exe: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    if desktop_entry_in(system_dirs, app_id) {
        return Ok(None);
    }
    write_desktop_entry_stub(data_home, app_id, exe)
}

/// [`ensure_desktop_entry`]'s write, against an explicit data directory.
#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "global-hotkeys"), allow(dead_code))]
fn write_desktop_entry_stub(
    data_home: &std::path::Path,
    app_id: &str,
    exe: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    if app_id.is_empty() || app_id.contains(['/', '\n']) {
        return Err(format!("{app_id:?} is not a usable app id"));
    }
    let dir = data_home.join("applications");
    let path = dir.join(format!("{app_id}.desktop"));
    if path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Exec is required for Type=Application; quoted per the desktop entry
    // specification, with its reserved characters escaped.
    let exec = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    let content = format!(
        "[Desktop Entry]\nType=Application\nName={app_id}\nExec=\"{exec}\"\nNoDisplay=true\n\
         X-Rinch-Generated=global-hotkeys\n{RINCH_STUB_MARKER}\n"
    );
    // `create_new`: a file that appeared since the check is not overwritten.
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => f
            .write_all(content.as_bytes())
            .map(|()| Some(path.clone()))
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(target_os = "linux")]
fn dirs_icon_fallback() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
}

/// Decode a PNG, resize to `target_size` x `target_size` with bilinear filtering, re-encode as PNG.
#[cfg(target_os = "linux")]
fn resize_png_icon(
    png_data: &[u8],
    target_size: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Decode source PNG
    let decoder = png::Decoder::new(png_data);
    let mut reader = decoder.read_info()?;
    let mut src_buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut src_buf)?;
    let src = &src_buf[..info.buffer_size()];
    let (sw, sh) = (info.width as usize, info.height as usize);

    // Convert to RGBA if needed
    let rgba_src: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(sw * sh * 4);
            for chunk in src.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => return Err(format!("Unsupported color type: {:?}", other).into()),
    };

    let ts = target_size as usize;
    let mut dst = vec![0u8; ts * ts * 4];

    // Bilinear downscale
    for dy in 0..ts {
        for dx in 0..ts {
            let sx_f = (dx as f64 + 0.5) * sw as f64 / ts as f64 - 0.5;
            let sy_f = (dy as f64 + 0.5) * sh as f64 / ts as f64 - 0.5;
            let x0 = (sx_f.floor() as isize).clamp(0, sw as isize - 1) as usize;
            let y0 = (sy_f.floor() as isize).clamp(0, sh as isize - 1) as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let fx = (sx_f - x0 as f64).clamp(0.0, 1.0);
            let fy = (sy_f - y0 as f64).clamp(0.0, 1.0);

            for c in 0..4 {
                let p00 = rgba_src[(y0 * sw + x0) * 4 + c] as f64;
                let p10 = rgba_src[(y0 * sw + x1) * 4 + c] as f64;
                let p01 = rgba_src[(y1 * sw + x0) * 4 + c] as f64;
                let p11 = rgba_src[(y1 * sw + x1) * 4 + c] as f64;
                let val = p00 * (1.0 - fx) * (1.0 - fy)
                    + p10 * fx * (1.0 - fy)
                    + p01 * (1.0 - fx) * fy
                    + p11 * fx * fy;
                dst[(dy * ts + dx) * 4 + c] = val.round() as u8;
            }
        }
    }

    // Encode to PNG
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, target_size, target_size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&dst)?;
    }
    Ok(out)
}

/// The Wayland/X11 application identity (issue #493 review).
///
/// These pin the property that makes the identity worth deriving at all: two
/// different rinch applications must not answer to the same `app_id`. A
/// constant default made every app that did not set one share a single desktop
/// identity — and because `install_wayland_icon` writes
/// `{app_id}.desktop` with `Name={app_id}`, that meant the first rinch app to
/// ship an icon supplied the dock icon and name for all the others.
#[cfg(all(test, target_os = "linux"))]
mod app_id_tests {
    use super::{
        app_id_from_exe_path, ensure_desktop_entry_in, icon_stub_carries, install_wayland_icon_in,
        resolved_app_id, write_desktop_entry_stub,
    };

    /// The icon stub never replaces the app's own launcher in the user's
    /// applications directory (an app whose installer writes its launcher
    /// there lost it to a NoDisplay stub on every start, and left the menu).
    /// Nothing there, or an earlier stub, may be written.
    #[test]
    fn the_icon_stub_never_replaces_a_users_launcher() {
        let root = std::env::temp_dir().join(format!("rinch-icon-stub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let launcher = root.join("notes.desktop");
        std::fs::write(
            &launcher,
            "[Desktop Entry]\nType=Application\nName=Notes\nExec=/opt/notes\n",
        )
        .unwrap();
        assert_eq!(icon_stub_carries(&launcher), None);
        let stub = root.join("stub.desktop");
        std::fs::write(
            &stub,
            "[Desktop Entry]\nType=Application\nName=stub\nIcon=/x.png\nNoDisplay=true\n",
        )
        .unwrap();
        assert_eq!(icon_stub_carries(&stub).as_deref(), Some(""));
        assert_eq!(
            icon_stub_carries(&root.join("absent.desktop")).as_deref(),
            Some("")
        );
        // A launcher the user hid through a menu editor: `NoDisplay=true`
        // beside its own `Exec=`. It is the user's, not a stub.
        let hidden = root.join("hidden.desktop");
        std::fs::write(
            &hidden,
            "[Desktop Entry]\nType=Application\nName=Notes\nExec=/opt/x\nNoDisplay=true\n",
        )
        .unwrap();
        assert_eq!(icon_stub_carries(&hidden), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[200u8; 16]).unwrap();
        }
        out
    }

    /// `install_wayland_icon` itself (through its data-directory half) leaves
    /// a launcher it does not own byte for byte as it was — the app's own and
    /// a user-hidden one alike — and writes its stub where there is none.
    #[test]
    fn installing_the_icon_keeps_a_launcher_it_does_not_own() {
        let root = std::env::temp_dir().join(format!("rinch-icon-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        let png = tiny_png();
        for (id, text) in [
            (
                "notes",
                "[Desktop Entry]\nType=Application\nName=Notes\nExec=/opt/notes\n",
            ),
            (
                "hidden",
                "[Desktop Entry]\nType=Application\nName=Hidden\nExec=/opt/x\nNoDisplay=true\n",
            ),
        ] {
            let path = apps.join(format!("{id}.desktop"));
            std::fs::write(&path, text).unwrap();
            install_wayland_icon_in(&root, id, &png);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{id}");
        }
        // Nothing there: the stub is written, naming the icon it installed.
        install_wayland_icon_in(&root, "fresh", &png);
        let stub = std::fs::read_to_string(apps.join("fresh.desktop")).unwrap();
        assert!(stub.contains("NoDisplay=true"), "{stub}");
        assert!(stub.contains("rinch/icons/fresh.png"), "{stub}");
        assert!(root.join("rinch/icons/fresh.png").exists());
        assert!(stub.contains("X-Rinch-Stub=true"), "{stub}");
        // An icon stub from before the marker is replaced (here: rewritten in
        // place), and carries the marker from then on.
        std::fs::write(
            apps.join("fresh.desktop"),
            "[Desktop Entry]\nType=Application\nName=fresh\nIcon=/old.png\nNoDisplay=true\n",
        )
        .unwrap();
        install_wayland_icon_in(&root, "fresh", &png);
        let again = std::fs::read_to_string(apps.join("fresh.desktop")).unwrap();
        assert!(again.contains("rinch/icons/fresh.png"), "{again}");
        assert!(again.contains("X-Rinch-Stub=true"), "{again}");
        // The global-hotkeys stub, as `ensure_desktop_entry` writes it: it is
        // rinch's own, so it gets the icon, and keeps the `Exec=` the portal
        // needs.
        let exe = std::path::Path::new("/opt/keys/bin");
        let written = write_desktop_entry_stub(&root, "keys", exe)
            .unwrap()
            .unwrap();
        let before = std::fs::read_to_string(&written).unwrap();
        assert!(before.contains("X-Rinch-Stub=true"), "{before}");
        install_wayland_icon_in(&root, "keys", &png);
        let keys = std::fs::read_to_string(&written).unwrap();
        assert!(keys.contains("rinch/icons/keys.png"), "{keys}");
        assert!(keys.contains("Exec=\"/opt/keys/bin\""), "{keys}");
        assert!(keys.contains("X-Rinch-Generated=global-hotkeys"), "{keys}");
        assert!(keys.contains("X-Rinch-Stub=true"), "{keys}");
        assert_eq!(keys.matches("Exec=").count(), 1, "{keys}");
        // Installing again changes nothing.
        install_wayland_icon_in(&root, "keys", &png);
        assert_eq!(std::fs::read_to_string(&written).unwrap(), keys);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A packaged launcher in a system directory is never shadowed: no stub
    /// is written for an app id one of them has.
    #[test]
    fn no_desktop_entry_stub_shadows_a_system_entry() {
        let root = std::env::temp_dir().join(format!("rinch-desktop-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sys, home) = (root.join("sys"), root.join("home"));
        std::fs::create_dir_all(sys.join("applications")).unwrap();
        std::fs::write(sys.join("applications/com.example.notes.desktop"), "x").unwrap();
        let dirs = std::env::join_paths([root.join("empty"), sys.clone()]).unwrap();
        let exe = std::path::Path::new("/bin/notes");
        assert_eq!(
            ensure_desktop_entry_in(&dirs, &home, "com.example.notes", exe),
            Ok(None)
        );
        assert!(!home.join("applications/com.example.notes.desktop").exists());
        // Positive control: an id no system directory has is written.
        assert!(
            ensure_desktop_entry_in(&dirs, &home, "com.example.other", exe)
                .unwrap()
                .is_some()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The desktop-entry stub the hotkey portal needs: written once with
    /// `NoDisplay`, and an existing file — the app's own, or the user's — is
    /// never overwritten.
    #[test]
    fn a_desktop_entry_stub_never_overwrites() {
        let home = std::env::temp_dir().join(format!("rinch-desktop-stub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let exe = std::path::Path::new("/opt/My App/bin/notes");
        let written = write_desktop_entry_stub(&home, "com.example.notes", exe).unwrap();
        let path = home.join("applications/com.example.notes.desktop");
        assert_eq!(written.as_deref(), Some(path.as_path()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("[Desktop Entry]\nType=Application\n"),
            "{text}"
        );
        assert!(
            text.contains("NoDisplay=true\n") && text.contains("Exec=\"/opt/My App/bin/notes\"\n"),
            "{text}"
        );

        std::fs::write(&path, "[Desktop Entry]\nName=Mine\n").unwrap();
        assert_eq!(
            write_desktop_entry_stub(&home, "com.example.notes", exe),
            Ok(None)
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Desktop Entry]\nName=Mine\n"
        );
        assert!(write_desktop_entry_stub(&home, "a/b", exe).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The identity `resolved_app_id` would derive from a given executable path.
    fn id_of(path: &str) -> Option<String> {
        app_id_from_exe_path(std::path::Path::new(path))
    }

    /// An explicit `app_id` is handed back untouched — never re-derived, and
    /// never blended with the executable name.
    #[test]
    fn an_explicit_app_id_is_used_verbatim() {
        assert_eq!(
            resolved_app_id(Some("com.example.notes")),
            "com.example.notes"
        );
    }

    /// The explicit value wins even when it happens to be the old constant, so
    /// an app that deliberately asks for `rinch-app` still gets it.
    #[test]
    fn an_explicit_app_id_equal_to_the_old_default_is_still_honoured() {
        assert_eq!(resolved_app_id(Some("rinch-app")), "rinch-app");
    }

    /// With nothing declared, the identity comes from *this* executable — the
    /// whole point. Asserting against the real `current_exe` file stem rather
    /// than a literal keeps the test true under `cargo test`, a renamed test
    /// binary, and CI alike.
    #[test]
    fn an_undeclared_app_id_is_derived_from_the_executable() {
        let expected = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .expect("the test binary has a path");
        assert!(!expected.is_empty());
        assert_eq!(resolved_app_id(None), expected);
    }

    /// The regression this replaces: an undeclared id must NOT be the shared
    /// constant. A test binary is never named `rinch-app`, so this fails the
    /// moment the derivation is dropped — which the equality test above would
    /// too, but this one names the defect.
    #[test]
    fn an_undeclared_app_id_is_not_the_shared_constant() {
        assert_ne!(
            resolved_app_id(None),
            "rinch-app",
            "every rinch app sharing one app_id makes compositors group them \
             as a single application and match one another's .desktop entry"
        );
    }

    /// Two different executables get two different identities. This is the
    /// property the collision fix rests on, and neither test above states it:
    /// both would pass a derivation that returned a constant of its own.
    #[test]
    fn different_executables_get_different_identities() {
        assert_ne!(id_of("/usr/bin/my-notes"), id_of("/usr/bin/my-paint"));
        assert_eq!(
            id_of("/home/u/target/release/my-notes"),
            Some("my-notes".into())
        );
    }

    /// A dotted, reverse-DNS binary name survives whole.
    ///
    /// This is the case that makes `file_name` the right call and `file_stem`
    /// the wrong one: a stem truncates at the last dot, so `com.example.notes`
    /// would register as `com.example` and collide with every sibling in that
    /// namespace — reintroducing, in a smaller blast radius, the exact
    /// collision this derivation exists to remove. A fixture using only
    /// undotted names cannot see the difference, because there `file_stem` and
    /// `file_name` agree.
    #[test]
    fn a_dotted_executable_name_is_not_truncated_at_the_dot() {
        assert_eq!(
            id_of("/usr/bin/com.example.notes"),
            Some("com.example.notes".into())
        );
        assert_ne!(
            id_of("/usr/bin/com.example.notes"),
            id_of("/usr/bin/com.example.paint")
        );
    }

    /// A path with no file name at all falls back rather than yielding an
    /// empty identity.
    #[test]
    fn a_path_with_no_file_name_yields_nothing_to_derive_from() {
        assert_eq!(id_of("/"), None);
    }
}
