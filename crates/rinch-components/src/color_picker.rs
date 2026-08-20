//! ColorPicker component.
//!
//! An interactive color picker with a saturation panel, hue slider,
//! optional alpha slider, hex input, and preset swatches.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, Drag, InputCallback, Signal, batch, get_click_context};

use crate::color_swatch::ColorSwatch;
use crate::color_utils::{
    ColorFormat, Hsva, denotes_same, format_color, hsv_to_rgb, hue_to_rgb_hex, parse_color,
    rgb_to_hex,
};

/// Reactive callback type for string state.
pub type ReactiveString = Rc<dyn Fn() -> String>;

/// Raises the "an external value is being applied" flag for as long as it lives.
///
/// RAII rather than a set/clear pair because the batched writes it spans
/// normally flush effects before the batch returns: arbitrary subscriber code
/// runs inside the guarded window, and a panic anywhere in there would leave
/// the flag up — muting the picker's `onchange` for the rest of the session,
/// which is a worse failure than the one being prevented.
///
/// The window is not always enough on its own: when the apply runs while an
/// ambient `batch()` is open (rinch-core batches nest), the flush is deferred
/// past this guard's drop, so the picker also records the applied colour in a
/// `last_external_apply` marker for the coordinating effect to recognise.
struct ApplyGuard<'a>(&'a Cell<bool>);

impl<'a> ApplyGuard<'a> {
    fn raise(flag: &'a Cell<bool>) -> Self {
        flag.set(true);
        ApplyGuard(flag)
    }
}

