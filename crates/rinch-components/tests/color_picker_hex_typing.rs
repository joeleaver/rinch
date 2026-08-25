//! ColorPicker/ColorInput: the field is the author's while they type (#231).
//!
//! `parse_color` accepts hex of length 3, 6, or 8, so a valid *prefix* of the
//! colour being typed — `#336` on the way to `#3366cc` — parses on `oninput`,
//! and the display effect used to write the normalized expansion (`#333366`)
//! back into the focused field: every remaining keystroke then landed on the
//! rewritten string, committing a colour nobody chose. The fix: the effect
//! skips the write-back while the field's text already denotes the colour the
//! picker holds (`denotes_same`), and rewrites only when the colour moves away
//! from it — a drag, a swatch, an external apply.
//!
//! Harness note: `dispatch_input_event` only invokes the handler — it does not
//! model the field's own text (the runtime/browser owns that on both
//! backends). The desktop runtime mirrors the typed text into the `value`
//! attribute *before* dispatching, so these tests do the same and then assert
//! the attribute still holds the author's text afterwards. On web the
//! attribute lags the live text instead, so one test dispatches without the
//! mirror and asserts no write landed at all.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_components::color_picker::ColorPicker;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{
    ClickContext, EventHandlerId, dispatch_event, dispatch_input_event, set_click_context,
};
use rinch_core::{Component, InputCallback};

/// Every keystroke state of an author typing `#3366cc` into an empty field.
const KEYSTROKES: [&str; 7] = ["#", "#3", "#33", "#336", "#3366", "#3366c", "#3366cc"];

struct Mounted {
    // Kept alive for the test's duration: the document owns the nodes the
    // effects patch, and the scope owns the effects and handlers.
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
    emissions: Rc<RefCell<Vec<String>>>,
}

impl Mounted {
    fn picker(value: &str, format: &str) -> Self {
        Self::mount(|emissions| {
            Box::new(ColorPicker {
                format: format.to_string(),
                value: value.to_string(),
                onchange: Some(record_into(emissions)),
                alpha: true,
                with_input: true,
                swatches: vec!["#22aa55".into()],
                ..Default::default()
            })
        })
    }

    fn color_input(value: &str) -> Self {
        Self::mount(|emissions| {
            Box::new(ColorInput {
                value: value.to_string(),
                onchange: Some(record_into(emissions)),
                swatches: vec!["#22aa55".into()],
                ..Default::default()
            })
        })
    }

    fn mount(build: impl FnOnce(&Rc<RefCell<Vec<String>>>) -> Box<dyn Component>) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let emissions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let root = build(&emissions).render(&mut scope, &[]);

        Self {
            _doc: doc,
            _scope: scope,
            root,
            emissions,
        }
    }

    fn emissions(&self) -> Vec<String> {
        self.emissions.borrow().clone()
    }

    /// The text field node — the picker's hex input or the ColorInput's input.
    fn field(&self) -> NodeHandle {
        find_by_class(&self.root, "rinch-color-picker__hex-input")
            .or_else(|| find_by_class(&self.root, "rinch-color-input__input"))
            .expect("a text field exists")
    }

    fn field_text(&self) -> String {
        self.field()
            .get_attribute("value")
            .expect("the field has a value")
    }

    fn field_handler(&self) -> EventHandlerId {
        EventHandlerId(
            self.field()
                .get_attribute("data-oninput")
                .expect("the field has an input handler")
                .parse()
                .expect("handler id is numeric"),
        )
    }

    fn field_change_handler(&self) -> EventHandlerId {
        EventHandlerId(
            self.field()
                .get_attribute("data-onchange")
                .expect("the field has a change handler")
                .parse()
                .expect("handler id is numeric"),
        )
    }

    /// One keystroke, desktop-shaped: the runtime mirrors the field's text
    /// into the `value` attribute, then dispatches `oninput` with it.
    fn type_text(&self, text: &str) {
        self.field().set_attribute("value", text);
        dispatch_input_event(self.field_handler(), text.to_string());
    }

    /// The commit boundary (#226): the runtime/browser fires `data-onchange`
    /// with the final text when the gesture ends (blur after modification,
    /// Enter).
    fn commit(&self, text: &str) {
        dispatch_input_event(self.field_change_handler(), text.to_string());
    }

    /// Click the first preset swatch (the picker's own, or the one inside a
    /// ColorInput's dropdown picker).
    fn click_swatch(&self) {
        let swatch = find_by_class(&self.root, "rinch-color-picker__swatches")
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
    }
}

