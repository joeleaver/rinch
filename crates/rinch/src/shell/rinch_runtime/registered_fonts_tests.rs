//! `rinch::font::register_font` through the runtime (#3042): the face sent
//! through the real native-event drain, every open window given it, one
//! event for a run of registrations, identical faces registered once, and
//! what happens with no runtime running yet or in an embed host.

use super::layer_drain_2829_tests::NoOsLoop;
use super::registered_fonts::{queue_font_for, take_new_fonts};
use super::*;
use crate::font::AppFont;
use parley::fontique::GenericFamily;
use rinch_core::element::WindowProps;

const FACE: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

/// The generics of each face `app` registered, in order.
fn registered(app: &RinchApp) -> Vec<GenericFamily> {
    app.pending_fonts
        .iter()
        .map(|f| f.font.generics[0])
        .collect()
}

/// How many native events are queued for the runtime: each test empties
/// the queue first, so these are what `register_font` sent.
fn register_events_queued() -> usize {
    __native_events_queued()
}

/// Make this test's thread one `register_font` accepts: another test in the
/// binary may have registered its own thread as the main one (a
/// `RinchContext` does), and a `TestApp` marks its thread the same way.
fn on_the_ui_thread() {
    rinch_core::mark_host_thread();
}

/// A runtime with its main window mounted and a second window open in
/// `others`, made without an OS window; nothing queued, nothing registered.
fn runtime_with_two_windows() -> RinchRuntime {
    on_the_ui_thread();
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    runtime.main.app.mount_component(800.0, 600.0);
    let handle = crate::open_window(
        WindowProps {
            visible: false,
            ..Default::default()
        },
        component,
    );
    let pending = crate::window_handle::take_pending(handle.id()).expect("waiting to be made");
    let window = RinchRuntime::mount_other_window(pending, &runtime.fonts, 1.0);
    runtime.others.push(window);
    // The open request: made by hand above, so not the drain's.
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let _ = take_new_fonts(&mut Vec::new());
    runtime
}

/// The face goes through the path a running app's does: `register_font`
/// queues it and sends `RegisterFonts`, the loop's wake drains the queue
/// (`drain_native_events`), and the event's handler gives the face to every
/// open window, the second one in `others` as well as the main one, and
/// keeps it for windows opened later.
#[test]
fn the_drained_event_gives_the_face_to_every_open_window() {
    let mut runtime = runtime_with_two_windows();
    assert_eq!(runtime.others.len(), 1);

    crate::font::register_font(AppFont::monospace(FACE));
    assert_eq!(register_events_queued(), 1, "the loop is woken once");
    runtime.drain_native_events(&NoOsLoop);

    let mono = vec![GenericFamily::Monospace];
    assert_eq!(registered(&runtime.main.app), mono, "the main window");
    assert_eq!(
        registered(&runtime.others[0].app),
        mono,
        "the second window"
    );
    assert_eq!(runtime.fonts.len(), 1, "kept for windows opened later");
}

/// A run of registrations before the loop's turn sends one event, and that
/// turn registers all of them, in order. The next registration after the
/// turn wakes the loop again.
#[test]
fn a_run_of_registrations_sends_one_event() {
    let mut runtime = runtime_with_two_windows();

    crate::font::register_font(AppFont::serif(FACE));
    crate::font::register_font(AppFont::sans_serif(FACE));
    crate::font::register_font(AppFont::monospace(FACE));
    assert_eq!(register_events_queued(), 1, "one wake for the run");
    runtime.drain_native_events(&NoOsLoop);

    let all = vec![
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
    ];
    assert_eq!(registered(&runtime.main.app), all);
    assert_eq!(registered(&runtime.others[0].app), all);

    crate::font::register_font(AppFont::new(FACE).with_generics(&[GenericFamily::Cursive]));
    assert_eq!(register_events_queued(), 1, "a later one wakes it again");
}

