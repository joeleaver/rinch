//! Minimal rinch app for Android.
//!
//! Build with:
//!   cargo ndk -t arm64-v8a build -p hello-android --release
//!
//! The android-activity crate provides the entry point via GameActivity.
//! On desktop, this binary is a no-op (the android runtime is target-gated).

use rinch::prelude::*;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div { style: "display: flex; flex-direction: column; padding: 40px; gap: 16px",
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
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    run_android(android_app, "Hello Android", 0, 0, app);
}
