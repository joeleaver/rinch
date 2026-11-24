//! Commands, input rules and key dispatch.

use super::*;

#[test]
fn command_toggles_mark_and_reprojects() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
    // Select the whole word, then bold it via the command.
    h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
    assert!(!h.handle.is_mark_active("bold"));
    assert!(h.handle.command("toggleBold"), "toggleBold applies");
    assert!(h.handle.is_mark_active("bold"), "state reports bold active");

    // The host re-projected: the run is now wrapped in <strong>.
    let p = children(&h, h.container_id)[0];
    let strong = children(&h, p)[0];
    assert_eq!(tag(&h, strong).as_deref(), Some("strong"));
    assert_eq!(text(&h, strong).as_deref(), Some("abcd"));
}

#[test]
fn block_type_command_and_query() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "title")]));
    h.handle.set_selection(Selection::cursor(Pos(2)));
    assert_eq!(h.handle.current_block_type().as_deref(), Some("paragraph"));
    assert!(h.handle.command("setHeading2"));
    assert_eq!(h.handle.current_block_type().as_deref(), Some("heading"));
    let block = children(&h, h.container_id)[0];
    assert_eq!(tag(&h, block).as_deref(), Some("h2"));
}

#[test]
fn update_inserts_text_through_one_path() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab")]));
    h.handle.set_selection(Selection::cursor(Pos(3))); // end of "ab"
    let applied = h.handle.update(|state| {
        let mut tr = state.tr();
        tr.insert_text("c").ok()?;
        Some(tr)
    });
    assert!(applied);
    let p = children(&h, h.container_id)[0];
    assert_eq!(text(&h, p).as_deref(), Some("abc"));
}

fn empty_para(s: &Schema) -> Node {
    s.branch("paragraph", Fragment::empty()).unwrap()
}

#[test]
fn insert_text_fires_block_input_rule() {
    // Typing "# " at the block start applies the heading input rule. This proves the
    // markdown rules are wired into the live text-input path — not merely
    // unit-tested in editor-core (they were unreachable before this).
    let s = schema();
    let h = mount(doc_node(&s, vec![empty_para(&s)]));
    h.handle.set_selection(Selection::cursor(Pos(1)));
    h.handle.insert_text("#");
    h.handle.insert_text(" ");
    assert_eq!(h.handle.current_block_type().as_deref(), Some("heading"));
    let block = children(&h, h.container_id)[0];
    assert_eq!(tag(&h, block).as_deref(), Some("h1"));
}

#[test]
fn insert_text_fires_mark_input_rule() {
    // "**bold**" typed at a cursor wraps "bold" in a bold mark and drops the markup —
    // verified on the projected DOM (a <strong> wrapping the text), not just the model.
    let s = schema();
    let h = mount(doc_node(&s, vec![empty_para(&s)]));
    h.handle.set_selection(Selection::cursor(Pos(1)));
    h.handle.insert_text("**bold**");
    let p = children(&h, h.container_id)[0];
    let strong = children(&h, p)[0];
    assert_eq!(tag(&h, strong).as_deref(), Some("strong"));
    assert_eq!(text(&h, strong).as_deref(), Some("bold"));
}

#[test]
fn insert_text_fires_task_list_input_rule() {
    // "[ ] " at the block start turns the block into a task list.
    let s = schema();
    let h = mount(doc_node(&s, vec![empty_para(&s)]));
    h.handle.set_selection(Selection::cursor(Pos(1)));
    h.handle.insert_text("[ ] ");
    assert_eq!(h.handle.doc().child(0).type_name(), "task_list");
}

#[test]
fn toggle_task_checked_flips_the_enclosing_item() {
    let s = schema();
    let item = s
        .branch("task_item", Fragment::from_node(para(&s, "x")))
        .unwrap();
    let list = s.branch("task_list", Fragment::from_node(item)).unwrap();
    let h = mount(doc_node(&s, vec![list]));
    // Cursor inside the item's paragraph (pos 3 = before "x").
    h.handle.set_selection(Selection::cursor(Pos(3)));

    let checked = |h: &Harness| h.handle.doc().child(0).child(0).attrs().get_bool("checked");
    assert_ne!(checked(&h), Some(true), "starts unchecked");
    assert!(
        h.handle.toggle_task_checked_at(3),
        "toggles the enclosing task item"
    );
    assert_eq!(checked(&h), Some(true), "now checked");
    assert!(h.handle.toggle_task_checked_at(3));
    assert_ne!(checked(&h), Some(true), "toggled back off");
}

#[test]
fn toggle_task_checked_is_a_noop_outside_a_task_item() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "plain")]));
    assert!(
        !h.handle.toggle_task_checked_at(2),
        "no task item at the cursor"
    );
}

#[test]
fn dispatch_key_runs_a_bound_command() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
    h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
    // Mod-b → toggleBold, from the aggregated keymap.
    let b = KeyBinding::parse("Mod-b").unwrap();
    assert_eq!(
        h.handle.dispatch_key(b),
        Some(true),
        "a bound key runs its command"
    );
    assert!(h.handle.is_mark_active("bold"));
}