/// Registering a face identical to one registered already (same bytes,
/// generics and fallback choice), before the turn or after it, changes
/// nothing: neither the windows' fonts nor the runtime's grow. The same
/// bytes claiming another generic are another registration.
#[test]
fn an_identical_face_is_registered_once() {
    let mut runtime = runtime_with_two_windows();

    crate::font::register_font(AppFont::monospace(FACE));
    crate::font::register_font(AppFont::monospace(FACE));
    runtime.drain_native_events(&NoOsLoop);
    crate::font::register_font(AppFont::monospace(FACE));
    // Owned bytes equal to the static ones are the same face too.
    crate::font::register_font(AppFont::from_data(FACE.to_vec()).with_generics(AppFont::MONOSPACE));
    runtime.drain_native_events(&NoOsLoop);

    let mono = vec![GenericFamily::Monospace];
    assert_eq!(registered(&runtime.main.app), mono);
    assert_eq!(registered(&runtime.others[0].app), mono);
    assert_eq!(runtime.fonts.len(), 1);

    crate::font::register_font(AppFont::serif(FACE));
    runtime.drain_native_events(&NoOsLoop);
    assert_eq!(
        registered(&runtime.main.app),
        vec![GenericFamily::Monospace, GenericFamily::Serif]
    );
    assert_eq!(runtime.fonts.len(), 2);
}

/// A face the build already bundles (`App::fonts`) is not registered again
/// when `register_font` queues it before the runtime is made.
#[test]
fn a_queued_face_the_build_bundles_is_not_registered_twice() {
    let _lock = TestQueueLock::hold();
    let _ = take_new_fonts(&mut Vec::new());
    queue_font_for(wake::Drainer::Coming, false, AppFont::monospace(FACE));
    let runtime = RinchRuntime::new("t", 800, 600, component, &[AppFont::monospace(FACE)]);
    assert_eq!(
        registered(&runtime.main.app),
        vec![GenericFamily::Monospace]
    );
    assert_eq!(runtime.fonts.len(), 1);
}

/// Before a runtime is made (an app's `main`), a face is queued for
/// `RinchRuntime::with_app` and no event is sent: calls in `main` keep none
/// of the native queue's `KEPT_BEFORE_RUN` places.
#[test]
fn before_the_runtime_a_face_waits_without_an_event() {
    let _lock = TestQueueLock::hold();
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let _ = take_new_fonts(&mut Vec::new());

    queue_font_for(wake::Drainer::Coming, false, AppFont::serif(FACE));
    queue_font_for(wake::Drainer::Coming, false, AppFont::monospace(FACE));
    assert_eq!(register_events_queued(), 0, "nothing to wake");

    let runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    assert_eq!(
        registered(&runtime.main.app),
        vec![GenericFamily::Serif, GenericFamily::Monospace],
        "the runtime takes them at construction"
    );
}

/// In an embed host nothing will ever take a queued face, so none is kept
/// and no event is sent; the host registers with
/// `RinchContext::register_app_font`.
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
#[test]
fn an_embed_host_keeps_nothing() {
    let _lock = TestQueueLock::hold();
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let _ = take_new_fonts(&mut Vec::new());

    queue_font_for(wake::Drainer::EmbedHost, false, AppFont::serif(FACE));
    assert_eq!(register_events_queued(), 0);
    assert!(take_new_fonts(&mut Vec::new()).is_empty(), "nothing kept");
}

/// A process that turns out to be an embed host (a `RinchContext` made
/// with no desktop runtime) drops the faces queued for the runtime that
/// will not come, as it drops the window requests kept for it. Before the
/// fix they stayed in the thread's queue for the life of the process.
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
#[test]
fn becoming_an_embed_host_drops_the_queued_faces() {
    let _ = take_new_fonts(&mut Vec::new());
    // Its own queue: the transition is under test, not the process's queue.
    let queue = Mutex::new(wake::NativeEventQueue::with_drainer(wake::Drainer::Coming));
    queue_font_for(wake::Drainer::Coming, false, AppFont::serif(FACE));
    wake::note_embed_host_in(&queue);
    assert_eq!(queue.lock().unwrap().drainer(), wake::Drainer::EmbedHost);
    assert!(take_new_fonts(&mut Vec::new()).is_empty(), "nothing kept");

    // A context made while a runtime runs changes nothing: its faces are
    // the runtime's.
    let running = Mutex::new(wake::NativeEventQueue::with_drainer(wake::Drainer::Runtime));
    queue_font_for(wake::Drainer::Coming, false, AppFont::serif(FACE));
    wake::note_embed_host_in(&running);
    assert_eq!(take_new_fonts(&mut Vec::new()).len(), 1, "kept");
}

/// Open a second window in `runtime`, made without an OS window, and return
/// its id; nothing left in the native queue.
fn open_second(runtime: &mut RinchRuntime) -> u64 {
    let handle = crate::open_window(
        WindowProps {
            visible: false,
            ..Default::default()
        },
        component,
    );
    let pending = crate::window_handle::take_pending(handle.id()).expect("waiting to be made");
    let window = RinchRuntime::mount_other_window(pending, &runtime.fonts, 1.0);
    let id = window.handle;
    runtime.others.push(window);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    id
}

