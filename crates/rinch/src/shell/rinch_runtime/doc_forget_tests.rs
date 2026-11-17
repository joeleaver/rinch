//! #2797: what a closed second window leaves in the registries keyed by its
//! document.

use super::*;

thread_local! {
    /// The root of the last window [`focusing`] built, for a test to free its
    /// children through.
    static ROOT: std::cell::RefCell<Option<NodeHandle>> = const { std::cell::RefCell::new(None) };
}

/// A window whose content is one 200×40 focus target at its top left.
fn focusing(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    let target = scope.create_element("div");
    target.set_attribute("style", "width: 200px; height: 40px");
    target.set_attribute("tabindex", "0");
    crate::focus_registry::register_focus_target(&target, crate::focus_registry::FocusEntry::new());
    root.append_child(&target);
    ROOT.with(|r| *r.borrow_mut() = Some(root.clone()));
    root
}

/// Click window `id` at (10, 10) through its own app, as the shell
/// dispatches a click, so its arbiter claims the target there.
fn click(runtime: &mut RinchRuntime, id: u64) {
    let window = runtime.others.iter_mut().find(|w| w.handle == id).unwrap();
    window.app.resolve_and_repaint(400.0, 300.0);
    let left = rinch_platform::MouseButton::Left;
    for event in [
        PlatformEvent::MouseMove { x: 10.0, y: 10.0 },
        PlatformEvent::MouseDown {
            x: 10.0,
            y: 10.0,
            button: left,
        },
        PlatformEvent::MouseUp {
            x: 10.0,
            y: 10.0,
            button: left,
        },
    ] {
        let _ = window.app.handle_event(event, (400, 300), 1.0);
    }
}

/// A closed second window's focus claim goes with it. The only purge used to
/// be `Drop for RinchApp`, which reads the document's key after
/// `dispose_document` has taken the document, so it read 0 and skipped it.
#[test]
fn a_closed_windows_focus_claim_is_forgotten() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_handle, id) = open(&mut runtime, focusing);
    let doc = doc_of(&runtime, id);
    click(&mut runtime, id);
    assert!(
        crate::focus_registry::claimed_for_tests(doc).is_some(),
        "control: the click claimed the target"
    );
    runtime.close_other(id);
    assert_eq!(crate::focus_registry::claimed_for_tests(doc), None);
}

/// A target freed while it held its document's claim is parked until the
/// arbiter releases the claim (#1509); a window closed before that leaves
/// nothing parked.
#[test]
fn a_closed_windows_parked_freed_target_is_forgotten() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_handle, id) = open(&mut runtime, focusing);
    let doc = doc_of(&runtime, id);
    click(&mut runtime, id);
    let root = ROOT
        .with(|r| r.borrow_mut().take())
        .expect("the window's root");
    root.set_inner_html("<p>replaced</p>");
    assert_eq!(
        crate::focus_registry::freed_for_tests(doc),
        1,
        "control: the claimed target was freed and parked"
    );
    runtime.close_other(id);
    assert_eq!(crate::focus_registry::freed_for_tests(doc), 0);
}

/// Every registry keyed by document that a second window's code reaches, by
/// the name `rinch_core::doc_entries` gives it (its static's path).
/// `FREED` is the one left out: it fills only when a claimed target is freed,
/// and [`a_closed_windows_parked_freed_target_is_forgotten`] covers it.
const EVERY_KIND: [&str; 14] = [
    "rinch::editor::virtualization::WINDOWS",
    "rinch::focus_registry::CLAIMED",
    "rinch::focus_registry::TARGETS",
    "rinch_core::events::app_key::LISTENERS",
    "rinch_core::events::dismiss::STACK",
    "rinch_core::events::display::CHANGES",
    "rinch_core::events::keyboard::KEYBOARD_INTERCEPTOR",
    "rinch_core::events::paste::PASTE_INTERCEPTOR",
    "rinch_core::events::selection::PENDING_FOCUS_REQUEST",
    "rinch_core::events::selection::PENDING_RELEASES",
    "rinch_core::events::selection::PENDING_TEXT_SELECTIONS",
    "rinch_core::events::selection::SAVED_SELECTION",
    "rinch_core::events::selection::SELECTION_CALLBACK",
    "rinch_core::events::selection::SELECTION_SYNC_CALLBACK",
];

/// One entry in every registry for document `doc`, made by `node`'s
/// window's code: what app code, a component or the runtime makes there.
/// A dismiss handle is kept (forgotten), as an app that holds one for its life
/// does; an app key's handle does not unregister it when dropped.
fn register_everywhere(doc: u64, node: &NodeHandle) {
    use rinch_core::events::*;
    let _ = rinch_core::events::display_scale_changes(doc);
    post_text_selection_request(doc, 1, 0, 0, rinch_core::dom::SelectionDirection::Forward);
    crate::focus_registry::note_claim(doc, Some(1));
    let _listener = rinch_core::on_app_key(|_| false);
    std::mem::forget(push_dismiss_handler(doc, || false));
    crate::focus_registry::register_focus_target(node, crate::focus_registry::FocusEntry::new());
    post_release_request(doc, 1);
    post_focus_request(doc, FocusRequest::Node(1));
    save_selection_snapshot();
    set_keyboard_interceptor(|_| false);
    set_paste_interceptor(|_| false);
    set_selection_callback(|_| Vec::new());
    set_selection_sync_callback(|_| {});
}

