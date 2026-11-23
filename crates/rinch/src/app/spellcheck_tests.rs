//! Spelling marks in text fields (#2862): which fields are checked, when,
//! and that an edit re-checks only the words it touched.
//!
//! Every test checks against the vendored `en_TEST` dictionary (never the
//! host's) and runs the checking thread's jobs inline
//! (`set_inline_jobs`), so a result is in the app's inbox as soon as it is
//! sent and `run_deferred_work` applies it.

use super::*;
use rinch_spell::runtime::{self as spell, SpellcheckConfig};
use rinch_spell::{HunspellChecker, SpellChecker};
use std::cell::Cell;
use std::sync::{Arc, Mutex};

const AFF: &[u8] = include_bytes!("../../../rinch-spell/tests/fixtures/en_TEST.aff");
const DIC: &[u8] = include_bytes!("../../../rinch-spell/tests/fixtures/en_TEST.dic");

/// The fixture checker, recording every word it is asked about.
struct Recording {
    inner: HunspellChecker,
    seen: Arc<Mutex<Vec<String>>>,
}

impl SpellChecker for Recording {
    fn check(&self, word: &str) -> bool {
        self.seen.lock().unwrap().push(word.to_string());
        self.inner.check(word)
    }
    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        self.inner.suggest(word, max)
    }
    fn add_to_dictionary(&self, word: &str) {
        self.inner.add_to_dictionary(word)
    }
    fn ignore(&self, word: &str) {
        self.inner.ignore(word)
    }
    fn languages(&self) -> Vec<String> {
        self.inner.languages()
    }
}

/// Install the fixture checker (no debounce, so a tick checks at once) and
/// answer the list of words it is asked about.
fn configure() -> Arc<Mutex<Vec<String>>> {
    spell::set_inline_jobs(true);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let checker = Recording {
        inner: HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap(),
        seen: seen.clone(),
    };
    spell::set_config(SpellcheckConfig::new().checker(checker).debounce_ms(0));
    seen
}

/// A `tag` field holding `value` with `attrs`, inside a `<div>` with
/// `parent_attrs`, and a plain `<input>` after it.
fn mount(
    tag: &'static str,
    value: &'static str,
    attrs: &[(&str, &str)],
    parent_attrs: &[(&str, &str)],
) -> (RinchApp, u64, u64) {
    let own = |a: &[(&str, &str)]| -> Vec<(String, String)> {
        a.iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    };
    let (attrs, parent_attrs) = (own(attrs), own(parent_attrs));
    let ids: Rc<Cell<(u64, u64)>> = Rc::new(Cell::new((0, 0)));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let wrap = scope.create_element("div");
        for (name, v) in &parent_attrs {
            wrap.set_attribute(name, v);
        }
        let field = scope.create_element(tag);
        field.set_attribute("style", "display: block; width: 300px; height: 30px");
        field.set_attribute("value", value);
        for (name, v) in &attrs {
            field.set_attribute(name, v);
        }
        let other = scope.create_element("input");
        other.set_attribute("style", "display: block; width: 200px; height: 30px");
        wrap.append_child(&field);
        root.append_child(&wrap);
        root.append_child(&other);
        ids_in.set((field.node_id().0, other.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (a, b) = ids.get();
    (app, a, b)
}

fn click(app: &mut RinchApp, id: u64) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
        (ax + w / 2.0, ay + h / 2.0)
    };
    for event in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        app.handle_event(event, (800, 600), 1.0);
    }
}

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
}

