//! A surface whose element the app builds itself (#2219):
//! `RenderSurfaceHandle::bind_element` gives the surface that element, so
//! its own pointer lock works, is the surface's (motion, `has_pointer_lock`),
//! and ends with the binding — on unbind, re-bind, the element leaving the
//! document, the binding's component unmounting, another surface taking the
//! element, and the handle's last clone dropping. Through the app's event
//! path, with the recording host of `pointer_lock::test_host`.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use crate::pointer_lock::test_host::{Log, install};
use crate::render_surface::{
    BindElementError, BindElementOptions, IfBoundElsewhere, LockOnRebind, RenderSurfaceHandle,
    SurfaceEvent, create_render_surface, create_render_surface_with_name,
};
use crate::window::{PointerLockError, PointerLockMode, pointer_lock, set_pointer_lock_host};
use rinch_core::Component;
use rinch_core::reactive::Signal;
use rinch_macros::rsx;

const VP: (u32, u32) = (800, 600);

type Slot = Rc<RefCell<Option<NodeHandle>>>;
type Heard = Rc<RefCell<Vec<(f32, f32)>>>;

/// How the branch binds `surface` while it renders, if it does.
#[derive(Clone, Copy)]
enum BindInRender {
    No,
    Default,
    Unscoped,
}

struct Fixture {
    app: RinchApp,
    /// Two elements the app built (`a` carries the surface's `data-viewport`).
    a: Slot,
    b: Slot,
    /// Unmounts the branch holding them.
    show: Signal<bool>,
}

/// `div { if show { a, b, [RenderSurface] } }`, every element built by the
/// app; `bind` says whether the branch binds `surface` to `a` as it renders.
fn mount(surface: &RenderSurfaceHandle, bind: BindInRender, with_component: bool) -> Fixture {
    set_pointer_lock_host(None);
    let show = Signal::new(true);
    let (a, b): (Slot, Slot) = (Rc::default(), Rc::default());
    let (sf, sa, sb) = (surface.clone(), a.clone(), b.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let (sf, sa, sb) = (sf.clone(), sa.clone(), sb.clone());
        rsx! {
            div {
                if show.get() {
                    {branch(__scope, &sf, &sa, &sb, bind, with_component)}
                }
            }
        }
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    Fixture { app, a, b, show }
}

fn branch(
    __scope: &mut RenderScope,
    surface: &RenderSurfaceHandle,
    a: &Slot,
    b: &Slot,
    bind: BindInRender,
    with_component: bool,
) -> NodeHandle {
    let root = rsx! { div { style: "width: 400px; height: 300px" } };
    let ea = rsx! { div { style: "width: 200px; height: 100px" } };
    ea.set_attribute("data-viewport", surface.viewport_name());
    let eb = rsx! { div { style: "width: 40px; height: 30px" } };
    root.append_child(&ea);
    root.append_child(&eb);
    if with_component {
        let canvas = crate::render_surface::RenderSurface {
            surface: Some(surface.clone()),
        }
        .render(__scope, &[]);
        root.append_child(&canvas);
    }
    match bind {
        BindInRender::No => {}
        BindInRender::Default => surface.bind_element(&ea),
        BindInRender::Unscoped => surface
            .bind_element_with(&ea, BindElementOptions::new().scoped(false))
            .unwrap(),
    }
    *a.borrow_mut() = Some(ea);
    *b.borrow_mut() = Some(eb);
    root
}

fn el(slot: &Slot) -> NodeHandle {
    slot.borrow().clone().unwrap()
}

fn same(a: &Option<NodeHandle>, b: &NodeHandle) -> bool {
    a.as_ref().is_some_and(|a| a.node_id() == b.node_id())
}

fn hear(surface: &RenderSurfaceHandle) -> Heard {
    let heard: Heard = Rc::default();
    let sink = heard.clone();
    surface.set_event_handler(move |e| {
        if let SurfaceEvent::PointerMotion { dx, dy } = e {
            sink.borrow_mut().push((dx, dy));
        }
    });
    heard
}

fn ev(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, VP, 1.0);
}

fn motion(app: &mut RinchApp, dx: f64, dy: f64) {
    ev(app, PlatformEvent::PointerMotion { dx, dy });
}

fn unlocks(log: &Rc<RefCell<Log>>) -> usize {
    log.borrow().unlocks
}

