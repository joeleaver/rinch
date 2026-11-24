//! Popups through the runtime without an event loop (#2552): a bar (a second
//! window) with a menu and a submenu made as the shell mounts windows, then
//! closed, pressed, keyed and dismissed through the runtime's own paths.

use super::*;
use crate::popup::{PopupAnchor, PopupProps, open_popup};
use rinch_core::element::{Dismissal, Placement, PopupKind};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<(&'static str, Dismissal)>>>;

thread_local! {
    static ROW: RefCell<Option<NodeHandle>> = const { RefCell::new(None) };
}

/// A 200x40 box whose 20 px first row is kept to anchor a popup to.
fn with_row(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("style", "width: 200px; height: 40px");
    let row = scope.create_element("div");
    row.set_attribute("style", "height: 20px");
    root.append_child(&row);
    ROW.with(|r| *r.borrow_mut() = Some(row));
    root
}

fn row() -> NodeHandle {
    ROW.with(|r| r.borrow().clone()).expect("a row is mounted")
}

/// A popup on the last row mounted, made as the shell mounts it (no OS
/// window), owned by nothing.
fn popup(runtime: &mut RinchRuntime, name: &'static str, log: &Log) -> (WindowHandle, u64) {
    let log = log.clone();
    let props = PopupProps {
        anchor: PopupAnchor::Node(row()),
        placement: Placement::BOTTOM_START,
        on_dismissed: Some(Rc::new(move |why| log.borrow_mut().push((name, why)))),
        ..PopupProps::default()
    };
    let handle = rinch_core::reactive::unowned(|| open_popup(props, with_row))
        .expect("popups are supported here");
    let id = make(runtime, handle);
    (handle, id)
}

/// A bar with a menu and a submenu, the bar pressed first.
fn bar_menu_sub() -> (RinchRuntime, u64, u64, u64, Log) {
    crate::popup::set_supported(true);
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, bar) = open(&mut runtime, with_row);
    crate::window_handle::note_press(doc_of(&runtime, bar));
    let log: Log = Rc::default();
    let (_, menu) = popup(&mut runtime, "menu", &log);
    let (_, sub) = popup(&mut runtime, "sub", &log);
    (runtime, bar, menu, sub, log)
}

#[test]
fn closing_the_bar_closes_its_chain_topmost_first_told_parent_closed() {
    let (mut runtime, bar, menu, sub, log) = bar_menu_sub();
    // The compositor closes the bar's layer surface (its output went): the
    // close it agrees to with no veto.
    runtime.close_soon(WindowRole::Other(bar));
    assert!(!runtime.settle_closes_now());
    assert!(runtime.others.is_empty());
    assert_eq!(
        *log.borrow(),
        vec![
            ("sub", Dismissal::ParentClosed),
            ("menu", Dismissal::ParentClosed)
        ]
    );
    for id in [bar, menu, sub] {
        assert!(!WindowHandle::from_id(id).is_open());
    }
    crate::popup::set_supported(false);
}

#[test]
fn keys_reaching_the_bar_go_to_the_topmost_menu_and_its_blur_is_ignored() {
    let (mut runtime, bar, menu, sub, _log) = bar_menu_sub();
    let ime = WindowEvent::Ime(winit::event::Ime::Enabled);
    for from in [bar, menu, sub] {
        assert_eq!(
            runtime.input_role(WindowRole::Other(from), &ime),
            WindowRole::Other(sub),
            "from {from}"
        );
    }
    // Modifiers too: the topmost menu reads them for its chords and clicks.
    let mods = WindowEvent::ModifiersChanged(winit::keyboard::ModifiersState::SHIFT.into());
    assert_eq!(
        runtime.input_role(WindowRole::Other(bar), &mods),
        WindowRole::Other(sub)
    );
    // The main window is in no chain: its keys stay.
    assert_eq!(runtime.input_role(WindowRole::Main, &ime), WindowRole::Main);
    // A pointer event is never moved.
    let left = WindowEvent::PointerLeft {
        device_id: None,
        primary: true,
        position: None,
        kind: winit::event::PointerKind::Mouse,
    };
    assert_eq!(
        runtime.input_role(WindowRole::Other(bar), &left),
        WindowRole::Other(bar)
    );
    assert!(runtime.in_menu_chain(WindowRole::Other(bar)));
    assert!(!runtime.in_menu_chain(WindowRole::Main));
    crate::popup::set_supported(false);
}

