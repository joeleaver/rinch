//! Issue #2689 on the embed path: `RinchContext::set_devtools(false)` stops
//! rinch's key fallback answering F12 with `ToggleDevTools`, so a host with no
//! DevTools gets the key back. F12 is answered only with the `windowing`
//! feature; Alt+I, which toggled inspect mode under the same switch, is an
//! ordinary key since #3062.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_devtools_keys_2689
//!
//! One test in this binary: `RinchContext::new` registers its thread as the
//! main thread. Assumes `RINCH_DEVTOOLS` is not set in the environment (it
//! overrides the context's setting by design).

#![cfg(any(feature = "gpu-renderer", feature = "embed"))]

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{AppAction, KeyCode, KeyRepeat, Modifiers, PlatformEvent};
use rinch::prelude::*;

fn key(key: KeyCode, text: Option<&str>, alt: bool) -> [PlatformEvent; 2] {
    let modifiers = Modifiers {
        alt,
        ..Modifiers::default()
    };
    [
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    ]
}

/// Whether F12 toggles DevTools; Alt+I, pressed alongside, must never.
fn toggles(ctx: &mut RinchContext) -> bool {
    let alt_i = ctx.update(&key(KeyCode::KeyI, Some("i"), true));
    assert!(
        !alt_i.contains(&AppAction::ToggleDevTools),
        "Alt+I is not a DevTools key: {alt_i:?}"
    );
    ctx.update(&key(KeyCode::F12, None, false))
        .contains(&AppAction::ToggleDevTools)
}

#[test]
fn set_devtools_false_stops_the_fallback_answering_f12() {
    if std::env::var_os(rinch::app::DEVTOOLS_ENV).is_some() {
        eprintln!("skipped: {} is set", rinch::app::DEVTOOLS_ENV);
        return;
    }
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 400,
            height: 300,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        |scope: &mut RenderScope| scope.create_element("div"),
    );
    ctx.update(&[]);

    // Control: on by default, as before. F12 is a desktop-feature key.
    assert_eq!(toggles(&mut ctx), cfg!(feature = "windowing"));
    assert!(ctx.app().devtools_enabled());

    ctx.set_devtools(false);
    assert!(!ctx.app().devtools_enabled());
    assert!(!toggles(&mut ctx), "off: F12 not answered");

    ctx.app_mut().set_devtools(true);
    assert!(ctx.app().devtools_enabled(), "back on through app_mut()");
    assert_eq!(toggles(&mut ctx), cfg!(feature = "windowing"));
}