/// A window that registers everywhere in its ordinary code (its mount) and
/// again in its `on_cleanup`, as the window is being disposed, and holds an
/// editor in a scroll container, which its layout virtualizes.
fn registering(scope: &mut RenderScope) -> NodeHandle {
    let doc = rinch_core::current_dispatching_doc().expect("a window's mount is its document's");
    let root = scope.create_element("div");
    let (editor, handle) = crate::editor::mount_editor(scope);
    editor.set_attribute("style", "height: 100px; overflow-y: auto");
    let mut html = String::new();
    for i in 0..40 {
        html.push_str(&format!("<p>Paragraph {i}</p>"));
    }
    handle.load_html(&html);
    root.append_child(&editor);
    let target = scope.create_element("div");
    root.append_child(&target);
    register_everywhere(doc, &target);
    scope.on_cleanup(move || register_everywhere(doc, &target));
    root
}

/// The registries that hold something for `doc` and are not in
/// `EVERY_KIND`: the window's code reaches a registry the list does not name.
fn unlisted_for(doc: u64) -> Vec<&'static str> {
    left_for(doc)
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !EVERY_KIND.contains(name))
        .collect()
}

/// The registries in `EVERY_KIND` that hold nothing for `doc`.
fn empty_of(doc: u64) -> Vec<&'static str> {
    let entries = rinch_core::doc_entries(doc);
    EVERY_KIND
        .into_iter()
        .filter(|kind| !entries.iter().any(|(name, n)| name == kind && *n > 0))
        .collect()
}

/// The registries that still hold something for `doc`, by name.
fn left_for(doc: u64) -> Vec<(&'static str, usize)> {
    rinch_core::doc_entries(doc)
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// Open a [`registering`] window and lay it out, which virtualizes its
/// editor. Answers its id and its document's key.
fn open_registering(runtime: &mut RinchRuntime) -> (u64, u64) {
    let (_handle, id) = open(runtime, registering);
    let doc = doc_of(runtime, id);
    let window = runtime.others.iter_mut().find(|w| w.handle == id).unwrap();
    window.app.resolve_and_repaint(400.0, 300.0);
    // The layout drained the focus and selection requests its mount posted,
    // as it should. Post them again, as the window's code does between
    // frames (a handler, a timer), so they are parked when it closes.
    {
        use rinch_core::events::*;
        let _doc = rinch_core::push_dispatching_doc(doc);
        post_text_selection_request(doc, 1, 0, 0, rinch_core::dom::SelectionDirection::Forward);
        post_release_request(doc, 1);
        post_focus_request(doc, FocusRequest::Node(1));
    }
    (id, doc)
}

/// A closed second window leaves nothing in any registry keyed by its
/// document: neither what its ordinary code registered nor what its cleanup
/// registered as it was disposed. Fails naming the registries that kept
/// something.
#[test]
fn a_closed_window_leaves_nothing_in_any_registry() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    // Every app key on the thread, whatever its document: a cleanup run as no
    // document's code would register one under none, which no count by
    // document sees.
    let keys_before = rinch_core::events::app_key_listener_count();
    let (id, doc) = open_registering(&mut runtime);
    assert_eq!(
        empty_of(doc),
        Vec::<&str>::new(),
        "control: every kind registered"
    );
    assert_eq!(
        unlisted_for(doc),
        Vec::<&str>::new(),
        "control: the window reaches registries EVERY_KIND does not list: add them"
    );
    runtime.close_other(id);
    assert_eq!(
        rinch_core::events::app_key_listener_count(),
        keys_before,
        "no app key the window registered stays, under its document or none"
    );
    assert_eq!(
        left_for(doc),
        [],
        "registries still holding the closed window's entries"
    );
}

/// Closing one window leaves another's entries as they were: the sweep takes
/// the closed document's alone.
#[test]
fn another_windows_entries_survive_a_close() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (gone, gone_doc) = open_registering(&mut runtime);
    let (_kept, kept_doc) = open_registering(&mut runtime);
    // The focus-request slot is one for every document, last write wins: the
    // closing window's cleanup posts over the other's request, as any later
    // post does. It is not a registry the other window keeps an entry in.
    let comparable = |doc| {
        let mut entries = left_for(doc);
        entries.retain(|(name, _)| *name != "rinch_core::events::selection::PENDING_FOCUS_REQUEST");
        entries.sort_unstable();
        entries
    };
    let before = comparable(kept_doc);
    assert!(!before.is_empty(), "control: the other window registered");
    runtime.close_other(gone);
    assert_eq!(left_for(gone_doc), []);
    assert_eq!(comparable(kept_doc), before, "the other window's entries");
}
