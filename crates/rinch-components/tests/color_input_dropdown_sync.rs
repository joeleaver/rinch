//! ColorInput: the dropdown picker follows `current_value` (#237).
//!
//! `ColorInput` mounts its dropdown `ColorPicker` once, seeded with the mount
//! colour. Pre-fix the picker had no `value_fn`, so its internal HSVA never
//! heard about anything that happened outside it: the author typed `#0000ff`
//! into a field mounted red, opened the dropdown — panel, thumbs and preview
//! still red — and nudged the saturation thumb one pixel: the picker emitted
//! a *red*-derived colour, the wrapper wrote it to `current_value`, and the
//! field was (legitimately, per its write-back guard) rewritten to a colour
//! nobody chose. An external `value_fn` change was equally invisible to it.
//!
//! The fix binds the picker's `value_fn` to `current_value`, riding the
//! #229/#227/#242 external-apply machinery: an inbound value applies
//! atomically and silently, and the picker's own emission — which the wrapper
//! writes back verbatim — folds as an echo. These tests read the picker's
//! thumbs (its internal degrees of freedom) and the wrapper's emissions;
//! the dropdown is `display: none` until opened but always mounted, so its
//! effects run without opening it.
//!
//! A second defect surfaced by the same wiring (#261 review note): the field
//! was *written* the raw external string but *judged* at the display format's
//! grid, so under `format: "hex"` a store speaking hsl could move the colour
//! (`hsl(200, 3%, 49%)` → `hsl(205, 3%, 49%)`, both `#797e81`) while the field
//! kept the stale spelling. The field is now a pure display-format view: what
//! it shows is the colour re-spelled in `format`, at mount and on every
//! rewrite.
//!
//! Harness note: as in `color_picker_hex_typing.rs`, the desktop runtime
//! mirrors typed text into the `value` attribute before dispatching `oninput`,
//! so `type_text` does the same; `commit` dispatches the #226 commit boundary.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_components::color_utils::parse_color;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{
    ClickContext, EventHandlerId, dispatch_event, dispatch_input_event, set_click_context,
    update_drag,
};
use rinch_core::reactive::Effect;
use rinch_core::{Component, InputCallback, Signal};

/// Red at full saturation and value — the picker's own fallback, and the
/// mount colour every scenario starts from unless it says otherwise.
const START: &str = "#ff0000";
/// Hue 240°: no channel in common with [`START`], so a stale-HSV emission is
/// unmistakable.
const BLUE: &str = "#0000ff";
/// A colour a peer would send: H 266.7° / S 0.69 / V 0.87.
const REMOTE: &str = "#8844dd";
/// A low-chroma hsl colour at which a 5° hue move does not change the 8-bit
/// rendering — both spell `#797e81` under `hex` (see #242).
const LOW_CHROMA: &str = "hsl(200, 3%, 49%)";
const LOW_CHROMA_MOVED: &str = "hsl(205, 3%, 49%)";
const LOW_CHROMA_HEX: &str = "#797e81";

/// Whether the consumer writes each emission back to the bound store — the
/// controlled-input shape.
#[derive(Clone, Copy, PartialEq)]
enum Echo {
    Back,
    Never,
}

struct Input {
    // Kept alive for the test's duration: the document owns the nodes the
    // effects patch, the scope owns the effects and handlers, and the recorder
    // observes the store.
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    _recorder: Option<Effect>,
    root: NodeHandle,
    store: Option<Signal<String>>,
    emissions: Rc<RefCell<Vec<String>>>,
    published: Rc<RefCell<Vec<String>>>,
}

impl Input {
    /// A ColorInput seeded with `value` and no `value_fn` — the uncontrolled
    /// call site, default (`hex`) format.
    fn unbound(value: &str) -> Self {
        Self::mount(value, "", None, Echo::Never)
    }

    /// A ColorInput seeded with `seed` and bound to a fresh store holding
    /// `stored`, displaying in `format`.
    fn bound(seed: &str, stored: &str, format: &str, echo: Echo) -> Self {
        Self::mount(seed, format, Some(Signal::new(stored.to_string())), echo)
    }

