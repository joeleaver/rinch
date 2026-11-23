//! #2416 stage 4: opening and closing windows, which window an event is for,
//! ownership, store scoping, and the exit rule, through the runtime without
//! an event loop (no OS window is made).

use super::*;
use crate::window_handle::WindowHandle;
use rinch_core::element::WindowProps;

#[path = "doc_forget_tests.rs"]
mod doc_forget;
#[path = "popups_tests.rs"]
mod popups;

#[path = "perf_attribution_2798_tests.rs"]
mod perf_attribution;

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

/// A runtime with its main window mounted, exiting as `when` says.
fn runtime(when: crate::ExitWhen) -> RinchRuntime {
    crate::window_handle::set_exit_when(when);
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    crate::window_handle::set_exit_when(crate::ExitWhen::MainWindowCloses);
    runtime.main.app.mount_component(800.0, 600.0);
    runtime
}

/// What `open_pending_window` does, but mounted without an OS window: the
/// window `handle` opened, made and in the runtime.
fn make(runtime: &mut RinchRuntime, handle: WindowHandle) -> u64 {
    let id = handle_id(handle);
    let pending = crate::window_handle::take_pending(id).expect("waiting to be made");
    let window = RinchRuntime::mount_other_window(pending, &runtime.fonts, 1.0);
    runtime.others.push(window);
    id
}

/// The id a handle names, read back through the runtime's own routing.
fn handle_id(handle: WindowHandle) -> u64 {
    // A handle is the queue's target: send something to it and read it back.
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    handle.minimize();
    let drained = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    match drained.as_slice() {
        [(EventFor::Window(id), RinchNativeEvent::MinimizeWindow)] => *id,
        other => panic!("one event for the handle, got {other:?}"),
    }
}

fn open(
    runtime: &mut RinchRuntime,
    content: fn(&mut RenderScope) -> NodeHandle,
) -> (WindowHandle, u64) {
    let handle =
        rinch_core::reactive::unowned(|| crate::open_window(WindowProps::default(), content));
    let id = make(runtime, handle);
    (handle, id)
}

fn doc_of(runtime: &RinchRuntime, id: u64) -> u64 {
    runtime
        .others
        .iter()
        .find(|w| w.handle == id)
        .unwrap()
        .app
        .doc_key()
}

#[test]
fn an_event_is_for_the_window_its_document_or_its_handle_names() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (handle, id) = open(&mut runtime, component);
    let doc = doc_of(&runtime, id);
    let main_doc = runtime.main.app.doc_key();
    assert_eq!(
        runtime.role_for(EventFor::Sender(Some(doc))),
        Some(WindowRole::Other(id))
    );
    assert_eq!(
        runtime.role_for(EventFor::Window(id)),
        Some(WindowRole::Other(id))
    );
    assert_eq!(
        runtime.role_for(EventFor::Sender(Some(main_doc))),
        Some(WindowRole::Main)
    );
    assert_eq!(
        runtime.role_for(EventFor::Sender(None)),
        Some(WindowRole::Main)
    );
    assert_eq!(
        runtime.role_for(EventFor::Window(runtime.main_handle)),
        Some(WindowRole::Main)
    );
    assert!(handle.is_open());

    runtime.close_other(id);
    assert_eq!(
        runtime.role_for(EventFor::Window(id)),
        None,
        "closed: for nothing"
    );
    assert!(!handle.is_open());
    assert_eq!(crate::windows(), vec![WindowHandle::main().unwrap()]);
}

#[test]
fn current_window_functions_in_a_second_window_act_on_it() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (handle, id) = open(&mut runtime, component);
    let doc = doc_of(&runtime, id);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    {
        let _doc = rinch_core::push_dispatching_doc(doc);
        crate::windows::minimize_current_window();
        assert_eq!(WindowHandle::current(), Some(handle));
    }
    let drained = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let (to, _) = drained[0];
    assert_eq!(runtime.role_for(to), Some(WindowRole::Other(id)));
    assert_eq!(
        WindowHandle::current(),
        WindowHandle::main(),
        "outside any document"
    );
}

#[test]
fn a_title_is_the_window_s_whose_code_set_it() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, id) = open(&mut runtime, component);
    let doc = doc_of(&runtime, id);
    let main_doc = runtime.main.app.doc_key();
    let other = runtime.title_owner(WindowRole::Other(id));
    let main = runtime.title_owner(WindowRole::Main);
    assert!(other(Some(doc)) && !other(Some(main_doc)) && !other(None));
    assert!(main(Some(main_doc)) && main(None) && !main(Some(doc)));
}

#[test]
fn a_window_closes_when_its_owner_goes_and_not_when_unowned() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let owner = rinch_core::reactive::Scope::new();
    let owned = owner.run(|| crate::open_window(WindowProps::default(), component));
    let owned_id = make(&mut runtime, owned);
    let (free, _) = open(&mut runtime, component);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();

    owner.dispose();
    let drained = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    assert_eq!(
        drained
            .iter()
            .filter(|(_, e)| matches!(e, RinchNativeEvent::CloseWindow))
            .map(|(to, _)| *to)
            .collect::<Vec<_>>(),
        vec![EventFor::Window(owned_id)],
        "the owned window, only"
    );
    assert!(free.is_open());
}

#[test]
fn a_window_opened_in_a_handler_belongs_to_the_scope_that_built_it() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let row = rinch_core::reactive::Scope::new();
    let opened = std::rc::Rc::new(std::cell::Cell::new(None));
    let handler = {
        let opened = opened.clone();
        row.run(|| {
            rinch_core::events::register_handler(std::rc::Rc::new(move || {
                opened.set(Some(crate::open_window(WindowProps::default(), component)));
            }))
        })
    };
    rinch_core::events::dispatch_event(handler);
    let id = make(&mut runtime, opened.get().unwrap());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    row.dispose();
    let drained = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    assert!(
        drained.iter().any(
            |(to, e)| *to == EventFor::Window(id) && matches!(e, RinchNativeEvent::CloseWindow)
        ),
        "the row that built the handler went: {drained:?}"
    );
}

