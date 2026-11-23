//! Issue #2247 — a `draggable="true"` element is `user-select: none`, as in
//! Chrome 158, so a selection across selectable text leaves a draggable
//! inline's text out of the highlight and the copy.
//!
//! Measured in Chrome 158 inside `user-select: text`: `getComputedStyle` of a
//! `<span draggable="true">` (and of `draggable="TRUE"`) is `none`, and of an
//! element inside it `none`; `draggable=""`, `"false"` and `"auto"` stay
//! `text`; an author `user-select` on the draggable wins (`text`, `all`). The
//! markup `<p>aa <span draggable="true">dd<b>bb</b></span> zz</p>` selected
//! whole gives `getSelection().toString()` `"aa  zz"`.

use super::hit_testing::painted_element_box;
use super::text_selection_across_tests::{drag, mount, node, press, release};
use super::*;
use rinch_dom::computed_style::UserSelectValue;

const SEL: &str = "user-select: text; font: 16px/20px sans-serif; width: 600px";

/// Press at the start of `#a`'s first line, drag past the end of its box.
fn select_all_of(app: &mut RinchApp, id: &str) -> String {
    let (x, y, w, h) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node(app, id))
    };
    let (a, b) = ((x, y + 10.0), (x + w - 1.0, y + h - 5.0));
    press(app, a);
    drag(app, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
    drag(app, b);
    release(app, b);
    app.text_selection_text().unwrap_or_default()
}

fn resolved(app: &RinchApp, id: &str) -> UserSelectValue {
    let d = app.doc.as_ref().unwrap().borrow();
    super::text_selection::resolved_user_select(&d.tree, node(app, id))
}

#[test]
fn a_draggable_inline_is_left_out_of_a_selection_across_it() {
    let mut app = mount(
        r#"<p id="a">aa <span id="d" draggable="true">dd<b id="b">bb</b></span> zz</p>"#,
        SEL,
    );
    assert_eq!(select_all_of(&mut app, "a"), "aa  zz");
}

/// The control: the same markup without `draggable` copies whole.
#[test]
fn the_same_span_without_draggable_is_selected() {
    let mut app = mount(
        r#"<p id="a">aa <span id="d">dd<b id="b">bb</b></span> zz</p>"#,
        SEL,
    );
    assert_eq!(select_all_of(&mut app, "a"), "aa ddbb zz");
}

#[test]
fn draggable_resolves_user_select_as_chrome_does() {
    let app = mount(
        r#"<p><span id="d1" draggable="true">x<b id="d1b">y</b></span><span id="d2" draggable="TRUE">x</span><span id="d3" draggable="">x</span><span id="d4" draggable="false">x</span><span id="d5" draggable="auto">x</span><span id="plain">x</span><span id="d6" draggable="true" style="user-select: text">x</span><span id="d7" draggable="true" style="user-select: all">x</span></p><div id="d8" draggable="true">blk</div>"#,
        SEL,
    );
    let got: Vec<(&str, UserSelectValue)> = [
        "d1", "d1b", "d2", "d3", "d4", "d5", "plain", "d6", "d7", "d8",
    ]
    .into_iter()
    .map(|id| (id, resolved(&app, id)))
    .collect();
    use UserSelectValue as U;
    assert_eq!(
        got,
        [
            ("d1", U::None),
            ("d1b", U::None),
            ("d2", U::None),
            ("d3", U::Text),
            ("d4", U::Text),
            ("d5", U::Text),
            ("plain", U::Text),
            ("d6", U::Text),
            ("d7", U::All),
            ("d8", U::None),
        ]
    );
}
