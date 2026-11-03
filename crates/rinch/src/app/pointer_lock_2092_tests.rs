//! Pointer lock and raw motion (issue #2092), through the app's event path:
//! `handle_event` with the `PlatformEvent::PointerMotion` the desktop shell
//! makes of winit's device motion (and a native embed host forwards), the
//! focus loss, the Escape and the unmount that end a lock, and what the host
//! is asked to do about each. The host is a fake that records its calls; the
//! desktop host's lock-or-confine choice is pinned in `shell::pointer_lock`.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use crate::pointer_lock::test_host::{Log, install};
use crate::render_surface::{RenderSurfaceHandle, SurfaceEvent, create_render_surface};
use crate::window::{
    PointerLockError, PointerLockMode, exit_pointer_lock, on_pointer_motion, pointer_lock,
    request_pointer_lock, request_pointer_lock_on, set_pointer_lock_host,
};
use rinch_core::Component;
use rinch_core::events::{on_app_key, push_dismiss_handler};
use rinch_core::reactive::Signal;
use rinch_macros::rsx;
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

struct Fixture {
    app: RinchApp,
    surface: RenderSurfaceHandle,
    /// The `PointerMotion` steps the surface heard.
    surface_motion: Rc<RefCell<Vec<(f32, f32)>>>,
    /// The other element of the `if show` branch, for an element's lock.
    pad: Rc<RefCell<Option<NodeHandle>>>,
    show: Signal<bool>,
}

fn surface_branch(
    __scope: &mut RenderScope,
    surface: RenderSurfaceHandle,
    pad: Rc<RefCell<Option<NodeHandle>>>,
) -> NodeHandle {
    let root = rsx! { div { style: "width: 400px; height: 300px" } };
    let element = rsx! { div { style: "width: 40px; height: 30px" } };
    *pad.borrow_mut() = Some(element.clone());
    root.append_child(&element);
    let canvas = crate::render_surface::RenderSurface {
        surface: Some(surface),
    }
    .render(__scope, &[]);
    root.append_child(&canvas);
    root
}

/// A surface and a pad inside `if show`, so both can be unmounted.
fn mount() -> Fixture {
    set_pointer_lock_host(None);
    let surface = create_render_surface();
    let show = Signal::new(true);
    let pad: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let (sf, pd) = (surface.clone(), pad.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                if show.get() {
                    {surface_branch(__scope, sf.clone(), pd.clone())}
                }
            }
        }
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let surface_motion: Rc<RefCell<Vec<(f32, f32)>>> = Rc::default();
    let seen = surface_motion.clone();
    surface.set_event_handler(move |e| {
        if let SurfaceEvent::PointerMotion { dx, dy } = e {
            seen.borrow_mut().push((dx, dy));
        }
    });
    Fixture {
        app,
        surface,
        surface_motion,
        pad,
        show,
    }
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn motion(app: &mut RinchApp, dx: f64, dy: f64) {
    ev(app, PlatformEvent::PointerMotion { dx, dy });
}

fn escape(app: &mut RinchApp) -> Vec<AppAction> {
    ev(
        app,
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: Some("Escape".into()),
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
    )
}

fn unlocks(log: &Rc<RefCell<Log>>) -> usize {
    log.borrow().unlocks
}

/// The surface that asked hears the motion, off the cursor's grid and
/// unclamped; an `on_pointer_motion` handler hears it too. Before the lock
/// the surface hears none.
#[test]
fn a_locked_surface_hears_raw_motion() {
    let mut f = mount();
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let heard = Rc::new(RefCell::new(Vec::new()));
    let _sub = {
        let heard = heard.clone();
        on_pointer_motion(move |m| heard.borrow_mut().push((m.dx, m.dy)))
    };

    motion(&mut f.app, 2.0, 1.0);
    assert!(
        f.surface_motion.borrow().is_empty(),
        "no lock, no surface motion"
    );

    assert_eq!(f.surface.request_pointer_lock(), Ok(()));
    assert_eq!(log.borrow().locks, ["element"]);
    assert!(f.surface.has_pointer_lock());
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));

    motion(&mut f.app, 37.25, -1500.5);
    motion(&mut f.app, -3.0, 0.75);
    assert_eq!(*f.surface_motion.borrow(), [(37.25, -1500.5), (-3.0, 0.75)]);
    assert_eq!(
        *heard.borrow(),
        [(2.0, 1.0), (37.25, -1500.5), (-3.0, 0.75)]
    );

    exit_pointer_lock();
    motion(&mut f.app, 5.0, 5.0);
    assert_eq!(f.surface_motion.borrow().len(), 2, "released: no more");
    assert_eq!(unlocks(&log), 1);
    set_pointer_lock_host(None);
}

