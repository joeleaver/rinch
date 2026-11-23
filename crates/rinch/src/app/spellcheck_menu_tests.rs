//! Spelling in the text context menu (#2862, stage 3), driven through the
//! real `PlatformEvent` path: a right press on a word the spellchecker marked
//! selects it and opens the menu with its suggestions on top, then "Add to
//! dictionary", then Cut / Copy / Paste / Select all, as Chrome 158 does
//! (measured: `docs/internals/spellcheck.md`, "The context menu").
//!
//! Every test checks against the vendored `en_TEST` dictionary (never the
//! host's) with the checking thread's jobs inline, so marks and suggestions
//! are there as soon as they are asked for. The words sit off the field's
//! edges ("the helo wrld": the marked word starts at byte 4, not 0). Text
//! geometry pins the local fonts, so every field declares its font and line
//! height.

use super::*;
use rinch_core::Component;
use rinch_core::dom::NodeId;
use rinch_core::events::{InputCallback, register_input_handler};
use rinch_editor_core::Pos;
use rinch_spell::runtime::{self as spell, SpellcheckConfig};
use rinch_spell::{HunspellChecker, PersonalDictionary, SpellChecker};
use std::sync::{Arc, Mutex};

const AFF: &[u8] = include_bytes!("../../../rinch-spell/tests/fixtures/en_TEST.aff");
const DIC: &[u8] = include_bytes!("../../../rinch-spell/tests/fixtures/en_TEST.dic");
const VP: (u32, u32) = (800, 600);
const FIELD: &str = "display: block; width: 400px; height: 30px; font-size: 16px; \
                     line-height: 20px; font-family: sans-serif";
const EDITOR: &str = "width: 400px; height: 120px; font-size: 16px; line-height: 24px; \
                      font-family: sans-serif";

fn checker() -> HunspellChecker {
    HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap()
}

fn configure(config: SpellcheckConfig) {
    spell::set_inline_jobs(true);
    spell::set_config(config.debounce_ms(0));
}

fn default_config() -> SpellcheckConfig {
    SpellcheckConfig::new().checker(checker())
}

type Log = Rc<RefCell<Vec<String>>>;

/// What a page holds: its fields by name, and its editor (when it has one).
struct Page {
    app: RinchApp,
    fields: Vec<u64>,
    editor: Option<crate::editor::EditorHandle>,
    log: Log,
}

/// A page with one field per `(tag, value)`, each logging its `oninput` as
/// `"<index>:<value>"`, and an `Editor` holding `editor_html` if given, whose
/// `on_change` logs `"editor"`.
fn page(fields: &[(&'static str, &'static str)], editor_html: Option<&'static str>) -> Page {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let handlers: Vec<usize> = (0..fields.len())
        .map(|i| {
            let log = log.clone();
            register_input_handler(InputCallback::new(move |v: String| {
                log.borrow_mut().push(format!("{i}:{v}"));
            }))
            .0
        })
        .collect();
    let fields_in: Vec<(&'static str, &'static str)> = fields.to_vec();
    type Slot = Rc<RefCell<(Vec<u64>, Option<crate::editor::EditorHandle>)>>;
    let slot: Slot = Rc::new(RefCell::new((Vec::new(), None)));
    let slot_in = slot.clone();
    let log_in = log.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 20px 30px");
        let mut ids = Vec::new();
        for (i, (tag, value)) in fields_in.iter().enumerate() {
            let f = scope.create_element(tag);
            f.set_attribute("style", FIELD);
            f.set_attribute("value", value);
            f.set_attribute("data-oninput", &handlers[i].to_string());
            root.append_child(&f);
            ids.push(f.node_id().0);
        }
        let mut editor = None;
        if let Some(html) = editor_html {
            let handle = crate::editor::create_editor();
            handle.load_html(html);
            let log = log_in.clone();
            handle.on_change(move || log.borrow_mut().push("editor".into()));
            let container = crate::editor::Editor {
                editor: Some(handle.clone()),
                ..Default::default()
            }
            .render(scope, &[]);
            container.set_attribute("style", EDITOR);
            root.append_child(&container);
            editor = Some(handle);
        }
        *slot_in.borrow_mut() = (ids, editor);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (fields, editor) = slot.borrow_mut().clone();
    Page {
        app,
        fields,
        editor,
        log,
    }
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    let actions = app.handle_event(event, VP, 1.0);
    app.run_deferred_work();
    actions
}

fn press(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) -> Vec<AppAction> {
    let mut actions = ev(app, PlatformEvent::MouseDown { x, y, button });
    actions.extend(ev(app, PlatformEvent::MouseUp { x, y, button }));
    app.resolve_and_repaint(800.0, 600.0);
    actions
}