#[test]
fn closing_a_window_disposes_its_document_then_says_it_closed() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (handle, id) = open(&mut runtime, component);
    let doc = doc_of(&runtime, id);
    let closed_at = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = closed_at.clone();
    let _watch = rinch_core::reactive::Effect::new(move || {
        if !handle.is_open() {
            seen.set(Some(()));
        }
    });
    runtime.close_other(id);
    assert!(closed_at.get().is_some(), "is_open is reactive");
    assert!(runtime.others.is_empty());
    assert!(
        runtime.role_for(EventFor::Sender(Some(doc))) == Some(WindowRole::Main),
        "its document is gone: code no longer finds it"
    );
}

#[test]
fn a_window_closed_before_it_was_made_is_never_made() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let handle =
        rinch_core::reactive::unowned(|| crate::open_window(WindowProps::default(), component));
    let id = handle_id(handle);
    runtime.close_other(id);
    assert!(crate::window_handle::take_pending(id).is_none());
    assert!(!handle.is_open());
}

#[test]
fn the_main_window_closing_ends_the_app_by_default() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    open(&mut runtime, component);
    assert!(
        runtime.close_soon(WindowRole::Main),
        "exits at once, as today"
    );
}

#[test]
fn last_window_closes_lets_the_others_outlive_the_main_one() {
    let mut runtime = runtime(crate::ExitWhen::LastWindowCloses);
    let (_, id) = open(&mut runtime, component);
    assert!(!runtime.close_soon(WindowRole::Main));
    assert!(!runtime.settle_closes_now(), "another window is open");
    assert!(runtime.main_closed);
    assert_eq!(
        runtime.role_for(EventFor::Sender(None)),
        None,
        "no main window"
    );
    assert!(WindowHandle::main().is_none());
    assert!(!runtime.close_soon(WindowRole::Other(id)));
    assert!(runtime.settle_closes_now(), "the last window closed");
}

#[test]
fn explicitly_runs_on_with_no_window_until_exit() {
    let mut runtime = runtime(crate::ExitWhen::Explicitly);
    let (_, id) = open(&mut runtime, component);
    runtime.close_soon(WindowRole::Main);
    runtime.close_soon(WindowRole::Other(id));
    assert!(
        !runtime.settle_closes_now(),
        "no window open, and still running"
    );
    assert!(crate::windows().is_empty());
}

#[derive(Clone)]
struct WindowStore(u32);

thread_local! {
    static SEEN_IN_WINDOW: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

fn content_with_a_store(scope: &mut RenderScope) -> NodeHandle {
    rinch_core::create_store(WindowStore(7));
    SEEN_IN_WINDOW.with(|s| s.set(rinch_core::try_use_store::<AppStore>().map(|a| a.0)));
    scope.create_element("div")
}

#[derive(Clone)]
struct AppStore(u32);

#[test]
fn a_store_made_in_a_second_window_is_its_own_and_the_app_s_is_seen_there() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    rinch_core::reactive::unowned(|| rinch_core::create_store(AppStore(3)));
    let (_, id) = open(&mut runtime, content_with_a_store);
    assert_eq!(
        SEEN_IN_WINDOW.with(|s| s.get()),
        Some(3),
        "the app's, from the window"
    );
    assert!(
        rinch_core::try_use_store::<WindowStore>().is_none(),
        "not visible outside its window"
    );
    let doc = doc_of(&runtime, id);
    let _root = rinch_core::push_context_root(doc);
    assert_eq!(
        rinch_core::try_use_store::<WindowStore>().map(|s| s.0),
        Some(7)
    );
}

thread_local! {
    static SHARED: std::cell::Cell<Option<rinch_core::Signal<u32>>> = const { std::cell::Cell::new(None) };
    static RUNS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn content_reading_the_shared_signal(scope: &mut RenderScope) -> NodeHandle {
    let shared = SHARED.with(|s| s.get()).unwrap();
    let div = scope.create_element("div");
    scope.create_effect(move || {
        let _ = shared.get();
        RUNS.with(|r| r.set(r.get() + 1));
    });
    div
}

#[test]
fn a_signal_written_in_one_window_runs_another_window_s_effect_in_the_same_flush() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let shared = rinch_core::reactive::unowned(|| rinch_core::Signal::new(0u32));
    SHARED.with(|s| s.set(Some(shared)));
    open(&mut runtime, content_reading_the_shared_signal);
    let before = RUNS.with(|r| r.get());
    {
        // The main window's code writes it.
        let _doc = rinch_core::push_dispatching_doc(runtime.main.app.doc_key());
        shared.set(1);
    }
    assert_eq!(
        RUNS.with(|r| r.get()),
        before + 1,
        "ran with the write, no turn in between"
    );
}

/// One turn's `AboutToWait` for every open window, as `about_to_wait_inner`
/// gives them (no event loop here).
#[cfg(feature = "theme")]
fn one_turn(runtime: &mut RinchRuntime) {
    for role in runtime.roles() {
        let w = runtime.w(role);
        let (size, scale) = (w.window_size(), w.scale_factor());
        let _ = w.app.handle_event(PlatformEvent::AboutToWait, size, scale);
    }
}

#[cfg(feature = "theme")]
fn applied_current_theme(w: &ShellWindow) -> bool {
    !w.app.theme_change_owed() && w.app.last_theme_key == Some(w.app.theme_key())
}

