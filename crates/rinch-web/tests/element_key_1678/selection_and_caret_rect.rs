//! The selection read in UTF-16 units and the caret rect in client px, for
//! focused, unfocused and hidden controls.

use super::*;

/// The selection reads back as the browser holds it, in UTF-16 units, with
/// its direction; a fresh control reads `(0, 0, …)` as desktop answers it.
#[wasm_bindgen_test]
fn selection_range_reads_the_controls_own_selection() {
    let f = Fixture::mount();
    let (s, e, d) = f.textarea.selection_range().expect("a text control");
    assert_eq!((s, e), (0, 0), "a fresh control");
    // Chrome's direction for a caret nobody set; desktop answers the same.
    assert_eq!(d, SelectionDirection::Forward, "fresh direction");

    let ta: web_sys::HtmlTextAreaElement = f.el("k-ta").dyn_into().unwrap();
    ta.set_value("a😀bc");
    ta.set_selection_range_with_direction(3, 5, "backward")
        .unwrap();
    assert_eq!(
        f.textarea.selection_range(),
        Some((3, 5, SelectionDirection::Backward))
    );
    f.textarea
        .set_selection_range(1, 3, SelectionDirection::Forward);
    assert_eq!(
        f.textarea.selection_range(),
        Some((1, 3, SelectionDirection::Forward))
    );
    f.teardown();
}

/// The caret rect is in client px, at the control's content box: offset 0
/// at the box's left + border + padding, offset 2 two monospace cells
/// further, the start of line 2 one `line-height` (20px) lower; a
/// single-line field's caret is inside its box.
#[wasm_bindgen_test]
fn caret_rect_is_in_client_px_at_the_offset() {
    let f = Fixture::mount();
    let ta: web_sys::HtmlTextAreaElement = f.el("k-ta").dyn_into().unwrap();
    ta.set_value("ab\ncd");
    let r = f.el("k-ta").get_bounding_client_rect();

    let c0 = f.textarea.caret_rect_at(0).expect("laid out");
    let c2 = f.textarea.caret_rect_at(2).unwrap();
    let c3 = f.textarea.caret_rect_at(3).unwrap();
    assert!(
        (c0.x as f64 - (r.left() + 2.0 + 6.0)).abs() < 0.5,
        "{c0:?} {}",
        r.left()
    );
    // The caret is the text's own height (the font's content area, a little
    // less than the 20px line), centred in its line.
    assert!(c0.height > 15.0 && c0.height <= 20.0, "{c0:?}");
    let line_top = r.top() + 2.0 + 4.0;
    assert!(
        (c0.y as f64 - (line_top + (20.0 - c0.height as f64) / 2.0)).abs() < 1.0,
        "{c0:?} {line_top}"
    );

    let probe = document().create_element("span").unwrap();
    probe
        .set_attribute("style", "font: 16px/20px monospace; white-space: pre")
        .unwrap();
    probe.set_text_content(Some("ab"));
    document().body().unwrap().append_child(&probe).unwrap();
    let ab = probe.get_bounding_client_rect().width() as f32;
    probe.remove();
    assert!((c2.x - c0.x - ab).abs() < 0.5, "{c0:?} {c2:?} ab={ab}");
    assert!(
        (c3.x - c0.x).abs() < 0.5 && (c3.y - c0.y - 20.0).abs() < 0.5,
        "{c0:?} {c3:?}"
    );

    // The head: a caret set at 4 is drawn where offset 4 is.
    f.textarea
        .set_selection_range(4, 4, SelectionDirection::Forward);
    let head = f.textarea.caret_rect().unwrap();
    let at4 = f.textarea.caret_rect_at(4).unwrap();
    assert_eq!((head.x, head.y), (at4.x, at4.y));

    let ir = f.el("k-in").get_bounding_client_rect();
    let ic = f.input.caret_rect().expect("an empty field has a caret");
    assert!(
        (ic.x as f64 - (ir.left() + 1.0 + 8.0)).abs() < 0.5,
        "{ic:?} box left {} clientLeft {} padding-left {:?}",
        ir.left(),
        f.el("k-in").client_left(),
        web_sys::window()
            .unwrap()
            .get_computed_style(&f.el("k-in"))
            .unwrap()
            .unwrap()
            .get_property_value("padding-left")
    );
    assert!(
        (ic.y as f64) > ir.top() && ((ic.y + ic.height) as f64) < ir.bottom(),
        "{ic:?} in {}..{}",
        ir.top(),
        ir.bottom()
    );
    f.teardown();
}

/// Review of #1686, F3/F5/F7: an unfocused control with a value reads what
/// desktop reads (Chrome's answers), an unspecified direction reads
/// `Forward`, and a `display: none` field has no caret rect.
#[wasm_bindgen_test]
fn an_unfocused_controls_selection_and_a_hidden_caret() {
    let (root, host, hs) = mount_with(|__scope| {
        let r = rsx! {
            div {
                input { value: "abc" }
                input {}
                textarea {}
                input { r#type: "email", value: "a@b" }
                textarea { "hello" }
                input { style: "display: none", value: "abc" }
            }
        };
        let mut k = r.children();
        k.push(r.clone());
        (r, k)
    });
    let fwd = SelectionDirection::Forward;
    assert_eq!(
        hs[0].selection_range(),
        Some((3, 3, fwd)),
        "input value=abc"
    );
    assert_eq!(hs[1].selection_range(), Some((0, 0, fwd)), "empty input");
    hs[1].set_attribute("value", "hello");
    assert_eq!(
        hs[1].selection_range(),
        Some((5, 5, fwd)),
        "after a value write"
    );
    hs[1].set_selection_range(1, 2, fwd);
    hs[1].set_attribute("value", "hello world");
    assert_eq!(
        hs[1].selection_range(),
        Some((11, 11, fwd)),
        "a write moves it to the end"
    );
    hs[2].set_attribute("value", "xy");
    assert_eq!(
        hs[2].selection_range(),
        Some((2, 2, fwd)),
        "textarea value write"
    );
    assert_eq!(hs[3].selection_range(), None, "email");
    assert_eq!(
        hs[4].selection_range(),
        Some((0, 0, fwd)),
        "textarea text child"
    );
    hs[1].set_selection_range(2, 4, SelectionDirection::None);
    assert_eq!(
        hs[1].selection_range(),
        Some((2, 4, fwd)),
        "None reads Forward"
    );
    assert_eq!(hs[5].caret_rect(), None, "display: none");
    assert!(hs[0].caret_rect().is_some(), "positive control");
    // Parsed markup sets the value with no setter: Chrome 155 reads a caret
    // at the start, and so does desktop.
    let parent = hs[6].clone();
    parent.set_inner_html("<input value=\"abcd\">");
    let parsed = parent.children()[0].clone();
    assert_eq!(parsed.selection_range(), Some((0, 0, fwd)), "parsed input");
    root.unmount();
    host.remove();
}