fn record_into(emissions: &Rc<RefCell<Vec<String>>>) -> InputCallback {
    let seen = emissions.clone();
    InputCallback::new(move |value: String| seen.borrow_mut().push(value))
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

/// The headline defect: typing `#3366cc` one keystroke at a time. At `#336`
/// the text is a valid three-digit hex; pre-fix the display effect expanded it
/// to `#333366` and wrote that over the author's text, so every later
/// keystroke landed on the rewritten string.
#[test]
fn a_parseable_prefix_is_not_expanded_under_the_authors_caret() {
    let picker = Mounted::picker("#ff0000", "hex");

    for text in KEYSTROKES {
        picker.type_text(text);
        assert_eq!(
            picker.field_text(),
            text,
            "the field must still hold exactly what the author has typed"
        );
    }

    assert_eq!(
        picker.emissions(),
        vec!["#333366".to_string(), "#3366cc".to_string()],
        "each parseable state reports the colour it denotes — the prefix as a \
         live preview, then the finished colour — and nothing else"
    );
}

/// Typing past six digits toward an alpha pair: `#3333666c` parses as
/// red/green/blue `333366` with alpha `6c`, which the hex format then drops —
/// pre-fix the field was truncated back to `#333366` mid-edit.
#[test]
fn typing_toward_an_alpha_pair_keeps_the_authors_text() {
    let picker = Mounted::picker("#333366", "hex");

    picker.type_text("#3333666");
    picker.type_text("#3333666c");

    assert_eq!(
        picker.field_text(),
        "#3333666c",
        "eight typed digits stay in the field even though the format drops the alpha pair"
    );
    // The alpha landed internally: the preview shows it even though the hex
    // format cannot.
    let preview = find_by_class(&picker.root, "rinch-color-picker__preview").expect("preview");
    let overlay_style = preview
        .children()
        .first()
        .expect("preview overlay")
        .get_attribute("style")
        .expect("overlay styled");
    assert!(
        overlay_style.contains("rgba(51, 51, 102, 0.42)"),
        "the typed alpha reached the preview: {overlay_style}"
    );
}

/// The web shape of the same defect: there the browser owns the live text and
/// the `value` attribute this component reads lags behind it, so the guard
/// cannot see the prefix in the attribute — the handler's own record of the
/// typed text has to suppress the write. An untouched attribute is the proof
/// no write landed (on web a write here replaces the field's real text via the
/// value-property mirror and throws the caret to the end).
#[test]
fn a_prefix_typed_while_the_attribute_lags_is_left_alone() {
    let picker = Mounted::picker("#ff0000", "hex");

    // No attribute mirror before dispatch — the attribute still holds the
    // mount-time text, as it does mid-typing on web.
    dispatch_input_event(picker.field_handler(), "#336".to_string());

    assert_eq!(
        picker.field_text(),
        "#ff0000",
        "the display effect must not write at all while the author's text \
         denotes the colour the picker holds"
    );
}

/// The guard is an agreement check, not a freeze: when the colour moves away
/// from the field's text — here a swatch click — the field is rewritten.
#[test]
fn the_field_is_rewritten_when_the_colour_moves_away_from_it() {
    let picker = Mounted::picker("#ff0000", "hex");
    picker.type_text("#336");
    assert_eq!(picker.field_text(), "#336");

    picker.click_swatch();

    assert_eq!(
        picker.field_text(),
        "#22aa55",
        "a swatch click is not typing — the field follows the colour"
    );
}

/// A non-hex output format is the same contract: pre-fix a picker with
/// `format: "rgba"` replaced the typed `#336` with `rgb(51, 51, 102)`.
#[test]
fn an_rgba_format_picker_leaves_the_typed_prefix_alone() {
    let picker = Mounted::picker("#ff0000", "rgba");

    picker.type_text("#336");

    assert_eq!(picker.field_text(), "#336");
    assert_eq!(
        picker.emissions(),
        vec!["rgb(51, 51, 102)".to_string()],
        "the consumer still receives the formatted colour"
    );
}

/// Control: a value arriving whole (paste, programmatic set) commits exactly.
#[test]
fn a_whole_value_arrives_exactly() {
    let picker = Mounted::picker("#ff0000", "hex");

    picker.type_text("#8844dd");

    assert_eq!(picker.field_text(), "#8844dd");
    assert_eq!(picker.emissions(), vec!["#8844dd".to_string()]);
}

/// ColorInput has the same handler→normalize→write-back loop around its own
/// text field, and pre-fix the same hijack: `#336` became `#333366` under the
/// author's caret.
///
/// Since #226 its `onchange` is a real commit boundary: typing previews live
/// (the swatch follows) but reports nothing; the commit reports once with the
/// final color. (Pre-#226 this emitted per parseable keystroke —
/// `["#333366", "#3366cc"]` here — which is what the prop name never
/// promised.)
#[test]
fn color_input_typing_is_not_hijacked_either() {
    let input = Mounted::color_input("#ff0000");

    for text in KEYSTROKES {
        input.type_text(text);
        assert_eq!(
            input.field_text(),
            text,
            "the field must still hold exactly what the author has typed"
        );
    }

    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "onchange is a commit boundary: nothing reports mid-gesture"
    );

    input.commit("#3366cc");
    assert_eq!(
        input.emissions(),
        vec!["#3366cc".to_string()],
        "one commit, one report, the final color"
    );
}