/// `rinch::update_theme` is process-wide: one call restyles every open
/// window's document in the turn that follows, and a window opened after it
/// is made with it.
#[cfg(feature = "theme")]
#[test]
fn one_theme_change_restyles_every_window_and_later_ones_are_made_with_it() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, first) = open(&mut runtime, component);
    crate::update_theme(&rinch_core::element::ThemeProviderProps {
        dark_mode: true,
        ..Default::default()
    });
    assert!(runtime.main.app.theme_change_owed());
    assert!(runtime.w(WindowRole::Other(first)).app.theme_change_owed());
    one_turn(&mut runtime);
    assert!(applied_current_theme(&runtime.main), "the main window's");
    assert!(
        applied_current_theme(runtime.w(WindowRole::Other(first))),
        "and the other window's, in the same turn"
    );

    let (_, later) = open(&mut runtime, component);
    assert!(
        applied_current_theme(runtime.w(WindowRole::Other(later))),
        "made with the theme current when it was opened"
    );
}

thread_local! {
    static CURRENT_SEEN: std::cell::RefCell<Vec<Option<WindowHandle>>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_current() {
    CURRENT_SEEN.with(|s| s.borrow_mut().push(WindowHandle::current()));
}

/// A child component's body: a function the root calls while it builds.
fn child_body(scope: &mut RenderScope) -> NodeHandle {
    note_current();
    scope.create_element("span")
}

/// The content closure and its root component's body.
fn content_noting_current(scope: &mut RenderScope) -> NodeHandle {
    note_current();
    let root = scope.create_element("div");
    let child = child_body(scope);
    root.append_child(&child);
    root
}

#[test]
fn current_in_a_window_s_own_content_is_that_window_from_its_first_line() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    CURRENT_SEEN.with(|s| s.borrow_mut().clear());
    let (handle, _) = open(&mut runtime, content_noting_current);
    let seen = CURRENT_SEEN.with(|s| s.borrow().clone());
    assert_eq!(
        seen,
        vec![Some(handle), Some(handle)],
        "the content, then a child's body"
    );
    assert_eq!(
        WindowHandle::current(),
        WindowHandle::main(),
        "after the mount, outside any document: the main window again"
    );
}

fn open_owned(
    runtime: &mut RinchRuntime,
    owner: &rinch_core::reactive::Scope,
    props: WindowProps,
) -> u64 {
    let handle = owner.run(|| crate::open_window(props, component));
    make(runtime, handle)
}

/// A long-lived owner that opens and closes windows, by each way a window
/// closes other than its owner going, keeps none of them: its cleanups are
/// as many after as before.
#[test]
fn an_owner_keeps_nothing_of_the_windows_it_opened_that_closed() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let owner = rinch_core::reactive::Scope::new();
    owner.on_cleanup(|| {});
    let before = owner.cleanup_count();
    for _ in 0..20 {
        // By its handle.
        let id = open_owned(&mut runtime, &owner, WindowProps::default());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        runtime.close_soon(WindowRole::Other(id));
        runtime.settle_closes_now();

        // By the user's close, which its app agrees to (no veto set).
        let id = open_owned(&mut runtime, &owner, WindowProps::default());
        user_closes(&mut runtime, id);

        // By the user's close, which `on_close_requested` agrees to.
        let props = WindowProps {
            on_close_requested: Some(std::sync::Arc::new(|| true)),
            ..Default::default()
        };
        let id = open_owned(&mut runtime, &owner, props);
        user_closes(&mut runtime, id);
    }
    assert!(runtime.others.is_empty());
    assert_eq!(
        owner.cleanup_count(),
        before,
        "the owner keeps none of them"
    );
}

/// The window's close button, as the runtime takes it: the app hears
/// `CloseRequested`, answers `Exit` when it agrees, and the runtime closes it.
fn user_closes(runtime: &mut RinchRuntime, id: u64) {
    let role = WindowRole::Other(id);
    let w = runtime.w(role);
    let (size, scale) = (w.window_size(), w.scale_factor());
    let actions = w
        .app
        .handle_event(PlatformEvent::CloseRequested, size, scale);
    assert!(
        actions.iter().any(|a| matches!(a, AppAction::Exit)),
        "{actions:?}"
    );
    runtime.close_soon(role);
    runtime.settle_closes_now();
}

