//! Review of PR #1527: is a plain `Scope` holding an effect, dropped from a
//! `thread_local!` at thread exit after the runtime's locals, an abort?
use rinch_core::reactive::{Effect, Scope, Signal};
use std::cell::RefCell;

#[test]
fn a_scope_dropped_at_thread_exit_does_not_abort() {
    std::thread::spawn(|| {
        thread_local! {
            static HOLD: RefCell<Option<Scope>> = const { RefCell::new(None) };
        }
        HOLD.with(|_| ());
        let on = Signal::new(false);
        let scope = Scope::new();
        scope.run(|| {
            Effect::new(move || {
                on.get();
            });
        });
        Scope::new().dispose();
        HOLD.with(|h| *h.borrow_mut() = Some(scope));
    })
    .join()
    .expect("the thread ends cleanly");
}
