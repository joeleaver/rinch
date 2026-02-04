//! UI Zoo desktop entry point.

use rinch::prelude::*;
use rinch::{WindowProps, run_rinch_with_window_props};

fn main() {
    let window_props = WindowProps {
        title: "UI Zoo - Rinch Widget Showcase".into(),
        width: 1200,
        height: 800,
        ..Default::default()
    };

    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    rinch::setup_theme_css(&theme);
    run_rinch_with_window_props(ui_zoo::app, window_props);
}