/// ColorInput on the web shape: the attribute lags the live text, so only
/// the handler's own record can recognise the author's prefix — a guard that
/// consults the attribute alone would rewrite the field here.
#[test]
fn color_input_prefix_with_lagging_attribute_is_left_alone() {
    let input = Mounted::color_input("#ff0000");

    // No attribute mirror before dispatch, as mid-typing on web.
    dispatch_input_event(input.field_handler(), "#336".to_string());

    assert_eq!(
        input.field_text(),
        "#ff0000",
        "the display effect must not write at all while the author's text \
         denotes the colour the input holds"
    );
}

/// And the same non-freeze control: a pick from the dropdown picker rewrites
/// the ColorInput's field. On the real desktop runtime a swatch click blurs
/// the field before the click handler runs, and the blur dispatches the
/// commit boundary — so the typed "#336" commits first (reporting its
/// normalized "#333366"), and then the pick commits "#22aa55".
#[test]
fn color_input_field_follows_the_dropdown_picker() {
    let input = Mounted::color_input("#ff0000");
    input.type_text("#336");
    assert_eq!(input.field_text(), "#336");

    input.commit("#336"); // the click's blur commits the typed prefix first
    input.click_swatch();

    assert_eq!(input.field_text(), "#22aa55");
    assert_eq!(
        input.emissions(),
        vec!["#333366".to_string(), "#22aa55".to_string()],
        "two commits: the blurred typed prefix, then the pick"
    );
}

/// The typed-text record must track every edit, parseable or not: a record
/// frozen at the last *parseable* text would survive a backspace into a dead
/// prefix and then veto the very rewrite a swatch click earns — even though
/// the field no longer holds the text the record describes.
#[test]
fn a_dead_prefix_does_not_veto_the_next_rewrite() {
    let picker = Mounted::picker("#ff0000", "hex");

    picker.type_text("#22aa55"); // parses; the guard leaves the field alone
    picker.type_text("#22aa5"); // backspace: unparseable, but still recorded

    picker.click_swatch(); // the swatch is "#22aa55" — what the STALE record
    // would have denoted, had it survived

    assert_eq!(
        picker.field_text(),
        "#22aa55",
        "the swatch click moved the colour; the field must follow, not stay \
         stuck on the dead prefix"
    );
}

/// Web shape of the same class: the `value` attribute holds only what was
/// last *written* (mount value or a previous rewrite), so once the author has
/// typed, the attribute must never speak for the field — otherwise a colour
/// moving BACK to the attribute's fossil value is wrongly judged "already
/// shown" and the live text is never fixed.
#[test]
fn a_return_to_the_last_written_colour_still_rewrites_on_web() {
    let picker = Mounted::picker("#ff0000", "hex");

    picker.click_swatch(); // rewrite lands: attribute (and live text) "#22aa55"
    assert_eq!(picker.field_text(), "#22aa55");

    // Web-shaped typing: the live text changes, the attribute does not.
    dispatch_input_event(picker.field_handler(), "#336".to_string());

    picker.click_swatch(); // back to "#22aa55" — equal to the fossil attribute

    // The write must land: on web it is the only thing that repairs the live
    // text (via the value-property mirror). The typed record ("#336") is the
    // field's truth here, and it disagrees.
    assert_eq!(
        picker.field_text(),
        "#22aa55",
        "the colour moved away from the author's text; the field must be rewritten \
         even though the stale attribute already spelled the target colour"
    );
}

