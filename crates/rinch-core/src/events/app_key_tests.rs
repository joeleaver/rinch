//! Tests for the app-wide key listeners (`app_key.rs`).

use super::*;
use crate::reactive::Scope;
use std::cell::RefCell;

type Log = Rc<RefCell<Vec<String>>>;

fn down(code: &str) -> KeyEventData {
    KeyEventData::new(code, code)
}
fn up(code: &str) -> KeyEventData {
    down(code).with_kind(super::super::KeyEventKind::Up)
}

fn clear() {
    let reaped = LISTENERS.with_mut(|l| std::mem::take(&mut **l));
    drop(reaped);
}

/// Every press the registered listeners hold, across all of them.
fn held_total() -> usize {
    LISTENERS.with(|l| l.iter().map(|e| e.held.len()).sum())
}

/// A listener that logs `name:Down:code` / `name:Up:code` and consumes
/// nothing.
fn logger(log: &Log, name: &'static str) -> AppKeyListener {
    let l = log.clone();
    on_app_key(move |k| {
        l.borrow_mut()
            .push(format!("{name}:{:?}:{}", k.kind, k.code));
        false
    })
}

/// Dispatch `data` as document `doc`'s key (`None`: as no document's).
fn dispatch_in(doc: Option<u64>, data: &KeyEventData) -> bool {
    match doc {
        Some(d) => {
            let _d = crate::context::push_dispatching_doc(d);
            dispatch_app_key(data)
        }
        None => dispatch_app_key(data),
    }
}

/// Two parties both hear a key, in registration order, and a consuming
/// press stops the later one. The release reaches only the listener that
/// heard the press: the later one never saw it go down (#3062).
#[test]
fn listeners_run_in_order_and_a_consumed_press_stops_later_ones() {
    clear();
    let log = Rc::new(RefCell::new(Vec::new()));
    let l1 = log.clone();
    let _a = on_app_key(move |k| {
        l1.borrow_mut().push(format!("a:{}", k.key));
        k.key == "Delete"
    });
    let l2 = log.clone();
    let _b = on_app_key(move |k| {
        l2.borrow_mut().push(format!("b:{}", k.key));
        false
    });
    assert!(!dispatch_app_key(&down("KeyZ")));
    assert!(dispatch_app_key(&down("Delete")));
    assert!(
        !dispatch_app_key(&up("Delete")),
        "a release is never consumed"
    );
    assert_eq!(*log.borrow(), ["a:KeyZ", "b:KeyZ", "a:Delete", "a:Delete"]);
    clear();
}

/// Disposing the scope that registered a listener releases it; a
/// listener registered outside any scope stays; `remove` releases early.
#[test]
fn a_listener_goes_with_its_scope_and_remove_releases_it() {
    clear();
    let hits = Rc::new(Cell::new(0));
    let scope = Scope::new();
    let h = hits.clone();
    scope.run(|| {
        on_app_key(move |_| {
            h.set(h.get() + 1);
            false
        });
    });
    let h = hits.clone();
    let app = on_app_key(move |_| {
        h.set(h.get() + 10);
        false
    });
    dispatch_app_key(&down("KeyA"));
    assert_eq!(hits.get(), 11);
    scope.dispose();
    assert_eq!(app_key_listener_count(), 1, "the scope's entry is gone");
    dispatch_app_key(&down("KeyA"));
    assert_eq!(hits.get(), 21);
    app.remove();
    dispatch_app_key(&down("KeyA"));
    assert_eq!(hits.get(), 21);
    assert_eq!(app_key_listener_count(), 0);
    clear();
}

/// Disposing a listener's scope, or removing it, drops the presses it held:
/// nothing is left behind, and a listener registered later is not handed the
/// release of a press it never heard.
#[test]
fn disposing_or_removing_a_listener_drops_its_presses() {
    clear();
    let log: Log = Default::default();
    let scope = Scope::new();
    scope.run(|| {
        logger(&log, "scoped");
    });
    let app = logger(&log, "app");
    dispatch_app_key(&down("KeyW"));
    dispatch_app_key(&down("KeyS"));
    assert_eq!(held_total(), 4, "each listener holds W and S");
    scope.dispose();
    assert_eq!(
        held_total(),
        2,
        "the scoped listener's presses went with it"
    );
    app.remove();
    assert_eq!(held_total(), 0, "and the app listener's with it");

    let _late = logger(&log, "late");
    dispatch_app_key(&up("KeyW"));
    dispatch_app_key(&up("KeyS"));
    assert_eq!(
        *log.borrow(),
        [
            "scoped:Down:KeyW",
            "app:Down:KeyW",
            "scoped:Down:KeyS",
            "app:Down:KeyS"
        ]
    );
    assert_eq!(held_total(), 0);
    clear();
}

