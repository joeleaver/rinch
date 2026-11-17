//! `Window { … }` in `rsx!` (#2416 stage 4b), as an app writes it, mounted in
//! an embedded context (no event loop: the window is opened and waits to be
//! made, which is all these checks need).
#![cfg(all(feature = "desktop", any(feature = "gpu", feature = "embed")))]

use std::cell::{Cell, RefCell};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

thread_local! {
    static SHOW: Cell<Option<Signal<bool>>> = const { Cell::new(None) };
    static OPENED: RefCell<Vec<WindowHandle>> = const { RefCell::new(Vec::new()) };
    static CLOSED: Cell<u32> = const { Cell::new(0) };
}

#[component]
fn app() -> NodeHandle {
    let show = SHOW.with(|s| s.get()).unwrap();
    let title = String::from("Settings");
    rsx! {
        div {
            p { "outside" }
            if show.get() {
                Window {
                    title: title.clone(),
                    width: 320,
                    height: 240,
                    on_open: move |h: WindowHandle| OPENED.with(|o| o.borrow_mut().push(h)),
                    on_closed: move || {
                        CLOSED.with(|c| c.set(c.get() + 1));
                        show.set(false);
                    },
                    p { "inside the window" }
                }
            }
        }
    }
}

fn context() -> RinchContext {
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 400,
            height: 300,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        app,
    );
    ctx.update(&[]);
    ctx
}

fn doc_text(ctx: &RinchContext) -> String {
    let doc = ctx.app().doc().expect("mounted");
    let d = doc.borrow();
    rinch_dom::testing::get_text_content(&d.tree, d.tree.root_id)
}

fn opened() -> Vec<WindowHandle> {
    OPENED.with(|o| o.borrow().clone())
}

#[test]
fn a_window_element_opens_its_window_and_closes_it_as_its_state_says() {
    let show = rinch_core::reactive::unowned(|| Signal::new(true));
    SHOW.with(|s| s.set(Some(show)));
    let mut ctx = context();

    // Mounted: its window is open, and its children are not in this document.
    let first = *opened().last().expect("on_open was called with its handle");
    assert!(first.is_open());
    assert!(windows().contains(&first));
    let text = doc_text(&ctx);
    assert!(text.contains("outside"), "{text}");
    assert!(
        !text.contains("inside the window"),
        "built in the window's document: {text}"
    );

    // Its own close (here before it was made): on_closed is called once, and
    // clears the state that mounted it.
    first.close();
    ctx.update(&[]);
    assert!(!first.is_open());
    assert_eq!(CLOSED.with(|c| c.get()), 1);
    assert!(!show.get(), "on_closed cleared the state that mounted it");

    // Shown again: a new window.
    show.set(true);
    ctx.update(&[]);
    let second = *opened().last().unwrap();
    assert_ne!(second, first);
    assert!(second.is_open());

    // Its element going closes it, and on_closed is not called.
    show.set(false);
    ctx.update(&[]);
    assert!(!second.is_open());
    assert_eq!(CLOSED.with(|c| c.get()), 1, "not for the element going");
}
