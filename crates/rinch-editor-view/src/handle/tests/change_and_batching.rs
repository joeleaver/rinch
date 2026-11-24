//! `on_change`, mounting order, and edits batched in one event handler.

use super::*;

#[test]
fn on_change_fires_for_edits_but_not_loads_or_selection() {
    use std::cell::Cell;

    let s = Rc::new(schema());
    let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

    let hits = Rc::new(Cell::new(0u32));
    h.on_change({
        let hits = hits.clone();
        move || hits.set(hits.get() + 1)
    });

    // A programmatic load is not a user edit — must not fire (otherwise an
    // autosave consumer would re-save every freshly opened document).
    assert!(h.load_html("<p>hello world</p>"));
    assert_eq!(hits.get(), 0, "load_doc/load_html must not fire on_change");

    // Typing (funnels through `update`) is an edit → fires.
    assert!(h.insert_text("!"));
    assert_eq!(hits.get(), 1, "an edit must fire on_change");

    // A command that changes the doc → fires.
    h.set_selection(Selection::text(Pos(1), Pos(3)));
    let before = hits.get();
    assert!(h.command("toggleBold"));
    assert_eq!(hits.get(), before + 1, "a doc-changing command must fire");

    // Selection-only movement leaves the doc identical → must not fire.
    let before = hits.get();
    h.set_selection(Selection::cursor(Pos(2)));
    assert_eq!(hits.get(), before, "selection-only change must not fire");
}

#[test]
fn on_change_callback_may_reenter_the_handle() {
    use std::cell::Cell;

    // The realistic autosave shape: the callback reads the document back out.
    // This double-borrows the inner RefCell unless the notifier drops its
    // borrow first, so this test is the regression guard for that.
    let s = Rc::new(schema());
    let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

    let seen = Rc::new(Cell::new(0usize));
    h.on_change({
        let h = h.clone();
        let seen = seen.clone();
        move || seen.set(h.doc().child_count())
    });

    assert!(h.insert_text("abc"));
    assert!(
        seen.get() > 0,
        "callback should have read the doc without panicking"
    );
}

#[test]
fn unmounted_handle_is_safe_and_edits_state() {
    let s = Rc::new(schema());
    let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

    // No host projection yet → view ops are safe no-ops, not panics.
    assert_eq!(h.container_id(), 0);
    assert_eq!(h.caret_address(Pos(0)), None);
    assert_eq!(h.pos_at(1, 0), None);
    assert_eq!(h.set_caret_blink(true), None);
    h.update_caret();

    // State edits still apply before mount (they render when the view attaches).
    assert!(h.load_html("<p>hi there</p>"));
    assert_eq!(h.doc().child_count(), 1);
}

#[test]
fn attach_projects_state_loaded_before_mount() {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
    let container_id = doc.borrow_mut().create_element("div");
    let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
    let s = Rc::new(schema());
    let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());

    // Load while unmounted (state only), then attach: the first view build
    // renders the loaded content directly.
    assert!(h.load_html("<p>before mount</p>"));
    h.attach(container, Rc::downgrade(&doc));

    assert_eq!(h.container_id(), container_id.0);
    let blocks = doc.borrow().get_children(container_id);
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        doc.borrow().text_content(blocks[0]).as_deref(),
        Some("before mount")
    );
}

// ── Read-only (`set_read_only`) ──────────────────────────────────────────
//
// The gate is one check in `EditorCore::commit`, so these go through the public
// entry points the platform glue calls — the keyboard's `insert_text` and
// `dispatch_key`, the clipboard's `replace_selection_with_*`, the IME's
// `ime_commit`, a toolbar's `command` — and assert on the document `Rc` itself:
// refused means *the same document object*, not an equal one.

// ── A handler that writes and runs editor commands (PR #882 review, E1) ─
//
// Every handler is a batch, and a `NodeHandle` operation made in one runs
// the effects queued so far. The editor patches its DOM through
// `NodeHandle`s while it holds `inner` mutably, so without the guards in
// `core_mut` a pending toolbar effect that reads editor state ran right
// there and panicked with `BorrowMutError`.
/// Toolbar pattern: an effect reads a signal and the editor's state; a
/// handler writes the signal, then runs a command. Unbatched (main) the
/// effect ran at the write; batched, the first NodeHandle op the command's
/// view patch makes flushes it while `command` holds `inner.borrow_mut()`.
#[test]
fn batched_write_then_command_in_one_batch() {
    use rinch_core::reactive::{Effect, Signal, batch};
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")]));
    let tick = Signal::new(0u32);
    let ed = h.handle.clone();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sn = seen.clone();
    let _e = Effect::new(move || {
        let t = tick.get();
        sn.borrow_mut().push((t, ed.is_mark_active("bold")));
    });
    h.handle.set_selection(Selection::text(Pos(1), Pos(4)));
    let ed2 = h.handle.clone();
    batch(|| {
        tick.set(1);
        ed2.command("toggleBold");
    });
    // Program order: the command's entry runs the effect the write queued,
    // so it sees `tick == 1` *before* the command made the text bold —
    // what it saw unbatched. (Deferred to the batch's end, it saw bold.)
    assert_eq!(*seen.borrow(), vec![(0, false), (1, false)]);
}