/// The issue's case: a named surface whose `data-viewport` element the app
/// built. Unbound it cannot lock; bound, its own request locks that element,
/// and the lock is the surface's.
#[test]
fn a_named_surface_locks_the_element_the_app_bound() {
    let surface = create_render_surface_with_name("bind-2219-named");
    let mut f = mount(&surface, BindInRender::No, false);
    let heard = hear(&surface);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    assert_eq!(
        surface.request_pointer_lock(),
        Err(PointerLockError::NotConnected),
        "the issue: without a binding the surface has no element"
    );
    assert_eq!(surface.element().map(|e| e.node_id()), None);

    surface.bind_element(&el(&f.a));
    assert!(same(&surface.element(), &el(&f.a)));
    assert_eq!(surface.request_pointer_lock(), Ok(()));
    assert_eq!(log.borrow().locks, ["element"]);
    assert!(surface.has_pointer_lock());
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    motion(&mut f.app, 3.0, -2.0);
    assert_eq!(
        *heard.borrow(),
        [(3.0, -2.0)],
        "the surface hears its motion"
    );
    ev(&mut f.app, PlatformEvent::AboutToWait);
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked), "still held");
    set_pointer_lock_host(None);
    unregister_surface(&surface);
}

/// The bound element leaving the document ends the lock on the next event,
/// as an element's own lock does; the binding stays, so the surface still
/// names the element, which can no longer be locked.
#[test]
fn the_bound_element_leaving_the_document_releases_the_lock() {
    let surface = create_render_surface_with_name("bind-2219-leaves");
    let mut f = mount(&surface, BindInRender::No, false);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let a = el(&f.a);
    surface.bind_element(&a);
    surface.request_pointer_lock().unwrap();
    f.show.set(false);
    ev(&mut f.app, PlatformEvent::AboutToWait);
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    assert!(!surface.has_pointer_lock());
    assert!(same(&surface.element(), &a), "the binding stays");
    assert_eq!(
        surface.request_pointer_lock(),
        Err(PointerLockError::NotConnected)
    );
    set_pointer_lock_host(None);
    unregister_surface(&surface);
}

/// Bound while its component renders, the binding is the component's: the
/// unmount undoes it and ends the lock at once, with no event in between.
#[test]
fn a_binding_made_while_rendering_goes_with_its_component() {
    let surface = create_render_surface_with_name("bind-2219-scoped");
    let f = mount(&surface, BindInRender::Default, false);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    assert!(same(&surface.element(), &el(&f.a)));
    surface.request_pointer_lock().unwrap();
    f.show.set(false);
    assert_eq!(pointer_lock(), None, "released by the unmount itself");
    assert_eq!(unlocks(&log), 1);
    assert!(surface.element().is_none(), "unbound");
    set_pointer_lock_host(None);
    unregister_surface(&surface);
}

/// `scoped(false)`: the same binding outlives its component (and its lock
/// ends only when the element is found gone, on the next event).
#[test]
fn an_unscoped_binding_outlives_its_component() {
    let surface = create_render_surface_with_name("bind-2219-unscoped");
    let mut f = mount(&surface, BindInRender::Unscoped, false);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let a = el(&f.a);
    surface.request_pointer_lock().unwrap();
    f.show.set(false);
    assert_eq!(
        pointer_lock(),
        Some(PointerLockMode::Locked),
        "no unbind at the unmount"
    );
    assert!(same(&surface.element(), &a));
    ev(&mut f.app, PlatformEvent::AboutToWait);
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    set_pointer_lock_host(None);
    unregister_surface(&surface);
}

/// Re-binding while locked releases the lock, as an unmount does;
/// `LockOnRebind::Move` asks for it again on the new element instead,
/// without releasing it first, and the motion keeps coming.
#[test]
fn rebinding_while_locked_releases_or_moves_the_lock() {
    let surface = create_render_surface_with_name("bind-2219-rebind");
    let mut f = mount(&surface, BindInRender::No, false);
    let heard = hear(&surface);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let (a, b) = (el(&f.a), el(&f.b));

    surface.bind_element(&a);
    surface.request_pointer_lock().unwrap();
    surface.bind_element(&a);
    assert_eq!(
        pointer_lock(),
        Some(PointerLockMode::Locked),
        "positive control: the same element again keeps it"
    );
    surface.bind_element(&b);
    assert_eq!(pointer_lock(), None, "re-bound: released");
    assert_eq!(unlocks(&log), 1);
    assert!(same(&surface.element(), &b));

    surface.bind_element(&a);
    surface.request_pointer_lock().unwrap();
    surface
        .bind_element_with(
            &b,
            BindElementOptions::new().lock_on_rebind(LockOnRebind::Move),
        )
        .unwrap();
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked), "moved");
    assert_eq!(unlocks(&log), 1, "not released on the way");
    assert_eq!(log.borrow().locks, ["element", "element", "element"]);
    assert!(surface.has_pointer_lock());
    motion(&mut f.app, 1.5, 2.5);
    assert_eq!(*heard.borrow(), [(1.5, 2.5)]);
    // Held by `b` now: unbinding `b` ends it.
    surface.unbind_element();
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 2);
    set_pointer_lock_host(None);
    unregister_surface(&surface);
}

