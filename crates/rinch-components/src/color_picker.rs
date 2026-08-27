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
    ColorFormat, Hsva, Notation, denotes_emitted, format_color, hsv_to_rgb, hue_to_rgb_hex,
    parse_color, parse_color_with_notation, rgb_to_hex, text_denotes,
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

/// Merge a genuinely-foreign parsed colour into the picker's current state,
/// keeping the degrees of freedom the value cannot carry (GH #227 part B):
/// a grey keeps the current hue, a black keeps hue and saturation, so
/// dragging back out of grey resumes the colour the author was working from.
///
/// Carryability is judged at 8-bit precision: a value whose *rendering* is
/// grey carries no usable hue — an `rgb()` written with fractional
/// near-equal channels parses to a microscopic saturation whose derived hue
/// is quantization noise, not intent — and a rendered black carries neither
/// hue nor saturation. One exception adopts instead of keeping: an `hsl()`
/// string *states* its hue outright, whatever its chroma, so a rendered grey
/// written in the hsl notation carries the hue it names — `hsl(240, 0%, 50%)`,
/// the sub-percent `hsl(205, 0.3%, 49%)` (GH #242), and `hsl(0, 0%, 50%)`
/// alike. The notation is the tell, not the parse: RGB-family greys parse to
/// hue exactly 0.0 by convention (`rgb_to_hsv`'s `delta == 0` arm) and never
/// reach this exception, so hue 0 needs no carve-out. One used to keep
/// `hsl(0, 0%, l)` reading as the convention; it only ever bit genuine hsl
/// emissions, putting a 1°-wide dead band at red on an hsl wire (a peer's
/// move to 0° at grey was kept out, then reverted by the next local act —
/// the #242 shape) and making the apply non-idempotent (the kept hue
/// re-formats as `hsl(267, 0%, l)`, which never echoes the store's
/// `hsl(0, 0%, l)`, so every effect run re-applied). The cost is disclosed:
/// a store that re-spells an RGB grey as hsl writes hue 0, and the picker
/// adopts it.
fn merge_unrepresentable(parsed: Hsva, notation: Notation, current: Hsva) -> Hsva {
    let rendered = hsv_to_rgb(parsed);
    let level = |c: f64| (c * 255.0).round() as u8;
    let (r, g, b) = (level(rendered.r), level(rendered.g), level(rendered.b));
    let grey = r == g && g == b;
    let black = grey && r == 0;
    let hue_stated = notation == Notation::Hsl;
    Hsva {
        h: if grey && !hue_stated {
            current.h
        } else {
            parsed.h
        },
        s: if black { current.s } else { parsed.s },
        ..parsed
    }
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
    /// Show alpha slider. Off unless set (`#[derive(Default)]`: false) —
    /// `component-props.md` documents the real default.
    pub alpha: bool,
    /// Preset swatch colors.
    pub swatches: Vec<String>,
    /// Number of swatches per row. Defaults to 7.
    pub swatches_per_row: Option<usize>,
    /// Size: xs, sm, md, lg, xl.
    pub size: String,
    /// Show hex text input. Off unless set (`#[derive(Default)]`: false).
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

            // The field's live text, as this component last heard it: every
            // `oninput` records here — parseable or not, because a record
            // that survives an edit it no longer describes would later veto
            // a legitimate rewrite (type "#336", backspace to "#33", click a
            // #333366 swatch: a parse-gated record still says "#336" and the
            // field would stay stuck at "#33"). The display effect prefers
            // this record over the `value` attribute: on desktop both track
            // the live text, but on web the attribute holds only what was
            // last *written* programmatically — during typing it is a fossil
            // that must not speak for the field. Cleared whenever the effect
            // rewrites the field (the write becomes the live text on both
            // backends).
            let typed: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

            {
                let typed = typed.clone();
                let handler_id = __scope.register_input_handler(move |value: String| {
                    let parsed = parse_color(&value);
                    *typed.borrow_mut() = Some(value);
                    if let Some(parsed) = parsed {
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

            // The commit boundary (issue #226): the typed gesture ended, so
            // the author's mid-typing claim to the field (GH #231) lapses.
            // Normalize a parseable commit to the canonical form ("336" →
            // "#333366") — the #231 guard protects the author's text only
            // while the gesture is live, and a committed shorthand left in
            // the field would mislead any attribute-reading consumer (#235's
            // residual). An unparseable commit reverts to the color the
            // picker still holds. Either rewrite clears the typed record,
            // like any effect rewrite. The signals are NOT touched here: every
            // parseable state already landed through `oninput`, and re-setting
            // them would re-notify the onchange coordinating effect.
            {
                let typed = typed.clone();
                let hex_input_commit = hex_input.clone();
                let handler_id = __scope.register_input_handler(move |value: String| {
                    let committed = match parse_color(&value) {
                        Some(parsed) => format_color(parsed, color_format),
                        None => format_color(
                            Hsva {
                                h: hue.get(),
                                s: sat.get(),
                                v: val.get(),
                                a: alpha.get(),
                            },
                            color_format,
                        ),
                    };
                    *typed.borrow_mut() = None;
                    hex_input_commit.set_attribute("value", &committed);
                });
                hex_input.set_attribute("data-onchange", &handler_id.to_string());
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
                    //
                    // "The field's text" is the `typed` record when one
                    // exists (the live text on both backends), else the
                    // `value` attribute (live only until the first keystroke
                    // on web — but `typed` covers from then on). Agreement is
                    // `text == next` (the steady state: the string this
                    // effect last wrote) or `text_denotes` — the full colour,
                    // alpha included, so a typed "#3333666c" survives while
                    // the picker really holds that alpha, yet an alpha-slider
                    // move under a hex format still rewrites it.
                    let next = format_color(hsv, color_format);
                    let field_text = typed
                        .borrow()
                        .clone()
                        .or_else(|| hex_input.get_attribute("value"));
                    let field_agrees = field_text
                        .is_some_and(|text| text.trim() == next || text_denotes(&text, hsv));
                    if !field_agrees {
                        *typed.borrow_mut() = None;
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
                    // A *deferred* external apply: the flush ran after the
                    // ApplyGuard fell (the apply happened under an ambient
                    // batch), but this is still the colour the caller handed
                    // us — silent, per GH #229. Exact equality is the right
                    // recognition (GH #227/#229): the marker holds what the
                    // apply actually *wrote* — the merged colour, kept
                    // channels included, never the raw parse — and no
                    // arithmetic sits between those signal writes and this
                    // read, so the caller's apply flushes back bit-identical,
                    // while anything an author changed in between fails the
                    // match and reports below, as it must.
                    if hsv == applied {
                        return;
                    }
                }
                onchange.invoke(format_color(hsv, color_format));
            });
        }

        // === value_fn binding: external value → internal signals ===
        //
        // Registered AFTER the coordinating effect above, and that order is
        // load-bearing: this effect also reads the four colour signals, so on
        // an author's act both are pending on the same write, and effects run
        // in registration order (#154). The coordinating effect emits first;
        // a consumer that writes the emission back into the bound value (as
        // `ColorInput` does) has done so by the time this effect reads it, so
        // the gate below sees the echo and folds it. Reversed, this effect
        // would read the stale bound value and re-apply it over every local
        // act — pinned by `tests/color_input_dropdown_sync.rs`.
        if let Some(ref value_fn) = self.value_fn {
            let value_fn = value_fn.clone();
            let last_applied = last_external_apply.clone();
            __scope.create_effect(move || {
                let external = value_fn();
                // Only a parseable external value can apply: garbage and
                // half-typed text change nothing.
                if let Some((parsed, notation)) = parse_color_with_notation(&external) {
                    let current = Hsva {
                        h: hue.get(),
                        s: sat.get(),
                        v: val.get(),
                        a: alpha.get(),
                    };
                    // Apply only a genuinely foreign value — never the round
                    // trip of this picker's own state (GH #227): every
                    // serializer quantizes, so the parse of an echoed
                    // emission routinely lands measurably off the signals it
                    // was formatted from, and per-channel epsilons once
                    // mistook that drift for an external change. The external
                    // value is "self" when it denotes the colour the picker
                    // holds (`text_denotes`, with the parse already in hand),
                    // or the colour the picker currently *emits*: a display
                    // format that drops alpha makes the emission legitimately
                    // differ from the held colour in alpha alone, so an alpha
                    // drag under `Hex` echoes back opaque and only the second
                    // comparison recognises it (which is also the only case
                    // where it differs from the first).
                    //
                    // Denotation is judged at the resolution of the picker's
                    // own emission in the notation the external value is
                    // written in (GH #242, `denotes_emitted`) — never the
                    // display format itself, which would erase an inbound
                    // alpha-only change. Judging every notation at 8-bit RGB
                    // folded a peer's genuine low-chroma hue move on an hsl
                    // wire, and the picker's next local act re-emitted the
                    // stale hue over the peer's. The residual is symmetric: a
                    // difference the inbound notation's grid cannot spell
                    // folds, because such a value is indistinguishable from a
                    // normalizing store's re-spelling of the emission. Under
                    // an alpha-dropping format that includes "rgba(r, g, b,
                    // 1)" restating the emission — alpha is externally
                    // drivable under the formats that carry it.
                    let held = format_color(current, notation.with_alpha());
                    let echoes_self = denotes_emitted(parsed, notation, &held)
                        || (color_format != notation.with_alpha()
                            && denotes_emitted(
                                parsed,
                                notation,
                                &format_color(current, color_format),
                            ));
                    if !echoes_self {
                        // A foreign value still cannot carry every degree of
                        // freedom — keep the channels it cannot express
                        // rather than adopting fabricated ones; see
                        // `merge_unrepresentable`.
                        let applied = merge_unrepresentable(parsed, notation, current);
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
                        // silent about it when the flush finally lands. The
                        // marker holds `applied` — what the batch writes,
                        // kept channels included — never the raw parse, so
                        // the coordinating effect can recognise the flush by
                        // exact equality.
                        last_applied.set(Some(applied));
                        let _applying = ApplyGuard::raise(&applying_external);
                        batch(|| {
                            hue.set(applied.h);
                            sat.set(applied.s);
                            val.set(applied.v);
                            alpha.set(applied.a);
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

#[cfg(test)]
mod merge_tests {
    use super::*;

    /// What the merge writes is a fixed point of the gate: applied once, the
    /// store's text reads as self on the next run of the `value_fn` effect.
    /// The hue-0 carve-out broke this for `hsl(0, 0%, l)` — the kept hue
    /// re-formatted as `hsl(267, 0%, 50%)`, never the store's spelling, so
    /// every run re-applied.
    #[test]
    fn an_applied_hsl_grey_echoes_on_the_next_run() {
        let current = parse_color("#8844dd").expect("a colour"); // h ≈ 266.7
        for external in [
            "hsl(0, 0%, 50%)",
            "hsl(240, 0%, 50%)",
            "hsl(205, 0.3%, 49%)",
        ] {
            let (parsed, notation) = parse_color_with_notation(external).expect("a colour");
            assert!(!text_denotes(external, current), "{external} is foreign");
            let applied = merge_unrepresentable(parsed, notation, current);
            assert_eq!(applied.h, parsed.h, "{external} states its hue");
            assert!(
                text_denotes(external, applied),
                "{external} is self once applied — the apply does not repeat"
            );
        }
    }

    /// The RGB-family arms never state a hue: a grey keeps the current one,
    /// a black keeps saturation too, whatever the parse's noise says.
    #[test]
    fn rgb_family_greys_keep_the_current_hue() {
        let current = parse_color("#8844dd").expect("a colour");
        for external in [
            "#808080",
            "gray",
            "rgb(127.9999999999, 128, 128)",
            "#000000",
        ] {
            let (parsed, notation) = parse_color_with_notation(external).expect("a colour");
            let applied = merge_unrepresentable(parsed, notation, current);
            assert_eq!(applied.h, current.h, "{external} carries no hue");
            assert!(
                text_denotes(external, applied),
                "{external} is self once applied"
            );
        }
    }
}