/// A release reaches a listener only when it heard the press (#3062): one
/// whose press stopped ahead of them is not theirs. A delivered press's
/// release is offered once, however many times the key repeated, and a press
/// a listener consumed has its release offered to that listener alone.
#[test]
fn a_release_is_offered_only_after_its_press_was() {
    clear();
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let _a = on_app_key(move |k| {
        l.borrow_mut().push(format!("a:{:?}:{}", k.kind, k.code));
        k.code == "KeyC"
    });
    let _b = logger(&log, "b");
    // KeyX's press was consumed before the listeners' stage.
    dispatch_app_key(&up("KeyX"));
    dispatch_app_key(&down("KeyY"));
    dispatch_app_key(&down("KeyY"));
    dispatch_app_key(&up("KeyY"));
    dispatch_app_key(&up("KeyY"));
    assert!(dispatch_app_key(&down("KeyC")));
    dispatch_app_key(&up("KeyC"));
    assert_eq!(
        *log.borrow(),
        [
            "a:Down:KeyY",
            "b:Down:KeyY",
            "a:Down:KeyY",
            "b:Down:KeyY",
            "a:Up:KeyY",
            "b:Up:KeyY",
            "a:Down:KeyC",
            "a:Up:KeyC",
        ]
    );
    clear();
}

/// A key held in one window and released in another (#3062 round 3): the
/// release reaches every listener that heard the press — one registered from
/// `main` and the first window's own — and no listener of the second window,
/// which never saw the key go down (#3082). Under the thread-wide record a
/// blur of the first window forgot the press and the release was withheld
/// from everyone, so the `main` listener's W stayed down for good.
#[test]
fn a_release_in_another_document_reaches_exactly_the_listeners_that_heard_its_press() {
    clear();
    let log: Log = Default::default();
    let _main = logger(&log, "main");
    {
        let _d = crate::context::push_dispatching_doc(7);
        logger(&log, "doc7");
    }
    {
        let _d = crate::context::push_dispatching_doc(8);
        logger(&log, "doc8");
    }
    dispatch_in(Some(7), &down("KeyW"));
    // Window 7 blurs, window 8 takes the keyboard; nothing is forgotten.
    dispatch_in(Some(8), &up("KeyW"));
    assert_eq!(
        *log.borrow(),
        [
            "main:Down:KeyW",
            "doc7:Down:KeyW",
            "main:Up:KeyW",
            "doc7:Up:KeyW"
        ]
    );
    assert_eq!(held_total(), 0);
    clear();
}

/// The race the blur-time clearing lost: the second window's press arrives
/// before the first window's blur. Nothing is cleared by a blur any more, so
/// the press made in the second window keeps its release.
#[test]
fn a_press_in_the_newly_focused_window_keeps_its_release() {
    clear();
    let log: Log = Default::default();
    let _main = logger(&log, "main");
    dispatch_in(Some(8), &down("KeyQ"));
    // (window 7's blur would arrive here.)
    dispatch_in(Some(8), &up("KeyQ"));
    assert_eq!(*log.borrow(), ["main:Down:KeyQ", "main:Up:KeyQ"]);
    clear();
}

/// The round-2 scenario: a press the listener heard loses its release (the
/// key let go in another app), then a fresh press of that key is consumed
/// ahead of the listeners (a field's `onkeydown`). The backend reports the
/// fresh press, which drops the stale record, so that press's release is not
/// the listener's.
#[test]
fn a_fresh_press_drops_a_stale_press_record() {
    clear();
    let log: Log = Default::default();
    let _a = logger(&log, "a");
    let _b = {
        let _d = crate::context::push_dispatching_doc(7);
        logger(&log, "doc7")
    };
    dispatch_in(Some(7), &down("F2"));
    // The release went to another app. Later, in document 8, a fresh F2
    // press is consumed before the listeners' stage.
    note_fresh_key_press("F2");
    dispatch_in(Some(8), &up("F2"));
    assert_eq!(*log.borrow(), ["a:Down:F2", "doc7:Down:F2"]);
    assert_eq!(held_total(), 0, "every listener's F2 went, doc7's included");
    clear();
}