#[test]
fn dispatch_key_returns_none_when_unbound() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "x")]));
    // A bare 'q' has no binding — the caller falls through to text insertion.
    assert_eq!(h.handle.dispatch_key(KeyBinding::parse("q").unwrap()), None);
    // selectAll is bound via Mod-a and always applies (consumes the key).
    assert_eq!(
        h.handle.dispatch_key(KeyBinding::parse("Mod-a").unwrap()),
        Some(true)
    );
}

/// #2993's review: Mod-Enter is the base keymap's one fall-through binding
/// (ProseMirror's keymap semantics), so Ctrl/Cmd+Enter in a paragraph, where
/// `exitCode` has no code block to leave, goes on to the page. At
/// 5f10b1475 it answered `Some(false)` — consumed.
#[test]
fn mod_enter_falls_through_where_it_does_not_apply() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
    h.handle.set_selection(Selection::cursor(Pos(3)));
    let before = h.handle.doc();
    assert_eq!(
        h.handle
            .dispatch_key(KeyBinding::parse("Mod-Enter").unwrap()),
        None,
        "Mod-Enter outside a code block falls through"
    );
    assert!(h.handle.doc().same_ref(&before), "and changes nothing");
    // In a code block the same binding applies and is consumed.
    let code = s
        .branch("code_block", Fragment::from_node(s.text("x").unwrap()))
        .unwrap();
    let h = mount(doc_node(&s, vec![code]));
    h.handle.set_selection(Selection::cursor(Pos(2)));
    assert_eq!(
        h.handle
            .dispatch_key(KeyBinding::parse("Mod-Enter").unwrap()),
        Some(true)
    );
}

/// #2993's second review: every base binding but Mod-Enter consumes its key
/// where its command does not apply, so a chord the editor advertises never
/// reaches the browser — Ctrl+Shift+R on a paragraph that is already
/// right-aligned would hard-reload the page, Mod-e in a read-only editor
/// would focus the address bar. At b7d5350ad (which consumed only a declared list)
/// these chords answered `None`.
#[test]
fn every_base_binding_but_mod_enter_keeps_its_key_where_it_does_not_apply() {
    let s = schema();
    let code = s
        .branch("code_block", Fragment::from_node(s.text("ab").unwrap()))
        .unwrap();
    let h = mount(doc_node(&s, vec![para(&s, "x"), code]));
    let key = |k: &str| h.handle.dispatch_key(KeyBinding::parse(k).unwrap());
    h.handle.set_selection(Selection::cursor(Pos(1)));
    // A top-level, left-aligned paragraph at the document's start: no list
    // item to sink or lift, nothing to delete backward, undo or redo, and
    // already a paragraph.
    for k in [
        "Tab",
        "Shift-Tab",
        "Backspace",
        "Mod-z",
        "Mod-y",
        "Mod-Shift-z",
        "Mod-Shift-0",
    ] {
        assert_eq!(key(k), Some(false), "{k} is consumed although it no-ops");
    }
    // Ctrl+Shift+R applies once, then the paragraph is right-aligned and the
    // command answers `false`: still the editor's key.
    assert_eq!(key("Mod-Shift-r"), Some(true), "aligns right");
    assert_eq!(key("Mod-Shift-r"), Some(false), "already right: consumed");
    // At the code block's end (the document's): the alignment chord refuses
    // (a code block has no alignment) and Delete deletes nothing.
    let end = h.handle.doc().content_size();
    h.handle.set_selection(Selection::cursor(Pos(end - 1)));
    for k in ["Mod-Shift-r", "Delete"] {
        assert_eq!(key(k), Some(false), "{k} in a code block is consumed");
    }
    // A read-only editor refuses every command: every binding but Mod-Enter
    // is still the editor's.
    h.handle.set_read_only(true);
    h.handle.set_selection(Selection::text(Pos(1), Pos(2)));
    for k in [
        "Mod-b",
        "Mod-i",
        "Mod-u",
        "Mod-e",
        "Mod-Shift-8",
        "Mod-Shift-b",
        "Mod-Shift-r",
        "Mod-Alt-1",
    ] {
        assert_eq!(key(k), Some(false), "{k} is consumed although refused");
    }
    assert_eq!(key("Mod-Enter"), None, "Mod-Enter falls through");
}

#[test]
fn insert_text_with_a_selection_skips_input_rules() {
    // Input rules only fire at a collapsed cursor: typing "**" over a selection just
    // replaces it (no half-applied rule against a non-empty range).
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
    h.handle.set_selection(Selection::text(Pos(1), Pos(5)));
    h.handle.insert_text("x");
    let p = children(&h, h.container_id)[0];
    assert_eq!(text(&h, p).as_deref(), Some("x"));
}

#[test]
fn handle_clones_share_one_editor() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab")]));
    let clone = h.handle.clone();
    clone.set_selection(Selection::cursor(Pos(3)));
    clone.update(|state| {
        let mut tr = state.tr();
        tr.insert_text("Z").ok()?;
        Some(tr)
    });
    // The original handle sees the mutation (shared Rc).
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("abZ")
    );
}

// ── Clipboard (copy / cut / paste) ───────────────────────────────────────