/// An owner gone in the turn its window was opened: the window is never
/// made, though its `OpenWindow` is queued before anything else.
#[test]
fn a_window_whose_owner_went_before_it_was_made_is_never_made() {
    let runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let owner = rinch_core::reactive::Scope::new();
    let handle = owner.run(|| crate::open_window(WindowProps::default(), component));
    owner.dispose();
    assert!(!handle.is_open());
    let queued = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let open_ids: Vec<u64> = queued
        .iter()
        .filter_map(|(_, e)| match e {
            RinchNativeEvent::OpenWindow(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(open_ids.len(), 1, "its OpenWindow is still queued");
    assert!(
        crate::window_handle::take_pending(open_ids[0]).is_none(),
        "and finds nothing to make"
    );
    assert!(runtime.others.is_empty());
}

/// An output unplugged under a panel, the panel owned by that output's row:
/// the compositor's close of its surface and the row going come in either
/// order. Either way the window closes once, nothing reopens it, and
/// nothing acts on a window that is gone.
#[test]
fn an_unplugged_output_s_panel_closes_once_in_either_order() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);

    // The compositor's close first, then the row going.
    let row = rinch_core::reactive::Scope::new();
    let first = open_owned(&mut runtime, &row, WindowProps::default());
    runtime.close_soon(WindowRole::Other(first));
    runtime.settle_closes_now();
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    row.dispose();
    for (to, event) in NATIVE_EVENT_QUEUE.lock().unwrap().drain() {
        // What the drain would do with it: nothing, the window being gone.
        assert_eq!(runtime.role_for(to), None, "{event:?} finds no window");
    }
    assert!(runtime.others.is_empty(), "nothing reopened");

    // The row going first, then the compositor's close for the dead surface.
    let row = rinch_core::reactive::Scope::new();
    let second = open_owned(&mut runtime, &row, WindowProps::default());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    row.dispose();
    let queued = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    assert!(queued.iter().any(
        |(to, e)| *to == EventFor::Window(second) && matches!(e, RinchNativeEvent::CloseWindow)
    ));
    let close = runtime.close_soon(WindowRole::Other(second));
    assert!(!close);
    runtime.settle_closes_now();
    assert!(runtime.others.is_empty());
    // The compositor's close arrives for a window id no window has now: the
    // event path finds no role for it (`role_of`), and a late handle event
    // finds none either.
    assert_eq!(runtime.role_for(EventFor::Window(second)), None);
}

/// A windowless app's runtime (`App::windowless`), its root mounted as the
/// event loop's start would, with no OS window.
fn windowless(root: fn(&mut RenderScope) -> NodeHandle) -> RinchRuntime {
    crate::window_handle::set_windowless(true);
    crate::window_handle::set_exit_when(crate::ExitWhen::Explicitly);
    let mut runtime = RinchRuntime::new("t", 800, 600, root, &[]);
    crate::window_handle::set_windowless(false);
    crate::window_handle::set_exit_when(crate::ExitWhen::MainWindowCloses);
    runtime.mount_windowless_root();
    runtime
}

fn root_opening_a_window(scope: &mut RenderScope) -> NodeHandle {
    crate::open_window(WindowProps::default(), component);
    scope.create_element("div")
}

#[test]
fn a_windowless_app_has_no_main_window_and_its_root_opens_the_windows() {
    let mut runtime = windowless(root_opening_a_window);
    assert!(WindowHandle::main().is_none());
    assert_eq!(
        runtime.role_for(EventFor::Sender(None)),
        None,
        "no main window"
    );
    let root_doc = runtime.main.app.doc_key();
    assert_eq!(
        runtime.role_for(EventFor::Sender(Some(root_doc))),
        None,
        "code in the root addresses no window"
    );
    {
        let _doc = rinch_core::push_dispatching_doc(root_doc);
        assert_eq!(WindowHandle::current(), None);
    }
    // The window its root opened is made as any other.
    let opened = crate::windows();
    assert_eq!(opened.len(), 1, "only what the root opened");
    let id = make(&mut runtime, opened[0]);
    assert_eq!(
        runtime.role_for(EventFor::Window(id)),
        Some(WindowRole::Other(id))
    );
    // Its last window closing does not end it: it runs until `exit()`.
    runtime.close_soon(WindowRole::Other(id));
    assert!(!runtime.settle_closes_now());
    assert!(
        !runtime.windowless_root_warned,
        "a root that holds windows is no content"
    );
}

fn root_with_text(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    let text = scope.create_text("hello");
    root.append_child(&text);
    root
}

#[test]
fn content_in_a_windowless_root_is_warned_of_once() {
    let mut runtime = windowless(root_with_text);
    assert!(runtime.windowless_root_warned);
    // Once: a later look warns no more (the flag stays, nothing else changes).
    runtime.warn_windowless_root_content();
    assert!(runtime.windowless_root_warned);
}

/// `prepare_current_window()` in a window's own content, during its mount,
/// prepares that window, not the main one (#2633): the document is not the
/// dispatching one yet, as `WindowHandle::current` found in stage 4a.
#[test]
fn prepare_current_window_in_a_windows_content_names_that_window_2633() {
    fn popup(scope: &mut RenderScope) -> NodeHandle {
        crate::windows::prepare_current_window();
        scope.create_element("div")
    }
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let handle = rinch_core::reactive::unowned(|| {
        crate::open_window(
            WindowProps {
                visible: false,
                ..Default::default()
            },
            popup,
        )
    });
    let id = handle_id(handle);
    let pending = crate::window_handle::take_pending(id).expect("waiting to be made");
    let mut window = RinchRuntime::new_other_window(pending, &runtime.fonts);
    crate::window_handle::mounting(id, || window.mount_before_window(1.0));
    crate::window_handle::note_mounted(id, window.app.doc_key());
    runtime.others.push(window);
    let prepares: Vec<EventFor> = NATIVE_EVENT_QUEUE
        .lock()
        .unwrap()
        .drain()
        .into_iter()
        .filter(|(_, e)| matches!(e, RinchNativeEvent::PrepareWindow(_)))
        .map(|(to, _)| to)
        .collect();
    assert_eq!(prepares.len(), 1, "{prepares:?}");
    assert_eq!(
        runtime.role_for(prepares[0]),
        Some(WindowRole::Other(id)),
        "{prepares:?}"
    );
}

/// #2657: every window that paints ends its own performance frames, numbered
/// in its own sequence and listed by `rinch::perf::windows` under its id; the
/// main window's keep the unlabelled `[PERF] frame N:` lines.
#[test]
fn each_window_ends_its_own_performance_frames_2657() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    runtime.main.register_perf(true);
    let (_, a) = open(&mut runtime, component);
    let (_, b) = open(&mut runtime, component);
    runtime.w(WindowRole::Other(a)).paint_window().unwrap();
    runtime.w(WindowRole::Other(a)).paint_window().unwrap();
    runtime.w(WindowRole::Other(b)).paint_window().unwrap();
    runtime.paint().unwrap();
    let frames = |id: u64| {
        crate::perf::windows()
            .into_iter()
            .find(|w| w.window.id() == id)
            .map(|w| (w.main, w.frames))
    };
    assert_eq!(frames(runtime.main_handle), Some((true, 1)));
    assert_eq!(frames(a), Some((false, 2)));
    assert_eq!(frames(b), Some((false, 1)));
    let label = |id: u64| {
        let w = runtime.others.iter().find(|w| w.handle == id).unwrap();
        let doc = w.app.doc.as_ref().unwrap().borrow();
        doc.tree.perf.label().map(str::to_string)
    };
    assert_eq!(label(a), Some(format!("window {a}")));
    assert_eq!(
        runtime
            .main
            .app
            .doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .perf
            .label(),
        None
    );
    // A closed window leaves the list.
    runtime.close_other(a);
    assert_eq!(frames(a), None);
    assert_eq!(frames(b), Some((false, 1)));
}

