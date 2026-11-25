//! `selection_range` read in UTF-16 units, and the selection a focus shows.

use super::*;

/// The selection reads back in UTF-16 code units, with its direction —
/// before the control is focused, while it is, and after it lost the
/// keyboard.
#[test]
fn selection_range_reads_utf16_units_focused_unfocused_and_after_blur() {
    let mut f = mount();
    let ta = f.textarea.clone();
    // Never focused: a caret at the start of an empty textarea, as Chrome
    // answers (a value is covered by `an_unfocused_controls_selection_is_chromes`).
    assert_eq!(
        ta.selection_range(),
        Some((0, 0, SelectionDirection::Forward))
    );
    assert_eq!(f.tile.selection_range(), None, "not a text control");
    assert_eq!(f.outer.selection_range(), None);

    f.click(&ta);
    f.type_str("a😀bc");
    // "a😀bc": 😀 is 4 bytes and 2 UTF-16 units; the caret is after `c`.
    assert_eq!(
        ta.selection_range(),
        Some((5, 5, SelectionDirection::Forward))
    );
    // Shift+Left twice: anchor 5, head 3 — backward.
    for _ in 0..2 {
        f.app.handle_event(
            PlatformEvent::KeyDown {
                key: KeyCode::ArrowLeft,
                logical_key: None,
                text: None,
                modifiers: Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                repeat: KeyRepeat::Unknown,
            },
            VP,
            1.0,
        );
    }
    assert_eq!(
        ta.selection_range(),
        Some((3, 5, SelectionDirection::Backward))
    );

    // Blur: the selection stays with the control.
    let input = f.input.clone();
    f.click(&input);
    assert_eq!(
        ta.selection_range(),
        Some((3, 5, SelectionDirection::Backward))
    );

    // A range set on an unfocused control reads back at once, swapped and
    // clamped, and is what the next focus shows.
    ta.set_selection_range(9, 1, SelectionDirection::Forward);
    assert_eq!(
        ta.selection_range(),
        Some((1, 5, SelectionDirection::Forward))
    );
    // Drained (`AboutToWait` applies posted requests): now it is the stash
    // that answers, and the next focus that shows it.
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert!(
        rinch_core::peek_pending_text_selection_request(f.app.doc_key(), ta.node_id().0).is_none(),
        "the request was drained"
    );
    assert_eq!(
        ta.selection_range(),
        Some((1, 5, SelectionDirection::Forward))
    );

    // An unspecified direction reads back `Forward`, before focus as after,
    // as Chrome answers.
    ta.set_selection_range(2, 4, SelectionDirection::None);
    assert_eq!(
        ta.selection_range(),
        Some((2, 4, SelectionDirection::Forward))
    );
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert_eq!(
        ta.selection_range(),
        Some((2, 4, SelectionDirection::Forward))
    );
}