/// The words `id` shows marked, after the app ran what the checker sent.
fn marked(app: &mut RinchApp, id: u64) -> Vec<String> {
    // A configuration change sends a check, whose result is the next turn's.
    for _ in 0..4 {
        if app.run_deferred_work() == 0 {
            break;
        }
    }
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

/// A press in a pre-filled textarea marks its misspelled words at once, as
/// Chrome 158 does on a pointer focus; before it, and after a focus from
/// script alone, nothing is marked.
#[test]
fn a_press_marks_a_prefilled_field_and_a_script_focus_does_not() {
    configure();
    let (mut app, field, _) = mount("textarea", "helo wrld is right", &[], &[]);
    assert!(
        marked(&mut app, field).is_empty(),
        "marked before any interaction"
    );
    app.try_focus_input(field);
    assert_eq!(app.focus_target, FocusTarget::Input(field));
    assert!(
        marked(&mut app, field).is_empty(),
        "a script focus marked the field"
    );
    click(&mut app, field);
    assert_eq!(marked(&mut app, field), ["helo", "wrld"]);
}

/// Tab onto a field is an interaction too.
#[test]
fn tab_onto_a_field_marks_it() {
    configure();
    let (mut app, field, _) = mount("input", "the cta", &[], &[]);
    press(&mut app, KeyCode::Tab, None);
    assert_eq!(app.focus_target, FocusTarget::Input(field));
    assert_eq!(marked(&mut app, field), ["cta"]);
}

/// `spellcheck` is inherited from the nearest ancestor that sets it.
#[test]
fn the_spellcheck_attribute_is_inherited() {
    configure();
    for (own, parent, checked) in [
        (None, Some("false"), false),
        (Some("true"), Some("false"), true),
        (Some(""), Some("false"), true),
        (Some("bogus"), Some("false"), false),
        (Some("FALSE"), None, false),
        (None, Some("true"), true),
        (None, None, true),
    ] {
        let attrs: Vec<(&str, &str)> = own.map(|v| ("spellcheck", v)).into_iter().collect();
        let parent_attrs: Vec<(&str, &str)> =
            parent.map(|v| ("spellcheck", v)).into_iter().collect();
        let (mut app, field, _) = mount("textarea", "helo", &attrs, &parent_attrs);
        click(&mut app, field);
        let want: Vec<String> = if checked { vec!["helo".into()] } else { vec![] };
        assert_eq!(
            marked(&mut app, field),
            want,
            "own {own:?}, parent {parent:?}"
        );
    }
}

/// Only prose inputs are checked, and no read-only field.
#[test]
fn only_prose_inputs_are_checked() {
    configure();
    for (ty, checked) in [
        (None, true),
        (Some("text"), true),
        (Some("search"), true),
        (Some("password"), false),
        (Some("email"), false),
        (Some("url"), false),
        (Some("tel"), false),
        (Some("number"), false),
    ] {
        let attrs: Vec<(&str, &str)> = ty.map(|t| ("type", t)).into_iter().collect();
        let (mut app, field, _) = mount("input", "helo", &attrs, &[]);
        click(&mut app, field);
        assert_eq!(!marked(&mut app, field).is_empty(), checked, "type {ty:?}");
    }
    let (mut app, field, _) = mount("textarea", "helo", &[("readonly", "")], &[]);
    click(&mut app, field);
    assert!(
        marked(&mut app, field).is_empty(),
        "a read-only field was checked"
    );
}

/// The app's switch: off, nothing is checked; turned on while the app runs,
/// the fields the user already interacted with are checked again.
#[test]
fn the_app_switch_turns_marking_off_and_back_on() {
    configure();
    let (mut app, field, _) = mount("textarea", "helo", &[], &[]);
    click(&mut app, field);
    assert_eq!(marked(&mut app, field), ["helo"]);
    spell::set_enabled(false);
    assert!(
        marked(&mut app, field).is_empty(),
        "marks kept after turning it off"
    );
    spell::set_enabled(true);
    assert_eq!(
        marked(&mut app, field),
        ["helo"],
        "not re-checked when turned on"
    );

    spell::set_config(false);
    let (mut app, field, _) = mount("textarea", "helo", &[], &[]);
    click(&mut app, field);
    press(&mut app, KeyCode::KeyA, Some("x"));
    assert!(marked(&mut app, field).is_empty());
    assert!(
        app.spelling.listener.is_none(),
        "an app with checking off did work"
    );
}

/// An edit re-checks only the words it touched, once typing pauses: the
/// other words' marks move with the text meanwhile, the edited word's goes
/// at once, and the checker is asked about the new word alone.
#[test]
fn a_typed_edit_rechecks_only_its_word() {
    let seen = configure();
    let (mut app, field, _) = mount("textarea", "helo wrld", &[], &[]);
    click(&mut app, field);
    assert_eq!(marked(&mut app, field), ["helo", "wrld"]);
    seen.lock().unwrap().clear();

    // The caret is at the end: type " zzz".
    for ch in [" ", "z", "z", "z"] {
        press(&mut app, KeyCode::KeyA, Some(ch));
    }
    assert_eq!(
        marked(&mut app, field),
        ["helo", "wrld"],
        "the new word was marked before the pause, or the others lost theirs"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "checked on the keystroke path"
    );

    app.spell_tick();
    assert_eq!(marked(&mut app, field), ["helo", "wrld", "zzz"]);
    assert_eq!(
        *seen.lock().unwrap(),
        ["zzz"],
        "re-checked more than the edit"
    );

    // Fix "helo" → "hello": its mark goes at once, "wrld" and "zzz" move.
    seen.lock().unwrap().clear();
    app.focused_input_state.as_mut().unwrap().selection = rinch_editable::Selection::cursor(3);
    press(&mut app, KeyCode::KeyA, Some("l"));
    assert_eq!(marked(&mut app, field), ["wrld", "zzz"]);
    app.spell_tick();
    assert_eq!(marked(&mut app, field), ["wrld", "zzz"]);
    assert_eq!(*seen.lock().unwrap(), ["hello"]);
}

/// A first edit of a field focused from script (no press, no Tab) checks
/// the whole field, as Chrome does after the first edit.
#[test]
fn an_edit_of_a_script_focused_field_checks_it_whole() {
    configure();
    let (mut app, field, _) = mount("textarea", "helo wrld ", &[], &[]);
    app.try_focus_input(field);
    press(&mut app, KeyCode::KeyA, Some("a"));
    assert!(
        marked(&mut app, field).is_empty(),
        "checked before the pause"
    );
    app.spell_tick();
    assert_eq!(marked(&mut app, field), ["helo", "wrld"]);
}

/// An app whose spellcheck was never configured does no spellcheck work at
/// all: no listener, no field record, no mark write.
#[test]
fn an_unconfigured_app_does_nothing() {
    spell::clear_config();
    let (mut app, field, _) = mount("textarea", "helo", &[], &[]);
    click(&mut app, field);
    press(&mut app, KeyCode::KeyA, Some("x"));
    app.spell_tick();
    assert!(marked(&mut app, field).is_empty());
    assert!(app.spelling.listener.is_none());
    let d = app.doc.as_ref().unwrap().borrow();
    assert_eq!(
        d.tree
            .perf
            .get(rinch_dom::perf::Counter::SpellingMarkWrites),
        0
    );
}

fn set_attr(app: &mut RinchApp, id: u64, name: &str, value: Option<&str>) {
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        match value {
            Some(v) => rinch_core::dom::DomDocument::set_attribute(
                &mut *d,
                rinch_core::dom::NodeId(id),
                name,
                v,
            ),
            None => rinch_core::dom::DomDocument::remove_attribute(
                &mut *d,
                rinch_core::dom::NodeId(id),
                name,
            ),
        }
    }
    app.resolve_and_repaint(800.0, 600.0);
}