/// The display comparison is against the *colour*, alpha included — not the
/// format-projected string. A typed 8-digit hex may keep asserting its alpha
/// only while the picker really holds that alpha; the moment the alpha slider
/// moves it, the text no longer denotes the colour and must be rewritten,
/// even though a hex format renders both states identically.
#[test]
fn an_alpha_move_rewrites_a_typed_alpha_pair_under_a_hex_format() {
    let picker = Mounted::picker("#333366", "hex");

    picker.type_text("#3333666c"); // alpha 0x6c lands internally; text kept
    assert_eq!(picker.field_text(), "#3333666c");

    // Drag the alpha slider to fully opaque.
    let overlay =
        find_by_class(&picker.root, "rinch-color-picker__alpha-overlay").expect("alpha overlay");
    let id = EventHandlerId(
        overlay
            .get_attribute("data-rid")
            .expect("alpha overlay is clickable")
            .parse()
            .expect("handler id is numeric"),
    );
    set_click_context(ClickContext {
        element_width: 200.0,
        element_height: 20.0,
        mouse_x: 200.0, // percent_x = 1.0 → alpha 1.0
        ..Default::default()
    });
    dispatch_event(id);

    assert_eq!(
        picker.field_text(),
        "#333366",
        "the picker no longer holds the alpha the text asserts; keeping \
         \"#3333666c\" would let a copy of the field reproduce a dead alpha"
    );
}

/// ColorInput's field-display effect must not react to the dropdown state:
/// clicking the input group (the text field included) toggles `opened`, and
/// an effect coupled to it would rewrite the field while the author's
/// mid-typing text is still unparseable — with no colour change at all.
#[test]
fn an_opened_toggle_does_not_clobber_unparseable_midtyping_text() {
    let input = Mounted::color_input("#ff0000");

    input.type_text("#33"); // unparseable: no colour change, nothing to show

    // The author clicks in the field to reposition the caret — the input
    // group's click handler toggles the dropdown.
    let group = find_by_class(&input.root, "rinch-color-input__input-group").expect("input group");
    let id = EventHandlerId(
        group
            .get_attribute("data-rid")
            .expect("group is clickable")
            .parse()
            .expect("handler id is numeric"),
    );
    dispatch_event(id);

    assert_eq!(
        input.field_text(),
        "#33",
        "no colour change happened; the author's text must survive the toggle"
    );
}

/// An emptied field is not "still the author's": select-all + delete records
/// the empty text, so the next real colour movement (a dropdown pick of the
/// previously typed colour included) repopulates the field.
#[test]
fn an_emptied_field_is_repopulated_by_the_next_pick() {
    let input = Mounted::color_input("#ff0000");

    input.type_text("#22aa55"); // commits; guard leaves the author's text
    input.type_text(""); // select-all + delete: recorded, unparseable

    input.click_swatch(); // picks "#22aa55" — the same colour again

    assert_eq!(
        input.field_text(),
        "#22aa55",
        "the field was empty; an explicit pick must repopulate it even though \
         the colour value did not change"
    );
}

/// The #231 residual (#226): the mid-typing guard leaves a committed
/// shorthand in the field forever — an attribute-reading consumer then sees
/// "336" where the picker holds #333366. The commit boundary ends the
/// author's claim: the field normalizes to the canonical form. The picker's
/// own `onchange` already reported the color when the typed transition landed
/// — the commit adds no second report.
#[test]
fn a_committed_shorthand_is_normalized_in_the_picker_field() {
    let picker = Mounted::picker("#ff0000", "hex");

    picker.type_text("336");
    assert_eq!(
        picker.field_text(),
        "336",
        "mid-gesture the field is still the author's (#231)"
    );
    assert_eq!(picker.emissions(), vec!["#333366".to_string()]);

    picker.commit("336");

    assert_eq!(
        picker.field_text(),
        "#333366",
        "the commit ended the gesture; the field denotes the colour canonically"
    );
    assert_eq!(
        picker.emissions(),
        vec!["#333366".to_string()],
        "the commit normalizes the field, it does not re-report the colour"
    );
}