/// A windowless app's runtime (`App::windowless`), its root mounted, with
/// `windows` second windows open; nothing queued, nothing registered.
fn windowless_with(windows: usize) -> (RinchRuntime, Vec<u64>) {
    on_the_ui_thread();
    crate::window_handle::set_windowless(true);
    crate::window_handle::set_exit_when(crate::ExitWhen::Explicitly);
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    crate::window_handle::set_windowless(false);
    crate::window_handle::set_exit_when(crate::ExitWhen::MainWindowCloses);
    runtime.mount_windowless_root();
    let ids = (0..windows).map(|_| open_second(&mut runtime)).collect();
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let _ = take_new_fonts(&mut Vec::new());
    (runtime, ids)
}

/// A windowless app has no main window for a `RegisterFonts` sent from
/// outside any document (a tray or menu callback, `Sender(None)`) to be
/// routed to. The face is the app's, so it reaches every open window. Before
/// the fix the event was "for a window that is not open", ignored, and the
/// queue it left behind sent no event for any later face.
#[test]
fn a_windowless_app_registers_a_face_from_outside_any_document() {
    let (mut runtime, _) = windowless_with(1);
    assert!(runtime.main_closed, "no main window");

    crate::font::register_font(AppFont::monospace(FACE));
    runtime.drain_native_events(&NoOsLoop);
    assert_eq!(
        registered(&runtime.others[0].app),
        vec![GenericFamily::Monospace]
    );
    assert_eq!(runtime.fonts.len(), 1, "kept for windows opened later");

    // The next face wakes the loop again and is registered too.
    crate::font::register_font(AppFont::serif(FACE));
    assert_eq!(register_events_queued(), 1, "the queue was not left stuck");
    runtime.drain_native_events(&NoOsLoop);
    assert_eq!(
        registered(&runtime.others[0].app),
        vec![GenericFamily::Monospace, GenericFamily::Serif]
    );
}

/// After the main window has closed (an app that runs on without it), a
/// face registered from outside any document reaches the windows still
/// open.
#[test]
fn a_face_registered_after_the_main_window_closed_reaches_the_rest() {
    let mut runtime = runtime_with_two_windows();
    // An app that runs on without its main window (`App::exit_when`).
    runtime.exit_when = crate::ExitWhen::Explicitly;
    assert!(!runtime.close_soon(WindowRole::Main), "the app runs on");
    assert!(!runtime.settle_closes_now());
    assert!(runtime.main_closed);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();

    crate::font::register_font(AppFont::monospace(FACE));
    runtime.drain_native_events(&NoOsLoop);
    assert_eq!(
        registered(&runtime.others[0].app),
        vec![GenericFamily::Monospace]
    );
    assert_eq!(runtime.fonts.len(), 1);
}

/// A face registered by a window's own code, which then closes before the
/// loop's turn, is still the app's: the windows left are given it, though
/// the event's sender has no window any more (in a windowless app, so there
/// is no main window to fall back to either).
#[test]
fn a_face_from_a_window_that_closed_before_the_turn_reaches_the_rest() {
    let (mut runtime, ids) = windowless_with(2);
    let sender = runtime.others[0].app.doc_key();
    {
        let _doc = rinch_core::push_dispatching_doc(sender);
        crate::font::register_font(AppFont::monospace(FACE));
    }
    runtime.close_other(ids[0]);
    runtime.drain_native_events(&NoOsLoop);

    assert_eq!(runtime.others.len(), 1);
    assert_eq!(
        registered(&runtime.others[0].app),
        vec![GenericFamily::Monospace]
    );
    assert_eq!(runtime.fonts.len(), 1);
}

/// However the event is lost, the loop's next turn takes the queue: here
/// the turn runs without it, and the face is still registered, and the next
/// face sends its event again.
#[test]
fn a_turn_takes_the_faces_whatever_became_of_the_event() {
    let mut runtime = runtime_with_two_windows();
    crate::font::register_font(AppFont::monospace(FACE));
    let _lost = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    runtime.run_native_events(
        vec![(EventFor::Sender(None), RinchNativeEvent::ReRender)],
        &NoOsLoop,
    );

    let mono = vec![GenericFamily::Monospace];
    assert_eq!(registered(&runtime.main.app), mono);
    assert_eq!(registered(&runtime.others[0].app), mono);
    crate::font::register_font(AppFont::serif(FACE));
    assert_eq!(register_events_queued(), 1, "the next face wakes the loop");
}