#[test]
fn a_press_in_the_bar_closes_the_chain_after_it_is_delivered() {
    let (mut runtime, bar, menu, sub, log) = bar_menu_sub();
    // On the bar's row, the menu's anchor.
    runtime.w(WindowRole::Other(bar)).app.cursor_pos = Some((5.0, 5.0));
    runtime.light_dismiss_press(WindowRole::Other(bar));
    // Not yet: closed at the end of the handler.
    assert!(WindowHandle::from_id(menu).is_open());
    runtime.settle_closes_now();
    assert!(!WindowHandle::from_id(menu).is_open() && !WindowHandle::from_id(sub).is_open());
    let told = log.borrow().clone();
    assert!(told.contains(&("menu", Dismissal::PressOutside { on_anchor: true })));
    assert!(told.contains(&("sub", Dismissal::PressOutside { on_anchor: false })));
    crate::popup::set_supported(false);
}

#[test]
fn popup_done_closes_with_no_veto_told_system() {
    let (mut runtime, _bar, menu, sub, log) = bar_menu_sub();
    assert!(runtime.popup_done(WindowRole::Other(menu)));
    runtime.settle_closes_now();
    assert_eq!(
        *log.borrow(),
        vec![
            ("sub", Dismissal::ParentClosed),
            ("menu", Dismissal::System)
        ]
    );
    assert!(!WindowHandle::from_id(sub).is_open());
    assert!(
        !runtime.popup_done(WindowRole::Main),
        "the main window is no popup"
    );
    crate::popup::set_supported(false);
}

#[test]
fn a_new_menu_replaces_the_open_chain_and_waits_a_turn() {
    let (mut runtime, bar, menu, sub, log) = bar_menu_sub();
    // Another menu, on the bar itself (not on the topmost menu): the next
    // tray item's.
    let props = PopupProps {
        kind: PopupKind::Menu,
        anchor: PopupAnchor::Rect {
            window: Some(WindowHandle::from_id(bar)),
            x: 50.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        },
        ..PopupProps::default()
    };
    let other = rinch_core::reactive::unowned(|| open_popup(props, with_row)).unwrap();
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let event_loop = RecordingLoop::default();
    runtime.open_pending_popup(other.id(), &event_loop);
    assert_eq!(
        *log.borrow(),
        vec![
            ("sub", Dismissal::ParentClosed),
            ("menu", Dismissal::Replaced)
        ]
    );
    assert!(!WindowHandle::from_id(menu).is_open() && !WindowHandle::from_id(sub).is_open());
    // Not made in this iteration: asked for again at its end.
    assert!(other.is_open());
    assert!(!runtime.others.iter().any(|w| w.handle == other.id()));
    runtime.popups_end_of_turn(&event_loop);
    let queued = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    assert!(
        queued.iter().any(
            |(to, e)| matches!(to, EventFor::Window(w) if *w == other.id())
                && matches!(e, RinchNativeEvent::OpenWindow(id) if *id == other.id())
        ),
        "{queued:?}"
    );
    crate::popup::set_supported(false);
}

#[test]
fn a_menu_closed_while_it_waits_for_the_keyboard_puts_the_bar_back() {
    crate::popup::set_supported(true);
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, bar) = open(&mut runtime, with_row);
    crate::window_handle::note_press(doc_of(&runtime, bar));
    let props = PopupProps {
        anchor: PopupAnchor::Node(row()),
        keyboard_via_parent: true,
        ..PopupProps::default()
    };
    let menu = rinch_core::reactive::unowned(|| open_popup(props, with_row)).unwrap();
    // Waiting for the bar's keyboard, the bar raised (as
    // `raise_parent_keyboard` leaves them).
    let pending = crate::window_handle::take_pending(menu.id()).unwrap();
    runtime.popups.awaiting_keyboard.push((bar, pending));
    runtime
        .popups
        .raised
        .push((bar, rinch_core::element::LayerKeyboard::None));
    // A second press on the tray icon closes it before the keyboard came.
    runtime.close_other(menu.id());
    assert!(
        runtime.popups.awaiting_keyboard.is_empty(),
        "it waits no more"
    );
    assert!(runtime.popups.raised.is_empty(), "and the bar is put back");
    assert!(!menu.is_open());
    crate::popup::set_supported(false);
}

