//! Mouse look on a `RenderSurface` (issue #2092): click the surface to lock
//! the pointer, move the mouse to turn, press Escape (or switch windows) to
//! let go. The readout shows what the lock is (`Locked`, or `Confined` on X11)
//! and the camera's yaw and pitch, which turn by the mouse's raw motion and do
//! not stop at the edge of the screen.
//!
//! `cargo run -p pointer-lock`
//!
//! `POINTER_LOCK_LOG=1` prints each request, lock change and motion step to
//! stdout; `POINTER_LOCK_AUTO=1` asks for the lock 500 ms (or
//! `POINTER_LOCK_AUTO_MS`) after start, with no click (a desktop grants it
//! without a gesture, to a focused window). Both are what the live check
//! of #2092 drives.

use rinch::prelude::*;

/// Degrees the camera turns per unit of motion.
const SENSITIVITY: f64 = 0.15;

fn log(line: String) {
    if std::env::var_os("POINTER_LOCK_LOG").is_some() {
        println!("pointer-lock: {line}");
    }
}

#[component]
fn app() -> NodeHandle {
    let yaw = Signal::new(0.0_f64);
    let pitch = Signal::new(0.0_f64);
    let surface = create_render_surface();
    let asking = surface.clone();
    surface.set_event_handler(move |event| match event {
        SurfaceEvent::MouseDown { .. } => {
            let result = asking.request_pointer_lock();
            log(format!("request {result:?}"));
        }
        SurfaceEvent::PointerMotion { dx, dy } => {
            log(format!("motion {dx} {dy}"));
            yaw.update(|y| *y = (*y + f64::from(dx) * SENSITIVITY).rem_euclid(360.0));
            pitch.update(|p| *p = (*p - f64::from(dy) * SENSITIVITY).clamp(-89.0, 89.0));
        }
        _ => {}
    });
    // Every change of the lock, granted or lost.
    let _ = rinch::Effect::new(|| log(format!("lock {:?}", pointer_lock())));
    if std::env::var_os("POINTER_LOCK_AUTO").is_some() {
        let auto = surface.clone();
        let delay = std::env::var("POINTER_LOCK_AUTO_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
            .unwrap_or(500);
        set_timeout(delay, move || {
            let result = auto.request_pointer_lock();
            log(format!("request {result:?}"));
        });
    }

    rsx! {
        div {
            style: "display: flex; flex-direction: column; height: 100%; font-family: sans-serif",
            div {
                style: "padding: 8px 12px; background: #1d2330; color: #e6e9ef",
                p {
                    {|| match pointer_lock() {
                        Some(PointerLockMode::Locked) => "Locked: move to look, Escape to let go".to_string(),
                        Some(PointerLockMode::Confined) => "Confined (X11): move to look, Escape to let go".to_string(),
                        _ => "Click the view to lock the pointer".to_string(),
                    }}
                }
                p { {|| format!("yaw {:6.1}°   pitch {:5.1}°", yaw.get(), pitch.get())} }
            }
            div {
                style: {|| format!(
                    "flex: 1; background: linear-gradient(hsl({:.0}, 45%, 35%), hsl({:.0}, 45%, 15%))",
                    yaw.get(),
                    yaw.get() + 40.0
                )},
                RenderSurface { surface: Some(surface) }
            }
        }
    }
}

fn main() {
    App::new(app).title("Pointer lock").size(640, 420).run();
}