/// Two commands in one handler, with the documented on_change → signal
/// bump driving a toolbar effect that reads editor state.
#[test]
fn batched_two_commands_with_on_change_tick() {
    use rinch_core::reactive::{Effect, Signal, batch};
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")]));
    let tick = Signal::new(0u32);
    h.handle.on_change(move || tick.update(|t| *t += 1));
    let ed = h.handle.clone();
    let _e = Effect::new(move || {
        let _ = tick.get();
        let _ = ed.is_mark_active("bold");
    });
    h.handle.set_selection(Selection::text(Pos(1), Pos(4)));
    let ed2 = h.handle.clone();
    batch(|| {
        ed2.command("toggleBold");
        ed2.command("toggleItalic");
    });
}

/// A signal written *during* the view's DOM patch — here by a
/// child-inserted observer on the editor's own container, the hook
/// `List`/`Stepper` use — leaves an effect pending while the core is
/// borrowed. The patch's next `NodeHandle` operation must not run it: the
/// effect reads editor state and would re-borrow the core mid-patch. The
/// entry flush cannot cover this one (nothing was pending at the entry);
/// only the suppression held with the borrow does.
#[test]
fn batched_a_write_made_during_the_patch_does_not_run_inside_it() {
    use rinch_core::reactive::{Effect, Signal, batch};
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")]));
    let tick = Signal::new(0u32);
    let container = NodeHandle::new(h.container_id, Rc::downgrade(&h.doc));
    rinch_core::dom::on_child_inserted(&container, move |_| tick.update(|t| *t += 1));
    let ed = h.handle.clone();
    let runs = Rc::new(std::cell::Cell::new(0));
    let r = runs.clone();
    let _e = Effect::new(move || {
        let _ = tick.get();
        let _ = ed.is_mark_active("bold");
        r.set(r.get() + 1);
    });
    h.handle.set_selection(Selection::text(Pos(1), Pos(4)));
    let ed2 = h.handle.clone();
    batch(|| {
        ed2.command("toggleBold");
    });
    assert!(tick.get() > 0, "precondition: the patch inserted a node");
    assert!(runs.get() >= 2, "the effect ran, after the patch");
}

/// Control: the same, unbatched (what dispatch_event did on main).
#[test]
fn batched_control_unbatched() {
    use rinch_core::reactive::{Effect, Signal};
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")]));
    let tick = Signal::new(0u32);
    h.handle.on_change(move || tick.update(|t| *t += 1));
    let ed = h.handle.clone();
    let _e = Effect::new(move || {
        let _ = tick.get();
        let _ = ed.is_mark_active("bold");
    });
    h.handle.set_selection(Selection::text(Pos(1), Pos(4)));
    tick.set(5);
    h.handle.command("toggleBold");
    h.handle.command("toggleItalic");
}

/// on_change runs with the suppression guard dropped: a DOM read in it
/// sees effects of a write made earlier in the same callback.
#[test]
fn batched_on_change_keeps_program_order() {
    use rinch_core::reactive::{Effect, Signal, batch};
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello")]));
    let probe = NodeHandle::new(
        h.doc.borrow_mut().create_element("span"),
        Rc::downgrade(&h.doc),
    );
    let cls = Signal::new(String::from("a"));
    let p2 = probe.clone();
    let _e = Effect::new(move || p2.set_attribute("class", &cls.get()));
    let seen = Rc::new(RefCell::new(None));
    let sn = seen.clone();
    let p3 = probe.clone();
    h.handle.on_change(move || {
        cls.set("b".into());
        *sn.borrow_mut() = p3.get_attribute("class");
    });
    h.handle.set_selection(Selection::text(Pos(1), Pos(4)));
    let ed = h.handle.clone();
    batch(|| {
        ed.command("toggleBold");
    });
    assert_eq!(seen.borrow().as_deref(), Some("b"));
}

/// An editor *query* made under a caller's own RefCell borrow runs no
/// effect: a query does no DOM work, so `core()` does not flush (PR #882
/// review, E3). When it did, an effect borrowing the same RefCell panicked
/// inside the read. The effect still runs, at the batch's end.
#[test]
fn batched_a_query_under_the_callers_own_borrow_runs_no_effect() {
    use rinch_core::reactive::{Effect, Signal, batch};
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello")]));
    let model = Rc::new(RefCell::new(0u32));
    let flag = Signal::new(0u32);
    let m2 = model.clone();
    let _e = Effect::new(move || {
        let f = flag.get();
        *m2.borrow_mut() += f;
    });
    let ed = h.handle.clone();
    let m3 = model.clone();
    batch(|| {
        flag.set(1);
        let mut m = m3.borrow_mut();
        *m += u32::from(ed.is_mark_active("bold"));
    });
    assert_eq!(*model.borrow(), 1, "the effect ran after the handler");
}
