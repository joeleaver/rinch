//! #2797: what a closed second window leaves in the registries keyed by its
//! document.

use super::*;

/// A window's document, as a scope hands it out.
type DocWeak = std::rc::Weak<std::cell::RefCell<dyn rinch_core::dom::DomDocument>>;

thread_local! {
    /// The root of the last window [`focusing`] built, for a test to free its
    /// children through.
    static ROOT: std::cell::RefCell<Option<NodeHandle>> = const { std::cell::RefCell::new(None) };
    /// The node the last [`registering`] window registers at.
    static TARGET: std::cell::RefCell<Option<NodeHandle>> = const { std::cell::RefCell::new(None) };
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
/// Left out, each covered by a test of its own: `FREED`, which fills only
/// when a claimed target is freed
/// ([`a_closed_windows_parked_freed_target_is_forgotten`]), and the sortable
/// list's preview wrappers and focus request, which fill only during and
/// after a drag (`sortable_2154_tests`).
const EVERY_KIND: [&str; 25] = [
    "rinch::app::DOC_RERENDERS",
    "rinch::editor::virtualization::WINDOWS",
    "rinch::focus_registry::CLAIMED",
    "rinch::focus_registry::TARGETS",
    "rinch_components::dropdown_menu_sub::SUBS",
    "rinch_components::sortable::LISTS",
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
    "rinch_core::reactive::DOC_COUNTERS",
    "rinch_editor_view::blink::CLOCKS",
    "rinch_editor_view::registry::DRAG",
    "rinch_editor_view::registry::EDITORS",
    "rinch_editor_view::registry::IMAGE_HOVER",
    "rinch_editor_view::registry::LINK_HOVER",
    "rinch_editor_view::registry::OVERLAY_PASSES_OWED",
    "rinch_editor_view::styles::INJECTED",
];

/// One entry in every registry for document `doc`, made by `node`'s
/// window's code: what app code, a component or the runtime makes there.
/// A dismiss handle is kept (forgotten), as an app that holds one for its life
/// does; an app key's handle does not unregister it when dropped.
fn register_everywhere(doc: u64, node: &NodeHandle, document: &DocWeak) {
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
    register_in_the_editor_view(doc, node);
    render_components_kept(node, document);
    // A `ReRender` it queued (#2798); its reactive work is counted from the
    // mount.
    crate::app::count_doc_rerender(Some(doc));
}

/// One entry in each of the editor view's registries for document `doc`, at
/// `node`'s id, made by its key as the runtime makes them: an editor
/// registered, its drag-select, an owed overlay pass, its blink clock, and
/// the link and image its pointer is over. No owner removes them.
fn register_in_the_editor_view(doc: u64, node: &NodeHandle) {
    use crate::editor::registry::{owe_overlay_pass_for, register_editor};
    use crate::editor::{
        ImageHover, LinkHover, LinkSpan, begin_drag, caret_blink_tick, create_editor,
        set_image_hover, set_link_hover,
    };
    let container = node.node_id().0;
    let editor = create_editor();
    register_editor(doc, container, editor.clone());
    begin_drag(Some(doc), container, 0);
    owe_overlay_pass_for(doc);
    let _ = caret_blink_tick(doc, Some(container));
    let link = LinkSpan {
        href: "https://example.com".into(),
        title: None,
        from: rinch_editor_core::Pos(0),
        to: rinch_editor_core::Pos(1),
    };
    set_link_hover(
        Some(doc),
        Some((
            editor.clone(),
            LinkHover {
                link,
                rect: Default::default(),
            },
        )),
    );
    set_image_hover(
        Some(doc),
        Some((
            editor,
            ImageHover {
                pos: rinch_editor_core::Pos(0),
                attrs: Default::default(),
                rect: Default::default(),
            },
        )),
    );
}

/// A `DropdownMenuSub` and a `SortableList` rendered under `node` in a scope
/// that is kept for the thread's life, as one an app holds and never drops:
/// their registrations are made and nothing of theirs removes them.
fn render_components_kept(node: &NodeHandle, document: &DocWeak) {
    use rinch_core::element::Component;
    let doc = document.upgrade().expect("the window's document");
    let mut scope = RenderScope::new(doc, node.node_id());
    let sub =
        rinch_components::dropdown_menu_sub::DropdownMenuSub::default().render(&mut scope, &[]);
    let list = rinch_components::sortable::SortableList::default().render(&mut scope, &[]);
    node.append_child(&sub);
    node.append_child(&list);
    std::mem::forget(scope);
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
    TARGET.with(|t| *t.borrow_mut() = Some(target.clone()));
    let document = scope.doc_weak();
    register_everywhere(doc, &target, &document);
    scope.on_cleanup(move || register_everywhere(doc, &target, &document));
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
    // The layout drained the focus and selection requests its mount posted
    // and the overlay pass it owed, as it should. Post them again, as the
    // window's code does between frames (a handler, a timer), so they are
    // parked when it closes.
    {
        use rinch_core::events::*;
        let _doc = rinch_core::push_dispatching_doc(doc);
        post_text_selection_request(doc, 1, 0, 0, rinch_core::dom::SelectionDirection::Forward);
        post_release_request(doc, 1);
        post_focus_request(doc, FocusRequest::Node(1));
        // And the overlay pass the layout ran.
        crate::editor::registry::owe_overlay_pass_for(doc);
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

/// The editor view's registrations and the components', made by a window's
/// ordinary code and by its cleanup with nothing to remove them, are gone
/// once the window closes: read through each registry's own public lookups,
/// not the sweep's inventory.
#[test]
fn a_closed_windows_editor_view_entries_are_forgotten() {
    use crate::editor::registry::all_editors;
    use crate::editor::{drag_anchor, editor_for_doc, overlay_pass_owed};
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let editors_before = all_editors().len();
    let (id, doc) = open_registering(&mut runtime);
    let container = TARGET
        .with(|t| t.borrow().clone())
        .expect("the window's target")
        .node_id()
        .0;
    assert!(
        editor_for_doc(doc, container).is_some() && drag_anchor(Some(doc)).is_some(),
        "control: registered"
    );
    runtime.close_other(id);
    assert_eq!(editor_for_doc(doc, container).map(|_| ()), None);
    assert_eq!(drag_anchor(Some(doc)), None);
    assert!(
        !overlay_pass_owed(doc),
        "an overlay pass owed to no document"
    );
    assert_eq!(all_editors().len(), editors_before, "no editor stays");
}

/// Many windows opened and closed leave the registries as they found them:
/// nothing grows with the number of windows the app has had.
#[test]
fn registries_stay_flat_over_many_windows() {
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let keys_before = rinch_core::events::app_key_listener_count();
    let editors_before = crate::editor::registry::all_editors().len();
    // Counted whatever their document: an entry whose document can no
    // longer be read (its nodes' document is gone) is in no count by
    // document.
    let components = || {
        (
            rinch_components::dropdown_menu_sub::__registered_count(),
            rinch_components::sortable::__registered_count(),
        )
    };
    let components_before = components();
    let mut docs = Vec::new();
    for _ in 0..5 {
        let (id, doc) = open_registering(&mut runtime);
        docs.push(doc);
        runtime.close_other(id);
        assert_eq!(
            rinch_core::events::app_key_listener_count(),
            keys_before,
            "app keys after a close"
        );
        assert_eq!(
            crate::editor::registry::all_editors().len(),
            editors_before,
            "editors after a close"
        );
        assert_eq!(
            components(),
            components_before,
            "submenus and sortable lists after a close"
        );
    }
    for doc in docs {
        assert_eq!(left_for(doc), [], "document {doc}'s entries");
    }
}
