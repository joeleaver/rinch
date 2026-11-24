//! Tests for `CreatorOwner` (issue #3027): every app-callback kind runs under
//! the owner ambient where it was created, and falls back to the owner of
//! whatever calls it once that creator is gone.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use super::{Owner, Scope, current_owner};
use crate::element::{Callback, ValueCallback};
use crate::events::{FileDropCallback, InputCallback, ScrollCallback, ScrollEvent};

/// Make a callback under a `creator` scope whose body records the ambient
/// owner, call it under another scope's owner, and answer `(creator, the
/// owner the body ran under)`.
fn ran_under<C>(make: impl FnOnce(Rc<dyn Fn()>) -> C, call: impl FnOnce(&C)) -> (Owner, Owner) {
    let creator = Scope::new();
    let caller = Scope::new();
    let seen: Rc<RefCell<Option<Owner>>> = Rc::default();
    let record: Rc<dyn Fn()> = {
        let seen = seen.clone();
        Rc::new(move || *seen.borrow_mut() = current_owner())
    };
    let callback = creator.owner().run(|| make(record));
    caller.owner().run(|| call(&callback));
    let ran = seen
        .borrow()
        .clone()
        .expect("the callback ran under an owner");
    (creator.owner(), ran)
}

#[test]
fn every_callback_kind_runs_under_its_creators_owner() {
    let (c, ran) = ran_under(|r| Callback::new(move || r()), |cb| cb.invoke());
    assert_eq!(ran, c, "Callback");
    let (c, ran) = ran_under(|r| ValueCallback::new(move |_: u8| r()), |cb| cb.invoke(1));
    assert_eq!(ran, c, "ValueCallback");
    let (c, ran) = ran_under(
        |r| InputCallback::new(move |_| r()),
        |cb| cb.invoke("x".into()),
    );
    assert_eq!(ran, c, "InputCallback");
    let (c, ran) = ran_under(
        |r| FileDropCallback::new(move |_| r()),
        |cb| cb.invoke(Vec::new()),
    );
    assert_eq!(ran, c, "FileDropCallback");
    let (c, ran) = ran_under(
        |r| ScrollCallback::new(move |_| r()),
        |cb| cb.invoke(ScrollEvent::default()),
    );
    assert_eq!(ran, c, "ScrollCallback");
}

/// A future that never completes and sets its flag when dropped.
struct Pending(Rc<Cell<bool>>);

impl Future for Pending {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

/// The creator unmounted before the call: the callback runs under the owner of
/// whatever calls it (the handler dispatching it), not under "no owner", so a
/// task it spawns is still cancelled when that owner goes.
#[test]
fn a_callback_whose_creator_is_gone_runs_under_its_callers_owner() {
    crate::task::set_task_scheduler(|_| {});
    let dropped = Rc::new(Cell::new(false));
    let creator = Scope::new();
    let callback = creator.owner().run(|| {
        let dropped = dropped.clone();
        Callback::new(move || {
            crate::task::spawn_local(Pending(dropped.clone()));
        })
    });
    creator.dispose();

    let caller = Scope::new();
    caller.owner().run(|| callback.invoke());
    assert!(!dropped.get(), "positive control: the task is running");
    caller.dispose();
    assert!(
        dropped.get(),
        "the task belonged to the calling handler's owner and ended with it"
    );
}