/// Review of #2876: with no dictionary for the locale the session goes away
/// once the first check loads nothing, and a field whose debounce was
/// pending must not re-arm a 0 ms timer for ever. Red before: the due
/// deadline stayed, so every tick armed another timer.
#[test]
fn no_dictionary_leaves_nothing_due() {
    spell::set_inline_jobs(true);
    spell::set_config(SpellcheckConfig::new().languages(["zz_ZZ"]).debounce_ms(0));
    let (mut app, field, other) = mount("textarea", "helo", &[], &[]);
    app.try_focus_input(field);
    press(&mut app, KeyCode::KeyA, Some("x"));
    assert!(
        app.spelling.any_due(),
        "positive control: the edit armed a check"
    );
    click(&mut app, other);
    app.run_deferred_work();
    assert!(spell::session().is_none(), "zz_ZZ found a dictionary");
    app.spell_tick();
    assert!(
        !app.spelling.any_due(),
        "a past deadline stayed: the timer re-arms for ever"
    );
}

/// Review of #2876: a result sent before an edit is dropped (the field's
/// generation moved on); without the guard it would mark the old words'
/// ranges in the new text.
#[test]
fn a_result_sent_before_an_edit_is_dropped() {
    configure();
    let (mut app, field, _) = mount("textarea", "helo wrld", &[], &[]);
    click(&mut app, field);
    // The press's check is in the inbox, not applied yet; edit first.
    app.focused_input_state.as_mut().unwrap().selection = rinch_editable::Selection::cursor(0);
    press(&mut app, KeyCode::KeyA, Some("a"));
    assert!(
        marked(&mut app, field).is_empty(),
        "a stale result was applied"
    );
    app.spell_tick();
    assert_eq!(marked(&mut app, field), ["ahelo", "wrld"]);
}

