//! ColorPicker: the `emitting` window (#283, review of PR #870).
//!
//! While the coordinating effect is inside a controlled `onchange`, an
//! external apply the handler provokes lands in the `emitting` window: it is
//! silent, and it arms no deferred-apply marker, because the flush that would
//! take the marker skips it. These two fixtures pin the window from the sides
//! the #283 tests do not: an apply of a value that is *not* the handler's
//! write-back, and a later author act that lands bit-exactly on the colour
//! the handler snapped to — which a leftover marker would swallow.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_picker::ColorPicker;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{
    ClickContext, EventHandlerId, dispatch_event, set_click_context, update_drag,
};
use rinch_core::{Component, InputCallback, Signal};

struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
    emissions: Rc<RefCell<Vec<String>>>,
}

fn mount(
    seed: &str,
    swatch: &str,
    value_fn: impl Fn() -> String + 'static,
    handler: impl Fn(&str) + 'static,
) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let emissions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = emissions.clone();
    let picker = ColorPicker {
        value: seed.to_string(),
        value_fn: Some(Rc::new(value_fn)),
        onchange: Some(InputCallback::new(move |value: String| {
            seen.borrow_mut().push(value.clone());
            handler(&value);
        })),
        alpha: true,
        with_input: true,
        swatches: vec![swatch.into()],
        ..Default::default()
    };
    let root = picker.render(&mut scope, &[]);
    Mounted {
        _doc: doc,
        _scope: scope,
        root,
        emissions,
    }
}

impl Mounted {
    fn emissions(&self) -> Vec<String> {
        self.emissions.borrow().clone()
    }
    fn displayed(&self) -> String {
        find_by_class(&self.root, "rinch-color-picker__hex-input")
            .unwrap()
            .get_attribute("value")
            .unwrap()
    }
    fn id(node: &NodeHandle) -> EventHandlerId {
        EventHandlerId(node.get_attribute("data-rid").unwrap().parse().unwrap())
    }
    fn press_saturation(&self, px: f32, py: f32) {
        set_click_context(ClickContext {
            mouse_x: px * 200.0,
            mouse_y: py * 200.0,
            element_x: 0.0,
            element_y: 0.0,
            element_width: 200.0,
            element_height: 200.0,
            ..Default::default()
        });
        let overlay = find_by_class(&self.root, "rinch-color-picker__saturation-overlay").unwrap();
        dispatch_event(Self::id(&overlay));
    }
    fn click_swatch(&self) {
        let swatch = find_by_class(&self.root, "rinch-color-picker__swatches")
            .unwrap()
            .children()
            .first()
            .unwrap()
            .clone();
        dispatch_event(Self::id(&swatch));
    }
}

fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    let matches = node
        .get_attribute("class")
        .is_some_and(|attr| attr.split_whitespace().any(|c| c == class));
    if matches {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_by_class(c, class))
}

/// The apply that lands inside the emitting window is NOT the
/// handler's write-back of the bound store — the handler writes the store AND
/// an unrelated override signal that `value_fn` prefers. The apply is still
/// silent, and the next act on the applied colour still reports.
#[test]
fn an_apply_of_an_unrelated_signal_inside_the_window_stays_silent_and_arms_nothing() {
    const OTHER: &str = "#3366cc";
    let store = Signal::new("#ff0000".to_string());
    let over: Signal<Option<String>> = Signal::new(None);
    let p = mount(
        "#ff0000",
        OTHER,
        move || over.get().unwrap_or_else(|| store.get()),
        move |v| {
            store.set(v.to_string());
            over.set(Some(OTHER.to_string()));
        },
    );
    p.press_saturation(0.5, 0.5);
    assert_eq!(p.emissions().len(), 1, "{:?}", p.emissions());
    assert_eq!(p.displayed(), OTHER, "the override applied");
    p.click_swatch();
    assert_eq!(
        p.emissions().len(),
        2,
        "the swatch click on the applied colour reports: {:?}",
        p.emissions()
    );
    assert_eq!(p.emissions()[1], OTHER);
}

/// A snapping handler on every drag frame. The second frame
/// lands bit-exactly on the snapped colour (#ff0000 = h0 s1 v1, and a drag to
/// the panel's top-right corner writes s = 1.0, v = 1.0 exactly), so a
/// leftover marker would swallow it.
#[test]
fn a_drag_frame_landing_on_the_snapped_colour_reports() {
    let store = Signal::new("#8844dd".to_string());
    let p = mount(
        "#8844dd",
        "#000000",
        move || store.get(),
        move |_| store.set("#ff0000".to_string()),
    );
    p.press_saturation(0.5, 0.5);
    assert_eq!(p.emissions().len(), 1);
    assert_eq!(p.displayed(), "#ff0000");
    update_drag(200.0, 0.0);
    assert_eq!(
        p.emissions().len(),
        2,
        "the frame on the snapped colour reports: {:?}",
        p.emissions()
    );
    update_drag(100.0, 100.0);
    assert_eq!(p.emissions().len(), 3, "{:?}", p.emissions());
}
