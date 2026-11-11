//! #2416 stage 4: the window registry, without a runtime.

use super::*;

fn content(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

/// A fresh registry for the test's thread, with a main window.
fn fresh() -> WindowHandle {
    reset();
    WindowHandle(register_main())
}

#[test]
fn windows_lists_the_main_window_first_then_oldest_first() {
    let main = fresh();
    let a = rinch_core::reactive::unowned(|| open_window(WindowProps::default(), content));
    let b = rinch_core::reactive::unowned(|| open_window(WindowProps::default(), content));
    assert_eq!(windows(), vec![main, a, b]);
    note_closed(a.0);
    assert_eq!(windows(), vec![main, b]);
    assert_eq!(WindowHandle::main(), Some(main));
    note_closed(main.0);
    assert_eq!(WindowHandle::main(), None);
    assert_eq!(windows(), vec![b]);
    reset();
}

#[test]
fn a_handle_is_usable_at_once_and_inert_once_closed() {
    fresh();
    let h = rinch_core::reactive::unowned(|| open_window(WindowProps::default(), content));
    assert!(h.is_open(), "open from the moment it is asked for");
    assert_eq!(h.size(), None, "not made yet: no facts");
    note_closed(h.0);
    assert!(!h.is_open());
    assert_eq!(h.size(), None);
    assert!(
        take_pending(h.0).is_none(),
        "closed before it was made: never made"
    );
    reset();
}

#[test]
fn current_is_the_window_whose_document_is_running() {
    let main = fresh();
    let h = rinch_core::reactive::unowned(|| open_window(WindowProps::default(), content));
    note_mounted(h.0, 4242);
    assert_eq!(WindowHandle::current(), Some(main), "outside any document");
    {
        let _doc = rinch_core::push_dispatching_doc(4242);
        assert_eq!(WindowHandle::current(), Some(h));
    }
    {
        let _doc = rinch_core::push_dispatching_doc(99);
        assert_eq!(
            WindowHandle::current(),
            Some(main),
            "a document that is no window's"
        );
    }
    reset();
}

#[test]
fn is_open_and_windows_are_reactive() {
    fresh();
    let h = rinch_core::reactive::unowned(|| open_window(WindowProps::default(), content));
    let runs = std::rc::Rc::new(std::cell::Cell::new(0));
    let r = runs.clone();
    let _effect = rinch_core::reactive::Effect::new(move || {
        let _ = (h.is_open(), windows());
        r.set(r.get() + 1);
    });
    let before = runs.get();
    note_closed(h.0);
    assert!(runs.get() > before);
    reset();
}
