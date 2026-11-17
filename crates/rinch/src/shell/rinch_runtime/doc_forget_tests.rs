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