#[test]
fn a_blur_held_while_the_chain_is_open_is_told_when_it_closes() {
    let (mut runtime, bar, menu, _sub, _log) = bar_menu_sub();
    let event_loop = RecordingLoop::default();
    runtime.w(WindowRole::Other(bar)).app.window_focused = true;
    runtime.window_event_for(
        WindowRole::Other(bar),
        &event_loop,
        WindowEvent::Focused(false),
    );
    assert!(
        runtime.w(WindowRole::Other(bar)).app.window_focused,
        "held while the menu is open"
    );
    runtime.close_other(menu);
    runtime.popups_end_of_turn(&event_loop);
    assert!(
        !runtime.w(WindowRole::Other(bar)).app.window_focused,
        "told once the chain closed"
    );
    crate::popup::set_supported(false);
}

/// A renderer on the default adapter, standing in for a window's.
#[cfg(feature = "gpu-renderer")]
fn offscreen_renderer() -> WgpuRenderer {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    WgpuRenderer::offscreen(
        std::sync::Arc::new(device),
        std::sync::Arc::new(queue),
        64,
        64,
        wgpu::TextureFormat::Rgba8Unorm,
    )
}

/// #2552 stage 2: a closed popup's GPU renderer is a next popup's, which
/// then builds none (Vello's renderer was most of a GPU popup's opening).
/// Only a popup's is kept, and only one that holds nothing of its document
/// beyond its next full frame.
#[cfg(feature = "gpu-renderer")]
#[test]
#[ignore]
fn gpu_a_closed_popups_renderer_is_kept_for_the_next_2552() {
    use super::super::popups::{drop_spare_renderers, take_spare_renderer};
    drop_spare_renderers();
    let (mut runtime, bar, menu, sub, _log) = bar_menu_sub();
    runtime.w(WindowRole::Other(sub)).renderer = Some(offscreen_renderer());
    let mut layered = offscreen_renderer();
    rinch_platform::PlatformRenderer::set_composite_layers(
        &mut layered,
        vec![rinch_platform::CompositeLayer {
            pixels: vec![0; 4],
            width: 1,
            height: 1,
            viewport: (0.0, 0.0, 1.0, 1.0),
            border_radius: [0.0; 4],
            clip_rect: None,
        }],
    );
    runtime.w(WindowRole::Other(menu)).renderer = Some(layered);

    runtime.close_other(sub);
    let kept = take_spare_renderer().expect("the submenu's renderer");
    assert!(kept.surface.is_none(), "kept with its surface gone");
    // One still holding its document's layers is not kept.
    runtime.close_other(menu);
    assert!(take_spare_renderer().is_none());
    // Nor a window's that is not a popup.
    runtime.w(WindowRole::Other(bar)).renderer = Some(kept);
    runtime.close_other(bar);
    assert!(take_spare_renderer().is_none());
    crate::popup::set_supported(false);
}

/// The kept renderers (#2552): the last `count` kept, the last given first,
/// none with a count of 0, and all of them once the idle period of the last
/// kept has passed.
#[test]
fn the_spare_pool_keeps_the_last_few_until_idle_2552() {
    use std::time::{Duration, Instant};
    let t0 = Instant::now();
    let idle = Some(Duration::from_secs(2));
    let mut pool = super::super::popups::SparePool::new();
    for i in 0..4 {
        pool.keep(i, 3, t0, idle);
    }
    let given: Vec<i32> = std::iter::from_fn(|| pool.take()).collect();
    assert_eq!(given, vec![3, 2, 1]);

    pool.keep(7, 0, t0, idle);
    assert_eq!(pool.take(), None, "a count of 0 keeps none");

    pool.keep(1, 3, t0, idle);
    pool.keep(2, 3, t0 + Duration::from_secs(1), idle);
    assert_eq!(
        pool.trim_if_idle(t0 + Duration::from_millis(2_500)),
        0,
        "kept 1.5 s ago"
    );
    assert_eq!(pool.trim_if_idle(t0 + Duration::from_secs(3)), 2);
    assert_eq!(pool.take(), None);

    pool.keep(1, 3, t0, None);
    assert_eq!(
        pool.trim_if_idle(t0 + Duration::from_secs(3600)),
        0,
        "no idle period"
    );
    assert_eq!(pool.take(), Some(1));
}
