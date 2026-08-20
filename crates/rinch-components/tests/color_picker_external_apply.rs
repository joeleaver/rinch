//! ColorPicker: an external value arrives whole, and silently (#229).
//!
//! `value_fn` applies an incoming value by setting four independent signals in
//! sequence, and the coordinating effect that fires `onchange` observes all
//! four. Without a guard, every external apply emits `onchange` once per set —
//! each carrying a mixture of new and old components — and a consumer that
//! writes that emission back to the store `value_fn` reads re-enters the
//! still-running apply.
//!
//! These tests drive the real component headlessly: `MockDomDocument` +
//! `RenderScope` build the DOM, and the handler registry dispatches the same
//! click/input events the shell dispatches.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_picker::ColorPicker;
use rinch_components::color_utils::parse_color;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{
    ClickContext, EventHandlerId, dispatch_event, dispatch_input_event, set_click_context,
    update_drag,
};
use rinch_core::reactive::Effect;
use rinch_core::{Component, InputCallback, Signal};

/// Red at full saturation and value — the picker's own fallback, and a colour
/// no component of the target shares.
const START: &str = "#ff0000";
/// The colour this defect was measured with: H 266.7° / S 0.69 / V 0.87.
/// Arriving from [`START`], its mid-sequence mixtures are `#7100ff` (the new
/// hue against the old saturation and value) then `#9d4eff`.
const REMOTE: &str = "#8844dd";

/// Whether the consumer writes each emission back to the bound store.
///
/// `Echo::Back` is the production shape this defect was measured in: a
/// collaborative store that `value_fn` reads and `onchange` writes.
#[derive(Clone, Copy, PartialEq)]
enum Echo {
    Back,
    Never,
}

struct Picker {
    // These three are kept alive for the test's duration: the document owns the
    // nodes the effects patch, the scope owns the picker's effects and
    // handlers, and the recorder observes the store.
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    _recorder: Effect,
    root: NodeHandle,
    store: Signal<String>,
    emissions: Rc<RefCell<Vec<String>>>,
    published: Rc<RefCell<Vec<String>>>,
}

impl Picker {
    /// A picker seeded with `initial` and bound to a store holding it too — the
    /// well-formed call site.
    fn mount(initial: &str, echo: Echo) -> Self {
        Self::mount_with(initial, initial, echo)
    }

    /// A picker bound to a store but given no `value:` — the #229 call site.
    /// It mounts on its internal fallback (pure red) and must adopt the bound
    /// colour without reporting the fallback to anyone.
    fn mount_bound_only(stored: &str, echo: Echo) -> Self {
        Self::mount_with("", stored, echo)
    }

    fn mount_with(seed: &str, stored: &str, echo: Echo) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let store = Signal::new(stored.to_string());
        let emissions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

        // Every value the store ever holds — what a peer on the other end of a
        // collaborative document would receive. Registered before the picker so
        // it sees each write in order.
        let published: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let recorded = published.clone();
        let recorder = Effect::new(move || {
            let value = store.get();
            recorded.borrow_mut().push(value);
        });

        let seen = emissions.clone();
        let picker = ColorPicker {
            value: seed.to_string(),
            value_fn: Some(Rc::new(move || store.get())),
            onchange: Some(InputCallback::new(move |value: String| {
                seen.borrow_mut().push(value.clone());
                if echo == Echo::Back {
                    store.set(value);
                }
            })),
            alpha: true,
            with_input: true,
            swatches: vec!["#22aa55".into()],
            ..Default::default()
        };
        let root = picker.render(&mut scope, &[]);

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

    fn emissions(&self) -> Vec<String> {
        self.emissions.borrow().clone()
    }

    fn published(&self) -> Vec<String> {
        self.published.borrow().clone()
    }

