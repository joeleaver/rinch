//! #2416 stage 4: opening and closing windows, which window an event is for,
//! ownership, store scoping, and the exit rule, through the runtime without
//! an event loop (no OS window is made).

use super::*;
use crate::window_handle::WindowHandle;
use rinch_core::element::WindowProps;

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
    let mut window = RinchRuntime::new_other_window(pending, &runtime.fonts);
    window.mount_before_window(1.0);
    crate::window_handle::note_mounted(id, window.app.doc_key());
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
