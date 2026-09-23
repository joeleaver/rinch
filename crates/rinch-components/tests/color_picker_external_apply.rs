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
#[derive(Clone, Copy)]
enum Echo {
    Back,
    /// A *normalizing* store (#262): every emission is written back
    /// re-spelled by a converter that is not rinch's — the same colour in
    /// another notation, rounded by another rule.
    Normalizing(fn(&str) -> String),
    /// A *transforming* controlled handler (#283): every emission is written
    /// back as this fixed colour — `|v| store.set(snap_to_palette(v))` with a
    /// one-colour palette.
    Snap(&'static str),
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

    /// The default display format (`format: ""` → `hex`).
    fn mount_with(seed: &str, stored: &str, echo: Echo) -> Self {
        Self::mount_with_format(seed, stored, "", echo)
    }

    /// A picker bound to a fresh store holding `stored`, emitting in `format`.
    fn mount_with_format(seed: &str, stored: &str, format: &str, echo: Echo) -> Self {
        Self::mount_on(seed, Signal::new(stored.to_string()), format, echo)
    }

    /// A picker bound to `store` — possibly one it shares with a peer. Its
    /// recorder is registered before the picker's own effects, so it sees
    /// each write in order.
    fn mount_on(seed: &str, store: Signal<String>, format: &str, echo: Echo) -> Self {
        let (published, recorder) = record_store(store);

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let emissions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

        let seen = emissions.clone();
        let picker = ColorPicker {
            format: format.to_string(),
            value: seed.to_string(),
            value_fn: Some(Rc::new(move || store.get())),
            onchange: Some(InputCallback::new(move |value: String| {
                seen.borrow_mut().push(value.clone());
                match echo {
                    Echo::Back => store.set(value),
                    Echo::Snap(colour) => store.set(colour.to_string()),
                    Echo::Normalizing(respell) => store.set(respell(&value)),
                    Echo::Never => {}
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

/// Record every value `store` ever holds — what a peer on the other end of a
/// collaborative document would receive — returning the log and the effect
/// that keeps it.
fn record_store(store: Signal<String>) -> (Rc<RefCell<Vec<String>>>, Effect) {
    let published: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let recorded = published.clone();
    let recorder = Effect::new(move || {
        let value = store.get();
        recorded.borrow_mut().push(value);
    });
    (published, recorder)
}

/// Two pickers synced through one store — the shape #242 was reported in:
/// each reads the store through `value_fn` and writes its own edits back
/// through `onchange`, so every act of one arrives at the other as an
/// external value. `a.store` is the shared store and `a.published()` its
/// history: `a`'s recorder is registered before either picker's effects.
struct Peers {
    a: Picker,
    b: Picker,
}

impl Peers {
    fn mount(stored: &str, format: &str) -> Self {
        let a = Picker::mount_with_format(stored, stored, format, Echo::Back);
        let b = Picker::mount_on(stored, a.store, format, Echo::Back);
        Self { a, b }
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
    let rest = &style[start..];
    let end = rest.find('%').expect("a % terminates the value");
    rest[..end].trim().parse().expect("the value is numeric")
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

/// A consumer that writes back a *transformed* colour does not leave the
/// picker believing a deferred apply is still pending (#283).
///
/// The handler runs inside the coordinating effect, so the snap's apply runs
/// inside it too, and its batch flush cannot re-run the effect that is still
/// on the stack — the branch that would have cleared the deferred-apply
/// marker never runs. The marker then outlived the apply and swallowed the
/// next author act that landed bit-exactly on the snapped colour: here, a
/// click on the very swatch the app snaps to. Off the fixed point: the drag
/// lands on a colour the snap moves away from, so the swallow is observable.
#[test]
fn a_transforming_handler_does_not_swallow_the_next_act() {
    let picker = Picker::mount(START, Echo::Snap("#22aa55"));

    // A drag frame: the author moves the colour, the app snaps it.
    let overlay = picker.handler("rinch-color-picker__saturation-overlay", "data-rid");
    click_at(0.5, 0.5);
    dispatch_event(overlay);
    assert_eq!(picker.emissions().len(), 1, "the drag reports once");
    assert_eq!(picker.store.get(), "#22aa55", "and the app snapped it");
    assert_eq!(
        picker.displayed(),
        "#22aa55",
        "the picker follows the snapped colour"
    );

    // Then the author clicks the swatch the app snapped to.
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
    dispatch_event(id);

    assert_eq!(
        picker.emissions().len(),
        2,
        "the swatch click is an author act and reports: {:?}",
        picker.emissions()
    );
    assert_eq!(picker.emissions()[1], "#22aa55");
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

/// An achromatic value that *states* its hue — `hsl(h, 0%, l)` — carries it.
///
/// An `hsl()` string states its hue whatever its chroma (the notation is the
/// tell), so the merge adopts the authored hue instead of keeping the current
/// one: a consumer persisting picker state as hsl round-trips grey without
/// losing the hue.
#[test]
fn a_foreign_hsl_grey_carries_its_stated_hue() {
    let picker = Picker::mount(REMOTE, Echo::Back); // h ≈ 266.7

    picker.store.set("hsl(240, 0%, 50%)".to_string());

    assert_eq!(picker.displayed(), "#808080", "the grey itself applies");
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - 240.0 / 3.6).abs() < 0.01,
        "the stated hue lands instead of being merged away: left {hue_left}%"
    );
    assert!(
        picker.emissions().is_empty(),
        "an external apply is silent: {:?}",
        picker.emissions()
    );
}

/// Carryability is judged at the 8-bit rendering: an `rgb()` grey written
/// with fractional channels parses to a microscopic saturation whose
/// derived hue is quantization noise — rendered grey, it carries no more hue
/// than `#808080` does, and the current hue is kept.
#[test]
fn a_fractional_near_grey_is_still_a_grey() {
    let picker = Picker::mount(REMOTE, Echo::Back);

    picker
        .store
        .set("rgb(127.9999999999, 128, 128)".to_string());

    assert_eq!(picker.displayed(), "#808080");
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - hue_of(REMOTE) / 3.6).abs() < 0.01,
        "sub-8-bit chroma is noise, not a stated hue: left {hue_left}%"
    );
}

/// A value that *renders* black carries neither hue nor saturation, even
/// when a sub-8-bit channel gives its parse a full-range saturation.
///
/// `rgb(0, 0, 0.001)` parses to s = 1.0, h = 240 — but it denotes `#000000`,
/// so adopting that parse would snap the panel to a corner nobody can see.
#[test]
fn a_sub_8bit_blue_black_is_still_a_black() {
    let picker = Picker::mount(REMOTE, Echo::Back);

    picker.store.set("rgb(0, 0, 0.001)".to_string());

    assert_eq!(picker.displayed(), "#000000");
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!((hue_left - hue_of(REMOTE) / 3.6).abs() < 0.01, "hue kept");
    let sat_left = percent_of(&picker.thumb_style("rinch-color-picker__thumb"), "left: ");
    assert!(
        (sat_left - sat_of(REMOTE) * 100.0).abs() < 0.01,
        "saturation kept: left {sat_left}%"
    );
}

/// The accepted dual of the emission comparison: under an alpha-dropping
/// display format, an inbound value that restates the current RGB with an
/// explicitly opaque alpha is indistinguishable from a normalizing store's
/// echo of the emission — it does not apply.
///
/// This pins the trade-off, not a defect: recognising it as foreign would
/// make a normalizing store's echo snap a mid-drag alpha back to opaque
/// (the exact #227 failure mode). Alpha is externally drivable under the
/// formats whose emission carries it (`hexa`, `rgba`, `hsla`).
#[test]
fn an_opaque_restatement_of_the_emission_reads_as_echo_under_hex() {
    let picker = Picker::mount(REMOTE, Echo::Back); // default format: Hex

    picker.store.set("rgba(136, 68, 221, 0.25)".to_string());
    picker.store.set("rgba(136, 68, 221, 1)".to_string());

    let left = percent_of(
        &picker.thumb_style("rinch-color-picker__alpha-thumb"),
        "left: ",
    );
    assert!(
        (left - 25.0).abs() < 0.01,
        "indistinguishable from a normalizing echo, so the alpha holds: left {left}%"
    );
}

// === #242: identity is judged in the notation the value is written in ===
//
// An hsl wire never touches 8-bit RGB: `hsla_to_css` writes integer degrees
// and integer percents, and the parser reads them back exactly, so the
// wire's whole quantization is ±0.5° / ±0.5% / ±0.5%. The #241 gate judged
// every inbound value at 8-bit RGB instead — right for hex/rgb wires, whose
// quantization that is, but at low chroma a multi-degree hue move renders
// to the same 8-bit hex (`hsl(200, 3%, 49%)` and `hsl(205, 3%, 49%)` are
// both `#797e81`), so a peer's genuine hue edit was folded as an echo.

/// A low-chroma hsl colour at which a 5° hue move does not change the
/// 8-bit rendering (the first hue delta that does is 6°).
const LOW_CHROMA: &str = "hsl(200, 3%, 49%)";
const LOW_CHROMA_MOVED: &str = "hsl(205, 3%, 49%)";

/// The single-picker repro: a peer's stated-hue move on an hsl wire applies.
///
/// Pre-fix the gate rendered both strings to `#797e81`, called the move an
/// echo, and the hue thumb stayed at 200° (55.56%).
#[test]
fn a_peers_low_chroma_hue_move_applies_on_an_hsl_wire() {
    let picker = Picker::mount_with_format(LOW_CHROMA, LOW_CHROMA, "hsla", Echo::Back);

    picker.store.set(LOW_CHROMA_MOVED.to_string());

    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - 205.0 / 3.6).abs() < 0.01,
        "the peer's 205° must land — the hsl wire wrote it exactly: left {hue_left}%"
    );
    assert!(
        picker.emissions().is_empty(),
        "an external apply is silent: {:?}",
        picker.emissions()
    );
}

/// The data loss #242 reports: peer A moves the hue; peer B folds it as an
/// echo, keeps the stale hue, and B's next local act writes that stale hue
/// back into the shared store — reverting A's edit.
#[test]
fn a_peers_hue_move_is_not_reverted_by_the_next_local_act() {
    let peers = Peers::mount(LOW_CHROMA, "hsla");

    // A drags the hue to 205°.
    let a_hue = peers
        .a
        .handler("rinch-color-picker__hue-overlay", "data-rid");
    click_at(205.0 / 360.0, 0.5);
    dispatch_event(a_hue);
    assert_eq!(
        peers.a.store.get(),
        LOW_CHROMA_MOVED,
        "A's edit reaches the shared store"
    );

    // B's next local act: a click in its saturation panel.
    let b_sat = peers
        .b
        .handler("rinch-color-picker__saturation-overlay", "data-rid");
    click_at(0.5, 0.5);
    dispatch_event(b_sat);

    let after_a: Vec<String> = peers.a.published().into_iter().skip(1).collect();
    assert!(
        !after_a.is_empty() && after_a.iter().all(|v| hue_of(v) == 205.0),
        "once A moved the hue to 205°, no write may carry it back to 200°: {after_a:?}"
    );
    assert_eq!(
        peers.a.store.get(),
        "hsl(205, 33%, 38%)",
        "B's saturation click is built on A's hue, not on the stale one"
    );
}

/// The regression guard for the gate's other edge: under an hsl wire a
/// sub-degree hue drag at high chroma tracks continuously, and its echo —
/// which the wire rounds to a whole degree — never snaps it back.
///
/// This passes before and after the fix (the echo is byte-identical to the
/// emission, which the gate has always recognised); it pins that judging
/// identity on the hsl wire's own grid does not make the gate *finer* than
/// the wire, where every emission would come back as a foreign colour.
#[test]
fn an_hsl_echo_of_a_sub_degree_hue_still_folds() {
    let picker = Picker::mount_with_format(
        "hsl(200, 100%, 50%)",
        "hsl(200, 100%, 50%)",
        "hsla",
        Echo::Back,
    );
    let overlay = picker.handler("rinch-color-picker__hue-overlay", "data-rid");

    click_at(0.5, 0.5); // 180°
    dispatch_event(overlay);

    // A 200px-wide slider: a quarter-pixel step is 0.45°.
    for step in 1..=12 {
        let x = 100.0 + step as f32 * 0.25;
        update_drag(x, 100.0);
        let hue_left = percent_of(
            &picker.thumb_style("rinch-color-picker__hue-thumb"),
            "left: ",
        );
        let expected = x as f64 / 200.0 * 100.0;
        assert!(
            (hue_left - expected).abs() < 0.001,
            "step {step}: the thumb must track the drag, not snap to the echo's \
             whole degree: left {hue_left}% (expected {expected}%)"
        );
    }
    assert_eq!(
        picker.emissions().len(),
        13,
        "every frame reported once, and no apply re-entered: {:?}",
        picker.emissions()
    );
    let last = picker.emissions().last().cloned().expect("an emission");
    assert_eq!(
        picker.store.get(),
        last,
        "the store holds what was reported"
    );
}

/// The same hole in the merge: a stated hue with a sub-percent saturation
/// renders 8-bit grey, and carryability judged at 8-bit discarded it.
///
/// `hsl(205, 0.3%, 49%)` parses to s ≈ 0.006 — not the exact zero the
/// stated-hue exception keyed on — so pre-fix the peer's 205° was merged
/// away and the current hue kept. A stated hue is a stated hue whatever
/// its chroma: the notation says so.
#[test]
fn a_stated_hue_survives_a_sub_percent_hsl_saturation() {
    let picker = Picker::mount(REMOTE, Echo::Back); // h ≈ 266.7

    picker.store.set("hsl(205, 0.3%, 49%)".to_string());

    assert_eq!(
        picker.displayed(),
        "#7d7d7d",
        "the near-grey itself applies"
    );
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - 205.0 / 3.6).abs() < 0.01,
        "the stated hue lands instead of being merged away: left {hue_left}%"
    );
    assert!(
        picker.emissions().is_empty(),
        "an external apply is silent: {:?}",
        picker.emissions()
    );
}

/// A stated hue of exactly 0 is a stated hue too: `hsl(0, 0%, 50%)` names
/// red at no saturation, and the merge adopts it like any other hsl grey.
///
/// The old rule kept the current hue at hue 0 (the RGB-grey convention,
/// `rgb_to_hsv`'s `delta == 0` arm) — but an hsl string never parses through
/// that arm, so the carve-out only ever bit genuine hsl emissions: a 1°-wide
/// dead band at red on an hsl wire, and an apply that was not a fixed point
/// (the kept hue re-formats as `hsl(267, 0%, 50%)`, never the store's text).
#[test]
fn an_hsl_stated_hue_of_zero_is_adopted() {
    let picker = Picker::mount(REMOTE, Echo::Back); // h ≈ 266.7

    picker.store.set("hsl(0, 0%, 50%)".to_string());

    assert_eq!(picker.displayed(), "#808080", "the grey itself applies");
    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        hue_left.abs() < 0.01,
        "hue 0 is stated, not the convention: left {hue_left}%"
    );
    let emissions = picker.emissions();
    assert!(
        emissions.is_empty(),
        "an external apply is silent: {emissions:?}"
    );
}

/// The dead band at red, in the two-picker shape: A drags the hue to 0° at
/// grey, and B must build its next act on 0°, not revert A to 200°.
#[test]
fn a_peers_hue_move_to_zero_at_grey_is_not_reverted() {
    let peers = Peers::mount("hsl(200, 0%, 50%)", "hsla");

    // A drags the hue to 0°.
    let a_hue = peers
        .a
        .handler("rinch-color-picker__hue-overlay", "data-rid");
    click_at(0.0, 0.5);
    dispatch_event(a_hue);
    assert_eq!(
        peers.a.store.get(),
        "hsl(0, 0%, 50%)",
        "A's edit reaches the shared store"
    );

    // B's next local act: a click in its saturation panel.
    let b_sat = peers
        .b
        .handler("rinch-color-picker__saturation-overlay", "data-rid");
    click_at(0.5, 0.5);
    dispatch_event(b_sat);

    let after_a: Vec<String> = peers.a.published().into_iter().skip(1).collect();
    assert!(
        !after_a.is_empty() && after_a.iter().all(|v| hue_of(v) == 0.0),
        "once A moved the hue to 0°, no write may carry it back to 200°: {after_a:?}"
    );
    assert_eq!(
        peers.a.store.get(),
        "hsl(0, 33%, 38%)",
        "B's saturation click is built on A's hue, not on the stale one"
    );
}

/// The field's write-back guard judges at the same resolution as the gate:
/// once a peer's low-chroma hue move applies on an hsl wire, the field —
/// whose text renders to the same 8-bit hex as the new colour — is rewritten
/// too, so the field, the thumb and the store agree.
///
/// Pre-fix the field guard compared at 8-bit only: the thumb moved to 200°
/// while the field kept saying 205°, and the commit boundary would have
/// normalized that stale text without touching the signals.
#[test]
fn the_field_follows_a_peers_low_chroma_hue_move_on_an_hsl_wire() {
    let picker = Picker::mount_with_format(LOW_CHROMA, LOW_CHROMA, "hsla", Echo::Back);

    // The author typed 205° — the field is theirs while it denotes the colour.
    picker.type_hex(LOW_CHROMA_MOVED);
    assert_eq!(picker.displayed(), LOW_CHROMA_MOVED);
    assert_eq!(picker.emissions(), vec![LOW_CHROMA_MOVED.to_string()]);

    // A peer moves the hue back to 200°: same hex, different hsl spelling.
    picker.store.set(LOW_CHROMA.to_string());

    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - 200.0 / 3.6).abs() < 0.01,
        "the peer's 200° lands: left {hue_left}%"
    );
    assert_eq!(
        picker.displayed(),
        LOW_CHROMA,
        "the field no longer denotes the colour at the hsl wire's resolution, so it is rewritten"
    );
    assert_eq!(
        picker.emissions(),
        vec![LOW_CHROMA_MOVED.to_string()],
        "the external apply and the rewrite are silent"
    );
}

