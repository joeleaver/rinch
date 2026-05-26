//! Minimal rinch app for Android.
//!
//! Build with:
//!   ./build-apk.sh
//!   # or: cargo ndk -t x86_64 build -p hello-android --release

use rinch::prelude::*;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);
    let input_text = Signal::new(String::new());
    let image_data_uri = Signal::new(String::new());
    let status = Signal::new(String::new());

    rsx! {
        div { style: "display: flex; flex-direction: column; padding: 40px; gap: 16px; height: 100%; overflow: auto",
            div { style: "font-size: 32px; font-weight: bold; color: #1976D2",
                "Hello from Android!"
            }
            div { style: "font-size: 18px; color: #555",
                "Rinch running natively with tiny-skia + softbuffer"
            }
            div { style: "display: flex; flex-direction: row; gap: 12px; align-items: center",
                button {
                    style: "padding: 12px 24px; background-color: #4CAF50; color: white; font-size: 18px",
                    onclick: move || count.update(|n| *n += 1),
                    "Tap me!"
                }
                div { style: "font-size: 24px",
                    "Count: " {|| count.get().to_string()}
                }
            }
            div { style: "font-size: 16px; color: #333; margin-top: 8px",
                "Type something (tap input to show keyboard):"
            }
            input {
                style: "padding: 12px; font-size: 18px; border: 2px solid #ccc; background-color: white",
                placeholder: "Type here...",
                oninput: move |value: String| input_text.set(value),
            }
            div { style: "font-size: 16px; color: #666",
                "You typed: " {|| input_text.get()}
            }

            // ── Image Picker / Camera ──────────────────────────────
            div { style: "margin-top: 16px; font-size: 20px; font-weight: bold; color: #1976D2",
                "Image Picker"
            }
            div { style: "display: flex; flex-direction: row; gap: 12px",
                button {
                    style: "padding: 12px 24px; background-color: #2196F3; color: white; font-size: 18px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        {
                            status.set("Opening gallery...".into());
                            rinch_android::camera::pick_image(move |bytes| match bytes {
                                Some(b) => {
                                    status.set(format!("Picked: {} bytes", b.len()));
                                    image_data_uri.set(rinch_android::camera::bytes_to_data_uri(&b));
                                }
                                None => status.set("Cancelled".into()),
                            });
                        }
                    },
                    "Gallery"
                }
                button {
                    style: "padding: 12px 24px; background-color: #4CAF50; color: white; font-size: 18px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        {
                            status.set("Opening camera...".into());
                            rinch_android::camera::take_photo(move |bytes| match bytes {
                                Some(b) => {
                                    status.set(format!("Photo: {} bytes", b.len()));
                                    image_data_uri.set(rinch_android::camera::bytes_to_data_uri(&b));
                                }
                                None => status.set("Cancelled".into()),
                            });
                        }
                    },
                    "Camera"
                }
            }
            div { style: "font-size: 14px; color: #666",
                {|| status.get()}
            }
            if !image_data_uri.get().is_empty() {
                img {
                    src: {|| image_data_uri.get()},
                    style: "max-width: 100%; max-height: 400px; object-fit: contain",
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