    fn mount(seed: &str, format: &str, store: Option<Signal<String>>, echo: Echo) -> Self {
        // The recorder is registered before the component's effects, so it
        // sees every write to the store in order.
        let (published, recorder) = match store {
            Some(store) => {
                let (published, recorder) = record_store(store);
                (published, Some(recorder))
            }
            None => (Rc::new(RefCell::new(Vec::new())), None),
        };

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let emissions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let seen = emissions.clone();
        let input = ColorInput {
            format: format.to_string(),
            value: seed.to_string(),
            value_fn: store.map(|store| -> Rc<dyn Fn() -> String> { Rc::new(move || store.get()) }),
            onchange: Some(InputCallback::new(move |value: String| {
                seen.borrow_mut().push(value.clone());
                if echo == Echo::Back
                    && let Some(store) = store
                {
                    store.set(value);
                }
            })),
            swatches: vec!["#22aa55".into()],
            ..Default::default()
        };
        let root = input.render(&mut scope, &[]);

        Self {
            _doc: doc,
            _scope: scope,
            _recorder: recorder,
            root,
            store,
            emissions,
            published,
        }
    }

    fn store(&self) -> Signal<String> {
        self.store.expect("this input is bound to a store")
    }

    fn emissions(&self) -> Vec<String> {
        self.emissions.borrow().clone()
    }

    fn published(&self) -> Vec<String> {
        self.published.borrow().clone()
    }

    fn field(&self) -> NodeHandle {
        find_by_class(&self.root, "rinch-color-input__input").expect("the text field exists")
    }

    fn field_text(&self) -> String {
        self.field()
            .get_attribute("value")
            .expect("the field has a value")
    }

    fn handler(&self, class: &str, attr: &str) -> EventHandlerId {
        let node = find_by_class(&self.root, class).expect("element exists");
        EventHandlerId(
            node.get_attribute(attr)
                .expect("element carries a handler id")
                .parse()
                .expect("handler id is numeric"),
        )
    }

    /// One keystroke, desktop-shaped: the runtime mirrors the field's text
    /// into the `value` attribute, then dispatches `oninput` with it.
    fn type_text(&self, text: &str) {
        self.field().set_attribute("value", text);
        dispatch_input_event(
            self.handler("rinch-color-input__input", "data-oninput"),
            text.to_string(),
        );
    }

    /// The commit boundary (#226): `data-onchange` with the final text when
    /// the gesture ends (blur after modification, Enter).
    fn commit(&self, text: &str) {
        dispatch_input_event(
            self.handler("rinch-color-input__input", "data-onchange"),
            text.to_string(),
        );
    }

    /// Press the dropdown picker's saturation panel at (`px`, `py`) of its
    /// area — s = px, v = 1 − py — starting a drag `update_drag` can continue.
    fn press_saturation(&self, px: f32, py: f32) {
        click_at(px, py);
        dispatch_event(self.handler("rinch-color-picker__saturation-overlay", "data-rid"));
    }

    /// The style attribute of a thumb in the dropdown picker — where the
    /// picker says a degree of freedom currently sits.
    fn thumb_style(&self, class: &str) -> String {
        find_by_class(&self.root, class)
            .expect("thumb exists")
            .get_attribute("style")
            .expect("thumb is positioned")
    }

    /// The hue thumb's position, in percent of the slider (h / 360 · 100).
    fn hue_thumb(&self) -> f64 {
        percent_of(&self.thumb_style("rinch-color-picker__hue-thumb"), "left: ")
    }

    /// The saturation-panel thumb's (left, top) in percent: (s · 100, (1 − v) · 100).
    fn sat_thumb(&self) -> (f64, f64) {
        let style = self.thumb_style("rinch-color-picker__thumb");
        (percent_of(&style, "left: "), percent_of(&style, "top: "))
    }

    /// Assert the dropdown picker's thumbs sit at `colour`'s HSV.
    #[track_caller]
    fn assert_picker_at(&self, colour: &str, why: &str) {
        let hsv = parse_color(colour).expect("a formatted colour parses");
        let hue = self.hue_thumb();
        let (left, top) = self.sat_thumb();
        assert!(
            (hue - hsv.h / 3.6).abs() < 0.01,
            "{why}: hue thumb at {hue}%, expected {}% ({colour})",
            hsv.h / 3.6
        );
        assert!(
            (left - hsv.s * 100.0).abs() < 0.01 && (top - (1.0 - hsv.v) * 100.0).abs() < 0.01,
            "{why}: saturation thumb at left {left}% top {top}%, expected left {}% top {}% ({colour})",
            hsv.s * 100.0,
            (1.0 - hsv.v) * 100.0
        );
    }
}

/// Record every value `store` ever holds — what a peer on the other end of a
/// controlled binding would receive — returning the log and the effect that
/// keeps it.
fn record_store(store: Signal<String>) -> (Rc<RefCell<Vec<String>>>, Effect) {
    let published: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let recorded = published.clone();
    let recorder = Effect::new(move || {
        let value = store.get();
        recorded.borrow_mut().push(value);
    });
    (published, recorder)
}

fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    let matches = node
        .get_attribute("class")
        .is_some_and(|attr| attr.split_whitespace().any(|c| c == class));
    if matches {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_by_class(c, class))
}

/// A click at (`px`, `py`) of a 200×200 element at the origin.
fn click_at(px: f32, py: f32) {
    set_click_context(ClickContext {
        mouse_x: px * 200.0,
        mouse_y: py * 200.0,
        element_x: 0.0,
        element_y: 0.0,
        element_width: 200.0,
        element_height: 200.0,
        ..Default::default()
    });
}

fn hue_of(color: &str) -> f64 {
    parse_color(color).expect("a formatted colour parses").h
}

/// The `key`-prefixed percentage in a thumb's style string, e.g.
/// `percent_of(&style, "left: ")`. Click-derived positions carry f32→f64
/// noise, so callers compare within a tolerance.
fn percent_of(style: &str, key: &str) -> f64 {
    let start = style.find(key).expect("style carries the key") + key.len();
    let rest = &style[start..];
    let end = rest.find('%').expect("a % terminates the value");
    rest[..end].trim().parse().expect("the value is numeric")
}

/// Typing a whole colour moves the dropdown picker to it.
///
/// Pre-fix the picker never heard about the field: its thumbs stayed at the
/// mount colour (hue 0%) however the text changed.
#[test]
fn a_typed_colour_reaches_the_dropdown_picker() {
    let input = Input::unbound(START);
    input.assert_picker_at(START, "mount");

    input.type_text(BLUE);

    input.assert_picker_at(BLUE, "the picker must follow the typed colour");
    assert_eq!(
        input.field_text(),
        BLUE,
        "the field is still the author's (#231)"
    );
    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "typing previews; nothing reports before the commit boundary (#226)"
    );
}

/// An external `value_fn` change moves the dropdown picker too.
#[test]
fn an_external_value_change_reaches_the_dropdown_picker() {
    let input = Input::bound(START, START, "", Echo::Never);

    input.store().set(BLUE.to_string());

    input.assert_picker_at(BLUE, "the picker must follow the bound store");
    assert_eq!(input.field_text(), BLUE, "and so does the field");
    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "an external apply is silent (#229)"
    );
}

/// The headline: a slider nudge after typing derives from the typed colour.
///
/// Pre-fix the picker still held the mount red, so the nudge emitted a
/// red-derived colour (`#e62e2e`, hue 0), the wrapper wrote it to
/// `current_value`, and the field was rewritten to a colour nobody chose.
#[test]
fn a_slider_nudge_after_typing_derives_from_the_typed_colour() {
    let input = Input::unbound(START);
    input.type_text(BLUE);
    input.commit(BLUE);
    assert_eq!(
        input.emissions(),
        vec![BLUE.to_string()],
        "the commit reports once"
    );

    input.press_saturation(0.8, 0.1); // s = 0.8, v = 0.9

    let emissions = input.emissions();
    assert_eq!(
        emissions.len(),
        2,
        "the press is exactly one more change: {emissions:?}"
    );
    let nudged = emissions.last().cloned().expect("an emission");
    assert!(
        (hue_of(&nudged) - hue_of(BLUE)).abs() < 1.0,
        "the nudge must be built on the typed blue, not the stale mount red: {nudged}"
    );
    assert_eq!(
        input.field_text(),
        nudged,
        "the field shows the colour the author actually made"
    );
}

/// The #229 silence contract holds across the wrapper boundary: a controlled
/// store's external change applies to the dropdown picker without the wrapper
/// reporting anything, so an echoing consumer's store holds only what was
/// authored.
#[test]
fn an_external_colour_arrives_through_the_wrapper_silently() {
    let input = Input::bound(START, START, "", Echo::Back);

    input.store().set(REMOTE.to_string());

    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "nothing was authored here, so nothing is reported"
    );
    assert_eq!(
        input.published(),
        vec![START.to_string(), REMOTE.to_string()],
        "the store holds only the seed and the peer's colour — no echo, no mixture"
    );
    assert_eq!(input.field_text(), REMOTE);
    input.assert_picker_at(REMOTE, "the dropdown picker adopted the external colour");
}

