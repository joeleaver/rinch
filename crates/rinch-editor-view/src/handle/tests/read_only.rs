//! The read-only switch.

use super::*;

/// A paragraph, a bullet list with one item and a task list with one item —
/// enough structure for every edit below to have something it would change.
///
/// Built **by the handle**: parsed by its own schema, wrapped by its own
/// commands, then reloaded so the history starts empty. Node types compare by
/// identity (#217), so a list assembled from a second `Schema::starter_kit()`
/// would make the list commands no-ops and their refusals prove nothing.
fn read_only_fixture() -> Harness {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "")]));
    assert!(
        h.handle
            .load_html("<p>hello world</p><p>item</p><p>todo</p>")
    );
    // "hello world" is 1..12; the second paragraph opens at 13.
    h.handle.set_selection(Selection::cursor(Pos(15)));
    assert!(h.handle.command("toggleBulletList"));
    // list 13, item 14, paragraph 15, "item" 16..20; the third opens at 23.
    h.handle.set_selection(Selection::cursor(Pos(25)));
    assert!(h.handle.command("toggleTaskList"));
    // task list 23, task item 24, paragraph 25, "todo" 26..30.
    h.handle.load_doc(h.handle.doc());
    let doc = h.handle.doc();
    assert_eq!(doc.child(1).type_name(), "bullet_list");
    assert_eq!(doc.child(2).type_name(), "task_list");
    h
}

