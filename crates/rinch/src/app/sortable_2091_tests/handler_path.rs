//! A plugin host applies the guest's DOM ops and proxies only its `data-*`
//! handler attributes (Pimble's `wasm32-wasip1` plugins): no pointer
//! capture, no runtime drag state, no dismiss stack. This drives a drag
//! and a keyboard move by reading the attributes and dispatching the
//! handlers they name, as such a host does.
use super::*;
use rinch_core::events::{self, ClickContext, EventHandlerId, KeyEventData};

fn handler(f: &Fixture, id: u64, attr: &str) -> EventHandlerId {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut at = Some(id);
    while let Some(n) = at {
        if let Some(v) = d.tree.nodes[n].attributes.get(attr) {
            return EventHandlerId(v.parse().unwrap());
        }
        at = d.tree.nodes[n].parent;
    }
    panic!("no {attr} at or above {id}")
}

fn context(f: &Fixture, id: u64, (x, y): (f32, f32)) {
    let (ex, ey, ew, eh) = box_of(f, id);
    events::set_click_context(ClickContext {
        mouse_x: x,
        mouse_y: y,
        element_x: ex,
        element_y: ey,
        element_width: ew,
        element_height: eh,
        ..Default::default()
    });
}

#[test]
fn a_drag_runs_on_the_element_handlers_alone() {
    let f = fixture();
    let a = item(&f, "a");
    // Read now, as a host keeps the drag source's handlers: the drop
    // re-renders the lanes, and the nodes they were are freed (#723).
    let dragend = handler(&f, a, "data-ondragend");
    events::dispatch_event(handler(&f, a, "data-ondragstart"));
    let d = item(&f, "d");
    // The target's too, for the same reason: `dragleave` follows the drop.
    let dragleave = handler(&f, d, "data-ondragleave");
    let at = point(&f, d, 0.5, 0.75);
    context(&f, d, at);
    events::dispatch_event(handler(&f, d, "data-ondragover"));
    events::dispatch_event(handler(&f, a, "data-ondragmove"));
    let expected = mv("cards", &["a"], "c1", "c2", place(Some("d"), None, 1));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    events::dispatch_event(handler(&f, d, "data-ondrop"));
    events::dispatch_event(dragleave);
    events::dispatch_event(dragend);
    assert_eq!(drops(), vec![expected]);
    assert!(f.app.active_dnd.is_none(), "the runtime's drag never ran");
}

#[test]
fn a_keyboard_move_runs_on_the_key_handler_alone() {
    let f = fixture();
    let a = item(&f, "a");
    let keys = handler(&f, a, "data-onkeydown");
    let press = |k: &str, code: &str| {
        events::dispatch_key_handler(keys, &KeyEventData::new(k, code));
    };
    press(" ", "Space");
    press("ArrowDown", "ArrowDown");
    press("ArrowDown", "ArrowDown");
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("c"), Some("i"), 2)
    );
    press("Enter", "Enter");
    assert_eq!(
        drops(),
        vec![mv(
            "cards",
            &["a"],
            "c1",
            "c1",
            place(Some("c"), Some("i"), 2)
        )]
    );
    // Escape too is the element's: a second lift is cancelled by it.
    let a = item(&f, "a");
    let keys = handler(&f, a, "data-onkeydown");
    events::dispatch_key_handler(keys, &KeyEventData::new(" ", "Space"));
    assert!(classes(&f, a).contains("rinch-sortable__item--lifted"));
    events::dispatch_key_handler(keys, &KeyEventData::new("Escape", "Escape"));
    assert!(!classes(&f, a).contains("rinch-sortable__item--lifted"));
}
