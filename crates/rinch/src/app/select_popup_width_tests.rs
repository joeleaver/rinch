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
    let sid: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
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
    /// Each row's text, and its computed (letter-spacing, word-spacing).
    row_text: Vec<String>,
    row_spacing: Vec<(f32, f32)>,
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
    let sid: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
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
        row_text: vec![],
        row_spacing: vec![],
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
                o.row_spacing.push((cs.letter_spacing, cs.word_spacing));
                if let Some(rinch_dom::node::NodeKind::Text(t)) = n
                    .children
                    .first()
                    .and_then(|&c| d.tree.get(c))
                    .map(|c| &c.kind)
                {
                    o.row_text.push(t.content.clone());
                }
            }
            _ => {}
        }
    }
    o
}

/// A select whose widest row needs more room than is left before the
/// viewport's right edge gets a popup that ends where the select ends,
/// right-aligned to it, as Chrome 155's does (#1132; measured: a 80px select at
/// x = 900 in a 1000px window opens a 464px popup whose right edge is the
/// select's, 980). The popup keeps the width its rows need, so no row clips.
/// Kills: the old cap at the viewport's edge (the panel shrinks and the widest
/// row clips), and a left shift that aligns the panel with the viewport's edge
/// instead of the select's.
#[test]
fn the_popup_moves_left_to_end_where_the_select_ends_1132() {
    let o = probe(
        &["A really very long option label indeed", "b"],
        "position: absolute; left: 540px; width: 200px; font-size: 12px",
        "",
        (800.0, 600.0),
    );
    assert_eq!((o.sel_x, o.sel_w), (540.0, 200.0));
    let rows_need = o.rows.iter().map(|r| r.0).fold(0.0, f32::max).ceil() + 30.0;
    assert!(
        rows_need > 800.0 - o.sel_x,
        "positive control: the rows need more than is left ({rows_need} vs {})",
        800.0 - o.sel_x
    );
    assert_eq!(
        o.panel_w, rows_need,
        "the popup keeps the width its rows need"
    );
    assert_eq!(
        o.panel_x + o.panel_w,
        740.0,
        "it ends where the select ends"
    );
    assert!(o.rows.iter().all(|(t, c)| t <= c), "{:?}", o.rows);
}

/// Where ending at the select's right edge would put the popup off the
/// viewport's left edge, it starts at 0; and a popup wider than the viewport
/// is the viewport's width. Kills: no clamp at 0, and no cap at the viewport.
#[test]
fn a_popup_that_cannot_fit_beside_the_select_stays_on_screen_1132() {
    let o = probe(
        &[
            "A really very long option label indeed, and longer still",
            "b",
        ],
        "position: absolute; left: 100px; width: 60px; font-size: 16px",
        "",
        (400.0, 600.0),
    );
    let rows_need = o.rows.iter().map(|r| r.0).fold(0.0, f32::max).ceil() + 30.0;
    assert!(rows_need > 160.0, "positive control: {rows_need}");
    if rows_need < 400.0 {
        assert_eq!((o.panel_x, o.panel_w), (0.0, rows_need));
    } else {
        assert_eq!((o.panel_x, o.panel_w), (0.0, 400.0));
    }
    let o = probe(
        &["Organisation settings and some more of them", "b"],
        "position: absolute; left: 100px; width: 60px; font-size: 16px",
        "",
        (200.0, 600.0),
    );
    assert_eq!(
        (o.panel_x, o.panel_w),
        (0.0, 200.0),
        "capped at the viewport"
    );
}

/// A popup that fits to the right of the select still opens at its left edge.
/// Kills: always right-aligning.
#[test]
fn a_popup_that_fits_opens_at_the_selects_left_edge_1132() {
    let o = probe(
        &["Organisation settings", "b"],
        "position: absolute; left: 100px; width: 60px; font-size: 12px",
        "",
        (800.0, 600.0),
    );
    assert!(o.panel_w > o.sel_w, "positive control: the popup is wider");
    assert_eq!(o.panel_x, 100.0);
}

/// #1130: Chrome 155's popup rows take neither letter- nor word-spacing (a
/// select with `letter-spacing: 3px` or `word-spacing: 30px` opens rows with
/// none), so neither does rinch's, whatever the body declares; and the row
/// still fits the box the popup was measured for. Before, the popup was
/// measured without the spacing a row then painted with, and the row
/// overflowed. Kills: rows inheriting the body's spacing.
#[test]
fn popup_rows_take_no_letter_or_word_spacing_1130() {
    let o = probe(
        &["Organisation settings", "Profile"],
        "font-size: 12px",
        "letter-spacing: 2px; word-spacing: 20px",
        (800.0, 600.0),
    );
    assert!(o.rows.iter().all(|(t, c)| t <= c), "{:?}", o.rows);
    assert!(
        o.row_spacing.iter().all(|&(l, w)| l == 0.0 && w == 0.0),
        "{:?}",
        o.row_spacing
    );
}