/// A local edit by name: where the selection sits for it, and the call the
/// platform glue (or a toolbar) makes, answering whether it applied.
type LocalEdit = (&'static str, Selection, fn(&EditorHandle) -> bool);

/// Every kind of local mutation there is an entry point for. In
/// [`read_only_fixture`] "hello world" is 1..12 ("world" 7..12), the list
/// item's "item" starts at 16 and the task item's "todo" at 26.
fn local_edits() -> Vec<LocalEdit> {
    /// For the entry points that answer `()`: did the document change?
    fn changed(h: &EditorHandle, edit: impl FnOnce(&EditorHandle)) -> bool {
        let before = h.doc();
        edit(h);
        !h.doc().same_ref(&before)
    }
    let world = || Selection::text(Pos(7), Pos(12));
    let end = || Selection::cursor(Pos(12));
    /// A key the keymap binds: consumed whether or not its command applies.
    fn bound_key(h: &EditorHandle, k: &str) -> bool {
        let applied = h.dispatch_key(KeyBinding::parse(k).unwrap());
        assert!(applied.is_some(), "{k} is bound, so consumed either way");
        applied == Some(true)
    }
    vec![
        ("typing over a selection", world(), |h| h.insert_text("X")),
        ("typing at a caret", end(), |h| h.insert_text("!")),
        ("a markdown input rule", end(), |h| h.insert_text(" **b** ")),
        ("an IME commit", end(), |h| {
            changed(h, |h| h.ime_commit("ね"))
        }),
        ("an IME surrounding-text delete", end(), |h| {
            changed(h, |h| h.ime_delete_surrounding(2, 0))
        }),
        ("cut / delete selection", world(), |h| {
            h.command("deleteSelection")
        }),
        ("backspace", end(), |h| h.command("deleteCharBackward")),
        ("forward delete", Selection::cursor(Pos(3)), |h| {
            h.command("deleteCharForward")
        }),
        ("the Backspace key", end(), |h| bound_key(h, "Backspace")),
        ("word backspace", end(), |h| h.command("deleteWordBackward")),
        ("word forward delete", Selection::cursor(Pos(3)), |h| {
            h.command("deleteWordForward")
        }),
        ("the word-delete chord", end(), |h| {
            bound_key(
                h,
                if cfg!(target_os = "macos") {
                    "Alt-Backspace"
                } else {
                    "Mod-Backspace"
                },
            )
        }),
        ("enter", end(), |h| h.command("enter")),
        ("the Enter key", end(), |h| bound_key(h, "Enter")),
        ("a hard break", end(), |h| h.command("insertHardBreak")),
        ("a horizontal rule", end(), |h| {
            h.command("insertHorizontalRule")
        }),
        ("a table", end(), |h| h.command("insertTable")),
        ("a plain-text paste", world(), |h| {
            h.replace_selection_with_text("pasted\nlines")
        }),
        ("a rich paste", world(), |h| {
            h.replace_selection_with_html("<p><b>pasted</b></p>")
        }),
        ("an image paste", end(), |h| {
            h.insert_image("data:image/png;base64,AAAA", "")
        }),
        ("bold over a range", world(), |h| h.command("toggleBold")),
        ("Mod-b over a range", world(), |h| bound_key(h, "Mod-b")),
        ("italic over a range", world(), |h| {
            h.command("toggleItalic")
        }),
        ("a link", world(), |h| h.toggle_link("https://example.com")),
        ("a heading", end(), |h| h.command("setHeading1")),
        ("alignment", end(), |h| h.command("setTextAlignCenter")),
        ("a blockquote wrap", end(), |h| {
            h.command("wrapInBlockquote")
        }),
        ("a list wrap", end(), |h| h.command("toggleBulletList")),
        ("outdent", Selection::cursor(Pos(17)), |h| {
            h.command("liftListItem")
        }),
        ("Shift-Tab in a list", Selection::cursor(Pos(17)), |h| {
            bound_key(h, "Shift-Tab")
        }),
        ("a task checkbox click", end(), |h| {
            h.toggle_task_checked_at(26)
        }),
        ("a hand-built transaction", end(), |h| {
            h.update(|state| {
                let mut tr = state.tr();
                tr.delete(1, 6).ok()?;
                Some(tr)
            })
        }),
    ]
}

/// Every local edit, refused: it answers "not applied", the document is the
/// very same object afterwards, the host still shows it, and `on_change` stayed
/// silent. Each is first shown to **apply** on an editable editor from the same
/// position — a command that would have done nothing anyway proves nothing.
#[test]
fn a_read_only_editor_refuses_every_local_edit() {
    use std::cell::Cell;
    for (what, selection, edit) in local_edits() {
        let control = read_only_fixture();
        control.handle.set_selection(selection.clone());
        let before = control.handle.doc();
        assert!(
            edit(&control.handle),
            "control: {what} applies when editable"
        );
        assert!(
            !control.handle.doc().same_ref(&before),
            "control: {what} changes the document when editable"
        );

        let h = read_only_fixture();
        let hits = Rc::new(Cell::new(0u32));
        h.handle.on_change({
            let hits = hits.clone();
            move || hits.set(hits.get() + 1)
        });
        assert!(!h.handle.is_read_only(), "editable by default");
        h.handle.set_read_only(true);
        assert!(h.handle.is_read_only());
        h.handle.set_selection(selection.clone());
        let before = h.handle.doc();
        let host_before: Vec<_> = children(&h, h.container_id)
            .into_iter()
            .map(|id| text(&h, id))
            .collect();

        assert!(!edit(&h.handle), "{what} must report that it did not apply");
        assert!(
            h.handle.doc().same_ref(&before),
            "{what} must leave the very same document"
        );
        assert_eq!(
            h.handle.selection(),
            selection,
            "{what} must not move the selection either"
        );
        let host_after: Vec<_> = children(&h, h.container_id)
            .into_iter()
            .map(|id| text(&h, id))
            .collect();
        assert_eq!(host_after, host_before, "{what}: the host is untouched");
        assert_eq!(hits.get(), 0, "{what}: on_change stayed silent");
    }
}

/// Undo and redo replay document changes, so they are refused like any other —
/// and the history they would have replayed is still there when the editor is
/// writable again. Flipping the switch back restores editing in full.
#[test]
fn undo_and_redo_are_refused_while_read_only_and_flipping_back_restores_editing() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abc")]));
    h.handle.set_selection(Selection::cursor(Pos(4)));
    assert!(h.handle.insert_text("d"));
    let para_text = |h: &Harness| text(h, children(h, h.container_id)[0]);
    assert_eq!(para_text(&h).as_deref(), Some("abcd"));

    h.handle.set_read_only(true);
    let locked = h.handle.doc();
    assert!(!h.handle.command("undo"), "undo is refused");
    assert!(h.handle.doc().same_ref(&locked));
    assert!(!h.handle.can_run("undo"), "and reported as unavailable");

    h.handle.set_read_only(false);
    assert!(!h.handle.is_read_only());
    assert!(h.handle.can_run("undo"));
    assert!(h.handle.command("undo"), "the history survived the lock");
    assert_eq!(para_text(&h).as_deref(), Some("abc"));

    h.handle.set_read_only(true);
    let locked = h.handle.doc();
    assert!(!h.handle.command("redo"), "redo is refused");
    assert!(h.handle.doc().same_ref(&locked));

    h.handle.set_read_only(false);
    assert!(h.handle.command("redo"));
    assert_eq!(para_text(&h).as_deref(), Some("abcd"));
    h.handle.set_selection(Selection::cursor(Pos(5)));
    assert!(h.handle.insert_text("e"), "typing works again");
    assert_eq!(para_text(&h).as_deref(), Some("abcde"));
}

