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

    /// The style attribute of a thumb element — where the picker says a
    /// degree of freedom currently sits. The #227 tests assert on thumbs (or
    /// emissions), never on the hex field: the field shows the round trip,
    /// which hides hue/sat loss at any s > 0.
    fn thumb_style(&self, class: &str) -> String {
        find_by_class(&self.root, class)
            .expect("thumb exists")
            .get_attribute("style")
            .expect("thumb is positioned")
    }

    /// Type `text` into the hex field the way the runtime delivers it: the
    /// field's text is mirrored into the `value` attribute *before* `oninput`
    /// dispatches. The #231 write-back guard reads that attribute — the field
    /// is the author's while its text denotes the colour the picker holds.
    fn type_hex(&self, text: &str) {
        find_by_class(&self.root, "rinch-color-picker__hex-input")
            .expect("hex input")
            .set_attribute("value", text);
        dispatch_input_event(
            self.handler("rinch-color-picker__hex-input", "data-oninput"),
            text.to_string(),
        );
    }
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

fn sat_of(color: &str) -> f64 {
    parse_color(color).expect("a formatted colour parses").s
}

/// The `key`-prefixed percentage in a thumb's style string, e.g.
/// `percent_of(&style, "left: ")`. Click-derived positions carry f32→f64
/// noise ("left: 40.000000596%"), so callers compare within a tolerance.
fn percent_of(style: &str, key: &str) -> f64 {
    let start = style.find(key).expect("style carries the key") + key.len();
    style[start..]
        .split('%')
        .next()
        .expect("a % terminates the value")
        .trim()
        .parse()
        .expect("the value is numeric")
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

/// The mount-adoption stays silent even when the whole mount happens inside an
/// ambient `batch()`.
///
/// Batches nest (#232): the apply's own inner `batch()` joins the caller's
/// transaction, so its flush lands only at the caller's batch exit — *after*
/// the `ApplyGuard` has fallen. The deferred-apply marker keeps #229's
/// contract on that path: the adopted colour was handed to us, so it is not
/// reported, and an echoing consumer's store is untouched.
#[test]
fn mounting_inside_an_ambient_batch_adopts_silently_too() {
    let picker = rinch_core::batch(|| Picker::mount_bound_only(REMOTE, Echo::Back));

    assert_eq!(
        picker.displayed(),
        REMOTE,
        "the bound colour is what the picker shows"
    );
    assert!(
        picker.emissions().is_empty(),
        "mounting is not an edit, batched or not: {:?}",
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

    picker.type_hex("#22aa55");

    assert_eq!(
        picker.emissions(),
        vec!["#22aa55".to_string()],
        "the picker is not muted by the apply that preceded this edit — and \
         the silent apply emitted nothing"
    );
    assert_eq!(picker.store.get(), "#22aa55");
}

/// A typed hex is a user act: it reports once, whole, and commits to the store.
///
/// One commit is one transition (the four component writes are batched), so
/// exactly one colour is reported — never the per-component mixtures
/// (`#7100ff`, `#9d4eff`) an unbatched sequence would leak to the consumer.
#[test]
fn a_hex_commit_reaches_the_consumer() {
    let picker = Picker::mount(START, Echo::Back);

    picker.type_hex(REMOTE);

    assert_eq!(
        picker.emissions(),
        vec![REMOTE.to_string()],
        "one commit reports once, with the completed colour"
    );
    assert_eq!(
        picker.published(),
        vec![START.to_string(), REMOTE.to_string()],
        "and the store never held a colour nobody typed"
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
        picker.emissions(),
        vec!["#22aa55".to_string()],
        "one click reports once, with the swatch's colour"
    );
    assert_eq!(picker.store.get(), "#22aa55");
}

/// A saturation drag reports every frame, and does not lose the hue it started
/// from. (This drag stays above s·v ≈ 0.235, where even the pre-#227 epsilon
/// gate survived its own round trip; the #227 tests below cover the region
/// underneath.)
#[test]
fn a_saturation_drag_reports_each_frame_and_keeps_its_hue() {
    let picker = Picker::mount(REMOTE, Echo::Back);
    let overlay = picker.handler("rinch-color-picker__saturation-overlay", "data-rid");

    click_at(0.8, 0.1); // s = 0.8, v = 0.9
    dispatch_event(overlay);
    assert_eq!(
        picker.emissions().len(),
        1,
        "the press is one change: saturation and value land together"
    );

    update_drag(120.0, 40.0); // s = 0.6, v = 0.8
    assert_eq!(
        picker.emissions().len(),
        2,
        "each drag frame reports exactly once"
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

// === #227: the picker's state survives its own round trip ===
//
// `format_color` quantizes to 8-bit RGB, and `rgb_to_hsv` amplifies that
// quantization by 60/(s·v): the round trip of the picker's own emission
// routinely differs from the internal state it was formatted from (mean hue
// error ≈ 0.072/(s·v) degrees against a 0.5° epsilon; at s = 0 the round
// trip returns hue exactly 0). Pre-fix, the `value_fn` gate compared that
// round trip against the live signals with per-channel epsilons and
// "corrected" the picker with its own echo. These tests assert the internal
// degrees of freedom — thumbs and emissions — never the hex field, which
// shows the round trip and therefore hides the loss.

/// The headline (#227): drag saturation to grey and back — the hue survives.
///
/// Pre-fix, the grey emission `#808080` round-trips to hue 0, the gate calls
/// that "meaningfully different", and the apply snaps the hue thumb to red;
/// re-approaching saturation then picks from red, observed emission
/// `#801a1a` (hue 0) instead of a violet (hue ≈ 266.7).
#[test]
fn dragging_saturation_to_grey_and_back_keeps_the_hue() {
    let picker = Picker::mount(REMOTE, Echo::Back); // h ≈ 266.7
    let overlay = picker.handler("rinch-color-picker__saturation-overlay", "data-rid");

    click_at(0.0, 0.5); // s = 0, v = 0.5 — the emission is a pure grey
    dispatch_event(overlay);

    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - hue_of(REMOTE) / 3.6).abs() < 0.01,
        "the hue thumb must hold at grey, not snap to red: left {hue_left}%"
    );

    update_drag(160.0, 100.0); // s = 0.8 — re-approach saturation
    let last = picker.emissions().last().cloned().expect("an emission");
    assert!(
        (hue_of(&last) - hue_of(REMOTE)).abs() < 1.0,
        "re-approaching saturation must resume the hue the drag started from: {last}"
    );
}

/// The everyday case: one click in the dark/desaturated region, echoed back.
///
/// At s = 0.3, v = 0.2 a single round trip moves the hue by ≈ 1.3° and the
/// saturation by ≈ 0.006 — both past the pre-fix epsilons in one frame, so
/// the thumbs drifted off the point the author had just clicked.
#[test]
fn a_dark_desaturated_click_survives_its_own_echo() {
    let picker = Picker::mount(REMOTE, Echo::Back);
    let overlay = picker.handler("rinch-color-picker__saturation-overlay", "data-rid");

    click_at(0.3, 0.8); // s = 0.3, v = 0.2
    dispatch_event(overlay);

    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - hue_of(REMOTE) / 3.6).abs() < 0.05,
        "the hue is not the echo's to move: left {hue_left}%"
    );
    let sat_left = percent_of(&picker.thumb_style("rinch-color-picker__thumb"), "left: ");
    assert!(
        (sat_left - 30.0).abs() < 0.05,
        "the saturation stays where it was clicked: left {sat_left}%"
    );
}

