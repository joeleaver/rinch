//! A test backend: a [`Matcher`] fed by the test, with conflicts it is told
//! about, delivering through the real `deliver`.

use std::cell::RefCell;
use std::rc::Rc;

use winit::keyboard::KeyCode;

use super::BackendImpl;
use crate::hotkey::matcher::{BindingKey, Matcher};
use crate::hotkey::{Backend, Binding, HotkeyError, Meta, Report, Shortcut, deliver};

#[derive(Default)]
pub(crate) struct FakeState {
    pub(crate) matcher: Matcher,
    /// Shortcuts "another application" holds.
    pub(crate) taken: Vec<Shortcut>,
    pub(crate) registered: Vec<(BindingKey, Option<Shortcut>, String)>,
    pub(crate) unregistered: Vec<BindingKey>,
}

pub(crate) struct Fake(pub(crate) Rc<RefCell<FakeState>>);

thread_local! {
    static PENDING: RefCell<Option<Fake>> = const { RefCell::new(None) };
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Make the next backend creation on this thread the fake; returns its state.
pub(crate) fn install() -> Rc<RefCell<FakeState>> {
    let state = Rc::new(RefCell::new(FakeState::default()));
    PENDING.with(|p| *p.borrow_mut() = Some(Fake(state.clone())));
    INSTALLED.with(|i| i.set(true));
    state
}

pub(crate) fn installed() -> bool {
    INSTALLED.with(|i| i.get())
}

pub(crate) fn take() -> Option<Box<dyn BackendImpl>> {
    PENDING
        .with(|p| p.borrow_mut().take())
        .map(|f| Box::new(f) as Box<dyn BackendImpl>)
}

/// Feed a key to the fake's matcher, delivering what it pressed or released.
///
/// Delivered inline rather than through `report`: the main-thread queue is
/// process-wide, and another test thread draining it would deliver this
/// thread's report into its own (empty) registry.
pub(crate) fn key(state: &Rc<RefCell<FakeState>>, code: KeyCode, down: bool) {
    let mut out = Vec::new();
    state.borrow_mut().matcher.key(code, down, &mut out);
    for (k, s) in out {
        deliver(k, Report::State(s));
    }
}

impl BackendImpl for Fake {
    fn kind(&self) -> Backend {
        Backend::Unsupported
    }

    fn register(
        &mut self,
        key: BindingKey,
        shortcut: Option<&Shortcut>,
        meta: &Meta,
    ) -> Result<Binding, HotkeyError> {
        let mut s = self.0.borrow_mut();
        let Some(sc) = shortcut else {
            return Err(HotkeyError::Unsupported("fake needs a shortcut".into()));
        };
        if s.taken.contains(sc) {
            return Err(HotkeyError::Conflict(sc.to_string()));
        }
        s.matcher.add(key, *sc);
        s.registered.push((key, Some(*sc), meta.id.clone()));
        Ok(Binding::Exact(*sc))
    }

    fn unregister(&mut self, key: BindingKey) {
        let mut s = self.0.borrow_mut();
        s.matcher.remove(key);
        s.unregistered.push(key);
    }
}