/// What a reader does: place the caret, move it, extend a selection, select a
/// word, a block, everything — and copy.
#[test]
fn a_read_only_editor_still_selects_moves_the_caret_and_copies() {
    let s = schema();
    let h = mount(doc_node(
        &s,
        vec![para(&s, "hello world"), para(&s, "second")],
    ));
    h.handle.set_read_only(true);

    h.handle.set_selection(Selection::cursor(Pos(3)));
    assert_eq!(h.handle.selection(), Selection::cursor(Pos(3)));
    assert!(h.handle.move_cursor(CursorMotion::CharRight, false));
    assert_eq!(h.handle.selection().head(), Pos(4));
    assert!(h.handle.move_cursor(CursorMotion::WordRight, true));
    assert!(!h.handle.selection().is_empty(), "shift+motion extends");
    assert!(h.handle.move_cursor(CursorMotion::DocEnd, false));

    assert!(h.handle.select_word_at(Pos(8)));
    let (html, plain) = h.handle.selection_clipboard().expect("a word to copy");
    assert_eq!(plain, "world");
    assert!(html.contains("world"));
    assert!(h.handle.select_block_at(Pos(15)));
    assert_eq!(h.handle.selection_clipboard().unwrap().1, "second");

    assert!(h.handle.can_run("selectAll"));
    assert!(
        h.handle.command("selectAll"),
        "select-all changes no document"
    );
    assert_eq!(
        h.handle.selection_clipboard().unwrap().1,
        "hello world\nsecond"
    );
    assert_eq!(
        h.handle.dispatch_key(KeyBinding::parse("Mod-a").unwrap()),
        Some(true)
    );
}

/// "Click Bold, then type" is an edit in waiting: refused on a read-only editor
/// (or the toolbar would light Bold for text nobody can type), dropped when the
/// switch goes on, and never in the way of a caret move.
#[test]
fn stored_marks_are_refused_and_dropped_by_a_read_only_editor() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abc")]));
    h.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(h.handle.command("toggleBold"), "editable: bold is stored");
    assert!(h.handle.is_mark_active("bold"));

    h.handle.set_read_only(true);
    assert!(
        !h.handle.is_mark_active("bold"),
        "going read-only drops the pending mark"
    );
    assert!(!h.handle.command("toggleBold"), "and none can be stored");
    assert!(!h.handle.is_mark_active("bold"));
    assert!(!h.handle.can_run("toggleBold"));

    // Stored marks left over in the state must not make a caret move look like
    // an edit: clearing them is what every caret move does.
    h.handle.set_read_only(false);
    assert!(h.handle.command("toggleItalic"));
    {
        // Lock without the tidy-up `set_read_only` does, to pin the rule itself.
        h.handle.inner.borrow_mut().read_only = true;
    }
    assert!(h.handle.state().stored_marks.is_some());
    assert!(h.handle.move_cursor(CursorMotion::CharRight, false));
    assert_eq!(h.handle.selection().head(), Pos(3), "the caret moved");
    assert!(h.handle.state().stored_marks.is_none());
}