/// The alpha slider works under the default Hex format with an echoing store.
///
/// Hex drops alpha, so the echo always parses opaque; pre-fix the gate read
/// `|1.0 − a| > 0.005` as an external change and re-applied opaque on every
/// frame — the thumb snapped back to 100% as it was dragged.
#[test]
fn an_alpha_drag_under_the_hex_format_keeps_its_alpha() {
    let picker = Picker::mount(REMOTE, Echo::Back); // default format: Hex
    let overlay = picker.handler("rinch-color-picker__alpha-overlay", "data-rid");

    click_at(0.4, 0.5);
    dispatch_event(overlay);
    let left = percent_of(
        &picker.thumb_style("rinch-color-picker__alpha-thumb"),
        "left: ",
    );
    assert!(
        (left - 40.0).abs() < 0.01,
        "the alpha thumb holds where it was pressed: left {left}%"
    );

    update_drag(120.0, 100.0); // alpha = 0.6
    let left = percent_of(
        &picker.thumb_style("rinch-color-picker__alpha-thumb"),
        "left: ",
    );
    assert!(
        (left - 60.0).abs() < 0.01,
        "and follows the drag instead of snapping opaque: left {left}%"
    );
}

/// A genuinely foreign grey applies — and keeps the hue it cannot carry.
///
/// A grey denotes no hue (`rgb_to_hsv` returns 0 by convention), so adopting
/// the parse would fabricate red. The apply keeps the current hue, stays
/// silent (#229), and a later saturation click resumes from the kept hue.
#[test]
fn a_foreign_grey_keeps_the_hue_it_cannot_carry() {
    let picker = Picker::mount(REMOTE, Echo::Back);

    picker.store.set("#404040".to_string());

    assert_eq!(picker.displayed(), "#404040", "the grey itself applies");
    assert!(
        picker.emissions().is_empty(),
        "an external apply is silent: {:?}",
        picker.emissions()
    );
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - hue_of(REMOTE) / 3.6).abs() < 0.01,
        "the hue the grey cannot express is kept: left {hue_left}%"
    );

    let overlay = picker.handler("rinch-color-picker__saturation-overlay", "data-rid");
    click_at(0.7, 0.13); // re-approach saturation
    dispatch_event(overlay);
    let last = picker.emissions().last().cloned().expect("an emission");
    assert!(
        (hue_of(&last) - hue_of(REMOTE)).abs() < 1.0,
        "and colour returns along the kept hue: {last}"
    );
}