/// #1130: Chrome 155's popup rows take the select's `text-transform` (a select
/// with `text-transform: uppercase` lists `A`, `WWW… LONG LABEL`). The row is
/// the transformed text, and is measured as such.
/// Kills: the rows left untransformed, and a popup measured from the
/// untransformed label (the uppercase row overflows).
#[test]
fn popup_rows_take_the_selects_text_transform_1130() {
    let o = probe(
        &["wide window label", "b"],
        "text-transform: uppercase; font-size: 12px",
        "",
        (800.0, 600.0),
    );
    assert_eq!(o.row_text, ["WIDE WINDOW LABEL", "B"]);
    assert!(o.rows.iter().all(|(t, c)| t <= c), "{:?}", o.rows);
}

/// #1130: a `label` attribute is the label verbatim, its spaces included, in
/// the closed control and in the popup row alike; the row keeps them (it is
/// `white-space: pre`) and fits.
#[test]
fn a_label_attributes_spaces_reach_the_popup_row_1130() {
    let sid: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let s2 = sid.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let select = scope.create_element("select");
        let o = scope.create_element("option");
        o.set_attribute("label", "  x     y ");
        let t = scope.create_text("zz");
        o.append_child(&t);
        select.append_child(&o);
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
    let row = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("class").map(|c| c.as_str()) == Some("rinch-nsel-option"))
        .map(|(_, n)| n)
        .expect("a row");
    let text = d.tree.get(row.children[0]).unwrap();
    match &text.kind {
        rinch_dom::node::NodeKind::Text(t) => assert_eq!(t.content, "  x     y "),
        _ => panic!("row text"),
    }
    let laid = row.text_layout.as_ref().expect("row laid out");
    let cs = &row.computed_style;
    let content_w = row.layout.width - cs.padding_left.to_px() - cs.padding_right.to_px();
    assert!(laid.layout.full_width() <= content_w);
    // The spaces are laid out: wider than "x y" would be (two glyphs and one
    // space at 14px is well under 30px).
    assert!(
        laid.layout.full_width() > 30.0,
        "{}",
        laid.layout.full_width()
    );
}

// ---- review of #1744 probes ----

/// A select that itself runs past the viewport's right edge: its popup is kept
/// on screen, ending at the viewport's edge.
#[test]
fn review1744_a_select_past_the_right_edge_keeps_its_popup_on_screen() {
    let o = probe(
        &["abc", "b"],
        "position: absolute; left: 700px; width: 200px; font-size: 12px",
        "",
        (800.0, 600.0),
    );
    assert_eq!((o.sel_x, o.sel_w), (700.0, 200.0));
    assert_eq!(o.panel_w, 200.0);
    assert_eq!(
        o.panel_x + o.panel_w,
        800.0,
        "x {} w {}",
        o.panel_x,
        o.panel_w
    );
}

/// ß uppercases to SS in the row and the row fits.
#[test]
fn review1744_eszett_row() {
    let o = probe(
        &["straße straße straße", "b"],
        "text-transform: uppercase; font-size: 12px",
        "",
        (800.0, 600.0),
    );
    assert_eq!(o.row_text, ["STRASSE STRASSE STRASSE", "B"]);
    assert!(o.rows.iter().all(|(t, c)| t <= c), "{:?}", o.rows);
}

/// Type-ahead labels: a label attribute's leading spaces do not hide it.
#[test]
fn review1744_typeahead_labels_are_collapsed() {
    let sid: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let s2 = sid.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let select = scope.create_element("select");
        for (l, t) in [("  zeta  one", "a"), ("", "  beta   two ")] {
            let o = scope.create_element("option");
            if !l.is_empty() {
                o.set_attribute("label", l);
            }
            let tx = scope.create_text(t);
            o.append_child(&tx);
            select.append_child(&o);
        }
        root.append_child(&select);
        s2.set(Some(select.node_id().0));
        root
    });
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    app.open_select_popup(sid.get().unwrap(), VP.0, VP.1);
    let open = app.open_select.as_ref().unwrap();
    assert_eq!(open.labels, ["zeta one", "beta two"]);
    assert_eq!(open.values, ["a", "beta two"]);
}