/// An unparseable commit reverts the field to the color the picker still
/// holds — committed garbage must not outlive the gesture that typed it.
#[test]
fn an_unparseable_commit_reverts_the_picker_field() {
    let picker = Mounted::picker("#ff0000", "hex");

    picker.type_text("#33");
    assert_eq!(picker.field_text(), "#33");

    picker.commit("#33");

    assert_eq!(
        picker.field_text(),
        "#ff0000",
        "no colour was committed; the field returns to the one the picker holds"
    );
    assert_eq!(picker.emissions(), Vec::<String>::new());
}

/// ColorInput: the same normalize-on-commit, and the store gets exactly one
/// commit ("336" typed → blur → field "#333366").
#[test]
fn color_input_normalizes_a_committed_shorthand() {
    let input = Mounted::color_input("#ff0000");

    input.type_text("336");
    assert_eq!(input.field_text(), "336");
    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "typing previews; nothing commits mid-gesture"
    );

    input.commit("336");

    assert_eq!(input.field_text(), "#333366");
    assert_eq!(
        input.emissions(),
        vec!["#333366".to_string()],
        "the store gets one commit, in canonical form"
    );
}

/// ColorInput: an unparseable commit reverts the field to the held color and
/// reports nothing — the color never changed.
#[test]
fn color_input_reverts_an_unparseable_commit() {
    let input = Mounted::color_input("#ff0000");

    input.type_text("#zz");
    assert_eq!(input.field_text(), "#zz");

    input.commit("#zz");

    assert_eq!(input.field_text(), "#ff0000");
    assert_eq!(input.emissions(), Vec::<String>::new());
}

/// A preview is not a commit: a parseable keystroke moves the internal colour
/// (the swatch follows) without reporting, so an unparseable commit must
/// revert to the last colour the app actually holds — never to that leaked
/// preview. Pre-fix the revert target was `current_value`, so typing "336",
/// spoiling it to "336x", and blurring left the field (and swatch) durably
/// showing #333366 while the app still believed #ff0000.
#[test]
fn a_preview_never_committed_is_reverted_on_unparseable_commit() {
    let input = Mounted::color_input("#ff0000");
    let swatch_style = || {
        find_by_class(&input.root, "rinch-color-input__swatch-preview")
            .expect("preview swatch")
            .children()
            .first()
            .expect("swatch overlay")
            .get_attribute("style")
            .expect("overlay styled")
    };

    input.type_text("336"); // parseable: previews internally, reports nothing
    assert_eq!(input.field_text(), "336");
    assert!(
        swatch_style().contains("#333366"),
        "the preview reached the swatch: {}",
        swatch_style()
    );
    input.type_text("336x"); // unparseable: the preview stays behind internally

    input.commit("336x");

    assert_eq!(
        input.field_text(),
        "#ff0000",
        "the revert target is the last committed colour, not the leaked preview"
    );
    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "nothing was ever committed, so nothing reports"
    );
    assert!(
        swatch_style().contains("#ff0000"),
        "the swatch returned with the field: {}",
        swatch_style()
    );
}

/// Re-spelling the held colour in another notation is not a colour change:
/// committing "rgb(255, 0, 0)" over #ff0000 still normalizes the field, but
/// reports nothing — the prop promises a report when a colour CHANGE commits,
/// and a phantom report would echo the app's own value back at it.
#[test]
fn a_notation_only_commit_reports_nothing() {
    let input = Mounted::color_input("#ff0000");

    input.type_text("rgb(255, 0, 0)");
    input.commit("rgb(255, 0, 0)");

    assert_eq!(
        input.field_text(),
        "#ff0000",
        "the commit still normalizes the field to the canonical form"
    );
    assert_eq!(
        input.emissions(),
        Vec::<String>::new(),
        "the colour did not change; nothing reports"
    );
}

/// Multibyte text whose hex part is 3, 6, or 8 *bytes* long ("#é3") must be
/// "not a colour", not a byte-slice panic — it flows through `parse_color`
/// on every keystroke and through the guard on every display-effect run.
#[test]
fn multibyte_text_is_not_a_colour_and_not_a_panic() {
    // Guard path: a multibyte value prop sits in the attribute at mount.
    let input = Mounted::color_input("#é3");
    assert_eq!(input.field_text(), "#é3");

    // Handler path: typing it into the picker's hex field.
    let picker = Mounted::picker("#ff0000", "hex");
    picker.type_text("#é3");
    assert_eq!(picker.field_text(), "#é3");
    assert_eq!(picker.emissions(), Vec::<String>::new());
}