/// A genuinely foreign black keeps both hue and saturation.
///
/// At v = 0 the parse carries neither: adopting it would reset the whole
/// saturation panel to its top-left corner.
#[test]
fn a_foreign_black_keeps_hue_and_saturation() {
    let picker = Picker::mount(REMOTE, Echo::Back);

    picker.store.set("#000000".to_string());

    assert_eq!(picker.displayed(), "#000000");
    assert!(picker.emissions().is_empty());
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!((hue_left - hue_of(REMOTE) / 3.6).abs() < 0.01);
    let sat_style = picker.thumb_style("rinch-color-picker__thumb");
    let sat_left = percent_of(&sat_style, "left: ");
    assert!(
        (sat_left - sat_of(REMOTE) * 100.0).abs() < 0.01,
        "saturation is kept: left {sat_left}%"
    );
    assert!(
        (percent_of(&sat_style, "top: ") - 100.0).abs() < 0.01,
        "while the value itself — what black does carry — applies"
    );
}

/// An inbound change that differs only in alpha still applies, even though
/// the display format drops alpha.
///
/// This is the boundary of the echo test: the gate must not compare under
/// the display format (Hex would erase the difference and the change would
/// never land) — the comparison is full-channel, so a value the picker's own
/// emission could not have produced is foreign.
#[test]
fn an_inbound_alpha_only_change_still_applies() {
    let picker = Picker::mount(REMOTE, Echo::Back); // default format: Hex

    picker.store.set("rgba(136, 68, 221, 0.25)".to_string());

    let left = percent_of(
        &picker.thumb_style("rinch-color-picker__alpha-thumb"),
        "left: ",
    );
    assert!(
        (left - 25.0).abs() < 0.01,
        "the foreign alpha lands: left {left}%"
    );
    assert!(
        picker.emissions().is_empty(),
        "and silently, like any external apply: {:?}",
        picker.emissions()
    );
}

/// A channel-preserving apply under an ambient batch is still recognised as
/// external when its flush finally lands.
///
/// The deferred-apply marker must record what the apply *wrote* — the merged
/// colour with the kept hue — not the raw parse: the coordinating effect
/// compares the flushed signals against the marker, and a marker holding the
/// parse's hue 0 would fail to match the kept hue, mis-reporting the adopted
/// value as an author's act (#229 broken on exactly the #227 path).
#[test]
fn a_foreign_grey_arriving_at_mount_inside_a_batch_stays_silent() {
    let picker = rinch_core::batch(|| Picker::mount_with(REMOTE, "#404040", Echo::Back));

    assert_eq!(picker.displayed(), "#404040");
    assert!(
        picker.emissions().is_empty(),
        "adopting the bound grey is not an edit, batched or not: {:?}",
        picker.emissions()
    );
    assert_eq!(
        picker.published(),
        vec!["#404040".to_string()],
        "and the bound data is untouched"
    );
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - hue_of(REMOTE) / 3.6).abs() < 0.01,
        "the seed's hue survives the grey it was bound to: left {hue_left}%"
    );
}
