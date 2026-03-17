//! ColorPicker component.
//!
//! An interactive color picker with a saturation panel, hue slider,
//! optional alpha slider, hex input, and preset swatches.

use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, Drag, InputCallback, Signal, get_click_context};

use crate::color_swatch::ColorSwatch;
use crate::color_utils::{
    ColorFormat, Hsva, format_color, hsv_to_rgb, hue_to_rgb_hex, parse_color, rgb_to_hex,
};

/// Reactive callback type for string state.
pub type ReactiveString = Rc<dyn Fn() -> String>;

/// An interactive color picker with saturation panel, hue/alpha sliders, hex input, and swatches.
#[derive(Default)]
pub struct ColorPicker {
    /// Output format: hex, hexa, rgb, rgba, hsl, hsla.
    pub format: String,
    /// Initial color value (any parseable string).
    pub value: String,
    /// Reactive external value binding.
    pub value_fn: Option<ReactiveString>,
    /// Fires formatted color string on change.
    pub onchange: Option<InputCallback>,
    /// Show alpha slider. Defaults to true.
    pub alpha: bool,
    /// Preset swatch colors.
    pub swatches: Vec<String>,
    /// Number of swatches per row. Defaults to 7.
    pub swatches_per_row: Option<usize>,
    /// Size: xs, sm, md, lg, xl.
    pub size: String,
    /// Show hex text input. Defaults to true.
    pub with_input: bool,
}

