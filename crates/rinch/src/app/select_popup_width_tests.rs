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

/// A popup placed by `probe`: its rows as (shaped label, row content box), and
/// the select's and the panel's painted x and width.
struct Placed {
    rows: Vec<(f32, f32)>,
    sel_x: f32,
    sel_w: f32,
    panel_x: f32,
    panel_w: f32,
}

/// Open a popup for `select(select_style) > option*` with `body_style` on the
/// body, at viewport `vp`. From the review of PR #1124, round 2.
fn probe(
    labels: &[&'static str],
    select_style: &'static str,
    body_style: &'static str,
    vp: (f32, f32),
) -> Placed {
    let sid: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let s2 = sid.clone();
    let labels: Vec<&'static str> = labels.to_vec();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let select = scope.create_element("select");
        select.set_attribute("style", select_style);
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
    app.mount_component(vp.0, vp.1);
    if !body_style.is_empty() {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let b = d.body();
        d.set_attribute(b, "style", body_style);
    }
    // A viewport change, so the second resolve is not an early return.
    app.resolve_and_repaint(vp.0 + 1.0, vp.1);
    app.resolve_and_repaint(vp.0, vp.1);
    let select = sid.get().unwrap();
    app.open_select_popup(select, vp.0, vp.1);
    app.resolve_and_repaint(vp.0, vp.1);
    let d = app.doc.as_ref().unwrap().borrow();
    let (sel_x, _, sel_w, _) = painted_element_box(&d.tree, select);
    let mut o = Placed {
        rows: vec![],
        sel_x,
        sel_w,
        panel_x: -1.0,
        panel_w: -1.0,
    };
    for (id, n) in d.tree.nodes.iter() {
        match n.attributes.get("class").map(|c| c.as_str()) {
            Some("rinch-nsel-panel") => {
                let (x, _, w, _) = painted_element_box(&d.tree, id);
                o.panel_x = x;
                o.panel_w = w;
            }
            Some("rinch-nsel-option") => {
                let cs = &n.computed_style;
                let content_w = n.layout.width - cs.padding_left.to_px() - cs.padding_right.to_px();
                let text_w = n
                    .text_layout
                    .as_ref()
                    .map(|l| l.layout.full_width())
                    .unwrap_or(-1.0);
                o.rows.push((text_w, content_w));
            }
            _ => {}
        }
    }
    o
}

/// A select whose widest row needs more room than is left before the
/// viewport's right edge gets a popup that stops at that edge, and is still no
/// narrower than the select. (The row then clips even with room to the left;
/// moving the popup left instead is #1132.) Kills a dropped cap and a cap at
/// `vp_w` rather than `vp_w - left`.
#[test]
fn the_popup_stops_at_the_viewports_right_edge() {
    let o = probe(
        &["A really very long option label indeed", "b"],
        "position: absolute; left: 540px; font-size: 12px",
        "",
        (800.0, 600.0),
    );
    assert!(o.sel_x + o.sel_w <= 800.0, "the select itself is on screen");
    assert!(o.panel_w >= o.sel_w, "never narrower than the select");
    let rows_need = o.rows.iter().map(|r| r.0).fold(0.0, f32::max) + 30.0;
    assert!(
        rows_need > 800.0 - o.sel_x,
        "positive control: the fixture needs the cap ({rows_need} vs {})",
        800.0 - o.sel_x
    );
    assert_eq!(o.panel_x + o.panel_w, 800.0, "stops at the viewport's edge");
}

/// #1130: the popup is measured from the body's family, size and weight only,
/// while its rows are painted by the ordinary IFC, which applies a body
/// `letter-spacing`; so such a row overflows its content box. Red until the
/// popup measures what it paints.
#[test]
#[ignore = "#1130: the popup measure ignores letter-spacing"]
fn a_body_letter_spacing_row_fits_1130() {
    let o = probe(
        &["Organisation settings", "Profile"],
        "font-size: 12px",
        "letter-spacing: 2px",
        (800.0, 600.0),
    );
    assert!(o.rows.iter().all(|(t, c)| t <= c), "{:?}", o.rows);
}
