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

    /// One keystroke, desktop-shaped: the runtime mirrors the field's text
    /// into the `value` attribute, then dispatches `oninput` with it.
    fn type_text(&self, text: &str) {
        self.field().set_attribute("value", text);
        dispatch_input_event(self.field_handler(), text.to_string());
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
        set_click_context(ClickContext {
            element_width: 200.0,
            element_height: 200.0,
            ..Default::default()
        });
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
        vec!["#333366".to_string(), "#3366cc".to_string()]
    );
}

/// And the same non-freeze control: a pick from the dropdown picker rewrites
/// the ColorInput's field.
#[test]
fn color_input_field_follows_the_dropdown_picker() {
    let input = Mounted::color_input("#ff0000");
    input.type_text("#336");
    assert_eq!(input.field_text(), "#336");

    input.click_swatch();

    assert_eq!(input.field_text(), "#22aa55");
    assert_eq!(
        input.emissions(),
        vec!["#333366".to_string(), "#22aa55".to_string()],
        "the prefix previewed, then the picked colour committed"
    );
}