// === #262: a store that re-spells the emission rounds a tie its own way ===
//
// The #242 gate's emission arm asks whether an inbound value denotes what the
// picker emits, at the inbound notation's grid — by re-spelling the emission
// with rinch's own serializer. A store that re-spells it with another
// converter lands on the same grid point except where the exact value sits
// on a tie between two points, and there it may round the other way: about
// 0.22% of 8-bit colours have an hsl hue on a half degree. The picker then
// read its own echo as foreign and applied it — under an alpha-dropping
// display format, the alpha snapped opaque, on every frame of a drag.
//
// The decision recorded on #262 is option (b): the emission arm accepts any
// rounding of the emission's exact value (at a tie, the neighbouring grid
// point); a value compared against the colour the picker *holds* is still
// judged exactly, and so is anything farther than a rounding away.

/// `#rrggbb` → `hsl(h, s%, l%)` in exact integer arithmetic, rounding every
/// half up — the tie rule of an exact-rational converter, which is not the
/// one rinch's float pipeline lands on.
fn hex_to_hsl_ties_up(hex: &str) -> String {
    let hex = hex.strip_prefix('#').expect("a hex emission");
    assert_eq!(hex.len(), 6, "an opaque 6-digit emission: {hex}");
    let channel = |i: usize| i64::from_str_radix(&hex[i..i + 2], 16).expect("hex digits");
    let (r, g, b) = (channel(0), channel(2), channel(4));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    // round(n / q) for n, q > 0 with halves rounded up.
    let round = |n: i64, q: i64| (2 * n + q).div_euclid(2 * q);
    let l = round(100 * (max + min), 510);
    if d == 0 {
        return format!("hsl(0, 0%, {l}%)");
    }
    // Hue as an exact fraction over `d`, in [0, 360).
    let hue_num = if max == r {
        (60 * (g - b)).rem_euclid(360 * d)
    } else if max == g {
        60 * (b - r) + 120 * d
    } else {
        60 * (r - g) + 240 * d
    };
    let h = round(hue_num, d).rem_euclid(360);
    let s = round(100 * d, 255 - (max + min - 255).abs());
    format!("hsl({h}, {s}%, {l}%)")
}