/// Review of #2876: `readonly` or `spellcheck="false"` set on a checked
/// field (or an ancestor) drops its marks, as in Chrome; setting it back
/// checks it again.
#[test]
fn turning_a_field_off_and_on_clears_and_rechecks() {
    configure();
    let (mut app, field, _) = mount("textarea", "helo", &[], &[]);
    click(&mut app, field);
    assert_eq!(marked(&mut app, field), ["helo"]);
    set_attr(&mut app, field, "readonly", Some(""));
    assert!(
        marked(&mut app, field).is_empty(),
        "a read-only field kept its marks"
    );
    set_attr(&mut app, field, "readonly", None);
    assert_eq!(marked(&mut app, field), ["helo"]);
    let parent = app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .get(field)
        .unwrap()
        .parent
        .unwrap();
    set_attr(&mut app, parent, "spellcheck", Some("false"));
    assert!(
        marked(&mut app, field).is_empty(),
        "spellcheck=false kept the marks"
    );
    set_attr(&mut app, parent, "spellcheck", None);
    assert_eq!(marked(&mut app, field), ["helo"]);
}

/// Review of #2876: a field removed from the document is forgotten at the
/// next frame, not only when a result for it arrives.
#[test]
fn a_removed_field_is_forgotten() {
    configure();
    let (mut app, field, _) = mount("textarea", "helo", &[], &[]);
    click(&mut app, field);
    assert!(!app.spelling.is_empty());
    let parent = app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .get(field)
        .unwrap()
        .parent
        .unwrap();
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        rinch_core::dom::DomDocument::remove_child(
            &mut *d,
            rinch_core::dom::NodeId(parent),
            rinch_core::dom::NodeId(field),
        );
    }
    app.resolve_and_repaint(800.0, 600.0);
    assert!(
        app.spelling.is_empty(),
        "the removed field is still remembered"
    );
}

/// The app's options (Joe's configurability rule): no interaction needed,
/// and an `<input>` that shows its marks blurred.
#[test]
fn the_interaction_and_blur_rules_are_options() {
    spell::set_inline_jobs(true);
    spell::set_config(
        SpellcheckConfig::new()
            .checker(HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap())
            .debounce_ms(0)
            .require_interaction(false)
            .hide_input_marks_on_blur(false),
    );
    let (mut app, field, _) = mount("input", "the cta", &[], &[]);
    app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(
        marked(&mut app, field),
        ["cta"],
        "checked without an interaction"
    );
    let d = app.doc.as_ref().unwrap().borrow();
    assert!(d.spelling_marks(field).unwrap().show_when_blurred);
}