/// `can_run` is what a toolbar greys its buttons from, so it has to follow the
/// switch — without taking the reader's own commands down with it.
#[test]
fn can_run_follows_the_switch() {
    let h = read_only_fixture();
    h.handle.set_selection(Selection::text(Pos(7), Pos(12)));
    let editing = ["toggleBold", "setHeading1", "toggleBulletList", "enter"];
    for name in editing {
        assert!(h.handle.can_run(name), "{name} applies while editable");
    }
    h.handle.set_read_only(true);
    let (doc, selection) = (h.handle.doc(), h.handle.selection());
    for name in editing {
        assert!(!h.handle.can_run(name), "{name} is unavailable read-only");
    }
    assert!(
        h.handle.can_run("selectAll"),
        "a reader can still select all"
    );
    assert!(
        h.handle.doc().same_ref(&doc) && h.handle.selection() == selection,
        "asking ran nothing: the answer never runs a dispatch branch"
    );
    h.handle.set_read_only(false);
    for name in editing {
        assert!(h.handle.can_run(name), "{name} is back");
    }
}

/// The app loading a document is not the user editing one: it is how a
/// read-only editor gets something to show, flag on, no toggling round it.
#[test]
fn a_read_only_editor_still_loads_a_document() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "old")]));
    h.handle.set_read_only(true);
    assert!(h.handle.load_html("<p>new content</p>"));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("new content")
    );
    h.handle.load_doc(doc_node(&s, vec![para(&s, "and again")]));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("and again")
    );
    assert!(h.handle.is_read_only(), "a load does not unlock it");
    assert!(!h.handle.insert_text("x"));
}

/// The switch lives on the handle, not the view: set before mount it is on the
/// container from the first build, a re-mount keeps it, and switching it off
/// takes the attribute away again.
#[test]
fn the_container_carries_the_switch_across_mounts() {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
    let mount_into = |h: &EditorHandle| {
        let id = doc.borrow_mut().create_element("div");
        h.attach(
            NodeHandle::new(id, Rc::downgrade(&doc)),
            Rc::downgrade(&doc),
        );
        id
    };
    let readonly_attr = |id: NodeId| doc.borrow().get_attribute(id, "data-pm-readonly");

    let s = Rc::new(schema());
    let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());
    h.set_read_only(true); // before any view exists
    assert!(!h.insert_text("x"), "refused before mount too");

    let first = mount_into(&h);
    assert_eq!(readonly_attr(first).as_deref(), Some("true"));
    let second = mount_into(&h); // a reactive re-mount
    assert_eq!(readonly_attr(second).as_deref(), Some("true"));

    h.set_read_only(false);
    assert_eq!(readonly_attr(second), None, "absent, not \"false\"");
    h.set_read_only(true);
    assert_eq!(readonly_attr(second).as_deref(), Some("true"));
}

/// The `Editor { read_only: true }` prop switches the handle on at mount; the
/// default `false` leaves a handle the app already locked alone.
#[test]
fn the_editor_components_read_only_prop_only_ever_locks() {
    use rinch_core::Component;
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
    let render = |editor: crate::Editor| {
        let root = doc.borrow_mut().create_element("div");
        let mut scope = RenderScope::new(doc.clone(), root);
        let container = editor.render(&mut scope, &[]);
        (scope, container)
    };

    let prop = crate::create_editor();
    let (_scope, container) = render(crate::Editor {
        editor: Some(prop.clone()),
        content: "<p>shown</p>".into(),
        read_only: true,
        spellcheck: None,
        ..Default::default()
    });
    assert!(prop.is_read_only());
    assert_eq!(
        container.get_attribute("data-pm-readonly").as_deref(),
        Some("true")
    );
    assert_eq!(
        prop.doc().child(0).child(0).text(),
        Some("shown"),
        "the content prop still loads"
    );
    assert!(!prop.insert_text("x"));

    let locked = crate::create_editor();
    locked.set_read_only(true);
    let (_scope, _container) = render(crate::Editor {
        editor: Some(locked.clone()),
        ..Default::default()
    });
    assert!(
        locked.is_read_only(),
        "`read_only: false` is the default, not an instruction to unlock"
    );
}