    /// What the hex field shows — the picker's internal state, as an author reads it.
    fn displayed(&self) -> String {
        find_by_class(&self.root, "rinch-color-picker__hex-input")
            .expect("hex input")
            .get_attribute("value")
            .expect("hex input has a value")
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
}

fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    if node.get_attribute("class").as_deref() == Some(class) {
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

/// An external value change is not a user act, so it emits nothing.
///
/// Pre-fix this emitted once per signal the apply wrote — `#7100ff`, `#9d4eff`,
/// `#8844dd`, `#8844dd` — so two of the four colours it reported were mixtures
/// nobody chose.
#[test]
fn an_external_value_change_emits_no_onchange() {
    let picker = Picker::mount(START, Echo::Never);
    assert!(picker.emissions().is_empty(), "mount is not a change");

    picker.store.set(REMOTE.to_string());

    assert_eq!(
        picker.emissions(),
        Vec::<String>::new(),
        "an external apply must be silent: the caller already has this value"
    );
    assert_eq!(
        picker.displayed(),
        REMOTE,
        "and it must land whole — every component applied"
    );
}

/// The measured case: a peer's colour, with the consumer writing emissions back.
///
/// Pre-fix each mid-sequence emission was written to the store — so a peer's
/// document momentarily carried `#7100ff` and `#9d4eff`, mixtures of the
/// arriving hue with the local saturation and value, and each of those writes
/// re-entered the still-running `value_fn` effect. Convergence is not the test:
/// what the store *published* is, because every write is an edit peers see.
#[test]
fn a_peers_colour_arrives_without_publishing_mixtures() {
    let picker = Picker::mount(START, Echo::Back);

    picker.store.set(REMOTE.to_string());

    assert_eq!(
        picker.published(),
        vec![START.to_string(), REMOTE.to_string()],
        "the store must hold only what was authored: the seed, then the peer's colour"
    );
    assert_eq!(picker.displayed(), REMOTE);
    assert!(
        picker.emissions().is_empty(),
        "nothing was authored here, so nothing is reported: {:?}",
        picker.emissions()
    );
}

/// A picker given only `value_fn` adopts the bound colour at mount, and reports
/// nothing — least of all its own fallback.
///
/// This is #229's headline: pre-fix, mounting without `value:` reported the red
/// fallback blended with the arriving colour, so a consumer that stores what it
/// hears had its data rewritten by opening the picker.
#[test]
fn mounting_with_only_a_binding_adopts_it_silently() {
    let picker = Picker::mount_bound_only(REMOTE, Echo::Back);

    assert_eq!(
        picker.displayed(),
        REMOTE,
        "the bound colour is what the picker shows"
    );
    assert!(
        picker.emissions().is_empty(),
        "mounting is not an edit: {:?}",
        picker.emissions()
    );
    assert_eq!(
        picker.published(),
        vec![REMOTE.to_string()],
        "and the bound data is untouched"
    );
}

/// The guard is a window, not a state: an author's next act reports normally.
#[test]
fn a_user_act_after_an_external_apply_still_reports() {
    let picker = Picker::mount(START, Echo::Back);
    picker.store.set(REMOTE.to_string());

    let hex = picker.handler("rinch-color-picker__hex-input", "data-oninput");
    dispatch_input_event(hex, "#22aa55".to_string());

    assert_eq!(
        picker.emissions().last().map(String::as_str),
        Some("#22aa55"),
        "the picker is not muted by the apply that preceded this edit"
    );
    assert_eq!(picker.store.get(), "#22aa55");
}

/// A typed hex is a user act: it reports, and it commits to the store.
#[test]
fn a_hex_commit_reaches_the_consumer() {
    let picker = Picker::mount(START, Echo::Back);
    let hex = picker.handler("rinch-color-picker__hex-input", "data-oninput");

    dispatch_input_event(hex, REMOTE.to_string());

    assert_eq!(
        picker.emissions().last().map(String::as_str),
        Some(REMOTE),
        "the committed colour is what the consumer last heard"
    );
    assert_eq!(picker.store.get(), REMOTE);
    assert_eq!(picker.displayed(), REMOTE);
}

/// A swatch click is a user act: same contract as the hex field.
#[test]
fn a_swatch_click_reaches_the_consumer() {
    let picker = Picker::mount(START, Echo::Back);
    let swatch = find_by_class(&picker.root, "rinch-color-picker__swatches")
        .expect("swatches grid")
        .children()
        .first()
        .expect("one swatch")
        .clone();
    let id = EventHandlerId(
        swatch
            .get_attribute("data-rid")
            .expect("swatch is clickable")
            .parse()
            .expect("handler id is numeric"),
    );

    click_at(0.5, 0.5);
    dispatch_event(id);

    assert_eq!(
        picker.emissions().last().map(String::as_str),
        Some("#22aa55")
    );
    assert_eq!(picker.store.get(), "#22aa55");
}

/// A saturation drag reports every frame, and does not lose the hue it started
/// from. (Hue *is* re-derived from the round trip below s·v ≈ 0.235 — issue
/// #227, untouched here; this drag stays well above that.)
#[test]
fn a_saturation_drag_reports_each_frame_and_keeps_its_hue() {
    let picker = Picker::mount(REMOTE, Echo::Back);
    let overlay = picker.handler("rinch-color-picker__saturation-overlay", "data-rid");

    click_at(0.8, 0.1); // s = 0.8, v = 0.9
    dispatch_event(overlay);
    let after_press = picker.emissions().len();
    assert!(after_press > 0, "the press itself is a change");

    update_drag(120.0, 40.0); // s = 0.6, v = 0.8
    assert!(
        picker.emissions().len() > after_press,
        "each drag frame reports"
    );

    let last = picker.emissions().last().cloned().expect("an emission");
    assert_eq!(
        picker.store.get(),
        last,
        "the store holds what was reported"
    );
    assert_eq!(picker.displayed(), last, "and the field agrees");
    assert!(
        (hue_of(&last) - hue_of(REMOTE)).abs() < 1.0,
        "dragging saturation must not move the hue: {last}"
    );
}