fn key_with(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
    app.resolve_and_repaint(800.0, 600.0);
}

fn undo(app: &mut RinchApp) {
    let primary = Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    };
    key_with(app, KeyCode::KeyZ, primary);
}

/// The middle of bytes `from..to` of field `id`'s text, in window px.
fn field_point(app: &mut RinchApp, id: u64, from: usize, to: usize) -> (f32, f32) {
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let mut caret = |offset| {
        RinchApp::input_caret_rect_with_affinity(
            &d.tree,
            &mut app.hit_test_font_cx,
            &mut app.hit_test_layout_cx,
            id,
            offset,
            rinch_core::dom::CaretAffinity::Downstream,
        )
        .unwrap()
    };
    let (a, b) = (caret(from), caret(to));
    ((a.0 + b.0) / 2.0, a.1 + a.3 / 2.0)
}

/// Inside the editor's document range `from..to` (one line), in window px.
fn editor_point(
    app: &RinchApp,
    h: &crate::editor::EditorHandle,
    from: usize,
    to: usize,
) -> (f32, f32) {
    let (x1, y, height) = app.editor_caret_point(h, Pos(from)).unwrap();
    let (x2, _, _) = app.editor_caret_point(h, Pos(to)).unwrap();
    ((x1 + x2) / 2.0, y + height / 2.0)
}

/// Start checking field `id` the way a user does (a press in it), and let
/// the result land.
fn activate(app: &mut RinchApp, id: u64) {
    let (x, y, w, h) = painted_element_box(&app.doc.as_ref().unwrap().borrow().tree, id);
    press(app, (x + w - 4.0, y + h / 2.0), MouseButton::Left);
}

