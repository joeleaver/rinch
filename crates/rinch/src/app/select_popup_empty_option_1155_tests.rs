//! An empty `<option>` gets a full row in the native `<select>` popup (#1155).
//!
//! `<option value=""></option>` is the usual "choose…" placeholder. Its popup
//! row holds an empty label, which generates no line box, so the row was only
//! its padding (12px) against 32px for a labelled row: a sliver that was hard
//! to hit and whose highlight was a line. Chrome's popup gives an empty option
//! a full row.
//!
//! The row's `font-size` and `line-height` are declared by the popup's own
//! stylesheet, so the heights compared here are declarations, not measurements
//! of the host's fonts.
use super::*;
use std::cell::Cell;

const VP: (f32, f32) = (800.0, 600.0);

/// Open the popup for `select > option*` and return each row's border-box
/// height, in option order.
fn row_heights(labels: &[&'static str]) -> Vec<f32> {
    let sid: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let s2 = sid.clone();
    let labels: Vec<&'static str> = labels.to_vec();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let select = scope.create_element("select");
        for l in &labels {
            let o = scope.create_element("option");
            if !l.is_empty() {
                let t = scope.create_text(l);
                o.append_child(&t);
            }
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
    let open = app.open_select.as_ref().expect("the popup opened");
    let d = app.doc.as_ref().unwrap().borrow();
    open.option_ids
        .iter()
        .map(|&id| d.tree.get(id).unwrap().layout.height)
        .collect()
}

/// Rows are 31.6px (12px of padding and a 19.6px line), and laid-out boxes are
/// pixel-snapped, so a row's rounded height depends on where it starts: 32, 31,
/// 32, ... The exact comparison is therefore row for row against a popup of
/// the same length whose every option is labelled.
fn assert_rows_match_labelled(labels: &[&'static str]) {
    let h = row_heights(labels);
    let labelled = row_heights(&vec!["One"; labels.len()]);
    assert_eq!(h.len(), labels.len(), "one row per option: {h:?}");
    // Positive control: a labelled row is its line plus its 12px of padding.
    assert!(
        labelled.iter().all(|&r| r > 12.0 + 10.0),
        "a labelled row has a line box: {labelled:?}"
    );
    assert_eq!(h, labelled, "{labels:?} against all-labelled rows");
}

#[test]
fn an_empty_option_row_is_as_tall_as_a_labelled_one() {
    assert_rows_match_labelled(&["", "One", ""]);
}

/// A label of spaces is trimmed to nothing, like an empty one, and is given the
/// same full row.
#[test]
fn a_whitespace_only_option_row_is_as_tall_as_a_labelled_one() {
    assert_rows_match_labelled(&["   ", "One"]);
}
