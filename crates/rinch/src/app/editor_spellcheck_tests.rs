//! The rich-text editor's spelling marks on the desktop host (#2862): a press
//! starts the checker, the misspelled words become `pm-spell-error`
//! decorations, and the stylesheet turns those into the wavy underline over
//! exactly those words, coloured and styled by the custom properties.
//!
//! Checked against the vendored `en_TEST` dictionary, jobs run inline.

use super::*;
use rinch_core::Component;
use rinch_dom::computed_style::TextDecorationStyleValue;
use rinch_editor_core::Pos;
use rinch_spell::HunspellChecker;
use rinch_spell::runtime::{self as spell, SpellcheckConfig};

const AFF: &[u8] = include_bytes!("../../../rinch-spell/tests/fixtures/en_TEST.aff");
const DIC: &[u8] = include_bytes!("../../../rinch-spell/tests/fixtures/en_TEST.dic");
const VP: (u32, u32) = (800, 600);
const STYLE: &str = "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
                     font-family: sans-serif";

fn configure() {
    spell::set_inline_jobs(true);
    spell::set_config(
        SpellcheckConfig::new()
            .checker(HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap())
            .debounce_ms(0),
    );
}

/// An `Editor` holding `html`, inside a `div` with `wrap_style`; `prop` is
/// its `spellcheck` prop.
fn page(
    html: &'static str,
    wrap_style: &'static str,
    prop: Option<bool>,
) -> (RinchApp, u64, crate::editor::EditorHandle) {
    let slot: Rc<RefCell<Option<(u64, crate::editor::EditorHandle)>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", wrap_style);
        let handle = crate::editor::create_editor();
        handle.load_html(html);
        let container = crate::editor::Editor {
            editor: Some(handle.clone()),
            spellcheck: prop,
            ..Default::default()
        }
        .render(scope, &[]);
        container.set_attribute("style", STYLE);
        root.append_child(&container);
        *slot_in.borrow_mut() = Some((container.node_id().0, handle));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, handle) = slot.borrow_mut().take().unwrap();
    (app, container, handle)
}

fn press_at(app: &mut RinchApp, handle: &crate::editor::EditorHandle, pos: usize) {
    let (x, y, h) = app.editor_caret_point(handle, Pos(pos)).unwrap();
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    app.resolve_and_repaint(800.0, 600.0);
}

/// A squiggle: its text, whether it underlines, its style and colour.
type Squiggle = (String, bool, TextDecorationStyleValue, Option<u32>);

/// Each mounted squiggle, in document order.
fn squiggles(app: &RinchApp) -> Vec<Squiggle> {
    let d = app.doc.as_ref().unwrap().borrow();
    let mut out: Vec<(u64, Squiggle)> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes.contains_key("data-pm-deco")
                && n.attributes
                    .get("class")
                    .is_some_and(|c| c.split_whitespace().any(|c| c == "pm-spell-error"))
        })
        .map(|(id, n)| {
            let td = &n.computed_style.text_decoration;
            (
                id,
                (
                    rinch_dom::testing::get_text_content(&d.tree, id),
                    td.underline,
                    td.style,
                    td.color.map(|c| c.to_rgba8().to_u32()),
                ),
            )
        })
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out.into_iter().map(|(_, s)| s).collect()
}

fn rgb(r: u8, g: u8, b: u8) -> Option<u32> {
    Some(peniko::Color::from_rgb8(r, g, b).to_rgba8().to_u32())
}

/// A press in the editor marks its misspelled words: a decoration over each,
/// computed as Chrome's red wave, and laid out as a wavy span over exactly
/// those bytes of the paragraph.
#[test]
fn a_press_marks_the_words_with_red_waves() {
    configure();
    let (mut app, _, handle) = page("<p>helo the wrld</p>", "", None);
    assert!(squiggles(&app).is_empty(), "marked before any interaction");
    press_at(&mut app, &handle, 6);
    let red = rgb(255, 0, 0);
    assert_eq!(
        squiggles(&app),
        [
            (
                "helo".to_string(),
                true,
                TextDecorationStyleValue::Wavy,
                red
            ),
            (
                "wrld".to_string(),
                true,
                TextDecorationStyleValue::Wavy,
                red
            ),
        ]
    );
    // The IFC carries a wavy span over those bytes of "helo the wrld".
    let d = app.doc.as_ref().unwrap().borrow();
    let spans: Vec<(usize, usize)> = d
        .tree
        .nodes
        .iter()
        .filter_map(|(_, n)| n.text_layout.as_ref())
        .flat_map(|l| l.decoration_spans.iter().map(|s| (s.start, s.end)))
        .collect();
    assert_eq!(spans, [(0, 4), (9, 13)]);
}

/// A focus from code alone marks nothing (the first edit after it does:
/// `rinch-editor-view`'s `spell_after_dispatch` on the user's input).
#[test]
fn a_script_focus_alone_marks_nothing() {
    configure();
    let (mut app, container, _) = page("<p>helo the wrld</p>", "", None);
    app.try_focus_input(container);
    assert_eq!(app.focus_target, FocusTarget::Editor(container));
    app.resolve_and_repaint(800.0, 600.0);
    assert!(squiggles(&app).is_empty());
}

/// The colour and the line come from the inherited custom properties.
#[test]
fn the_custom_properties_style_the_marks() {
    configure();
    let (mut app, _, handle) = page(
        "<p>helo</p>",
        "--rinch-spellcheck-color: rgb(0, 0, 255); --rinch-spellcheck-style: dotted",
        None,
    );
    press_at(&mut app, &handle, 2);
    assert_eq!(
        squiggles(&app),
        [(
            "helo".to_string(),
            true,
            TextDecorationStyleValue::Dotted,
            rgb(0, 0, 255)
        )]
    );

    let (mut app, _, handle) = page("<p>helo</p>", "--rinch-spellcheck-style: none", None);
    press_at(&mut app, &handle, 2);
    let s = squiggles(&app);
    assert_eq!(s.len(), 1, "the decoration is still there");
    assert!(!s[0].1, "`none` still drew a line");
}

/// The `spellcheck` prop, and the attribute inherited from an ancestor.
#[test]
fn the_prop_turns_the_editor_off() {
    configure();
    let (mut app, _, handle) = page("<p>helo</p>", "", Some(false));
    press_at(&mut app, &handle, 2);
    assert!(
        squiggles(&app).is_empty(),
        "checked with `spellcheck: false`"
    );
}
