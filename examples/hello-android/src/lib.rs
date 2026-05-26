//! Rinch Android demo + performance stress test.

use rinch::prelude::*;

#[cfg(target_os = "android")]
fn safe_padding() -> String {
    let insets = rinch_android::display::safe_area_insets();
    let scale = rinch_android::display::density_dpi().unwrap_or(360) as f32 / 160.0;
    let top = (insets.top as f32 / scale).round() as i32;
    let bottom = (insets.bottom as f32 / scale).round() as i32;
    let left = (insets.left as f32 / scale).round() as i32;
    let right = (insets.right as f32 / scale).round() as i32;
    format!("padding: {top}px {right}px {bottom}px {left}px")
}

#[cfg(not(target_os = "android"))]
fn safe_padding() -> String {
    String::new()
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[component]
fn app() -> NodeHandle {
    let stress_mode = Signal::new(false);
    let safe = safe_padding();

    rsx! {
        div { style: {format!("height: 100%; overflow: hidden; {safe}")},
            if stress_mode.get() {
                StressTest { on_back: move || stress_mode.set(false) }
            } else {
                Demo { on_stress: move || stress_mode.set(true) }
            }
        }
    }
}

// ── Demo dashboard ──────────────────────────────────────────────────────

#[component]
fn Demo(on_stress: Callback, children: &[NodeHandle]) -> NodeHandle {
    let accel = Signal::new([0.0f32; 3]);
    let gyro = Signal::new([0.0f32; 3]);
    let light = Signal::new(0.0f32);
    let location_text = Signal::new(String::new());
    let image_data_uri = Signal::new(String::new());
    let img_status = Signal::new(String::new());

    #[cfg(target_os = "android")]
    {
        use rinch_android::sensors::{DELAY_UI, SensorType};
        rinch_android::sensors::start(SensorType::Accelerometer, DELAY_UI, move |d| {
            accel.set([d.values[0], d.values[1], d.values[2]]);
        });
        rinch_android::sensors::start(SensorType::Gyroscope, DELAY_UI, move |d| {
            gyro.set([d.values[0], d.values[1], d.values[2]]);
        });
        rinch_android::sensors::start(SensorType::Light, DELAY_UI, move |d| {
            light.set(d.values[0]);
        });
    }

    rsx! {
        div { style: "display: flex; flex-direction: column; padding: 16px; gap: 12px; height: 100%; overflow: auto; font-size: 16px",
            div { style: "font-size: 28px; font-weight: bold; color: #1976D2",
                "Rinch Android Demo"
            }
            button {
                style: "padding: 10px 20px; background-color: #E91E63; color: white; font-size: 16px; align-self: flex-start",
                onclick: move || on_stress.invoke(),
                "Stress Test"
            }

            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px", "Sensors" }
            div { style: "font-family: monospace; font-size: 14px; color: #333; line-height: 1.6",
                div { "Accel: " {|| { let a = accel.get(); format!("{:7.2} {:7.2} {:7.2}", a[0], a[1], a[2]) }} }
                div { "Gyro:  " {|| { let g = gyro.get(); format!("{:7.3} {:7.3} {:7.3}", g[0], g[1], g[2]) }} }
                div { "Light: " {|| format!("{:.0} lux", light.get())} }
            }

            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px", "Location" }
            button {
                style: "padding: 10px 20px; background-color: #FF9800; color: white; font-size: 16px; align-self: flex-start",
                onclick: move || {
                    #[cfg(target_os = "android")]
                    {
                        location_text.set("Requesting location...".into());
                        rinch_android::location::start(1000, 0.0, move |loc| {
                            location_text.set(format!(
                                "{:.6}, {:.6}\nalt {:.0}m  acc {:.0}m  spd {:.1}m/s\n{}",
                                loc.latitude, loc.longitude, loc.altitude,
                                loc.accuracy, loc.speed, loc.provider
                            ));
                        });
                    }
                },
                "Start GPS"
            }
            div { style: "font-family: monospace; font-size: 14px; color: #333; white-space: pre-wrap",
                {|| location_text.get()}
            }

            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px", "Notifications & Share" }
            div { style: "display: flex; flex-direction: row; gap: 12px",
                button {
                    style: "padding: 10px 20px; background-color: #9C27B0; color: white; font-size: 16px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        rinch_android::notification::show("Hello from Rinch!", "This notification was sent from Rust");
                    },
                    "Notify"
                }
                button {
                    style: "padding: 10px 20px; background-color: #607D8B; color: white; font-size: 16px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        rinch_android::share::share_text("Sent from Rinch on Android!");
                    },
                    "Share Text"
                }
            }

            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px", "Image Picker" }
            div { style: "display: flex; flex-direction: row; gap: 12px",
                button {
                    style: "padding: 10px 20px; background-color: #2196F3; color: white; font-size: 16px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        {
                            img_status.set("Opening gallery...".into());
                            rinch_android::camera::pick_image(move |bytes| match bytes {
                                Some(b) => {
                                    img_status.set(format!("Picked: {} bytes", b.len()));
                                    image_data_uri.set(rinch_android::camera::bytes_to_data_uri(&b));
                                }
                                None => img_status.set("Cancelled".into()),
                            });
                        }
                    },
                    "Gallery"
                }
                button {
                    style: "padding: 10px 20px; background-color: #4CAF50; color: white; font-size: 16px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        {
                            img_status.set("Opening camera...".into());
                            rinch_android::camera::take_photo(move |bytes| match bytes {
                                Some(b) => {
                                    img_status.set(format!("Photo: {} bytes", b.len()));
                                    image_data_uri.set(rinch_android::camera::bytes_to_data_uri(&b));
                                }
                                None => img_status.set("Cancelled".into()),
                            });
                        }
                    },
                    "Camera"
                }
            }
            div { style: "font-size: 14px; color: #666", {|| img_status.get()} }
            if !image_data_uri.get().is_empty() {
                img {
                    src: {|| image_data_uri.get()},
                    style: "max-width: 100%; max-height: 300px; object-fit: contain",
                }
            }
        }
    }
}

