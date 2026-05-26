//! Minimal rinch app for Android.
//!
//! Build with:
//!   ./build-apk.sh
//!   # or: cargo ndk -t x86_64 build -p hello-android --release

use rinch::prelude::*;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[component]
fn app() -> NodeHandle {
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
        div { style: "display: flex; flex-direction: column; padding: 32px; gap: 12px; height: 100%; overflow: auto; font-size: 16px",
            div { style: "font-size: 28px; font-weight: bold; color: #1976D2",
                "Rinch Android Demo"
            }

            // ── Sensors ────────────────────────────────────────────
            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px",
                "Sensors"
            }
            div { style: "font-family: monospace; font-size: 14px; color: #333; line-height: 1.6",
                div { "Accel: " {|| { let a = accel.get(); format!("{:7.2} {:7.2} {:7.2}", a[0], a[1], a[2]) }} }
                div { "Gyro:  " {|| { let g = gyro.get(); format!("{:7.3} {:7.3} {:7.3}", g[0], g[1], g[2]) }} }
                div { "Light: " {|| format!("{:.0} lux", light.get())} }
            }

            // ── Location ───────────────────────────────────────────
            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px",
                "Location"
            }
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

            // ── Notifications + Share ──────────────────────────────
            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px",
                "Notifications & Share"
            }
            div { style: "display: flex; flex-direction: row; gap: 12px",
                button {
                    style: "padding: 10px 20px; background-color: #9C27B0; color: white; font-size: 16px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        rinch_android::notification::show("Hello from Rinch!", "This notification was sent from Rust 🦀");
                    },
                    "Notify"
                }
                button {
                    style: "padding: 10px 20px; background-color: #607D8B; color: white; font-size: 16px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        rinch_android::share::share_text("Sent from Rinch on Android! 🚀");
                    },
                    "Share Text"
                }
            }

            // ── Image Picker / Camera ──────────────────────────────
            div { style: "font-size: 18px; font-weight: bold; color: #1976D2; margin-top: 8px",
                "Image Picker"
            }
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
            div { style: "font-size: 14px; color: #666",
                {|| img_status.get()}
            }
            if !image_data_uri.get().is_empty() {
                img {
                    src: {|| image_data_uri.get()},
                    style: "max-width: 100%; max-height: 300px; object-fit: contain",
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
