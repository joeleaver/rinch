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
    let picked_uri = Signal::new(String::new());
    let file_size = Signal::new(String::new());
    let perm_status = Signal::new(String::new());

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

            // ── File Picker Test ────────────────────────────────────
            div { style: "margin-top: 16px; font-size: 20px; font-weight: bold; color: #1976D2",
                "File Picker Test"
            }
            div { style: "display: flex; flex-direction: row; gap: 12px",
                button {
                    style: "padding: 12px 24px; background-color: #2196F3; color: white; font-size: 18px",
                    onclick: move || {
                        #[cfg(target_os = "android")]
                        rinch_android::file_picker::pick_file(move |uri| {
                            match uri {
                                Some(u) => {
                                    picked_uri.set(u.clone());
                                    match rinch_android::file_picker::read_content_uri(&u) {
                                        Ok(bytes) => file_size.set(format!("{} bytes", bytes.len())),
                                        Err(e) => file_size.set(format!("read error: {e}")),
                                    }
                                }
                                None => {
                                    picked_uri.set("(cancelled)".into());
                                    file_size.set(String::new());
                                }
                            }
                        });
                    },
                    "Pick File"
                }
            }
            div { style: "font-size: 14px; color: #666; word-break: break-all",
                "URI: " {|| picked_uri.get()}
            }
            div { style: "font-size: 14px; color: #666",
                "Size: " {|| file_size.get()}
            }

            // ── Permission Test ─────────────────────────────────────
            div { style: "margin-top: 16px; font-size: 20px; font-weight: bold; color: #1976D2",
                "Permission Test"
            }
            button {
                style: "padding: 12px 24px; background-color: #FF9800; color: white; font-size: 18px",
                onclick: move || {
                    #[cfg(target_os = "android")]
                    {
                        let has = rinch_android::permissions::has_permission("android.permission.CAMERA");
                        if has {
                            perm_status.set("CAMERA: already granted".into());
                        } else {
                            perm_status.set("Requesting CAMERA...".into());
                            rinch_android::permissions::request_permission(
                                "android.permission.CAMERA",
                                move |granted| {
                                    perm_status.set(if granted {
                                        "CAMERA: granted!".into()
                                    } else {
                                        "CAMERA: denied".into()
                                    });
                                },
                            );
                        }
                    }
                },
                "Request Camera Permission"
            }
            div { style: "font-size: 14px; color: #666",
                {|| perm_status.get()}
            }

            div { style: "margin-top: 12px; font-size: 16px; color: #333",
                "Scroll down to see more content..."
            }
            for i in 0..20 {
                div { key: i, style: "padding: 16px; margin: 4px 0; background-color: #f0f0f0; font-size: 16px",
                    {format!("Item {i}: scroll to see this content")}
                }
            }
            div { style: "padding: 20px; font-size: 18px; font-weight: bold; color: #4CAF50",
                "You reached the bottom!"
            }
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    run_android(android_app, "Hello Android", 0, 0, app);
}