impl Drop for ApplyGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// Whether two colours are within the tolerance the `value_fn` binding uses to
/// decide an external value is "meaningfully different". Kept as the single
/// definition so a deferred apply is recognised by exactly the values that
/// were considered worth writing.
fn same_hsva(a: Hsva, b: Hsva) -> bool {
    (a.h - b.h).abs() <= 0.5
        && (a.s - b.s).abs() <= 0.005
        && (a.v - b.v).abs() <= 0.005
        && (a.a - b.a).abs() <= 0.005
}

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

        // Raised while the `value_fn` binding at the bottom of this function is
        // writing those four signals, so the coordinating effect can tell the
        // picker restoring itself from an author editing it.
        let applying_external = Rc::new(Cell::new(false));

        // The colour of the most recent external apply whose flush did NOT
        // land inside the `ApplyGuard` window. Normally the apply's batch
        // flushes before it returns (still inside the window) and this stays
        // unused — but when the apply's effect body runs while an ambient
        // `batch()` is open (component construction inside a caller's batch:
        // rinch-core batches nest since #232), the flush is deferred past the
        // guard's drop. The coordinating effect recognises the recorded colour
        // and stays silent about it — GH #229 must hold on that path too.
        let last_external_apply: Rc<Cell<Option<Hsva>>> = Rc::new(Cell::new(None));

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
                // One gesture position = one transition: batched, so every
                // observer (onchange included) sees saturation and value land
                // together, never a mixture of new saturation with stale value.
                batch(|| {
                    sat.set(px);
                    val.set(1.0 - py);
                });

                Drag::percent()
                    .on_move(move |px, py| {
                        batch(|| {
                            sat.set(px as f64);
                            val.set(1.0 - py as f64);
                        });
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

            // The last field text that parsed — what the field really holds
            // where this component can't see it. The desktop runtime mirrors
            // typed text into the `value` attribute before dispatching
            // `oninput`, but on web the browser owns the live text and the
            // attribute lags behind it, so the display effect below needs the
            // handler's own record to recognise the author's text. Cleared
            // whenever the effect actually rewrites the field.
            let typed: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

            {
                let typed = typed.clone();
                let handler_id = __scope.register_input_handler(move |value: String| {
                    if let Some(parsed) = parse_color(&value) {
                        *typed.borrow_mut() = Some(value);
                        // One typed colour = one transition: batched, so
                        // onchange reports the committed colour once, not once
                        // per component with mixtures in between.
                        batch(|| {
                            hue.set(parsed.h);
                            sat.set(parsed.s);
                            val.set(parsed.v);
                            alpha.set(parsed.a);
                        });
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
                    // Update hex input — unless its text already denotes this
                    // colour. A valid prefix mid-typing ("#336" on the way to
                    // "#3366cc") parses and lands here through the oninput
                    // handler; writing the normalized expansion ("#333366")
                    // back would replace the text under the author's caret,
                    // and every remaining keystroke would land on the
                    // rewritten string (GH #231). The field is the author's
                    // while its text and the picker agree on the colour; it is
                    // rewritten only when the colour moves away from it — a
                    // drag, a swatch, an external apply.
                    let next = format_color(hsv, color_format);
                    let field_agrees = hex_input
                        .get_attribute("value")
                        .is_some_and(|current| denotes_same(&current, &next, color_format))
                        || typed
                            .borrow()
                            .as_deref()
                            .is_some_and(|text| denotes_same(text, &next, color_format));
                    if !field_agrees {
                        typed.borrow_mut().take();
                        hex_input.set_attribute("value", &next);
                    }
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
                                // One swatch = one transition (see the hex
                                // handler above).
                                batch(|| {
                                    hue.set(parsed.h);
                                    sat.set(parsed.s);
                                    val.set(parsed.v);
                                    alpha.set(parsed.a);
                                });
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
        // Skip the initial run: onchange should report changes, not mount-time state
        // (the caller already knows — they seeded `value:`). Firing on mount can
        // re-enter `flush_effects` from inside `run_effect`'s borrow_mut and panic
        // when the parent is mid-re-render. See GH #23.
        //
        // An external apply is the same principle carried to its conclusion: the
        // caller knows every value it hands us, not just the first. The apply's
        // four writes are batched into one transition, so this effect sees only
        // the completed colour — but even that one coherent report would echo
        // state the caller handed us, and a consumer that stores what it hears
        // would re-enter the still-running apply with it. So an external apply
        // is atomic and silent, and only an author's act is reported. See
        // GH #229.
        if let Some(ref onchange) = self.onchange {
            let onchange = onchange.clone();
            let applying_external = applying_external.clone();
            let last_applied = last_external_apply.clone();
            let mut first_run = true;
            __scope.create_effect(move || {
                // Read all four before any early return, so this effect stays
                // subscribed to each of them whatever it decides to do.
                let hsv = Hsva {
                    h: hue.get(),
                    s: sat.get(),
                    v: val.get(),
                    a: alpha.get(),
                };
                if first_run {
                    first_run = false;
                    return;
                }
                if applying_external.get() {
                    // Observed inside the guard window — the deferred-apply
                    // marker below is not needed for this apply.
                    last_applied.set(None);
                    return;
                }
                if let Some(applied) = last_applied.take() {
                    if same_hsva(hsv, applied) {
                        // A *deferred* external apply: the flush ran after the
                        // ApplyGuard fell (the apply happened under an ambient
                        // batch), but this is still the colour the caller
                        // handed us — silent, per GH #229.
                        return;
                    }
                }
                onchange.invoke(format_color(hsv, color_format));
            });
        }

        // === value_fn binding: external value → internal signals ===
        if let Some(ref value_fn) = self.value_fn {
            let value_fn = value_fn.clone();
            let last_applied = last_external_apply.clone();
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
                    if !same_hsva(parsed, current) {
                        // These four writes are one apply: batched, so every
                        // observer runs once against the completed colour, and
                        // silent — the caller handed us this value. When this
                        // body runs during a flush, the batch below is a fresh
                        // outermost batch and flushes before it returns, still
                        // inside the guard's window; the guard drops on the
                        // way out of the block, including on unwind. When it
                        // runs while an ambient batch is open (creation run
                        // inside a caller's `batch()`), the flush is deferred
                        // past the guard's drop — the marker records the
                        // applied colour so the coordinating effect can stay
                        // silent about it when the flush finally lands.
                        last_applied.set(Some(parsed));
                        let _applying = ApplyGuard::raise(&applying_external);
                        batch(|| {
                            hue.set(parsed.h);
                            sat.set(parsed.s);
                            val.set(parsed.v);
                            alpha.set(parsed.a);
                        });
                    }
                }
            });
        }

        root
    }
}

#[cfg(test)]
mod apply_guard_tests {
    use super::*;

    /// The flag falls even when the apply unwinds.
    ///
    /// The batched writes an `ApplyGuard` spans flush effects before the batch
    /// returns, so a panic in any subscriber lands inside the guarded window.
    /// A flag left up there would mute `onchange` permanently. The panic this
    /// test provokes is deliberate — its message on stderr is expected output.
    #[test]
    fn the_flag_falls_when_an_apply_panics() {
        let flag = Rc::new(Cell::new(false));

        let raised = flag.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _applying = ApplyGuard::raise(&raised);
            assert!(raised.get(), "raised for the duration of the apply");
            panic!("a subscriber blew up mid-apply");
        }));

        assert!(outcome.is_err(), "the panic was not swallowed");
        assert!(!flag.get(), "and the picker is not left muted");
    }
}
