//! Program order between a handler's signal writes and its own DOM calls,
//! over [`MockDomDocument`] — the web-shaped half of PR #882's D1/C1.
//!
//! An event handler runs as a batch, so its effects run when it returns. On the
//! web an effect *is* the DOM mutation, so without a rule a handler that
//! reveals something through a signal and then focuses it, scrolls to it or
//! reads it acted on the DOM as it was before the handler. The rule:
//! every `NodeHandle` operation made from handler code first runs the effects
//! the batch has queued (`flush_pending_effects`). These fixtures fail with
//! `NodeHandle::accessed_doc` reverted to a plain `Weak::upgrade`.

use crate::dom::mock::MockDomDocument;
use crate::dom::{DomDocument, NodeHandle, RenderScope};
use crate::events::{dispatch_event, register_handler};
use crate::reactive::{Effect, Signal, flush_pending_effects, reactive_counters};
use crate::show_dom;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn doc() -> Rc<RefCell<MockDomDocument>> {
    Rc::new(RefCell::new(MockDomDocument::new()))
}

fn scope(doc: &Rc<RefCell<MockDomDocument>>) -> RenderScope {
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    RenderScope::new(dyn_doc, body)
}

/// Reveal-by-effect, then act on the revealed node in the same handler: the
/// handler sees the node shown (on the web, `focus()` on a still-hidden
/// element is refused), and the scroll request lands on it.
#[test]
fn a_handler_acts_on_the_node_its_own_write_just_revealed() {
    let doc = doc();
    let mut scope = scope(&doc);
    let panel = scope.create_element("div");
    scope.body_handle().append_child(&panel);

    let open = Signal::new(false);
    let p = panel.clone();
    let _reveal = Effect::new(move || {
        if open.get() {
            p.remove_attribute("hidden");
        } else {
            p.set_attribute("hidden", "");
        }
    });
    assert!(panel.get_attribute("hidden").is_some(), "precondition");

    let seen_hidden: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let s = Rc::clone(&seen_hidden);
    let target = panel.clone();
    let id = register_handler(Rc::new(move || {
        open.set(true);
        s.set(Some(target.get_attribute("hidden").is_some()));
        target.scroll_into_view();
    }));
    assert!(dispatch_event(id));

    assert_eq!(
        seen_hidden.get(),
        Some(false),
        "the reveal ran before the read"
    );
    assert_eq!(
        doc.borrow_mut().drain_scroll_into_view_requests(),
        vec![panel.node_id()]
    );
}

/// "Append a message, then scroll to the bottom": a row a `show_dom` branch
/// inserts is in the list by the time the handler looks.
#[test]
fn a_handler_sees_the_row_its_own_write_inserted() {
    let doc = doc();
    let mut scope = scope(&doc);
    let list = scope.create_element("ul");
    scope.body_handle().append_child(&list);
    let has_row = Signal::new(false);
    show_dom(
        &mut scope,
        &list,
        move || has_row.get(),
        |s: &mut RenderScope| s.create_element("li"),
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let li_count = |l: &NodeHandle| {
        l.children()
            .iter()
            .filter(|c| c.tag_name().as_deref() == Some("li"))
            .count()
    };
    assert_eq!(li_count(&list), 0);

    let counted = Rc::new(Cell::new(usize::MAX));
    let c = Rc::clone(&counted);
    let l = list.clone();
    let id = register_handler(Rc::new(move || {
        has_row.set(true);
        c.set(li_count(&l));
    }));
    assert!(dispatch_event(id));
    assert_eq!(counted.get(), 1);
}

/// A direct DOM write after a signal write lands last, as it always did: the
/// effect bound to the signal no longer overwrites it at the end of the batch.
#[test]
fn a_direct_write_after_a_signal_write_wins() {
    let doc = doc();
    let mut scope = scope(&doc);
    let node = scope.create_element("div");
    scope.body_handle().append_child(&node);
    let cls = Signal::new("a".to_string());
    let n = node.clone();
    let _bind = Effect::new(move || n.set_attribute("class", &cls.get()));

    let target = node.clone();
    let id = register_handler(Rc::new(move || {
        cls.set("from-signal".into());
        target.set_attribute("class", "direct");
    }));
    assert!(dispatch_event(id));
    assert_eq!(node.get_attribute("class").as_deref(), Some("direct"));
}

/// A handler that never touches the DOM itself still flushes once — the flush
/// on DOM access is not a per-write flush in disguise.
#[test]
fn a_handler_that_only_writes_signals_still_flushes_once() {
    let doc = doc();
    let mut scope = scope(&doc);
    let node = scope.create_element("div");
    scope.body_handle().append_child(&node);
    let a = Signal::new(0);
    let b = Signal::new(0);
    let n = node.clone();
    let _bind = Effect::new(move || n.set_attribute("data-sum", &(a.get() + b.get()).to_string()));

    let before = reactive_counters().effect_runs;
    let id = register_handler(Rc::new(move || {
        a.set(1);
        b.set(2);
    }));
    assert!(dispatch_event(id));
    assert_eq!(reactive_counters().effect_runs - before, 1);
    assert_eq!(node.get_attribute("data-sum").as_deref(), Some("3"));
}

/// Inside an effect body the queue belongs to the flush that is running it,
/// so a DOM call there must not drain it. Here an effect *created* inside a
/// handler (its first run happens while the batch is open) touches the DOM;
/// the effect the handler queued before it still runs after it, at the
/// batch's end, not nested inside its first run.
#[test]
fn a_dom_call_inside_an_effect_does_not_drain_the_queue() {
    let doc = doc();
    let mut scope = scope(&doc);
    let node = scope.create_element("div");
    scope.body_handle().append_child(&node);
    let x = Signal::new(0);
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = Rc::clone(&log);
    let _queued = Effect::new(move || {
        x.get();
        l.borrow_mut().push("queued");
    });
    log.borrow_mut().clear();

    let keep: Rc<RefCell<Vec<Effect>>> = Rc::new(RefCell::new(Vec::new()));
    let k = Rc::clone(&keep);
    let l = Rc::clone(&log);
    let n = node.clone();
    let id = register_handler(Rc::new(move || {
        x.set(1);
        let l = Rc::clone(&l);
        let n = n.clone();
        k.borrow_mut().push(Effect::new(move || {
            l.borrow_mut().push("created:start");
            let _ = n.get_attribute("class");
            l.borrow_mut().push("created:end");
        }));
    }));
    assert!(dispatch_event(id));
    assert_eq!(
        *log.borrow(),
        vec!["created:start", "created:end", "queued"]
    );
}

/// Outside any batch there is nothing to flush early: a write has already
/// flushed. The call is a no-op rather than an error.
#[test]
fn flush_pending_effects_outside_a_batch_is_a_no_op() {
    let n = Signal::new(0);
    let runs = Rc::new(Cell::new(0));
    let r = Rc::clone(&runs);
    let _e = Effect::new(move || {
        n.get();
        r.set(r.get() + 1);
    });
    n.set(1);
    flush_pending_effects();
    assert_eq!(runs.get(), 2);
}