/// The words field `id` shows marked.
fn field_marks(app: &mut RinchApp, id: u64) -> Vec<String> {
    while app.run_deferred_work() > 0 {}
    let d = app.doc.as_ref().unwrap().borrow();
    d.spelling_marks(id)
        .map(|m| {
            m.ranges
                .iter()
                .map(|r| m.text[r.clone()].to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The words the editor shows marked, in order.
fn editor_marks(h: &crate::editor::EditorHandle) -> Vec<String> {
    h.misspellings().into_iter().map(|m| m.word).collect()
}

/// The editor's text (one paragraph in these tests).
fn editor_text(h: &crate::editor::EditorHandle) -> String {
    let doc = h.doc();
    let mut out = String::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, _, _| {
        if let Some(t) = n.text() {
            out.push_str(t);
        }
        true
    });
    out
}

/// The field's `(value, selection)` as its editable state holds them.
fn field(app: &RinchApp) -> (String, usize, usize) {
    let s = app.focused_input_state.as_ref().unwrap();
    (
        s.document.to_text(),
        s.selection.start().0,
        s.selection.end().0,
    )
}

/// The open menu's rows as `(action, enabled)`, in menu order.
fn rows(app: &RinchApp) -> Vec<(TextEditAction, bool)> {
    app.open_text_menu
        .as_ref()
        .map(|m| m.items.iter().map(|r| (r.action, r.enabled)).collect())
        .unwrap_or_default()
}

/// What the open panel shows, top to bottom: each row's label (a disabled
/// row's in brackets) and `"--"` for a separator, as laid out (a hidden node
/// has no box).
fn panel_lines(app: &RinchApp) -> Vec<String> {
    let open = app.open_text_menu.as_ref().expect("the menu is open");
    let d = app.doc.as_ref().unwrap().borrow();
    let panel = d.tree.get(open.panel_id).unwrap();
    let mut out = Vec::new();
    for &child in &panel.children {
        let n = d.tree.get(child).unwrap();
        if n.layout.height <= 0.0 {
            continue;
        }
        let class = n.attributes.get("class").cloned().unwrap_or_default();
        if class.contains("rinch-tcm-sep") {
            out.push("--".to_string());
            continue;
        }
        let label = n
            .children
            .first()
            .map(|&l| rinch_dom::testing::get_text_content(&d.tree, l))
            .unwrap_or_default();
        out.push(if n.attributes.contains_key("data-disabled") {
            format!("[{label}]")
        } else {
            label
        });
    }
    out
}

fn row_point(app: &RinchApp, action: TextEditAction) -> (f32, f32) {
    let id = app
        .open_text_menu
        .as_ref()
        .and_then(|m| m.items.iter().find(|r| r.action == action))
        .map(|r| r.node_id)
        .expect("the menu has the row");
    let (x, y, w, h) = painted_element_box(&app.doc.as_ref().unwrap().borrow().tree, id);
    (x + w / 2.0, y + h / 2.0)
}

const EDIT_ROWS: [TextEditAction; 4] = [
    TextEditAction::Cut,
    TextEditAction::Copy,
    TextEditAction::Paste,
    TextEditAction::SelectAll,
];

/// The edit rows' labels as the panel shows them.
fn edit_lines() -> Vec<String> {
    edit_lines_for(true)
}

/// The edit rows as drawn: Cut and Copy need a selection, and all three
/// clipboard rows need the `clipboard` feature.
fn edit_lines_for(selection: bool) -> Vec<String> {
    let clipboard = cfg!(feature = "clipboard");
    let line = |l: &str, on: bool| if on { l.to_string() } else { format!("[{l}]") };
    vec![
        line("Cut", clipboard && selection),
        line("Copy", clipboard && selection),
        line("Paste", clipboard),
        "--".into(),
        "Select all".into(),
    ]
}

/// A right press on a marked word in a `<textarea>` selects the word, as
/// Chrome does, and the menu shows its suggestions (the engine's, at most
/// three), a separator, "Add to dictionary", a separator, then the edit
/// rows. The state a shell reads says the same.
#[test]
fn a_marked_word_opens_its_suggestions_above_the_edit_rows() {
    configure(default_config());
    let mut p = page(&[("textarea", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    assert_eq!(field_marks(&mut p.app, id), ["helo", "wrld"]);

    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    assert_eq!(
        field(&p.app),
        ("the helo wrld".into(), 4, 8),
        "the word is selected"
    );

    let expected = checker().suggest("helo", 3);
    assert_eq!(expected, ["hello", "house"], "the fixture's suggestions");
    let mut want: Vec<(TextEditAction, bool)> = vec![
        (TextEditAction::Suggestion(0), true),
        (TextEditAction::Suggestion(1), true),
        (TextEditAction::AddToDictionary, true),
    ];
    let clip = cfg!(feature = "clipboard");
    want.extend([(TextEditAction::Cut, clip), (TextEditAction::Copy, clip)]);
    want.extend([
        (TextEditAction::Paste, clip),
        (TextEditAction::SelectAll, true),
    ]);
    assert_eq!(rows(&p.app), want);
    let mut lines: Vec<String> = vec!["hello".into(), "house".into(), "--".into()];
    lines.push("Add to dictionary".into());
    lines.push("--".into());
    lines.extend(edit_lines());
    assert_eq!(panel_lines(&p.app), lines);

    let s = p.app.text_edit_state().unwrap().spelling.unwrap();
    assert_eq!(s.word, "helo");
    assert_eq!(s.suggestions, ["hello", "house"]);
    assert!(s.can_add_to_dictionary && !s.suggestions_pending && !s.show_no_suggestions);
}

/// A checker with five suggestions for every misspelled word, recording the
/// most it was asked for.
struct Many(HunspellChecker, Arc<Mutex<Vec<usize>>>);

impl SpellChecker for Many {
    fn check(&self, word: &str) -> bool {
        self.0.check(word)
    }
    fn suggest(&self, _: &str, max: usize) -> Vec<String> {
        self.1.lock().unwrap().push(max);
        ["one", "two", "three", "four", "five"]
            .iter()
            .take(max)
            .map(|s| s.to_string())
            .collect()
    }
    fn add_to_dictionary(&self, word: &str) {
        self.0.add_to_dictionary(word)
    }
    fn ignore(&self, word: &str) {
        self.0.ignore(word)
    }
    fn languages(&self) -> Vec<String> {
        self.0.languages()
    }
}

/// The menu shows the engine's suggestions up to the configured number
/// (default three, Chrome's), best first, and asks the engine for no more;
/// `0` asks for none.
#[test]
fn the_suggestion_count_is_configurable() {
    for (count, asked) in [(None, vec![3]), (Some(1), vec![1]), (Some(0), vec![])] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut config = SpellcheckConfig::new().checker(Many(checker(), seen.clone()));
        if let Some(n) = count {
            config = config.menu_suggestions(n);
        }
        configure(config);
        let mut p = page(&[("input", "a wrold")], None);
        let id = p.fields[0];
        activate(&mut p.app, id);
        let at = field_point(&mut p.app, id, 3, 5);
        press(&mut p.app, at, MouseButton::Right);
        let n = asked.first().copied().unwrap_or(0);
        let got: Vec<TextEditAction> = rows(&p.app)
            .into_iter()
            .map(|(a, _)| a)
            .filter(|a| matches!(a, TextEditAction::Suggestion(_)))
            .collect();
        let expect: Vec<TextEditAction> = (0..n).map(TextEditAction::Suggestion).collect();
        assert_eq!(got, expect, "menu_suggestions({count:?})");
        assert_eq!(&panel_lines(&p.app)[..n], &["one", "two", "three"][..n]);
        assert_eq!(*seen.lock().unwrap(), asked, "{count:?}");
    }
}

/// A word with no suggestion shows no row for it (Chrome 158 shows none,
/// only "Add to dictionary"); the app can ask for Chrome's disabled "No
/// spelling suggestions" row, and can take "Add to dictionary" away.
#[test]
fn no_suggestions_shows_add_to_dictionary_or_the_optional_row() {
    assert!(checker().suggest("qwzx", 3).is_empty());
    for (no_row, add, spell_lines) in [
        (false, true, vec!["Add to dictionary", "--"]),
        (
            true,
            true,
            vec!["[No spelling suggestions]", "--", "Add to dictionary", "--"],
        ),
        (true, false, vec!["[No spelling suggestions]", "--"]),
        (false, false, vec![]),
    ] {
        configure(
            default_config()
                .menu_no_suggestions_row(no_row)
                .menu_add_to_dictionary(add),
        );
        let mut p = page(&[("input", "the qwzx is")], None);
        let id = p.fields[0];
        activate(&mut p.app, id);
        let at = field_point(&mut p.app, id, 5, 7);
        press(&mut p.app, at, MouseButton::Right);
        let mut want: Vec<String> = spell_lines.iter().map(|s| s.to_string()).collect();
        want.extend(edit_lines());
        assert_eq!(panel_lines(&p.app), want, "no_row {no_row}, add {add}");
        // The stand-in row is not an item: the keyboard and the pointer pass it.
        let mut items: Vec<TextEditAction> = Vec::new();
        if add {
            items.push(TextEditAction::AddToDictionary);
        }
        items.extend(EDIT_ROWS);
        assert_eq!(
            rows(&p.app).into_iter().map(|(a, _)| a).collect::<Vec<_>>(),
            items
        );
    }
}

/// A right press on a word that is not marked opens the plain menu, with the
/// caret at the press (no word selected); a press on a marked word inside a
/// wider selection keeps the selection and offers no spelling, as Chrome
/// does. Neither leaves spelling in the state.
#[test]
fn an_unmarked_word_or_a_wider_selection_gets_no_spelling() {
    configure(default_config());
    let mut p = page(&[("textarea", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    assert_eq!(field_marks(&mut p.app, id), ["helo", "wrld"]);

    let at = field_point(&mut p.app, id, 1, 2);
    press(&mut p.app, at, MouseButton::Right);
    let (_, start, end) = field(&p.app);
    assert_eq!(start, end, "a caret, no selection");
    assert!(start <= 3);
    assert_eq!(
        rows(&p.app).iter().map(|r| r.0).collect::<Vec<_>>(),
        EDIT_ROWS
    );
    assert_eq!(panel_lines(&p.app), edit_lines_for(false));
    assert_eq!(p.app.text_edit_state().unwrap().spelling, None);
    key_with(&mut p.app, KeyCode::Escape, Modifiers::default());

    p.app.focused_input_state.as_mut().unwrap().selection = rinch_editable::Selection::new(2, 11);
    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    assert_eq!(field(&p.app), ("the helo wrld".into(), 2, 11), "kept");
    assert_eq!(
        rows(&p.app).iter().map(|r| r.0).collect::<Vec<_>>(),
        EDIT_ROWS
    );
    assert_eq!(p.app.text_edit_state().unwrap().spelling, None);
}

/// Choosing a suggestion in an `<input>` and in a `<textarea>` replaces the
/// word as typing would: `oninput` once with the new value, the caret after
/// the replacement, the word's mark gone at once and not put back by the
/// re-check, and one undo bringing the word back.
#[test]
fn a_suggestion_replaces_the_word_in_a_field_as_one_undoable_edit() {
    for tag in ["input", "textarea"] {
        configure(default_config());
        let mut p = page(&[(tag, "the helo wrld")], None);
        let id = p.fields[0];
        activate(&mut p.app, id);
        let at = field_point(&mut p.app, id, 5, 7);
        press(&mut p.app, at, MouseButton::Right);
        p.log.borrow_mut().clear();

        let row = row_point(&p.app, TextEditAction::Suggestion(0));
        press(&mut p.app, row, MouseButton::Left);
        assert!(!p.app.is_text_context_menu_open(), "{tag}");
        assert_eq!(field(&p.app), ("the hello wrld".into(), 9, 9), "{tag}");
        assert_eq!(*p.log.borrow(), ["0:the hello wrld"], "{tag}: oninput once");
        assert_eq!(field_marks(&mut p.app, id), ["wrld"], "{tag}: at once");
        p.app.spell_tick();
        assert_eq!(
            field_marks(&mut p.app, id),
            ["wrld"],
            "{tag}: not re-marked"
        );
        assert_eq!(p.app.text_edit_state().unwrap().spelling, None, "{tag}");

        undo(&mut p.app);
        assert_eq!(field(&p.app).0, "the helo wrld", "{tag}: one undo");
        assert_eq!(
            p.log.borrow().last().map(String::as_str),
            Some("0:the helo wrld"),
            "{tag}"
        );
        p.app.spell_tick();
        assert_eq!(field_marks(&mut p.app, id), ["helo", "wrld"], "{tag}");
    }
}

/// The same in the editor: the word selected by the press, the suggestion
/// in its place through the typing path (`on_change` once), the caret after
/// it, the mark gone, and one undo bringing the word back, selected (the
/// history restores the selection the edit started from, as Chrome does).
#[test]
fn a_suggestion_replaces_the_word_in_the_editor_as_one_undoable_edit() {
    configure(default_config());
    let mut p = page(&[], Some("<p>the helo wrld</p>"));
    let h = p.editor.clone().unwrap();
    let at = editor_point(&p.app, &h, 13, 14);
    press(&mut p.app, at, MouseButton::Left);
    assert_eq!(editor_marks(&h), ["helo", "wrld"]);

    // "the helo wrld": the paragraph opens at 0, "helo" is 5..9.
    let at = editor_point(&p.app, &h, 6, 8);
    press(&mut p.app, at, MouseButton::Right);
    let sel = h.selection();
    assert_eq!(
        (sel.from(), sel.to()),
        (Pos(5), Pos(9)),
        "the word is selected"
    );
    assert_eq!(
        rows(&p.app)[..3],
        [
            (TextEditAction::Suggestion(0), true),
            (TextEditAction::Suggestion(1), true),
            (TextEditAction::AddToDictionary, true)
        ]
    );
    assert_eq!(
        panel_lines(&p.app)[..5],
        ["hello", "house", "--", "Add to dictionary", "--"]
    );
    p.log.borrow_mut().clear();

    let row = row_point(&p.app, TextEditAction::Suggestion(0));
    press(&mut p.app, row, MouseButton::Left);
    assert_eq!(editor_text(&h), "the hello wrld");
    let sel = h.selection();
    assert_eq!(
        (sel.from(), sel.to()),
        (Pos(10), Pos(10)),
        "the caret after it"
    );
    assert_eq!(*p.log.borrow(), ["editor"], "on_change once");
    assert_eq!(editor_marks(&h), ["wrld"]);

    undo(&mut p.app);
    assert_eq!(editor_text(&h), "the helo wrld", "one undo");
    let sel = h.selection();
    assert_eq!((sel.from(), sel.to()), (Pos(5), Pos(9)));
}

/// "Add to dictionary" puts the word in the app's personal dictionary (its
/// listener hears it, so the app can save it) and every mark of it goes at
/// once: in the field pressed, in another field, and in the editor, while
/// the other words stay marked and the selection stays.
#[test]
fn add_to_dictionary_clears_the_word_everywhere_and_tells_the_app() {
    let personal = Arc::new(PersonalDictionary::new());
    let heard = Arc::new(Mutex::new(Vec::new()));
    let heard_in = heard.clone();
    personal.add_listener(move |c| heard_in.lock().unwrap().push(format!("{c:?}")));
    configure(
        SpellcheckConfig::new().checker(checker().with_personal_dictionary(personal.clone())),
    );
    let mut p = page(
        &[("textarea", "the helo wrld"), ("input", "a helo")],
        Some("<p>helo cta</p>"),
    );
    let (a, b) = (p.fields[0], p.fields[1]);
    let h = p.editor.clone().unwrap();
    let at = editor_point(&p.app, &h, 7, 8);
    press(&mut p.app, at, MouseButton::Left);
    activate(&mut p.app, b);
    activate(&mut p.app, a);
    assert_eq!(field_marks(&mut p.app, a), ["helo", "wrld"]);
    assert_eq!(field_marks(&mut p.app, b), ["helo"]);
    assert_eq!(editor_marks(&h), ["helo", "cta"]);

    let at = field_point(&mut p.app, a, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    // The press alone, with no inbox turn after it: the marks go in the same
    // event, not on a later turn.
    let (x, y) = row_point(&p.app, TextEditAction::AddToDictionary);
    let button = MouseButton::Left;
    p.app
        .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    assert!(!p.app.is_text_context_menu_open());

    assert_eq!(*heard.lock().unwrap(), [r#"Added("helo")"#]);
    assert_eq!(personal.words(), ["helo"]);
    let d = p.app.doc.as_ref().unwrap().borrow();
    let marks = |id| {
        d.spelling_marks(id)
            .map(|m| {
                m.ranges
                    .iter()
                    .map(|r| m.text[r.clone()].to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    assert_eq!(
        marks(a),
        ["wrld"],
        "the field pressed, before any inbox turn"
    );
    assert_eq!(marks(b), Vec::<String>::new(), "another field");
    drop(d);
    assert_eq!(editor_marks(&h), ["cta"], "the editor");
    assert_eq!(
        field(&p.app),
        ("the helo wrld".into(), 4, 8),
        "the selection stays"
    );
    assert!(p.log.borrow().is_empty(), "no input event");
}

/// The menu row and the seam run the same thing: a shell presentation gets
/// the spelling in `text_edit_state` and `perform_text_edit(Suggestion(0))`
/// leaves the field, its events and its history exactly as the DOM row did.
#[test]
fn the_menu_row_and_the_seam_agree() {
    let run = |shell: bool| {
        configure(default_config());
        let mut p = page(&[("input", "the helo wrld")], None);
        let id = p.fields[0];
        activate(&mut p.app, id);
        if shell {
            p.app
                .set_text_context_menu_presentation(TextContextMenuPresentation::Shell);
        }
        let at = field_point(&mut p.app, id, 5, 7);
        let actions = press(&mut p.app, at, MouseButton::Right);
        assert_eq!(actions.contains(&AppAction::ShowTextContextMenu), shell);
        let state = p.app.text_edit_state().unwrap();
        let s = state.spelling.unwrap();
        assert_eq!(s.word, "helo");
        assert_eq!(s.suggestions, ["hello", "house"]);
        p.log.borrow_mut().clear();
        if shell {
            p.app.perform_text_edit(TextEditAction::Suggestion(0));
        } else {
            let row = row_point(&p.app, TextEditAction::Suggestion(0));
            press(&mut p.app, row, MouseButton::Left);
        }
        let after = (
            field(&p.app),
            p.log.borrow().clone(),
            field_marks(&mut p.app, id),
        );
        undo(&mut p.app);
        (after, field(&p.app).0)
    };
    let by_row = run(false);
    assert_eq!(by_row.0.0, ("the hello wrld".into(), 9, 9));
    assert_eq!(run(true), by_row);

    // And the seam does nothing once the word is no longer the selection.
    configure(default_config());
    let mut p = page(&[("input", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    p.app
        .set_text_context_menu_presentation(TextContextMenuPresentation::Shell);
    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    p.app.focused_input_state.as_mut().unwrap().selection = rinch_editable::Selection::cursor(1);
    assert_eq!(p.app.text_edit_state().unwrap().spelling, None);
    assert!(
        p.app
            .perform_text_edit(TextEditAction::Suggestion(0))
            .is_empty()
    );
    assert!(
        p.app
            .perform_text_edit(TextEditAction::AddToDictionary)
            .is_empty()
    );
    assert_eq!(field(&p.app).0, "the helo wrld");
}

/// A checker that takes its time over suggestions.
struct Slow(HunspellChecker, std::time::Duration);

impl SpellChecker for Slow {
    fn check(&self, word: &str) -> bool {
        self.0.check(word)
    }
    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        std::thread::sleep(self.1);
        self.0.suggest(word, max)
    }
    fn add_to_dictionary(&self, word: &str) {
        self.0.add_to_dictionary(word)
    }
    fn ignore(&self, word: &str) {
        self.0.ignore(word)
    }
    fn languages(&self) -> Vec<String> {
        self.0.languages()
    }
}

/// Slow suggestions do not hold the press: past `menu_suggest_wait_ms` the
/// menu opens with "Add to dictionary" and the state says they are pending;
/// when they arrive (from the suggestion thread, through the app's inbox) the
/// open menu shows them, and the row the user had arrowed onto stays
/// highlighted.
#[test]
fn slow_suggestions_are_filled_in_when_they_arrive() {
    let slow = std::time::Duration::from_millis(800);
    configure(
        SpellcheckConfig::new()
            .checker(Slow(checker(), slow))
            .menu_suggest_wait_ms(50),
    );
    let mut p = page(&[("textarea", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    assert_eq!(field_marks(&mut p.app, id), ["helo", "wrld"]);

    spell::set_inline_jobs(false);
    let started = std::time::Instant::now();
    let at = field_point(&mut p.app, id, 5, 7);
    let mut actions = p.app.handle_event(
        PlatformEvent::MouseDown {
            x: at.0,
            y: at.1,
            button: MouseButton::Right,
        },
        VP,
        1.0,
    );
    let waited = started.elapsed();
    actions.extend(p.app.handle_event(
        PlatformEvent::MouseUp {
            x: at.0,
            y: at.1,
            button: MouseButton::Right,
        },
        VP,
        1.0,
    ));
    assert!(waited < slow, "the press waited {waited:?}");
    let s = p.app.text_edit_state().unwrap().spelling.unwrap();
    assert!(s.suggestions_pending && s.suggestions.is_empty());
    let mut lines = vec!["Add to dictionary".to_string(), "--".into()];
    lines.extend(edit_lines());
    assert_eq!(panel_lines(&p.app), lines);
    // The user arrows past "Add to dictionary" onto an edit row.
    key_with(&mut p.app, KeyCode::ArrowDown, Modifiers::default());
    assert_eq!(highlighted(&p.app), Some(TextEditAction::AddToDictionary));
    key_with(&mut p.app, KeyCode::ArrowDown, Modifiers::default());
    let edit_row = highlighted(&p.app).unwrap();
    assert!(EDIT_ROWS.contains(&edit_row), "{edit_row:?}");
    assert!(
        p.app
            .text_edit_state()
            .unwrap()
            .spelling
            .unwrap()
            .suggestions_pending
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while p.app.run_deferred_work() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    spell::set_inline_jobs(true);
    assert!(p.app.is_text_context_menu_open());
    assert_eq!(
        panel_lines(&p.app)[..4],
        ["hello", "house", "--", "Add to dictionary"]
    );
    let s = p.app.text_edit_state().unwrap().spelling.unwrap();
    assert!(!s.suggestions_pending);
    assert_eq!(s.suggestions, ["hello", "house"]);
    assert_eq!(highlighted(&p.app), Some(edit_row), "the highlight is kept");
    let _ = actions;
}

fn highlighted(app: &RinchApp) -> Option<TextEditAction> {
    let m = app.open_text_menu.as_ref()?;
    m.highlighted.map(|i| m.items[i].action)
}

/// A late answer lands only on the request it answers: one for an older
/// word, arriving while a newer word's suggestions are pending, changes
/// nothing, and the newer word's own answer is taken.
#[test]
fn an_old_late_fill_does_not_land_on_a_newer_word() {
    configure(default_config());
    let mut p = page(&[("textarea", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    let old = p.app.text_spell.as_ref().unwrap().request;
    key_with(&mut p.app, KeyCode::Escape, Modifiers::default());
    let at = field_point(&mut p.app, id, 10, 12);
    press(&mut p.app, at, MouseButton::Right);
    let t = p.app.text_spell.as_mut().unwrap();
    assert_eq!(t.word, "wrld");
    let new = t.request;
    assert!(new > old);
    // Make the newer word's answer the one still awaited.
    t.pending = true;
    t.suggestions.clear();

    p.app.text_spell_arrived(old, vec!["stale".into()]);
    let s = p.app.text_edit_state().unwrap().spelling.unwrap();
    assert!(s.suggestions_pending && s.suggestions.is_empty(), "{s:?}");

    p.app.text_spell_arrived(new, vec!["world".into()]);
    let s = p.app.text_edit_state().unwrap().spelling.unwrap();
    assert!(!s.suggestions_pending);
    assert_eq!(s.suggestions, ["world"]);
    assert_eq!(
        panel_lines(&p.app)[..3],
        ["world", "--", "Add to dictionary"]
    );
}

/// The value changes under the open menu: choosing a suggestion then does
/// nothing, whether the app rewrote the value elsewhere (the marks were made
/// for the old text) or an edit put a different misspelled word where the
/// selection is.
#[test]
fn a_suggestion_does_nothing_once_the_value_changed() {
    // The app writes the value: the word is still at the selection, but the
    // marks were made for the old text.
    configure(default_config());
    let mut p = page(&[("input", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    assert!(p.app.text_edit_state().unwrap().spelling.is_some());
    p.app
        .doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_attribute(NodeId(id), "value", "the helo world");
    assert_eq!(p.app.text_edit_state().unwrap().spelling, None);
    assert_eq!(field(&p.app), ("the helo world".into(), 4, 8));
    p.log.borrow_mut().clear();
    assert!(
        p.app
            .perform_text_edit(TextEditAction::Suggestion(0))
            .is_empty()
    );
    assert_eq!(field(&p.app).0, "the helo world");
    assert!(p.log.borrow().is_empty());

    // An edit makes the word another misspelled word, marked at the same
    // place, and the selection is put back over it.
    configure(default_config());
    let mut p = page(&[("input", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    key_with(&mut p.app, KeyCode::Escape, Modifiers::default());
    p.app.focused_input_state.as_mut().unwrap().selection = rinch_editable::Selection::new(5, 6);
    p.app
        .handle_input_edit_command(rinch_editable::EditCommand::InsertText("x".into()));
    p.app.spell_tick();
    assert_eq!(field_marks(&mut p.app, id), ["hxlo", "wrld"]);
    p.app.focused_input_state.as_mut().unwrap().selection = rinch_editable::Selection::new(4, 8);
    assert_eq!(p.app.text_edit_state().unwrap().spelling, None);
    p.log.borrow_mut().clear();
    assert!(
        p.app
            .perform_text_edit(TextEditAction::Suggestion(0))
            .is_empty()
    );
    assert_eq!(field(&p.app).0, "the hxlo wrld");
    assert!(p.log.borrow().is_empty());
}

/// The keyboard walks the spelling rows like the edit rows: Down from
/// nothing to the first suggestion, on through "Add to dictionary" into the
/// edit rows, Up wrapping back; Enter runs the highlighted suggestion. A
/// disabled "No spelling suggestions" row is passed over.
#[test]
fn the_keyboard_walks_the_spelling_rows() {
    configure(default_config());
    let mut p = page(&[("textarea", "the helo wrld")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    let down = |p: &mut Page| key_with(&mut p.app, KeyCode::ArrowDown, Modifiers::default());
    down(&mut p);
    assert_eq!(highlighted(&p.app), Some(TextEditAction::Suggestion(0)));
    down(&mut p);
    assert_eq!(highlighted(&p.app), Some(TextEditAction::Suggestion(1)));
    down(&mut p);
    assert_eq!(highlighted(&p.app), Some(TextEditAction::AddToDictionary));
    down(&mut p);
    let first_edit = if cfg!(feature = "clipboard") {
        TextEditAction::Cut
    } else {
        TextEditAction::SelectAll
    };
    assert_eq!(highlighted(&p.app), Some(first_edit));
    key_with(&mut p.app, KeyCode::Home, Modifiers::default());
    key_with(&mut p.app, KeyCode::ArrowUp, Modifiers::default());
    assert_eq!(
        highlighted(&p.app),
        Some(TextEditAction::SelectAll),
        "wraps"
    );
    key_with(&mut p.app, KeyCode::ArrowDown, Modifiers::default());
    key_with(&mut p.app, KeyCode::ArrowDown, Modifiers::default());
    assert_eq!(highlighted(&p.app), Some(TextEditAction::Suggestion(1)));
    key_with(&mut p.app, KeyCode::Enter, Modifiers::default());
    assert!(!p.app.is_text_context_menu_open());
    assert_eq!(field(&p.app), ("the house wrld".into(), 9, 9));

    // The disabled stand-in row is never highlighted.
    configure(default_config().menu_no_suggestions_row(true));
    let mut p = page(&[("input", "the qwzx is")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    let at = field_point(&mut p.app, id, 5, 7);
    press(&mut p.app, at, MouseButton::Right);
    assert_eq!(panel_lines(&p.app)[0], "[No spelling suggestions]");
    down(&mut p);
    assert_eq!(highlighted(&p.app), Some(TextEditAction::AddToDictionary));
    key_with(&mut p.app, KeyCode::End, Modifiers::default());
    key_with(&mut p.app, KeyCode::ArrowDown, Modifiers::default());
    assert_eq!(
        highlighted(&p.app),
        Some(TextEditAction::AddToDictionary),
        "wraps past it"
    );
}

/// Nothing new runs for a right press when no word is marked: the spelling
/// rows are never built, so a menu on a page with no marks has the panel it
/// always had.
#[test]
fn a_menu_without_marks_builds_no_spelling_rows() {
    configure(default_config());
    let mut p = page(&[("input", "the cat")], None);
    let id = p.fields[0];
    activate(&mut p.app, id);
    let at = field_point(&mut p.app, id, 5, 6);
    press(&mut p.app, at, MouseButton::Right);
    assert!(p.app.is_text_context_menu_open());
    let panel = p.app.text_menu_panel.as_ref().unwrap();
    assert!(panel.spell.is_none());
    let d = p.app.doc.as_ref().unwrap().borrow();
    let children = d.tree.get(panel.panel_id).unwrap().children.len();
    assert_eq!(children, 5, "four rows and a separator");
}