/// An unfocused control with a value reads what Chrome 155 reads: a caret
/// after the value once a `value` write set it (rinch-web writes through the
/// value setter, which moves it there), at the start of a parsed or default
/// value; and
/// `None` for the input types whose `selectionStart` is `null`.
#[test]
fn an_unfocused_controls_selection_is_chromes() {
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        let mut add = |tag: &str, attrs: &[(&str, &str)]| {
            let el = __scope.create_element(tag);
            for (k, v) in attrs {
                el.set_attribute(k, v);
            }
            root.append_child(&el);
            o.borrow_mut().push(el);
        };
        add("input", &[("value", "abc")]);
        add("input", &[]);
        add("textarea", &[]);
        add("input", &[("type", "email"), ("value", "a@b")]);
        add("input", &[("type", "Number"), ("value", "1")]);
        add("input", &[("type", "date")]);
        let ta = out_text(__scope);
        root.append_child(&ta);
        o.borrow_mut().push(ta);
        o.borrow_mut().push(root.clone());
        root
    });
    fn out_text(scope: &mut RenderScope) -> NodeHandle {
        let ta = scope.create_element("textarea");
        ta.append_child(&scope.create_text("hello"));
        ta
    }
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let h = out.borrow().clone();
    let fwd = SelectionDirection::Forward;
    assert_eq!(h[0].selection_range(), Some((3, 3, fwd)), "input value=abc");
    assert_eq!(h[1].selection_range(), Some((0, 0, fwd)), "empty input");
    h[1].set_attribute("value", "hello");
    assert_eq!(
        h[1].selection_range(),
        Some((5, 5, fwd)),
        "after a value write"
    );
    // A range set and not yet applied: the write still wins (review 2, G1).
    h[1].set_selection_range(1, 2, fwd);
    h[1].set_attribute("value", "hello world");
    assert_eq!(
        h[1].selection_range(),
        Some((11, 11, fwd)),
        "a write moves it to the end"
    );
    h[2].set_attribute("value", "xy");
    assert_eq!(
        h[2].selection_range(),
        Some((2, 2, fwd)),
        "textarea value write"
    );
    for (i, what) in [(3, "email"), (4, "number"), (5, "date")] {
        assert_eq!(h[i].selection_range(), None, "{what}");
    }
    assert_eq!(
        h[6].selection_range(),
        Some((0, 0, fwd)),
        "textarea text child"
    );
    // Parsed markup sets its value with no setter: Chrome 155 reads the
    // untouched `<input>`'s caret at the start.
    h[7].set_inner_html("<input value=\"abcd\">");
    let parsed = h[7].children()[0].clone();
    assert_eq!(parsed.selection_range(), Some((0, 0, fwd)), "parsed input");
}

/// Review 2 of #1686, G1/G6/N1: the selection a control shows when it gains
/// the keyboard is the one `selection_range()` read for it — after a range
/// set and then overridden by a `value` write (posted or stashed), typing
/// lands at the end, as in Chrome; an untouched textarea's caret is at its
/// start; an equal `value` write keeps the selection.
#[test]
fn focus_shows_the_selection_selection_range_reads() {
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        for i in 0..4 {
            let tag = if i == 2 { "textarea" } else { "input" };
            let el = __scope.create_element(tag);
            el.set_attribute("style", "width: 200px; height: 30px; display: block");
            let id = __scope.register_input_handler(|_v: String| {});
            el.set_attribute("data-oninput", &id.0.to_string());
            if i == 2 {
                el.append_child(&__scope.create_text("hello"));
            } else {
                el.set_attribute("value", "hello");
            }
            root.append_child(&el);
            o.borrow_mut().push(el);
        }
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let h = out.borrow().clone();
    let fwd = SelectionDirection::Forward;
    let mut f = Fixture {
        app,
        outer: h[0].clone(),
        textarea: h[0].clone(),
        input: h[0].clone(),
        tile: h[0].clone(),
        log: Rc::new(RefCell::new(Vec::new())),
    };
    let focus_and_type = |f: &mut Fixture, n: &NodeHandle| {
        n.focus();
        f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        f.type_str("Z");
        f.value(n)
    };

    // Posted, then overridden.
    h[0].set_selection_range(1, 2, fwd);
    h[0].set_attribute("value", "hello world");
    assert_eq!(h[0].selection_range(), Some((11, 11, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[0]), "hello worldZ");

    // Stashed, then overridden.
    h[1].set_selection_range(1, 2, fwd);
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    h[1].set_attribute("value", "hello world");
    assert_eq!(h[1].selection_range(), Some((11, 11, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[1]), "hello worldZ");

    // Untouched textarea: the caret starts where the read says.
    assert_eq!(h[2].selection_range(), Some((0, 0, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[2]), "Zhello");

    // A write that leaves the value as it was keeps the selection (Chrome's
    // value setter does too): "hel\nlo" sanitizes to "hello" in an <input>.
    // (A write of the very same attribute string changes nothing at all.)
    h[3].set_selection_range(1, 2, fwd);
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    h[3].set_attribute("value", "hel\nlo");
    assert_eq!(h[3].selection_range(), Some((1, 2, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[3]), "hZllo");
}
