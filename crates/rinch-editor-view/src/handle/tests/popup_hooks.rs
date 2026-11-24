use super::*;
use std::cell::Cell;

/// A callback registered while a component's scope is ambient stops
/// once that scope is disposed (#147/#183): a component that mounts an
/// app-level handle, registers the popup hooks against its own signals
/// and unmounts, after which the app moves the still-live handle's
/// selection. Before the owner was recorded this read a freed signal
/// and panicked.
#[test]
fn selection_callback_stops_when_its_scope_is_disposed() {
    use rinch_core::reactive::{Scope, Signal};
    let h = two_paragraphs();
    let scope = Scope::new();
    let fired = Rc::new(Cell::new(0u32));
    scope.run(|| {
        let sig = Signal::new(0u32);
        let fired = fired.clone();
        h.handle.on_selection_change(move |_| {
            fired.set(fired.get() + 1);
            let _ = sig.get();
        });
    });
    h.handle.set_selection(Selection::cursor(Pos(3)));
    assert_eq!(fired.get(), 1, "control: live");
    scope.dispose();
    h.handle.set_selection(Selection::cursor(Pos(4)));
    assert_eq!(
        fired.get(),
        1,
        "a disposed component's callback must not run"
    );
}

/// `on_change` and `on_key` follow the same owner rule; a callback
/// registered outside any scope keeps app lifetime.
#[test]
fn change_and_key_callbacks_stop_with_their_scope_and_ownerless_ones_live_on() {
    use rinch_core::reactive::{Scope, Signal};
    let h = two_paragraphs();
    let scope = Scope::new();
    let changes = Rc::new(Cell::new(0u32));
    let keys = Rc::new(Cell::new(0u32));
    scope.run(|| {
        let sig = Signal::new(0u32);
        h.handle.on_change({
            let changes = changes.clone();
            move || {
                changes.set(changes.get() + 1);
                let _ = sig.get();
            }
        });
        h.handle.on_key({
            let keys = keys.clone();
            move |_| {
                keys.set(keys.get() + 1);
                let _ = sig.get();
                true
            }
        });
    });
    let esc = key("Escape");
    h.handle.set_selection(Selection::cursor(Pos(3)));
    h.handle.insert_text("x");
    assert!(h.handle.offer_key(&esc), "control: a live on_key claims");
    assert_eq!((changes.get(), keys.get()), (1, 1), "control: live");
    scope.dispose();
    h.handle.insert_text("y");
    assert!(!h.handle.offer_key(&esc), "a dead on_key claims nothing");
    assert_eq!(
        (changes.get(), keys.get()),
        (1, 1),
        "neither runs after dispose"
    );

    // Registered with no ambient owner: app lifetime.
    let late = Rc::new(Cell::new(0u32));
    h.handle.on_change({
        let late = late.clone();
        move || late.set(late.get() + 1)
    });
    h.handle.insert_text("z");
    assert_eq!(late.get(), 1, "an ownerless callback keeps firing");
}

/// Issue #931: every hook runs untracked. `register` installs the hook
/// under test with a body that calls the `reads` it is handed (which
/// counts a firing and reads a signal nothing else reads); `drive`
/// makes the hook fire, and is called from an app effect created
/// inside another effect's run — two frames deep on the observer
/// stack, which is what tells whole-stack suspension apart from plain
/// `untracked` (#932). Writing the hook's signal must re-run neither
/// effect.
fn assert_hook_untracked(register: impl FnOnce(Rc<dyn Fn()>), drive: impl Fn() + 'static) {
    use rinch_core::reactive::{Effect, Signal};
    let (own, store) = (Signal::new(0u32), Signal::new(0u32));
    let fired = Rc::new(Cell::new(0u32));
    register({
        let fired = fired.clone();
        Rc::new(move || {
            fired.set(fired.get() + 1);
            let _ = store.get();
        })
    });
    let drive = Rc::new(drive);
    let (outer, inner) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(0u32)));
    let slot: Rc<RefCell<Option<Effect>>> = Rc::default();
    let _outer = Effect::new({
        let (outer, inner, slot) = (outer.clone(), inner.clone(), slot.clone());
        move || {
            outer.set(outer.get() + 1);
            let (inner, drive) = (inner.clone(), drive.clone());
            slot.replace(Some(Effect::new(move || {
                let _ = own.get();
                inner.set(inner.get() + 1);
                drive();
            })));
        }
    });
    assert_eq!(fired.get(), 1, "control: the hook fired inside the effect");
    store.set(1);
    assert_eq!(
        (outer.get(), inner.get()),
        (1, 1),
        "a signal only the hook read re-ran the app effect that drove it"
    );
    own.set(1);
    assert_eq!(inner.get(), 2, "positive control: the app effect is live");
    assert_eq!(fired.get(), 2, "and its re-run fired the hook again");
}

