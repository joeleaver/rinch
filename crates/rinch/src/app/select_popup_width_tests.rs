//! The native `<select>` popup is at least as wide as the closed control and
//! wide enough for its widest row (#1098, review of PR #1124).
//!
//! Since #1098 the closed control is exactly `ceil(label) + 22` wide at the
//! select's own font, so a panel of that width leaves no room for its own 30px
//! of chrome (border 1 + padding 4 each side, row padding 10 each side), and the
//! rows are set at 14px in the body's font rather than the select's: a long
//! label at a smaller select font was cut off. Chrome's popup is at least the
//! select's width and grows to fit its content.
//!
//! The assertion is relative (the row's shaped text against the row's own
//! content box), so it does not pin the host's font set.
use super::*;
use std::cell::Cell;

const VP: (f32, f32) = (800.0, 600.0);

/// Open a popup for `select > option*` under a `font` wrapper and return, per
/// row, (shaped label width, row content-box width), plus the select's width
/// and the panel's.
fn popup_rows(labels: &[&'static str], font: &'static str) -> (Vec<(f32, f32)>, f32, f32) {
    popup_rows_styled(labels, font, "")
}

fn popup_rows_styled(
    labels: &[&'static str],
    font: &'static str,
    select_style: &'static str,
) -> (Vec<(f32, f32)>, f32, f32) {
    let sid: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let s2 = sid.clone();
    let labels: Vec<&'static str> = labels.to_vec();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", font);
        let select = scope.create_element("select");
        if !select_style.is_empty() {
            select.set_attribute("style", select_style);
        }
        for l in &labels {
            let o = scope.create_element("option");
            let t = scope.create_text(l);
            o.append_child(&t);
            select.append_child(&o);
        }
        root.append_child(&select);
        s2.set(Some(select.node_id().0));
        root
    });
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    let select = sid.get().unwrap();
    app.open_select_popup(select, VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    let d = app.doc.as_ref().unwrap().borrow();
    let sel_w = d.tree.get(select).unwrap().layout.width;
    let mut panel_w = -1.0;
    let mut rows = vec![];
    for (_, n) in d.tree.nodes.iter() {
        match n.attributes.get("class").map(|c| c.as_str()) {
            Some("rinch-nsel-panel") => panel_w = n.layout.width,
            Some("rinch-nsel-option") => {
                let cs = &n.computed_style;
                let content_w = n.layout.width - cs.padding_left.to_px() - cs.padding_right.to_px();
                let text_w = n
                    .text_layout
                    .as_ref()
                    .map(|l| l.layout.full_width())
                    .expect("a popup row lays its label out");
                rows.push((text_w, content_w));
            }
            _ => {}
        }
    }
    (rows, sel_w, panel_w)
}

#[test]
fn popup_rows_fit_their_labels() {
    for (labels, font) in [
        (&["a", "b"][..], "font: 16px sans-serif"),
        (&["Medium", "Small"][..], "font: 16px sans-serif"),
        (&["Yes", "No"][..], "font: 13px sans-serif"),
        (
            &["Organisation settings", "Profile"][..],
            "font: 12px sans-serif",
        ),
    ] {
        let (rows, sel_w, panel_w) = popup_rows(labels, font);
        assert_eq!(rows.len(), labels.len(), "{labels:?}: one row per option");
        // Never narrower than the control it drops from.
        assert!(
            panel_w >= sel_w,
            "{labels:?} {font}: panel {panel_w} narrower than the select {sel_w}"
        );
        for (text, content) in &rows {
            assert!(
                *text > 0.0 && text <= content,
                "{labels:?} {font}: label {text} does not fit its row's content box {content} \
                 (select {sel_w}, panel {panel_w})"
            );
        }
    }
}

/// The widening is only as much as the rows need: a select wider than any row
/// keeps the popup at its own width, as Chrome's does.
#[test]
fn a_wide_select_keeps_its_popup_at_its_own_width() {
    let (rows, sel_w, panel_w) = popup_rows_styled(&["a", "b"], "", "width: 400px");
    assert_eq!(rows.len(), 2);
    assert_eq!(sel_w, 400.0);
    assert_eq!(panel_w, 400.0, "a 400px select's popup is 400px");
}
