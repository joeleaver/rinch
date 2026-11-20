//! Issue #2689 on the embed path: `RinchContext::set_devtools(false)` stops
//! rinch's key fallback answering F12 and Alt+I with `ToggleDevTools` /
//! `ToggleInspectMode`, so a host with no DevTools gets those keys back.
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

fn toggles(ctx: &mut RinchContext) -> (bool, bool) {
    let mut actions = ctx.update(&key(KeyCode::F12, None, false));
    actions.extend(ctx.update(&key(KeyCode::KeyI, Some("i"), true)));
    (
        actions.contains(&AppAction::ToggleDevTools),
        actions.contains(&AppAction::ToggleInspectMode),
    )
}

#[test]
fn set_devtools_false_stops_the_fallback_answering_f12_and_alt_i() {
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
    let (devtools, inspect) = toggles(&mut ctx);
    assert!(inspect, "Alt+I toggles inspect mode by default");
    assert_eq!(devtools, cfg!(feature = "windowing"));
    assert!(ctx.app().devtools_enabled());

    ctx.set_devtools(false);
    assert!(!ctx.app().devtools_enabled());
    assert_eq!(
        toggles(&mut ctx),
        (false, false),
        "off: neither key answered"
    );

    ctx.app_mut().set_devtools(true);
    assert!(toggles(&mut ctx).1, "back on through app_mut()");
}