/// A caret that moves on every call: `a`, `b`, `a`, `b`, …
fn alternating(a: usize, b: usize) -> impl Fn() -> Selection {
    let flip = Cell::new(false);
    move || {
        flip.set(!flip.get());
        Selection::cursor(Pos(if flip.get() { a } else { b }))
    }
}

#[test]
fn the_selection_hook_run_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs().handle;
    let next = alternating(3, 4);
    let driven = h.clone();
    assert_hook_untracked(
        |reads| h.on_selection_change(move |_| reads()),
        move || driven.set_selection(next()),
    );
}

/// The same, for a hook a component registered: it runs inside its
/// owner, which is a different branch of `Hook::invoke`.
#[test]
fn an_owned_hook_run_from_an_effect_subscribes_nobody() {
    use rinch_core::reactive::Scope;
    let h = two_paragraphs().handle;
    let scope = Scope::new();
    let next = alternating(3, 4);
    let driven = h.clone();
    assert_hook_untracked(
        |reads| scope.run(|| h.on_selection_change(move |_| reads())),
        move || driven.set_selection(next()),
    );
}

#[test]
fn the_change_hook_run_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs().handle;
    let driven = h.clone();
    assert_hook_untracked(
        |reads| h.on_change(move || reads()),
        move || {
            driven.insert_text("x");
        },
    );
}

#[test]
fn the_key_hook_run_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs().handle;
    let driven = h.clone();
    assert_hook_untracked(
        |reads| {
            h.on_key(move |_| {
                reads();
                false
            })
        },
        move || {
            driven.offer_key(&key("Escape"));
        },
    );
}

#[test]
fn the_caret_moved_hook_run_from_an_effect_subscribes_nobody() {
    // Empty paragraphs, laid out by hand: the mock measures no text,
    // and a caret needs somewhere to land (0[p 1]2[p 3]4).
    let s = schema();
    let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
    let h = mount(doc_node(&s, vec![empty(), empty()]));
    let blocks = children(&h, h.container_id);
    {
        let mut m = h.mock.borrow_mut();
        for (i, b) in blocks.iter().enumerate() {
            m.__set_node_layout(*b, 0.0, i as f32 * 20.0, 200.0, 20.0);
        }
    }
    let next = alternating(3, 1);
    let driven = h.handle.clone();
    assert_hook_untracked(
        |reads| h.handle.on_caret_moved(move || reads()),
        move || {
            driven.set_selection(next());
            driven.update_caret();
        },
    );
}