/// A move to an element that cannot be locked ends the old lock.
#[test]
fn a_move_to_a_removed_element_releases() {
    let surface = create_render_surface_with_name("bind-2219-move-gone");
    let f = mount(&surface, BindInRender::No, false);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let stray = el(&f.b);
    stray.remove();
    surface.bind_element(&el(&f.a));
    surface.request_pointer_lock().unwrap();
    surface
        .bind_element_with(
            &stray,
            BindElementOptions::new().lock_on_rebind(LockOnRebind::Move),
        )
        .unwrap();
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    set_pointer_lock_host(None);
    unregister_surface(&surface);
}

/// An element bound to another surface is taken from it by default (its
/// lock on the element ends, it has no element any more); `Refuse` changes
/// nothing; `Take` with `Move` hands the lock over.
#[test]
fn an_element_bound_elsewhere_is_taken_or_refused() {
    let first = create_render_surface();
    let second = create_render_surface_with_name("bind-2219-elsewhere");
    let f = mount(&second, BindInRender::No, false);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let (a, b) = (el(&f.a), el(&f.b));

    first.bind_element(&a);
    first.request_pointer_lock().unwrap();
    assert_eq!(
        second.bind_element_with(
            &a,
            BindElementOptions::new().if_bound_elsewhere(IfBoundElsewhere::Refuse)
        ),
        Err(BindElementError::BoundElsewhere {
            surface: first.id()
        })
    );
    assert!(first.has_pointer_lock(), "refused: nothing changed");
    assert!(second.element().is_none());

    second.bind_element(&a);
    assert_eq!(pointer_lock(), None, "taken: the old holder's lock ends");
    assert_eq!(unlocks(&log), 1);
    assert!(first.element().is_none());
    assert_eq!(
        first.request_pointer_lock(),
        Err(PointerLockError::NotConnected)
    );
    assert!(same(&second.element(), &a));
    assert_eq!(second.request_pointer_lock(), Ok(()));
    assert!(second.has_pointer_lock());

    first.bind_element(&b);
    first.request_pointer_lock().unwrap();
    assert!(!second.has_pointer_lock(), "first's request moved the lock");
    second
        .bind_element_with(
            &b,
            BindElementOptions::new().lock_on_rebind(LockOnRebind::Move),
        )
        .unwrap();
    assert!(second.has_pointer_lock(), "handed over");
    assert!(!first.has_pointer_lock());
    assert_eq!(pointer_lock(), Some(PointerLockMode::Locked));
    set_pointer_lock_host(None);
    unregister_surface(&second);
}

/// A binding wins over the component's element while both exist; unbinding
/// ends the bound element's lock and goes back to the component's.
#[test]
fn unbinding_releases_and_falls_back_to_the_components_element() {
    let surface = create_render_surface();
    let f = mount(&surface, BindInRender::No, true);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    let a = el(&f.a);
    let own = surface.element().expect("the component's element");
    assert!(!same(&Some(own.clone()), &a));

    surface.bind_element(&a);
    assert!(same(&surface.element(), &a));
    surface.request_pointer_lock().unwrap();
    surface.unbind_element();
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    assert!(same(&surface.element(), &own));
    surface.request_pointer_lock().unwrap();
    surface.unbind_element();
    assert_eq!(
        pointer_lock(),
        Some(PointerLockMode::Locked),
        "nothing bound: the component's lock is not the binding's"
    );
    set_pointer_lock_host(None);
}

/// A handle the component never mounted, bound to an element: its lock's
/// motion reaches its handler, and the lock ends when the last clone of the
/// handle is dropped (not before).
#[test]
fn dropping_the_last_handle_releases_its_lock() {
    // The app's render closure keeps the handle it mounts: bind another.
    let mounted = create_render_surface();
    let mut f = mount(&mounted, BindInRender::No, false);
    let surface = create_render_surface();
    let heard = hear(&surface);
    let log = install(Ok(Some(PointerLockMode::Locked)));
    surface.bind_element(&el(&f.a));
    surface.request_pointer_lock().unwrap();
    motion(&mut f.app, 2.0, 0.5);
    assert_eq!(
        *heard.borrow(),
        [(2.0, 0.5)],
        "an unregistered surface hears it"
    );

    let clone = surface.clone();
    drop(surface);
    assert_eq!(
        pointer_lock(),
        Some(PointerLockMode::Locked),
        "a clone is left"
    );
    drop(clone);
    assert_eq!(pointer_lock(), None);
    assert_eq!(unlocks(&log), 1);
    set_pointer_lock_host(None);
}

/// Named surfaces register themselves for the thread's life; let each
/// test's go.
fn unregister_surface(surface: &RenderSurfaceHandle) {
    crate::render_surface::unregister_render_surface(surface.id());
}
