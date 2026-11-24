//! Shared harness for the handler-task-owner tests (issues #3027, #3030): a
//! task scheduler that records polls per thread, a future that never completes
//! and reports its drop, and a mount that builds under a root scope's owner.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::events::EventHandlerId;
use rinch_core::task::set_task_scheduler;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

thread_local! {
    static POLLS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

fn record_poll(id: u64) {
    POLLS.with(|q| q.borrow_mut().push(id));
}

/// Run every scheduled poll until none is left. The scheduler records per
/// thread, so tests running in parallel do not see each other's tasks.
pub fn run_polls() {
    loop {
        let ids: Vec<u64> = POLLS.with(|q| std::mem::take(&mut *q.borrow_mut()));
        if ids.is_empty() {
            return;
        }
        for id in ids {
            rinch_core::task::poll_task(id);
        }
    }
}

/// What a task did: how often it was polled, and whether its future was
/// dropped (aborted, or dropped with its owner).
#[derive(Clone, Default)]
pub struct Trace {
    pub polls: Rc<Cell<u32>>,
    pub dropped: Rc<Cell<bool>>,
}

/// A future that never completes, standing in for a request in flight.
pub struct Pending(pub Trace);

impl Future for Pending {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        self.0.polls.set(self.0.polls.get() + 1);
        Poll::Pending
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.dropped.set(true);
    }
}

/// A mounted tree: the document (a `NodeHandle` holds only a `Weak` to it) and
/// the root scope, which owns everything the build created. Dropping the
/// scope is the unmount.
pub struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    pub scope: Option<RenderScope>,
    pub root: NodeHandle,
}

impl Mounted {
    pub fn unmount(&mut self) {
        if let Some(scope) = self.scope.take() {
            scope.dispose();
        }
    }
}

/// Mount `build` the way `mount_tree` does: under the root scope's owner.
pub fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Mounted {
    set_task_scheduler(record_poll);
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = {
        let _owner = scope.push_owner();
        build(&mut scope)
    };
    Mounted {
        _doc: doc,
        scope: Some(scope),
        root,
    }
}

/// The click handler ids (`data-rid`) of every node under `node` that
/// `pick` accepts, in tree order.
pub fn click_handlers(
    node: &NodeHandle,
    pick: &dyn Fn(&NodeHandle) -> bool,
) -> Vec<EventHandlerId> {
    let mut out = Vec::new();
    if pick(node)
        && let Some(rid) = node.get_attribute("data-rid")
    {
        out.push(EventHandlerId(rid.parse().unwrap()));
    }
    for child in node.children() {
        out.extend(click_handlers(&child, pick));
    }
    out
}

/// Whether `node` carries `class` among its classes.
pub fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .is_some_and(|c| c.split_whitespace().any(|c| c == class))
}