/// `go <a>here</a> and <a>there</a>`: "here" is 4..8, "there" 13..18.
fn linked() -> EditorHandle {
    let h = crate::create_editor();
    assert!(h.load_html(r#"<p>go <a href="a:1">here</a> and <a href="b:2">there</a></p>"#));
    h
}

#[test]
fn the_link_click_hook_run_from_an_effect_subscribes_nobody() {
    let h = linked();
    let click = crate::LinkClick {
        link: h.link_at(Pos(4)).expect("on a link"),
        primary: true,
        ctrl: true,
        meta: false,
        shift: false,
        alt: false,
    };
    let driven = h.clone();
    assert_hook_untracked(
        |reads| {
            h.on_link_click(move |_| {
                reads();
                true
            })
        },
        move || {
            driven.dispatch_link_click(&click);
        },
    );
}

#[test]
fn the_link_hover_hook_run_from_an_effect_subscribes_nobody() {
    let h = linked();
    let hovers = [4, 13].map(|p| crate::LinkHover {
        link: h.link_at(Pos(p)).expect("on a link"),
        rect: rinch_core::reactive::ElementBounds::default(),
    });
    let flip = Cell::new(0usize);
    let driven = h.clone();
    assert_hook_untracked(
        |reads| h.on_link_hover(move |_| reads()),
        move || {
            // A different link each run, so each one is a change.
            flip.set(flip.get() + 1);
            let hover = hovers[flip.get() % 2].clone();
            crate::registry::set_link_hover(Some(931), Some((driven.clone(), hover)));
        },
    );
    crate::registry::set_link_hover(Some(931), None);
}

/// `<p>hello</p><p>world</p>`: "hello" is 1..6, "world" is 8..13.
fn two_paragraphs() -> Harness {
    let s = schema();
    mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")]))
}

/// Record every selection the callback reports.
fn record(h: &EditorHandle) -> Rc<RefCell<Vec<Selection>>> {
    let seen: Rc<RefCell<Vec<Selection>>> = Rc::default();
    h.on_selection_change({
        let seen = seen.clone();
        move |sel| seen.borrow_mut().push(sel.clone())
    });
    seen
}

/// The text of the document's `i`th block.
fn block_text(h: &EditorHandle, i: usize) -> String {
    let doc = h.doc();
    let block = doc.child(i);
    (0..block.child_count())
        .filter_map(|j| block.child(j).text().map(str::to_string))
        .collect()
}

fn key(name: &str) -> EditorKey<'_> {
    EditorKey {
        key: name,
        primary: false,
        ctrl: false,
        meta: false,
        shift: false,
        alt: false,
        repeat: false,
    }
}

#[test]
fn the_selection_callback_fires_once_per_change_and_never_for_none() {
    let h = two_paragraphs().handle;
    h.set_selection(Selection::cursor(Pos(3)));
    let seen = record(&h);

    h.set_selection(Selection::cursor(Pos(3)));
    assert!(seen.borrow().is_empty(), "the caret where it already is");

    assert!(h.insert_text("x"));
    assert_eq!(
        *seen.borrow(),
        vec![Selection::cursor(Pos(4))],
        "typing moves the caret on: one call, with the new selection"
    );

    assert!(h.move_cursor(CursorMotion::CharRight, false));
    assert_eq!(seen.borrow().len(), 2, "an arrow key's motion");

    h.set_selection(Selection::text(Pos(1), Pos(4)));
    seen.borrow_mut().clear();
    assert!(h.command("toggleBold"));
    assert!(
        seen.borrow().is_empty(),
        "formatting a range leaves the range as it was"
    );

    // An app's edit after the caret maps nothing.
    h.set_selection(Selection::cursor(Pos(2)));
    seen.borrow_mut().clear();
    assert!(h.update(|st| {
        let mut tr = st.tr();
        let bang = Fragment::from_node(st.schema().text("!").unwrap());
        tr.replace_with(9, 9, bang).ok()?;
        Some(tr)
    }));
    assert!(seen.borrow().is_empty(), "an edit after the caret");

    // ...and one before it moves it.
    assert!(h.update(|st| {
        let mut tr = st.tr();
        let bang = Fragment::from_node(st.schema().text("!").unwrap());
        tr.replace_with(1, 1, bang).ok()?;
        Some(tr)
    }));
    assert_eq!(*seen.borrow(), vec![Selection::cursor(Pos(3))]);

    // A load that resets the selection is a change of selection.
    seen.borrow_mut().clear();
    assert!(h.load_html("<p>other</p>"));
    assert_eq!(seen.borrow().len(), 1, "a load that moves the caret");
}

#[test]
fn a_refused_edit_reports_nothing() {
    let h = two_paragraphs().handle;
    h.set_selection(Selection::cursor(Pos(3)));
    h.set_read_only(true);
    let seen = record(&h);
    assert!(!h.insert_text("x"));
    assert!(seen.borrow().is_empty(), "read-only refused the typing");
    h.set_selection(Selection::cursor(Pos(4)));
    assert_eq!(seen.borrow().len(), 1, "the caret still moves, and says so");
}

/// The callback runs with nothing borrowed: it may read, measure, move the
/// selection again (which reports again, from inside itself) and edit.
#[test]
fn the_selection_callback_may_reenter_the_handle() {
    let h = two_paragraphs().handle;
    let calls = Rc::new(Cell::new(0u32));
    h.on_selection_change({
        let (h, calls) = (h.clone(), calls.clone());
        move |sel| {
            calls.set(calls.get() + 1);
            assert_eq!(&h.selection(), sel, "the state already holds it");
            let _ = h.doc();
            let _ = h.caret_rect(sel.head());
            // Snap every caret in the second paragraph to its start:
            // one more change, then none.
            if sel.head().0 > 8 {
                h.set_selection(Selection::cursor(Pos(8)));
            }
        }
    });
    h.set_selection(Selection::cursor(Pos(11)));
    assert_eq!(h.selection(), Selection::cursor(Pos(8)));
    assert_eq!(calls.get(), 2, "the move, then the callback's own move");

    // An edit from inside the callback, too.
    h.on_selection_change({
        let h = h.clone();
        move |sel| {
            if sel.head() == Pos(3) {
                assert!(h.insert_text("Z"));
            }
        }
    });
    h.set_selection(Selection::cursor(Pos(3)));
    assert_eq!(block_text(&h, 0), "heZllo", "the callback's edit landed");
}

#[test]
fn a_key_is_offered_to_the_callback_and_its_answer_decides() {
    let h = two_paragraphs().handle;
    assert!(!h.offer_key(&key("ArrowDown")), "no callback: not consumed");

    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    h.on_key({
        let (h, seen) = (h.clone(), seen.clone());
        move |k| {
            seen.borrow_mut().push(k.key.to_string());
            // Re-entering mutably from the callback must not panic.
            if k.key == "Enter" {
                assert!(h.insert_text("!"));
            }
            matches!(k.key, "ArrowDown" | "Enter")
        }
    });
    assert!(h.offer_key(&key("ArrowDown")));
    assert!(h.offer_key(&key("Enter")));
    assert!(!h.offer_key(&key("a")), "declined: the editor's to handle");
    assert_eq!(*seen.borrow(), ["ArrowDown", "Enter", "a"]);
    assert_eq!(block_text(&h, 0), "!hello");
}

/// A panic already in progress when the handle is borrowed does not
/// silence the callbacks. On `wasm32` that is every borrow after any
/// panic the page survived: a panic there aborts without unwinding and
/// the panic count is never taken back. Simulated natively by editing
/// from a destructor that runs while a panic unwinds.
#[test]
fn a_panic_in_progress_before_the_borrow_does_not_silence_callbacks() {
    struct EditOnDrop(EditorHandle);
    impl Drop for EditOnDrop {
        fn drop(&mut self) {
            assert!(std::thread::panicking());
            self.0.set_selection(Selection::cursor(Pos(4)));
        }
    }
    let h = two_paragraphs().handle;
    h.set_selection(Selection::cursor(Pos(2)));
    let seen = record(&h);
    let edit = EditOnDrop(h.clone());
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _edit = edit;
        panic!("a panic unrelated to the editor");
    }));
    assert!(unwound.is_err());
    assert_eq!(*seen.borrow(), vec![Selection::cursor(Pos(4))]);
}