impl std::fmt::Debug for ColorPicker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorPicker")
            .field("format", &self.format)
            .field("value", &self.value)
            .field("alpha", &self.alpha)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Component for ColorPicker {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let color_format = ColorFormat::parse(&self.format).unwrap_or(ColorFormat::Hex);
        let show_alpha = self.alpha;
        let show_input = self.with_input;
        let swatches_per_row = self.swatches_per_row.unwrap_or(7);

        // Parse initial color
        let initial = if !self.value.is_empty() {
            parse_color(&self.value).unwrap_or(Hsva {
                h: 0.0,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            })
        } else {
            Hsva {
                h: 0.0,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }
        };

        // Internal signals
        let hue = Signal::new(initial.h);
        let sat = Signal::new(initial.s);
        let val = Signal::new(initial.v);
        let alpha = Signal::new(initial.a);

        // Root container
        let size_class = match self.size.as_str() {
            "sm" => " rinch-color-picker--sm",
            "lg" => " rinch-color-picker--lg",
            "xl" => " rinch-color-picker--xl",
            _ => " rinch-color-picker--md",
        };
        let root = rinch_macros::rsx! { div {} };
        root.set_attribute("class", &format!("rinch-color-picker{}", size_class));

        // === Saturation panel ===
        let sat_panel = rinch_macros::rsx! { div { class: "rinch-color-picker__saturation" } };

        // Background: solid hue color
        let sat_bg = rinch_macros::rsx! { div { class: "rinch-color-picker__saturation-bg" } };
        sat_bg.set_attribute(
            "style",
            &format!("background-color: {}", hue_to_rgb_hex(initial.h)),
        );

        // White gradient overlay
        let sat_white =
            rinch_macros::rsx! { div { class: "rinch-color-picker__saturation-white" } };

        // Black gradient overlay
        let sat_black =
            rinch_macros::rsx! { div { class: "rinch-color-picker__saturation-black" } };

        // Thumb
        let sat_thumb = rinch_macros::rsx! { div { class: "rinch-color-picker__thumb" } };
        sat_thumb.set_attribute(
            "style",
            &format!(
                "left: {}%; top: {}%",
                initial.s * 100.0,
                (1.0 - initial.v) * 100.0
            ),
        );

        // Click overlay
        let sat_overlay =
            rinch_macros::rsx! { div { class: "rinch-color-picker__saturation-overlay" } };

        {
            let handler_id = __scope.register_handler(move || {
                let ctx = get_click_context();
                let px = ctx.percent_x() as f64;
                let py = ctx.percent_y() as f64;
                sat.set(px);
                val.set(1.0 - py);

                Drag::percent()
                    .on_move(move |px, py| {
                        sat.set(px as f64);
                        val.set(1.0 - py as f64);
                    })
                    .start();
            });
            sat_overlay.set_attribute("data-rid", &handler_id.to_string());
        }

        sat_panel.append_child(&sat_bg);
        sat_panel.append_child(&sat_white);
        sat_panel.append_child(&sat_black);
        sat_panel.append_child(&sat_thumb);
        sat_panel.append_child(&sat_overlay);
        root.append_child(&sat_panel);

        // Update saturation panel reactively
        {
            let sat_bg = sat_bg.clone();
            let sat_thumb = sat_thumb.clone();
            __scope.create_effect(move || {
                let h = hue.get();
                let s = sat.get();
                let v = val.get();
                sat_bg.set_attribute("style", &format!("background-color: {}", hue_to_rgb_hex(h)));
                sat_thumb.set_attribute(
                    "style",
                    &format!("left: {}%; top: {}%", s * 100.0, (1.0 - v) * 100.0),
                );
            });
        }

        // === Hue slider ===
        let hue_slider = rinch_macros::rsx! { div { class: "rinch-color-picker__hue" } };

        let hue_thumb = rinch_macros::rsx! { div { class: "rinch-color-picker__hue-thumb" } };
        hue_thumb.set_attribute("style", &format!("left: {}%", initial.h / 360.0 * 100.0));

        let hue_overlay = rinch_macros::rsx! { div { class: "rinch-color-picker__hue-overlay" } };

        {
            let handler_id = __scope.register_handler(move || {
                let ctx = get_click_context();
                let px = ctx.percent_x() as f64;
                hue.set(px * 360.0);

                Drag::percent()
                    .on_move(move |px, _py| {
                        hue.set(px as f64 * 360.0);
                    })
                    .start();
            });
            hue_overlay.set_attribute("data-rid", &handler_id.to_string());
        }

        hue_slider.append_child(&hue_thumb);
        hue_slider.append_child(&hue_overlay);
        root.append_child(&hue_slider);

        // Update hue thumb reactively
        {
            let hue_thumb = hue_thumb.clone();
            __scope.create_effect(move || {
                let h = hue.get();
                hue_thumb.set_attribute("style", &format!("left: {}%", h / 360.0 * 100.0));
            });
        }

        // === Alpha slider (optional) ===
        if show_alpha {
            let alpha_slider = rinch_macros::rsx! { div { class: "rinch-color-picker__alpha" } };

            let alpha_checker =
                rinch_macros::rsx! { div { class: "rinch-color-picker__alpha-checkerboard" } };

            let alpha_gradient =
                rinch_macros::rsx! { div { class: "rinch-color-picker__alpha-gradient" } };

            let alpha_thumb =
                rinch_macros::rsx! { div { class: "rinch-color-picker__alpha-thumb" } };
            alpha_thumb.set_attribute("style", &format!("left: {}%", initial.a * 100.0));

            let alpha_overlay =
                rinch_macros::rsx! { div { class: "rinch-color-picker__alpha-overlay" } };

            {
                let handler_id = __scope.register_handler(move || {
                    let ctx = get_click_context();
                    let px = ctx.percent_x() as f64;
                    alpha.set(px);

                    Drag::percent()
                        .on_move(move |px, _py| {
                            alpha.set(px as f64);
                        })
                        .start();
                });
                alpha_overlay.set_attribute("data-rid", &handler_id.to_string());
            }

            alpha_slider.append_child(&alpha_checker);
            alpha_slider.append_child(&alpha_gradient);
            alpha_slider.append_child(&alpha_thumb);
            alpha_slider.append_child(&alpha_overlay);
            root.append_child(&alpha_slider);

            // Update alpha slider reactively
            {
                let alpha_gradient = alpha_gradient.clone();
                let alpha_thumb = alpha_thumb.clone();
                __scope.create_effect(move || {
                    let h = hue.get();
                    let s = sat.get();
                    let v = val.get();
                    let a = alpha.get();
                    let rgb = hsv_to_rgb(Hsva { h, s, v, a: 1.0 });
                    let hex = rgb_to_hex(rgb, false);
                    alpha_gradient.set_attribute(
                        "style",
                        &format!(
                            "background: linear-gradient(to right, transparent, {})",
                            hex
                        ),
                    );
                    alpha_thumb.set_attribute("style", &format!("left: {}%", a * 100.0));
                });
            }
        }

        // === Controls row: preview swatch + hex input ===
        if show_input {
            let controls = rinch_macros::rsx! { div { class: "rinch-color-picker__controls" } };

            // Preview swatch
            let preview_swatch = ColorSwatch {
                color: format_color(initial, ColorFormat::Rgba),
                size: "28px".into(),
                radius: "sm".into(),
                ..Default::default()
            };
            let preview_node = preview_swatch.render(__scope, &[]);
            preview_node.set_attribute("class", "rinch-color-swatch rinch-color-picker__preview");
            controls.append_child(&preview_node);

            // Hex input
            let hex_input = rinch_macros::rsx! { input { class: "rinch-color-picker__hex-input" } };
            hex_input.set_attribute("value", &format_color(initial, color_format));

            {
                let handler_id = __scope.register_input_handler(move |value: String| {
                    if let Some(parsed) = parse_color(&value) {
                        hue.set(parsed.h);
                        sat.set(parsed.s);
                        val.set(parsed.v);
                        alpha.set(parsed.a);
                    }
                });
                hex_input.set_attribute("data-oninput", &handler_id.to_string());
            }

            controls.append_child(&hex_input);
            root.append_child(&controls);

            // Update preview and hex input reactively
            {
                let preview_overlay = preview_node.clone();
                let hex_input = hex_input.clone();
                __scope.create_effect(move || {
                    let hsv = Hsva {
                        h: hue.get(),
                        s: sat.get(),
                        v: val.get(),
                        a: alpha.get(),
                    };
                    let color_str = format_color(hsv, ColorFormat::Rgba);
                    // Update preview swatch overlay
                    let children = preview_overlay.children();
                    if let Some(overlay_child) = children.first() {
                        overlay_child.set_attribute(
                            "style",
                            &format!(
                                "background-color: {}; border-radius: var(--rinch-radius-sm)",
                                color_str
                            ),
                        );
                    }
                    // Update hex input
                    hex_input.set_attribute("value", &format_color(hsv, color_format));
                });
            }
        }

        // === Swatches grid ===
        if !self.swatches.is_empty() {
            let swatches_grid =
                rinch_macros::rsx! { div { class: "rinch-color-picker__swatches" } };
            let swatch_size = format!(
                "{}px",
                (200 - (swatches_per_row - 1) * 4) / swatches_per_row
            );

            for swatch_color in &self.swatches {
                let color = swatch_color.clone();
                let swatch = ColorSwatch {
                    color: color.clone(),
                    size: swatch_size.clone(),
                    radius: "sm".into(),
                    onclick: Some(rinch_core::Callback::new({
                        let color = color.clone();
                        move || {
                            if let Some(parsed) = parse_color(&color) {
                                hue.set(parsed.h);
                                sat.set(parsed.s);
                                val.set(parsed.v);
                                alpha.set(parsed.a);
                            }
                        }
                    })),
                    ..Default::default()
                };
                let swatch_node = swatch.render(__scope, &[]);
                swatches_grid.append_child(&swatch_node);
            }

            root.append_child(&swatches_grid);
        }

        // === Coordinating effect: fire onchange when any signal changes ===
        if let Some(ref onchange) = self.onchange {
            let onchange = onchange.clone();
            __scope.create_effect(move || {
                let hsv = Hsva {
                    h: hue.get(),
                    s: sat.get(),
                    v: val.get(),
                    a: alpha.get(),
                };
                onchange.invoke(format_color(hsv, color_format));
            });
        }

        // === value_fn binding: external value → internal signals ===
        if let Some(ref value_fn) = self.value_fn {
            let value_fn = value_fn.clone();
            __scope.create_effect(move || {
                let external = value_fn();
                if let Some(parsed) = parse_color(&external) {
                    // Only update if meaningfully different to avoid feedback loops
                    let current = Hsva {
                        h: hue.get(),
                        s: sat.get(),
                        v: val.get(),
                        a: alpha.get(),
                    };
                    if (parsed.h - current.h).abs() > 0.5
                        || (parsed.s - current.s).abs() > 0.005
                        || (parsed.v - current.v).abs() > 0.005
                        || (parsed.a - current.a).abs() > 0.005
                    {
                        hue.set(parsed.h);
                        sat.set(parsed.s);
                        val.set(parsed.v);
                        alpha.set(parsed.a);
                    }
                }
            });
        }

        root
    }
}
