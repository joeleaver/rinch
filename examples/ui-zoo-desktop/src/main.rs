//! UI Zoo desktop entry point.
//!
//! This is a thin wrapper around the shared ui-zoo library that configures
//! desktop-specific window properties (borderless, transparent) and theme.

use rinch::prelude::*;
use rinch::{WindowProps, run_rinch_with_window_props};

fn main() {
    eprintln!("[DEBUG MAIN] Starting ui-zoo-desktop");
    let window_props = WindowProps {
        title: "UI Zoo - Rinch Component Library".into(),
        width: 1200,
        height: 800,
        borderless: true,
        transparent: true,
        ..Default::default()
    };

    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    // Set up theme CSS (loads into thread-local, picked up by rinch-dom runtime)
    rinch::setup_theme_css(&theme);

    run_rinch_with_window_props(ui_zoo::app, window_props);
}