/// A key pressed during an IME composition belongs to the input method.
#[test]
fn no_key_is_offered_while_composing() {
    let h = two_paragraphs().handle;
    let offered = Rc::new(Cell::new(0u32));
    h.on_key({
        let offered = offered.clone();
        move |_| {
            offered.set(offered.get() + 1);
            true
        }
    });
    h.ime_set_preedit("ka", None);
    assert!(!h.offer_key(&key("ArrowDown")));
    assert_eq!(offered.get(), 0);
    h.ime_commit("か");
    assert!(h.offer_key(&key("ArrowDown")));
    assert_eq!(offered.get(), 1);
}

/// Geometry needs a host that has some: an unmounted editor and the
/// geometry-less mock answer `None` (the desktop and web answers are
/// pinned in their own crates).
#[test]
fn caret_rect_is_none_without_geometry() {
    let unmounted = crate::create_editor();
    assert!(unmounted.load_html("<p>hello</p>"));
    assert_eq!(unmounted.caret_rect(Pos(1)), None, "not mounted");

    let h = two_paragraphs().handle;
    assert_eq!(h.caret_rect(Pos(1)), None, "the mock has no geometry");
    assert_eq!(h.caret_rect(Pos(0)), None, "not inside a textblock");
}

/// A peer's edit that maps the caret reports it; one after the caret
/// does not.
#[cfg(feature = "collaboration")]
#[test]
fn a_remote_edit_that_moves_the_caret_reports_it() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    let wire: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let snapshot = host
        .start_collaboration_host({
            let wire = wire.clone();
            move |d| wire.borrow_mut().push(d)
        })
        .unwrap();
    guest.start_collaboration_guest(&snapshot, |_| {}).unwrap();
    guest.set_selection(Selection::cursor(Pos(3)));
    let seen = record(&guest);
    let deliver = || {
        for d in wire.borrow_mut().drain(..) {
            guest.collab_receive(&d);
        }
    };

    host.set_selection(Selection::cursor(Pos(6)));
    assert!(host.insert_text("!"));
    deliver();
    assert_eq!(block_text(&guest, 0), "hello!");
    assert!(seen.borrow().is_empty(), "an edit after the caret");

    host.set_selection(Selection::cursor(Pos(1)));
    assert!(host.insert_text(">"));
    deliver();
    assert_eq!(block_text(&guest, 0), ">hello!");
    assert_eq!(*seen.borrow(), vec![Selection::cursor(Pos(4))]);
}