/// #2657: a windowless app's windows write frames, and the list has no main
/// window.
#[test]
fn a_windowless_apps_windows_end_their_frames_2657() {
    let mut runtime = windowless(root_opening_a_window);
    let opened = crate::windows();
    let id = make(&mut runtime, opened[0]);
    runtime.w(WindowRole::Other(id)).paint_window().unwrap();
    let listed: Vec<(u64, bool, u64)> = crate::perf::windows()
        .into_iter()
        .filter(|w| w.window.id() == id || w.main)
        .map(|w| (w.window.id(), w.main, w.frames))
        .collect();
    assert!(listed.contains(&(id, false, 1)), "{listed:?}");
    assert!(
        !listed
            .iter()
            .any(|(i, main, _)| *main && *i == runtime.main_handle),
        "no main window: {listed:?}"
    );
}

thread_local! {
    static STALE_HANDLE: std::cell::Cell<Option<WindowHandle>> = const { std::cell::Cell::new(None) };
    static STALE_CLOSED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A root with a `Window` element placed on output 42 by id (#2658).
fn root_with_a_window_on_output_42(scope: &mut RenderScope) -> NodeHandle {
    let props = WindowProps {
        title: "wallpaper".into(),
        wayland_layer: Some(rinch_core::element::LayerSurface {
            output_id: Some(rinch_core::element::OutputId::from_raw(42)),
            ..Default::default()
        }),
        ..Default::default()
    };
    crate::__window_component(
        scope,
        props,
        |h| STALE_HANDLE.with(|c| c.set(Some(h))),
        || STALE_CLOSED.with(|c| c.set(c.get() + 1)),
        component,
    )
}

/// The output a window is placed on by id goes between `outputs()` and the
/// window's creation (#2658): the window is not made, rather than put on the
/// compositor's choice of output. Its handle reports closed, it leaves
/// `windows()`, and the element's `on_closed` is called once. With the output
/// still there, nothing is refused.
#[test]
fn a_window_on_an_output_that_went_is_not_made() {
    crate::window_handle::set_exit_when(crate::ExitWhen::MainWindowCloses);
    let mut runtime = RinchRuntime::new("t", 800, 600, root_with_a_window_on_output_42, &[]);
    runtime.main.app.mount_component(800.0, 600.0);
    let handle = STALE_HANDLE
        .with(|c| c.get())
        .expect("the element opened its window");
    let id = handle_id(handle);
    let pending = crate::window_handle::take_pending(id).expect("waiting to be made");

    // Output 42 still connected: made as usual (not refused).
    assert!(!RinchRuntime::refuse_stale_output(
        id,
        &pending.props,
        Backend::Wayland,
        &[41, 42]
    ));
    // Not Wayland: the id places nothing, so it is not checked.
    assert!(!RinchRuntime::refuse_stale_output(
        id,
        &pending.props,
        Backend::X11,
        &[41]
    ));
    assert!(handle.is_open());
    assert_eq!(STALE_CLOSED.with(|c| c.get()), 0);

    // Output 42 unplugged since the app listed it.
    assert!(RinchRuntime::refuse_stale_output(
        id,
        &pending.props,
        Backend::Wayland,
        &[41]
    ));
    assert!(!handle.is_open());
    assert!(!crate::windows().contains(&handle));
    assert!(runtime.others.is_empty());
    assert_eq!(STALE_CLOSED.with(|c| c.get()), 1, "on_closed once");
    // Asking again changes nothing: it is closed once.
    assert!(RinchRuntime::refuse_stale_output(
        id,
        &pending.props,
        Backend::Wayland,
        &[41]
    ));
    assert_eq!(STALE_CLOSED.with(|c| c.get()), 1);
}

thread_local! {
    static ROOT_SAW_ROLES: std::cell::Cell<Option<Option<crate::window::WindowRoles>>> =
        const { std::cell::Cell::new(None) };
}

fn root_reading_window_roles(scope: &mut RenderScope) -> NodeHandle {
    ROOT_SAW_ROLES.with(|c| c.set(Some(crate::window::window_roles())));
    scope.create_element("div")
}

/// The event loop's start publishes the window roles before a windowless
/// root runs, so its first run can decide whether to open a window (#2659).
#[test]
fn a_windowless_roots_first_run_sees_the_window_roles() {
    crate::window_handle::set_exit_when(crate::ExitWhen::Explicitly);
    let mut runtime = RinchRuntime::new("t", 800, 600, root_reading_window_roles, &[]);
    crate::window_handle::set_exit_when(crate::ExitWhen::MainWindowCloses);
    runtime.windowless = true;
    let roles = crate::window::WindowRoles::of_wayland_interfaces(["zwlr_layer_shell_v1"]);
    assert!(runtime.start_surfaces(roles), "windowless: no main window");
    assert_eq!(ROOT_SAW_ROLES.with(|c| c.get()), Some(Some(roles)));
}

/// A second window destroyed from outside the app (#2489: another X client's
/// `XDestroyWindow`, which winit reports as `WindowEvent::Destroyed`) closes,
/// without its `on_close_requested` being asked (it would refuse), and the app
/// runs on with its main window.
#[test]
fn a_second_window_destroyed_from_outside_closes_and_the_app_runs_on() {
    use std::sync::atomic::{AtomicU32, Ordering};
    static ASKED: AtomicU32 = AtomicU32::new(0);
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let props = WindowProps {
        on_close_requested: Some(std::sync::Arc::new(|| {
            ASKED.fetch_add(1, Ordering::Relaxed);
            false
        })),
        ..Default::default()
    };
    let handle = rinch_core::reactive::unowned(|| crate::open_window(props, component));
    let id = make(&mut runtime, handle);
    let (_, other) = open(&mut runtime, component);

    assert!(
        !runtime.window_destroyed(WindowRole::Other(id)),
        "the event loop does not end"
    );
    assert!(!runtime.settle_closes_now(), "nor once it is closed");
    assert!(!handle.is_open());
    assert_eq!(ASKED.load(Ordering::Relaxed), 0, "no veto is asked");
    assert_eq!(
        runtime.role_for(EventFor::Window(other)),
        Some(WindowRole::Other(other)),
        "the other second window stays"
    );
    assert_eq!(runtime.others.len(), 1);
    assert!(!runtime.main_closed);
}

/// The main window destroyed from outside ends an app that exits with it, as
/// its close would; under `LastWindowCloses` with another window open the app
/// runs on without it.
#[test]
fn a_main_window_destroyed_from_outside_closes_as_the_exit_rule_says() {
    {
        let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
        assert!(
            runtime.window_destroyed(WindowRole::Main),
            "the main window's going ends the event loop"
        );
    }

    let mut runtime = runtime(crate::ExitWhen::LastWindowCloses);
    let (handle, _) = open(&mut runtime, component);
    assert!(!runtime.window_destroyed(WindowRole::Main));
    assert!(!runtime.settle_closes_now(), "a window is still open");
    assert!(runtime.main_closed);
    assert!(handle.is_open());
    assert_eq!(crate::windows(), vec![handle]);
}

/// An event loop for `window_event_for` with no OS behind it: it records
/// `exit`, and anything else it is asked for is a test failure.
#[derive(Debug, Default)]
struct RecordingLoop {
    exited: std::cell::Cell<bool>,
}

impl winit::event_loop::ActiveEventLoop for RecordingLoop {
    fn create_proxy(&self) -> winit::event_loop::EventLoopProxy {
        unimplemented!("not asked by a Destroyed")
    }
    fn create_window(
        &self,
        _: winit::window::WindowAttributes,
    ) -> Result<Box<dyn winit::window::Window>, winit::error::CreateWindowError> {
        unimplemented!("not asked by a Destroyed")
    }
    fn create_custom_cursor(
        &self,
        _: winit::cursor::CustomCursorSource,
    ) -> Result<winit::cursor::CustomCursor, winit::error::CustomCursorError> {
        unimplemented!("not asked by a Destroyed")
    }
    fn available_monitors(&self) -> Box<dyn Iterator<Item = winit::monitor::MonitorHandle>> {
        Box::new(std::iter::empty())
    }
    fn primary_monitor(&self) -> Option<winit::monitor::MonitorHandle> {
        None
    }
    fn listen_device_events(&self, _: winit::event_loop::DeviceEvents) {}
    fn system_theme(&self) -> Option<winit::window::Theme> {
        None
    }
    fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
    fn control_flow(&self) -> winit::event_loop::ControlFlow {
        winit::event_loop::ControlFlow::Wait
    }
    fn exit(&self) {
        self.exited.set(true);
    }
    fn exiting(&self) -> bool {
        self.exited.get()
    }
    fn owned_display_handle(&self) -> winit::event_loop::OwnedDisplayHandle {
        unimplemented!("not asked by a Destroyed")
    }
    fn rwh_06_handle(&self) -> &dyn winit::raw_window_handle::HasDisplayHandle {
        unimplemented!("not asked by a Destroyed")
    }
}

/// `WindowEvent::Destroyed` reaches `window_destroyed` through the runtime's
/// event path (#2489): a second window's closes it and leaves the loop
/// running; the main window's ends the loop of an app that exits with it.
#[test]
fn a_destroyed_event_closes_its_window_through_the_event_path() {
    {
        let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
        let (handle, id) = open(&mut runtime, component);
        let event_loop = RecordingLoop::default();
        runtime.window_event_for(WindowRole::Other(id), &event_loop, WindowEvent::Destroyed);
        runtime.settle_closes(&event_loop);
        assert!(!handle.is_open(), "the second window closed");
        assert!(runtime.others.is_empty());
        assert!(!event_loop.exited.get(), "and the app runs on");
    }
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let event_loop = RecordingLoop::default();
    runtime.window_event_for(WindowRole::Main, &event_loop, WindowEvent::Destroyed);
    assert!(
        event_loop.exited.get(),
        "the main window's going ends the loop"
    );
}

// A window per output that keeps its window while its output moves, resizes
// or rescales (#2705).

thread_local! {
    /// Each window a per-output root opened, with the output it was for.
    static PER_OUTPUT_OPENED: std::cell::RefCell<Vec<(u64, WindowHandle)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn per_output_opened() -> Vec<(u64, WindowHandle)> {
    PER_OUTPUT_OPENED.with(|o| o.borrow().clone())
}

/// The guide's panel per output (#2705): the `for` iterates the ids, and the
/// output's other facts are read inside the row with `output(id)`.
fn root_with_a_window_per_output_id(__scope: &mut RenderScope) -> NodeHandle {
    use crate as rinch;
    use crate::outputs::{output, output_ids};
    use rinch_core::element::LayerSurface;
    rinch_macros::rsx! {
        div {
            for id in output_ids() {
                Window {
                    key: id,
                    wayland_layer: Some(LayerSurface { output_id: Some(id), ..Default::default() }),
                    on_open: move |h: WindowHandle| {
                        PER_OUTPUT_OPENED.with(|o| o.borrow_mut().push((id.raw(), h)))
                    },
                    div { {move || output(id).map(|o| format!("{:?}", o.size)).unwrap_or_default()} }
                }
            }
        }
    }
}

/// The loop the guide showed before #2705: the row's item is the whole
/// `Output`.
fn root_with_a_window_per_whole_output(__scope: &mut RenderScope) -> NodeHandle {
    use crate as rinch;
    use crate::outputs::outputs;
    use rinch_core::element::LayerSurface;
    rinch_macros::rsx! {
        div {
            for output in outputs() {
                Window {
                    key: output.id,
                    wayland_layer: Some(LayerSurface { output_id: Some(output.id), ..Default::default() }),
                    on_open: {
                        let id = output.id.raw();
                        move |h: WindowHandle| PER_OUTPUT_OPENED.with(|o| o.borrow_mut().push((id, h)))
                    },
                    div { "panel" }
                }
            }
        }
    }
}

fn test_output(id: u64, x: f64, w: f64, scale: f64) -> crate::outputs::Output {
    crate::outputs::Output {
        id: rinch_core::element::OutputId::from_raw(id),
        name: Some(format!("OUT-{id}")),
        position: (x, 0.0),
        size: (w, 1080.0),
        scale,
    }
}

fn publish_outputs(list: Vec<crate::outputs::Output>) {
    crate::outputs::publish(list, crate::outputs::OutputTracking::Live);
}

/// Two outputs, each with its window from a `for` over `output_ids()`. The
/// first is moved, then resized, the second rescaled: no window closes and
/// none opens. Unplugging the second closes its window, once, and only it.
#[test]
fn a_window_per_output_survives_its_output_moving_resizing_and_rescaling_2705() {
    publish_outputs(vec![
        test_output(1, 0.0, 1920.0, 1.0),
        test_output(2, 1920.0, 1280.0, 1.0),
    ]);
    let _runtime = windowless(root_with_a_window_per_output_id);
    let opened = per_output_opened();
    assert_eq!(
        opened.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [1, 2],
        "a window per output"
    );

    let changes = [
        // Moved.
        vec![
            test_output(1, 100.0, 1920.0, 1.0),
            test_output(2, 2020.0, 1280.0, 1.0),
        ],
        // Resized.
        vec![
            test_output(1, 100.0, 2560.0, 1.0),
            test_output(2, 2660.0, 1280.0, 1.0),
        ],
        // Rescaled.
        vec![
            test_output(1, 100.0, 2560.0, 1.0),
            test_output(2, 2660.0, 853.0, 1.5),
        ],
    ];
    for list in changes {
        publish_outputs(list.clone());
        assert_eq!(
            per_output_opened(),
            opened,
            "no window opened after {list:?}"
        );
        assert!(
            opened.iter().all(|(_, h)| h.is_open()),
            "no window closed after {list:?}"
        );
        assert_eq!(crate::windows().len(), 2);
    }

    // The second output unplugged: its window closes, the first stays.
    publish_outputs(vec![test_output(1, 100.0, 2560.0, 1.0)]);
    assert_eq!(per_output_opened(), opened, "nothing reopened");
    assert!(
        opened[0].1.is_open(),
        "the remaining output keeps its window"
    );
    assert!(
        !opened[1].1.is_open(),
        "the unplugged output's window closed"
    );
    assert_eq!(crate::windows(), vec![opened[0].1]);
}

/// Why the guide's loop iterates ids (#2705): a keyed `for` renders a row
/// again when its item changes (`PartialEq`), so a row whose item is the whole
/// `Output` closes its window and opens another at a move, though its key is
/// the same.
#[test]
fn a_row_whose_item_is_the_whole_output_reopens_its_window_when_it_moves_2705() {
    publish_outputs(vec![test_output(1, 0.0, 1920.0, 1.0)]);
    let _runtime = windowless(root_with_a_window_per_whole_output);
    let first = per_output_opened();
    assert_eq!(first.len(), 1);
    publish_outputs(vec![test_output(1, 100.0, 1920.0, 1.0)]);
    let after = per_output_opened();
    assert_eq!(
        after.len(),
        2,
        "the row was rendered again: a second window"
    );
    assert!(!first[0].1.is_open(), "and the first one closed");
    assert!(after[1].1.is_open());
}

thread_local! {
    /// What each per-output window's content showed, in order: (output, text).
    static PER_OUTPUT_SHOWN: std::cell::RefCell<Vec<(u64, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn per_output_shown(id: u64) -> Vec<String> {
    PER_OUTPUT_SHOWN.with(|s| {
        s.borrow()
            .iter()
            .filter(|(i, _)| *i == id)
            .map(|(_, t)| t.clone())
            .collect()
    })
}

/// The guide's pattern with content that reads `output(id)`, and shows
/// nothing (never unwraps) when it is `None`.
fn root_with_content_reading_its_output(__scope: &mut RenderScope) -> NodeHandle {
    use crate as rinch;
    use crate::outputs::{output, output_ids};
    use rinch_core::element::LayerSurface;
    rinch_macros::rsx! {
        div {
            for id in output_ids() {
                Window {
                    key: id,
                    wayland_layer: Some(LayerSurface { output_id: Some(id), ..Default::default() }),
                    on_open: move |h: WindowHandle| {
                        PER_OUTPUT_OPENED.with(|o| o.borrow_mut().push((id.raw(), h)))
                    },
                    div {
                        {move || {
                            let text = match output(id) {
                                Some(o) => format!("{:?} {:?} x{}", o.position, o.size, o.scale),
                                None => "NONE".to_string(),
                            };
                            PER_OUTPUT_SHOWN.with(|s| s.borrow_mut().push((id.raw(), text.clone())));
                            text
                        }}
                    }
                }
            }
        }
    }
}

/// A window's own content follows its output through a move and a rescale by
/// `output(id)`, in the same window. When the output is unplugged, the
/// content's reader runs once more with `None` before the row (and its
/// window) goes, which is why the guide says never to unwrap it (#2705).
#[test]
fn a_per_output_windows_content_follows_its_output_and_sees_none_once_on_unplug_2705() {
    publish_outputs(vec![
        test_output(1, 0.0, 1920.0, 1.0),
        test_output(2, 1920.0, 1280.0, 1.0),
    ]);
    let mut runtime = windowless(root_with_content_reading_its_output);
    let opened = per_output_opened();
    assert_eq!(opened.len(), 2);
    let made: Vec<u64> = opened.iter().map(|(_, h)| make(&mut runtime, *h)).collect();
    assert_eq!(per_output_shown(2), ["(1920.0, 0.0) (1280.0, 1080.0) x1"]);

    publish_outputs(vec![
        test_output(1, 100.0, 1920.0, 1.0),
        test_output(2, 2020.0, 1280.0, 1.5),
    ]);
    assert_eq!(per_output_opened(), opened, "no window opened");
    assert_eq!(
        per_output_shown(2),
        [
            "(1920.0, 0.0) (1280.0, 1080.0) x1",
            "(2020.0, 0.0) (1280.0, 1080.0) x1.5"
        ],
        "the content followed its output in the same window"
    );
    assert_eq!(
        per_output_shown(1).last().map(String::as_str),
        Some("(100.0, 0.0) (1920.0, 1080.0) x1")
    );

    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    publish_outputs(vec![test_output(1, 100.0, 1920.0, 1.0)]);
    assert_eq!(
        per_output_shown(2).last().map(String::as_str),
        Some("NONE"),
        "the gone output's reader ran once with None before its row went"
    );
    assert_eq!(per_output_shown(2).len(), 3, "once, not again after");
    // The row going closed its window: the runtime is told to close it (and
    // only it), and does.
    let closes: Vec<EventFor> = NATIVE_EVENT_QUEUE
        .lock()
        .unwrap()
        .drain()
        .into_iter()
        .filter(|(_, e)| matches!(e, RinchNativeEvent::CloseWindow))
        .map(|(to, _)| to)
        .collect();
    assert_eq!(closes, [EventFor::Window(made[1])]);
    runtime.close_soon(WindowRole::Other(made[1]));
    runtime.settle_closes_now();
    assert!(!opened[1].1.is_open());
    assert!(opened[0].1.is_open());
    assert_eq!(per_output_opened(), opened, "nothing reopened");
}

thread_local! {
    /// What the `on_cleanup` of [`reading_at_cleanup`] read: the window's
    /// logical size.
    static READ_AT_CLEANUP: std::cell::Cell<Option<(f64, f64)>> =
        const { std::cell::Cell::new(None) };
}

fn reading_at_cleanup(scope: &mut RenderScope) -> NodeHandle {
    scope.on_cleanup(|| READ_AT_CLEANUP.with(|read| read.set(Some(crate::window::window_size()))));
    scope.create_element("div")
}

/// Code a window's disposal runs (an `on_cleanup`) reads its own window's
/// facts, not the thread's: the document is disposed before its facts are
/// forgotten.
#[test]
fn a_closing_windows_cleanup_reads_its_own_facts() {
    READ_AT_CLEANUP.with(|read| read.set(None));
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_handle, id) = open(&mut runtime, reading_at_cleanup);
    crate::window::publish(crate::window::WindowFacts {
        size: Some((1111.0, 777.0)),
        ..Default::default()
    });
    let window = runtime.others.iter_mut().find(|w| w.handle == id).unwrap();
    let _ = window.app.handle_event(
        PlatformEvent::Resized {
            width: 321,
            height: 123,
        },
        (321, 123),
        1.0,
    );
    runtime.close_other(id);
    assert_eq!(
        READ_AT_CLEANUP.with(std::cell::Cell::get),
        Some((321.0, 123.0))
    );
}

thread_local! {
    /// The document a callback queued by [`queuing_at_cleanup`]'s cleanup
    /// ran under, once it ran (`Some(None)`: no document).
    static RAN_UNDER: std::cell::Cell<Option<Option<u64>>> = const { std::cell::Cell::new(None) };
}

fn queuing_at_cleanup(scope: &mut RenderScope) -> NodeHandle {
    scope.on_cleanup(|| {
        rinch_core::queue_main_callback(Box::new(|| {
            RAN_UNDER.with(|r| r.set(Some(rinch_core::current_dispatching_doc())));
        }));
        crate::window::set_window_title("bye");
    });
    scope.create_element("div")
}

/// A closing window's cleanup still reads as its document's code, but what
/// it makes for later records no document: a callback it queues runs after
/// the close under no document, not the gone one. And a title it stores is
/// not kept for the gone document.
#[test]
fn what_a_closing_windows_cleanup_leaves_belongs_to_no_document() {
    RAN_UNDER.with(|r| r.set(None));
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_handle, id) = open(&mut runtime, queuing_at_cleanup);
    let doc = doc_of(&runtime, id);
    runtime.close_other(id);
    rinch_core::drain_main_callbacks();
    assert_eq!(RAN_UNDER.with(std::cell::Cell::get), Some(None));
    assert_eq!(
        crate::window::take_stored_title(|d| d == Some(doc)),
        None,
        "nothing kept for the gone document"
    );
}

/// #2821: a window opened with `open_window` (as a `Window` element is) is
/// made with its own surface's namespace, through the same path as the
/// main window.
#[cfg(target_os = "linux")]
#[test]
fn a_second_window_names_its_layer_surface() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let props = WindowProps {
        app_id: Some("org.example.Panel".into()),
        wayland_layer: Some(rinch_core::element::LayerSurface {
            namespace: Some("dock".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let handle = rinch_core::reactive::unowned(|| crate::open_window(props, component));
    let id = make(&mut runtime, handle);
    let window = runtime.others.iter().find(|w| w.handle == id).unwrap();
    assert_eq!(window.layer_spec(true).namespace, "dock");
}