/// A press the backend cannot classify is not reported, so a stale record
/// survives it: at worst one extra release reaches the listener that heard
/// that key's earlier press, and only one.
#[test]
fn an_unreported_press_lets_at_most_one_extra_release_through() {
    clear();
    let log: Log = Default::default();
    let _a = logger(&log, "a");
    dispatch_app_key(&down("KeyS"));
    // Release lost; a later S press is consumed ahead of the listeners and
    // not reported fresh (a host answering `KeyRepeat::Unknown`).
    dispatch_app_key(&up("KeyS"));
    dispatch_app_key(&up("KeyS"));
    assert_eq!(*log.borrow(), ["a:Down:KeyS", "a:Up:KeyS"]);
    clear();
}

/// An auto-repeat consumed ahead of the listeners (a surface focused while
/// the key is held) is not a fresh press: the listener that heard the key go
/// down still hears it come up.
#[test]
fn a_held_key_consumed_ahead_on_repeat_still_releases_to_its_listener() {
    clear();
    let log: Log = Default::default();
    let _a = logger(&log, "a");
    dispatch_app_key(&down("KeyD"));
    // Repeats of D claimed by a surface: no dispatch, no fresh report.
    dispatch_app_key(&up("KeyD"));
    assert_eq!(*log.borrow(), ["a:Down:KeyD", "a:Up:KeyD"]);
    clear();
}

/// A listener registered for one document does not hear another's presses;
/// one registered outside any document hears both.
#[test]
fn a_documents_listener_hears_only_its_documents_keys() {
    clear();
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    {
        let _d = crate::context::push_dispatching_doc(7);
        on_app_key(move |k| {
            l.borrow_mut().push(format!("doc7:{}", k.key));
            false
        });
    }
    let l = log.clone();
    on_app_key(move |k| {
        l.borrow_mut().push(format!("any:{}", k.key));
        false
    });
    dispatch_in(Some(8), &down("a"));
    dispatch_in(Some(7), &down("b"));
    assert_eq!(*log.borrow(), ["any:a", "doc7:b", "any:b"]);
    clear();
}

/// A listener that removes a later one during dispatch stops it hearing
/// that same key, press or release.
#[test]
fn a_listener_removed_mid_dispatch_is_not_called() {
    clear();
    let hits = Rc::new(Cell::new(0));
    let victim: Rc<RefCell<Option<AppKeyListener>>> = Rc::new(RefCell::new(None));
    let v = victim.clone();
    let _a = on_app_key(move |k| {
        if k.is_up()
            && let Some(l) = v.borrow_mut().take()
        {
            l.remove();
        }
        false
    });
    let h = hits.clone();
    *victim.borrow_mut() = Some(on_app_key(move |_| {
        h.set(h.get() + 1);
        false
    }));
    dispatch_app_key(&down("KeyA"));
    assert_eq!(hits.get(), 1);
    dispatch_app_key(&up("KeyA"));
    assert_eq!(hits.get(), 1, "removed by the first listener's release");
    assert_eq!(held_total(), 0);
    clear();
}

/// The same for a press: a listener removed by an earlier one does not hear
/// the press that removed it.
#[test]
fn a_listener_removed_mid_press_is_not_called() {
    clear();
    let hits = Rc::new(Cell::new(0));
    let victim: Rc<RefCell<Option<AppKeyListener>>> = Rc::new(RefCell::new(None));
    let v = victim.clone();
    let _a = on_app_key(move |_| {
        if let Some(l) = v.borrow_mut().take() {
            l.remove();
        }
        false
    });
    let h = hits.clone();
    *victim.borrow_mut() = Some(on_app_key(move |_| {
        h.set(h.get() + 1);
        false
    }));
    dispatch_app_key(&down("KeyA"));
    assert_eq!(hits.get(), 0);
    clear();
}
