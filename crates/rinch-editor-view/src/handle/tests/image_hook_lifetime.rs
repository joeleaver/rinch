use super::*;
use crate::images::{ImageHover, clear_image_source, image_source, set_image_source};
use crate::registry::set_image_hover;
use rinch_core::reactive::{Effect, ElementBounds, Scope, Signal};
use std::cell::Cell;

fn hov() -> ImageHover {
    ImageHover {
        pos: Pos(1),
        attrs: rinch_editor_core::Attrs::new(),
        rect: ElementBounds {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        },
    }
}

/// R1: an image-hover callback registered by a component that has
/// since unmounted must not run (the #147/#183 rule the other hooks keep).
#[test]
fn r1_image_hover_callback_stops_with_its_scope() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let scope = Scope::new();
    let fired = Rc::new(Cell::new(0u32));
    scope.run(|| {
        let sig = Signal::new(0u32);
        let fired = fired.clone();
        h.on_image_hover(move |_| {
            fired.set(fired.get() + 1);
            let _ = sig.get();
        });
    });
    set_image_hover(Some(5), Some((h.clone(), hov())));
    assert_eq!(fired.get(), 1, "control: live");
    scope.dispose();
    set_image_hover(Some(5), None);
    assert_eq!(
        fired.get(),
        1,
        "a disposed component's callback must not run"
    );
}

/// R2: a source function installed during a render that has since
/// unmounted must not run (install_scoped_slot's lifetime rule).
#[test]
fn r2_image_source_installed_in_a_scope_is_released_with_it() {
    let scope = Scope::new();
    scope.run(|| {
        let sig = Signal::new(String::from("x"));
        set_image_source(move |_| Some(sig.get()));
    });
    let attrs =
        rinch_editor_core::Attrs::new().with("src", rinch_editor_core::AttrValue::from("a.png"));
    assert_eq!(image_source(&attrs), "x", "control: live");
    scope.dispose();
    let got = image_source(&attrs);
    clear_image_source();
    assert_eq!(got, "a.png");
}

/// R3: a source function that writes a signal (a memo cache, a counter)
/// whose effect touches the editor: does a load panic?
#[test]
fn r3_source_fn_writing_a_signal_observed_by_an_editor_effect() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let asked = Signal::new(0u32);
    let seen = Rc::new(Cell::new(0usize));
    let (h2, seen2) = (h.clone(), seen.clone());
    let _e = Effect::new(move || {
        let _ = asked.get();
        seen2.set(h2.doc().content_size());
    });
    set_image_source(move |_| {
        asked.update(|n| *n += 1);
        None
    });
    let ok = h.load_html(r#"<p>a<img src="x.png">b</p>"#);
    clear_image_source();
    assert!(ok);
    assert!(seen.get() > 0);
}