// ── Stress test ─────────────────────────────────────────────────────────

const GRID: usize = 16;
const CELLS: usize = GRID * GRID;

#[component]
fn StressTest(on_back: Callback, children: &[NodeHandle]) -> NodeHandle {
    let frame = Signal::new(0u32);
    let accel = Signal::new([0.0f32; 3]);
    let fps_text = Signal::new(String::from("..."));

    #[cfg(target_os = "android")]
    {
        let frame_times = Signal::new(Vec::<f64>::new());
        use rinch_android::sensors::{DELAY_FASTEST, SensorType};
        rinch_android::sensors::start(SensorType::Accelerometer, DELAY_FASTEST, move |d| {
            accel.set([d.values[0], d.values[1], d.values[2]]);
            frame.update(|f| *f = f.wrapping_add(1));

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let mut times = frame_times.get();
            times.push(now);
            while times.len() > 60 {
                times.remove(0);
            }
            let fps_str = if times.len() >= 2 {
                let elapsed = times.last().unwrap() - times.first().unwrap();
                if elapsed > 0.0 {
                    format!("{:.0} FPS", (times.len() - 1) as f64 / elapsed)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            frame_times.set(times);
            if !fps_str.is_empty() {
                fps_text.set(fps_str);
            }
        });
    }

    rsx! {
        div { style: "display: flex; flex-direction: column; height: 100%; padding: 8px; gap: 8px",
            // Header
            div { style: "display: flex; flex-direction: row; justify-content: space-between; align-items: center",
                div { style: "font-size: 20px; font-weight: bold; color: #E91E63",
                    "Stress Test"
                }
                div { style: "font-size: 24px; font-weight: bold; font-family: monospace; color: #333",
                    {|| fps_text.get()}
                }
                button {
                    style: "padding: 8px 16px; background-color: #666; color: white; font-size: 14px",
                    onclick: move || on_back.invoke(),
                    "Back"
                }
            }
            div { style: "font-size: 12px; color: #666",
                {format!("{CELLS} cells, each updating color every frame from accelerometer")}
            }

            // Grid
            div { style: "flex: 1; display: flex; flex-direction: column; gap: 1px",
                for row in 0..GRID {
                    div { key: row, style: "display: flex; flex-direction: row; gap: 1px; flex: 1",
                        for col in 0..GRID {
                            div { key: col,
                                style: {move || {
                                    let f = frame.get() as f32;
                                    let a = accel.get();
                                    let r = ((((row as f32 * 17.0 + f * 2.3 + a[0] * 20.0).sin() * 0.5 + 0.5) * 255.0) as u8).max(30);
                                    let g = ((((col as f32 * 13.0 + f * 1.7 + a[1] * 20.0).sin() * 0.5 + 0.5) * 255.0) as u8).max(30);
                                    let b = ((((row as f32 + col as f32) * 7.0 + f * 3.1 + a[2] * 5.0).sin() * 0.5 + 0.5) * 255.0) as u8;
                                    format!("flex: 1; background-color: rgb({r},{g},{b})")
                                }},
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    run_android(android_app, "Hello Android", 0, 0, app);
}