/// A window that loses focus gives the pointer back, and says so.
#[test]
fn a_focus_loss_releases_the_lock() {
    let mut f = mount();
    let log = install(Ok(Some(PointerLockMode::Locked)));
    ev(&mut f.app, PlatformEvent::WindowFocus(true));
    request_pointer_lock().unwrap();
    let lost = Rc::new(Cell::new(0));
    let _reader = {
        let lost = lost.clone();
        rinch_core::Effect::new(move || {
            if pointer_lock().is_none() {
                lost.set(lost.get() + 1);
            }
        })
    };
    assert_eq!(lost.get(), 0);
    ev(&mut f.app, PlatformEvent::WindowFocus(false));
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    assert_eq!(lost.get(), 1, "a reader hears the loss");
    set_pointer_lock_host(None);
}

/// Escape ends a held lock and goes no further: not to the dismiss stack, not
/// to an app key listener. The next Escape is the app's as usual.
#[test]
fn escape_releases_the_lock_and_is_consumed() {
    let mut f = mount();
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let dismissed = Rc::new(Cell::new(0));
    let _entry = {
        let dismissed = dismissed.clone();
        push_dismiss_handler(f.app.doc_key(), move || {
            dismissed.set(dismissed.get() + 1);
            true
        })
    };
    let app_keys = Rc::new(Cell::new(0));
    let _listener = {
        let app_keys = app_keys.clone();
        on_app_key(move |_| {
            app_keys.set(app_keys.get() + 1);
            false
        })
    };

    f.surface.request_pointer_lock().unwrap();
    let actions = escape(&mut f.app);
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    assert_eq!(dismissed.get(), 0, "the lock's Escape closes nothing");
    assert_eq!(app_keys.get(), 0);
    assert!(actions.contains(&AppAction::RequestRedraw));

    escape(&mut f.app);
    assert_eq!(dismissed.get(), 1, "without a lock Escape dismisses");
    assert_eq!(unlocks(&log), 1);
    set_pointer_lock_host(None);
}

/// A surface that unmounts takes its lock with it, at once.
#[test]
fn an_unmounted_surface_releases_its_lock() {
    let mut f = mount();
    let log = install(Ok(Some(PointerLockMode::Locked)));
    f.surface.request_pointer_lock().unwrap();
    f.show.set(false);
    assert_eq!(pointer_lock(), None, "released by the unmount itself");
    assert_eq!(unlocks(&log), 1);
    assert!(!f.surface.has_pointer_lock());
    assert_eq!(
        f.surface.request_pointer_lock(),
        Err(PointerLockError::NotConnected),
        "an unmounted surface cannot ask"
    );
    motion(&mut f.app, 1.0, 1.0);
    assert!(f.surface_motion.borrow().is_empty());
    set_pointer_lock_host(None);
}

/// An element's lock ends on the first event after it leaves the document —
/// `AboutToWait`, which ends every turn of the desktop loop.
#[test]
fn an_element_that_leaves_the_document_releases_its_lock() {
    let mut f = mount();
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let pad = f.pad.borrow().clone().unwrap();
    request_pointer_lock_on(&pad).unwrap();
    ev(&mut f.app, PlatformEvent::AboutToWait);
    assert_eq!(
        pointer_lock(),
        Some(PointerLockMode::Locked),
        "positive control: a connected element keeps it"
    );
    f.show.set(false);
    ev(&mut f.app, PlatformEvent::AboutToWait);
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    assert_eq!(
        request_pointer_lock_on(&pad),
        Err(PointerLockError::NotConnected)
    );
    set_pointer_lock_host(None);
}

/// What the host granted is what the app reads: a confinement (X11) is not
/// reported as a lock, and a refusal holds nothing.
#[test]
fn the_granted_mode_is_reported() {
    let mut f = mount();
    let log = install(Ok(Some(PointerLockMode::Confined)));
    f.surface.request_pointer_lock().unwrap();
    assert_eq!(pointer_lock(), Some(PointerLockMode::Confined));
    // A confined lock is a lock: motion, Escape and release all apply.
    motion(&mut f.app, 4.0, -4.0);
    assert_eq!(*f.surface_motion.borrow(), [(4.0, -4.0)]);
    escape(&mut f.app);
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);

    let refusing = install(Err(PointerLockError::Refused("no grab".into())));
    assert_eq!(
        f.surface.request_pointer_lock(),
        Err(PointerLockError::Refused("no grab".into()))
    );
    assert_eq!(pointer_lock(), None);
    assert!(!f.surface.has_pointer_lock());
    escape(&mut f.app);
    assert_eq!(unlocks(&refusing), 0, "nothing was held to release");
    set_pointer_lock_host(None);
}

/// A handler that writes a signal the document shows gets its frame.
#[test]
fn motion_that_changes_the_document_asks_for_a_frame() {
    let mut f = mount();
    let turned = Signal::new(0.0_f64);
    let label = f.pad.borrow().clone().unwrap();
    let _effect = rinch_core::Effect::new(move || label.set_text(&turned.get().to_string()));
    let _sub = on_pointer_motion(move |m| turned.update(|t| *t += m.dx));
    ev(&mut f.app, PlatformEvent::AboutToWait);
    let actions = ev(
        &mut f.app,
        PlatformEvent::PointerMotion { dx: 3.0, dy: 0.0 },
    );
    assert_eq!(turned.get(), 3.0);
    assert!(actions.contains(&AppAction::RequestRedraw));
}