/// `hsl(h, s%, l%)` with integer channels → `#rrggbb`, rounding every half
/// down — the other tie direction, for the other notation pair.
fn hsl_to_hex_ties_down(hsl: &str) -> String {
    let inner = hsl
        .strip_prefix("hsl(")
        .and_then(|t| t.strip_suffix(')'))
        .expect("an opaque hsl emission");
    let parts: Vec<f64> = inner
        .split(',')
        .map(|t| t.trim().trim_end_matches('%').parse().expect("a number"))
        .collect();
    let (h, s, l) = (parts[0], parts[1] / 100.0, parts[2] / 100.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0 - 0.5).ceil() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

/// The emission whose hsl hue sits on a tie: 60 × (121 − 126) / 8 + 240 is
/// exactly 202.5°.
const HUE_TIE: &str = "#797e81";

#[test]
fn the_tie_converters_disagree_with_rinch_where_the_fixtures_say() {
    use rinch_components::color_utils::{ColorFormat, format_color};
    // Positive controls: each fixture below rests on the store spelling the
    // emission differently from rinch — otherwise it pins nothing.
    let rinch = format_color(parse_color(HUE_TIE).unwrap(), ColorFormat::Hsl);
    let store = hex_to_hsl_ties_up(HUE_TIE);
    assert_ne!(rinch, store, "the tie is spelled two ways");
    assert_eq!(store, "hsl(203, 3%, 49%)");
    // Off the tie the two converters agree, grey included.
    for hex in ["#797e82", "#8844dd", "#404040", "#ff0000", "#22aa55"] {
        assert_eq!(
            format_color(parse_color(hex).unwrap(), ColorFormat::Hsl),
            hex_to_hsl_ties_up(hex),
            "{hex}"
        );
    }
    let rinch = format_color(parse_color("hsl(0, 0%, 50%)").unwrap(), ColorFormat::Hex);
    assert_eq!(rinch, "#808080");
    assert_eq!(hsl_to_hex_ties_down("hsl(0, 0%, 50%)"), "#7f7f7f");
    assert_eq!(hsl_to_hex_ties_down("hsl(0, 100%, 50%)"), "#ff0000");
}

/// The #262 repro: an alpha drag under the default hex format, behind a store
/// that re-spells every emission as hsl and rounds the half-degree tie up.
/// The echo `hsl(203, 3%, 49%)` is the picker's own `#797e81`; pre-fix it was
/// applied as a foreign colour and the alpha snapped opaque on every frame.
#[test]
fn an_alpha_drag_survives_a_store_that_rounds_the_hue_tie_the_other_way() {
    let picker = Picker::mount(HUE_TIE, Echo::Normalizing(hex_to_hsl_ties_up));
    let overlay = picker.handler("rinch-color-picker__alpha-overlay", "data-rid");

    click_at(0.4, 0.5);
    dispatch_event(overlay);
    assert_eq!(
        picker.store.get(),
        "hsl(203, 3%, 49%)",
        "the store re-spelled"
    );
    let left = percent_of(
        &picker.thumb_style("rinch-color-picker__alpha-thumb"),
        "left: ",
    );
    assert!(
        (left - 40.0).abs() < 0.01,
        "the echo is the picker's own emission, so the alpha holds: left {left}%"
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
    assert_eq!(
        picker.emissions(),
        vec![HUE_TIE.to_string(), HUE_TIE.to_string()],
        "one report per frame, and no apply re-entered"
    );
}

/// The other notation pair and the other tie direction: an hsl display
/// behind a store that re-spells it as hex, rounding 127.5 down.
#[test]
fn an_alpha_drag_survives_a_store_that_rounds_an_rgb_tie_down() {
    const MID_GREY: &str = "hsl(0, 0%, 50%)";
    let picker = Picker::mount_with_format(
        MID_GREY,
        MID_GREY,
        "hsl",
        Echo::Normalizing(hsl_to_hex_ties_down),
    );
    let overlay = picker.handler("rinch-color-picker__alpha-overlay", "data-rid");

    click_at(0.4, 0.5);
    dispatch_event(overlay);
    assert_eq!(picker.store.get(), "#7f7f7f", "the store re-spelled");
    let left = percent_of(
        &picker.thumb_style("rinch-color-picker__alpha-thumb"),
        "left: ",
    );
    assert!(
        (left - 40.0).abs() < 0.01,
        "the echo is the picker's own emission, so the alpha holds: left {left}%"
    );
    assert_eq!(
        picker.displayed(),
        MID_GREY,
        "and the colour is not moved to 127"
    );
}

/// The half of the decision that keeps #242 fixed: the tolerance is a
/// *rounding* of the emission, not a free degree. Off the tie, a peer's
/// genuine 1° move under a hex display is farther from the emission's exact
/// hue than any rounding of it, and applies.
///
/// `#797e82`'s hue is exactly 206⅔°, which rinch spells 207; the peer writes
/// 206, two thirds of a degree away. A flat ±1-degree window would fold it.
#[test]
fn a_peers_one_degree_move_off_the_tie_still_applies_under_hex() {
    let picker = Picker::mount("#797e82", Echo::Back); // default format: Hex

    picker.store.set("hsl(206, 4%, 49%)".to_string());

    let hue_left = percent_of(
        &picker.thumb_style("rinch-color-picker__hue-thumb"),
        "left: ",
    );
    assert!(
        (hue_left - 206.0 / 3.6).abs() < 0.01,
        "the peer's 206° lands: left {hue_left}%"
    );
    assert!(
        picker.emissions().is_empty(),
        "an external apply is silent: {:?}",
        picker.emissions()
    );
}

// === #262 option (b), channel by channel: a rounding folds, a move applies ===
//
// Each case mounts a picker whose emission is `emission` under `format`, with
// the store echoing, then has a peer write `inbound` into the store. "Folded"
// means no thumb moves; "applied" means some thumb moves (and, either way,
// nothing is emitted — an external apply is silent).

fn thumbs(picker: &Picker) -> [String; 3] {
    [
        picker.thumb_style("rinch-color-picker__hue-thumb"),
        picker.thumb_style("rinch-color-picker__thumb"),
        picker.thumb_style("rinch-color-picker__alpha-thumb"),
    ]
}

/// Whether a peer's `inbound` moves a picker holding `emission` under `format`.
fn peer_applies(emission: &str, format: &str, inbound: &str) -> bool {
    let picker = Picker::mount_with_format(emission, emission, format, Echo::Back);
    let before = thumbs(&picker);
    picker.store.set(inbound.to_string());
    assert!(
        picker.emissions().is_empty(),
        "{inbound} over {emission} ({format}): an external value emits nothing: {:?}",
        picker.emissions()
    );
    thumbs(&picker) != before
}

fn check(emission: &str, format: &str, folded: &[&str], applied: &[&str]) {
    for inbound in folded {
        assert!(
            !peer_applies(emission, format, inbound),
            "{inbound} is a rounding of {emission} ({format}) and must fold"
        );
    }
    for inbound in applied {
        assert!(
            peer_applies(emission, format, inbound),
            "{inbound} is a peer's move off {emission} ({format}) and must apply"
        );
    }
}

/// Hue, under both 8-bit display formats: at the 202.5° tie both neighbours
/// fold and the next grid points apply; off the tie (206⅔°) only 207 folds.
#[test]
fn tie_tolerance_hue_folds_only_roundings_under_hex_and_rgb() {
    for (emission, format) in [("#797e81", ""), ("rgb(121, 126, 129)", "rgb")] {
        check(
            emission,
            format,
            &["hsl(202, 3%, 49%)", "hsl(203, 3%, 49%)"],
            &["hsl(201, 3%, 49%)", "hsl(204, 3%, 49%)"],
        );
    }
    for (emission, format) in [("#797e82", ""), ("rgb(121, 126, 130)", "rgb")] {
        check(
            emission,
            format,
            &["hsl(207, 4%, 49%)"],
            &["hsl(206, 4%, 49%)", "hsl(208, 4%, 49%)"],
        );
    }
}

/// Saturation on a tie (`#168a50` is hsl(150, 72.5%, 31.37…%)) and lightness,
/// which has no tie on the 8-bit → hsl map at all (20·(max+min) = 51·odd has
/// no solution), so both its neighbours apply.
#[test]
fn tie_tolerance_saturation_tie_and_lightness_neighbours() {
    check(
        "#168a50",
        "",
        &["hsl(150, 72%, 31%)", "hsl(150, 73%, 31%)"],
        &[
            "hsl(150, 71%, 31%)",
            "hsl(150, 74%, 31%)",
            "hsl(150, 73%, 30%)",
            "hsl(150, 73%, 32%)",
            "hsl(149, 73%, 31%)",
            "hsl(151, 73%, 31%)",
        ],
    );
    // Off every tie: #8844dd is hsl(266.67, 69.4%, 56.7%) → rinch 267/69/57.
    check(
        "#8844dd",
        "",
        &["hsl(267, 69%, 57%)"],
        &[
            "hsl(266, 69%, 57%)",
            "hsl(268, 69%, 57%)",
            "hsl(267, 68%, 57%)",
            "hsl(267, 70%, 57%)",
            "hsl(267, 69%, 56%)",
            "hsl(267, 69%, 58%)",
        ],
    );
}

/// 8-bit channels under an hsl display, hex and rgb wires: mid grey is 127.5
/// on every channel, so 127 and 128 fold per channel and 126/129 apply; 40%
/// grey is 102 exactly, so only 102 folds.
#[test]
fn tie_tolerance_rgb_channels_under_an_hsl_display() {
    check(
        "hsl(0, 0%, 50%)",
        "hsl",
        &["#7f7f7f", "#808080", "#807f80", "rgb(127, 128, 127)"],
        &[
            "#7e7f7f",
            "#7f817f",
            "#7f7f7e",
            "#818080",
            "rgb(126, 127, 127)",
            "rgb(127, 127, 129)",
        ],
    );
    check(
        "hsl(0, 0%, 40%)",
        "hsl",
        &["#666666", "rgb(102, 102, 102)"],
        &["#676666", "#666566", "#666667", "rgb(101, 102, 102)"],
    );
    // Off the grey axis, sampled off 0/255 on every channel.
    // hsl(200, 50%, 40%) is r=51, g=119, b=153 exactly: no tie anywhere.
    check(
        "hsl(200, 50%, 40%)",
        "hsl",
        &["#337799"],
        &[
            "#347799", "#327799", "#337899", "#337699", "#33779a", "#337798",
        ],
    );
}

/// Alpha: hsla 0.50 is 127.5 of 255 (a tie on the hex wire), 0.51 is 130.05
/// (not one); the rgba wire shares hsla's 100-step grid, so no tie there.
#[test]
fn tie_tolerance_alpha_ties_and_neighbours() {
    check(
        "hsla(0, 100%, 50%, 0.50)",
        "hsla",
        &["#ff00007f", "#ff000080", "rgba(255, 0, 0, 0.5)"],
        &[
            "#ff00007e",
            "#ff000081",
            "rgba(255, 0, 0, 0.49)",
            "rgba(255, 0, 0, 0.51)",
        ],
    );
    check(
        "hsla(0, 100%, 50%, 0.51)",
        "hsla",
        &["#ff000082"],
        &["#ff000081", "#ff000083"],
    );
    // Hexa display, hsla wire: 0x80 is 50.196%, rinch writes 0.50.
    check(
        "#ff000080",
        "hexa",
        &["hsla(0, 100%, 50%, 0.50)"],
        &["hsla(0, 100%, 50%, 0.49)", "hsla(0, 100%, 50%, 0.51)"],
    );
}

/// `hsla(h, s%, l%, a)` → `#rrggbbaa`, every half rounded DOWN.
fn hsla_to_hexa_ties_down(hsla: &str) -> String {
    let inner = hsla
        .strip_prefix("hsla(")
        .and_then(|t| t.strip_suffix(')'))
        .expect("an hsla emission");
    let parts: Vec<f64> = inner
        .split(',')
        .map(|t| t.trim().trim_end_matches('%').parse().expect("a number"))
        .collect();
    let (h, s, l, a) = (parts[0], parts[1] / 100.0, parts[2] / 100.0, parts[3]);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| (v * 255.0 - 0.5).ceil().max(0.0) as u8;
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        byte(r + m),
        byte(g + m),
        byte(b + m),
        byte(a)
    )
}

/// The alpha tie, end to end: an alpha drag under `hsla` behind a store that
/// re-spells the emission as `#rrggbbaa` rounding 127.5 down to 0x7f. The
/// echo is the picker's own emission; the thumb must stay at the drag.
#[test]
fn tie_tolerance_an_alpha_drag_survives_a_store_that_rounds_the_alpha_tie_down() {
    let seed = "hsla(0, 100%, 50%, 1.00)";
    assert_eq!(
        hsla_to_hexa_ties_down("hsla(0, 100%, 50%, 0.50)"),
        "#ff00007f",
        "positive control: the store rounds the tie away from rinch's 0x80"
    );
    let picker = Picker::mount_with_format(
        seed,
        seed,
        "hsla",
        Echo::Normalizing(hsla_to_hexa_ties_down),
    );
    let overlay = picker.handler("rinch-color-picker__alpha-overlay", "data-rid");
    click_at(0.5, 0.5);
    dispatch_event(overlay);
    assert_eq!(picker.store.get(), "#ff00007f", "the store re-spelled");
    let left = percent_of(
        &picker.thumb_style("rinch-color-picker__alpha-thumb"),
        "left: ",
    );
    assert!(
        (left - 50.0).abs() < 0.01,
        "the echo is the picker's own emission, so the alpha holds: left {left}%"
    );
    assert_eq!(
        picker.emissions(),
        vec!["hsla(0, 100%, 50%, 0.50)".to_string()]
    );
}