/// The anti-loop proof: with an echoing store, every drag frame's emission
/// comes back through `current_value` into the picker's `value_fn`, and must
/// fold as the picker's own echo — never re-apply as a foreign colour and
/// revert the frame. This pins the registration order the fold depends on
/// (the picker's coordinating effect emits before its `value_fn` effect
/// re-reads the store): reversed, the gate would see the stale store first
/// and snap every local act back.
#[test]
fn a_saturation_drag_is_not_reverted_by_its_own_echo() {
    let input = Input::bound(REMOTE, REMOTE, "", Echo::Back);

    input.press_saturation(0.8, 0.1); // s = 0.8, v = 0.9
    update_drag(120.0, 40.0); // s = 0.6, v = 0.8
    update_drag(100.0, 60.0); // s = 0.5, v = 0.7
    update_drag(80.0, 80.0); // s = 0.4, v = 0.6

    let emissions = input.emissions();
    assert_eq!(
        emissions.len(),
        4,
        "the press and each drag frame report exactly once: {emissions:?}"
    );
    for emitted in &emissions {
        assert!(
            (hue_of(emitted) - hue_of(REMOTE)).abs() < 1.0,
            "a saturation drag never moves the hue: {emitted}"
        );
    }
    let last = emissions.last().cloned().expect("an emission");
    assert_eq!(
        input.store().get(),
        last,
        "the store holds what was reported"
    );
    assert_eq!(input.field_text(), last, "and the field agrees");
    let (left, top) = input.sat_thumb();
    assert!(
        (left - 40.0).abs() < 0.01 && (top - 40.0).abs() < 0.01,
        "the thumb sits where the last frame put it, not where an echo re-applied it: \
         left {left}% top {top}%"
    );
    let mut expected = vec![REMOTE.to_string()];
    expected.extend(emissions.iter().cloned());
    assert_eq!(
        input.published(),
        expected,
        "the store saw the seed and one write per frame — nothing else"
    );
}

/// The #261 review note: under `format: "hex"` a store speaking hsl moves the
/// colour by a hue delta the 8-bit rendering cannot show. The picker must
/// still follow (the apply gate judges at the hsl wire's grid, #242), and the
/// field — a display-format view — shows `#797e81` throughout: pre-fix it
/// was written the raw hsl string at mount and then kept the stale
/// `hsl(200, …)` spelling while the store held `hsl(205, …)`.
#[test]
fn the_field_and_the_thumbs_follow_an_hsl_store_under_a_hex_display() {
    let input = Input::bound(LOW_CHROMA, LOW_CHROMA, "hex", Echo::Never);
    assert_eq!(
        input.field_text(),
        LOW_CHROMA_HEX,
        "the field shows the mount colour in the output format"
    );
    let hue = input.hue_thumb();
    assert!(
        (hue - 200.0 / 3.6).abs() < 0.01,
        "the picker mounts on the stated 200°: {hue}%"
    );

    input.store().set(LOW_CHROMA_MOVED.to_string());

    let hue = input.hue_thumb();
    assert!(
        (hue - 205.0 / 3.6).abs() < 0.01,
        "the peer's 205° must reach the dropdown picker: {hue}%"
    );
    assert_eq!(
        input.field_text(),
        LOW_CHROMA_HEX,
        "the field still shows the colour in the output format"
    );
    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "an external apply is silent"
    );
}

/// An external value is displayed in the output format's spelling, not
/// verbatim: a store handing `"red"` to a `hex` input shows `#ff0000`.
#[test]
fn an_external_value_is_shown_in_the_output_format() {
    let input = Input::bound(BLUE, BLUE, "hex", Echo::Never);

    input.store().set("red".to_string());

    assert_eq!(
        input.field_text(),
        "#ff0000",
        "the field is a display-format view of the colour"
    );
    input.assert_picker_at("#ff0000", "the picker followed the named colour");
    assert_eq!(input.emissions(), Vec::<String>::new());
}

/// The #231 guard on the new coupling: a parseable prefix moves the picker
/// (the preview is live) but the field keeps the author's text — the picker's
/// silent apply must not come back around as a rewrite under the caret.
#[test]
fn typing_moves_the_picker_without_rewriting_the_field() {
    let input = Input::unbound(START);

    input.type_text("#336");

    input.assert_picker_at("#333366", "the prefix previews in the dropdown picker");
    assert_eq!(
        input.field_text(),
        "#336",
        "the field must still hold exactly what the author has typed"
    );
    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "a silent apply reports nothing, and typing reports only on commit"
    );
}

/// The `value: ""` seed: the wrapper falls back to black while the picker
/// used to mount on its own internal red — two components disagreeing about
/// the colour before anything happened.
#[test]
fn an_unseeded_input_and_its_picker_agree_on_black() {
    let input = Input::unbound("");

    assert_eq!(input.field_text(), "#000000");
    input.assert_picker_at("#000000", "the picker mounts on the wrapper's fallback");
    assert_eq!(input.emissions(), Vec::<String>::new());
}
