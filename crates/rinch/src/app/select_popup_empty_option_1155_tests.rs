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

// Review of PR #1162: the replacement reaches only an empty label, and the
// full row is a hit target.

use std::cell::RefCell;

/// A mounted `select` whose options are `(value, label, disabled)`, and the
/// values its `data-oninput` handler was handed.
struct Mounted {
    app: RinchApp,
    picks: Rc<RefCell<Vec<String>>>,
}

fn mount(
    root_style: &'static str,
    select_style: &'static str,
    opts: &[(&'static str, &'static str, bool)],
) -> Mounted {
    let sel: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let s2 = sel.clone();
    let picks: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let p2 = picks.clone();
    let opts: Vec<(&'static str, &'static str, bool)> = opts.to_vec();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", root_style);
        let select = scope.create_element("select");
        select.set_attribute("style", select_style);
        for (v, l, disabled) in &opts {
            let o = scope.create_element("option");
            o.set_attribute("value", v);
            if *disabled {
                o.set_attribute("disabled", "");
            }
            if !l.is_empty() {
                let t = scope.create_text(l);
                o.append_child(&t);
            }
            select.append_child(&o);
        }
        let hid = scope.register_input_handler(move |v| p2.borrow_mut().push(v));
        select.set_attribute("data-oninput", &hid.0.to_string());
        root.append_child(&select);
        s2.set(Some(select.node_id().0));
        root
    });
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    let select = sel.get().unwrap();
    app.open_select_popup(select, VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    assert!(app.is_select_open());
    Mounted { app, picks }
}

/// Each open row's DOM text and painted box.
fn rows(m: &Mounted) -> Vec<(String, (f32, f32, f32, f32))> {
    let open = m.app.open_select.as_ref().unwrap();
    let d = m.app.doc.as_ref().unwrap().borrow();
    open.option_ids
        .iter()
        .map(|&id| {
            (
                rinch_dom::testing::get_text_content(&d.tree, id),
                super::hit_testing::painted_element_box(&d.tree, id),
            )
        })
        .collect()
}

/// Only an empty label is replaced: a one-letter label, a disabled labelled
/// option and an ordinary one keep their text.
#[test]
fn labelled_rows_keep_their_text() {
    let m = mount(
        "",
        "",
        &[
            ("", "", false),
            ("a", "A", false),
            ("off", "Off", true),
            ("one", "One", false),
            ("ws", "   ", false),
        ],
    );
    let t: Vec<String> = rows(&m).into_iter().map(|r| r.0).collect();
    assert_eq!(
        t,
        vec![
            "\u{200B}".to_string(),
            "A".into(),
            "Off".into(),
            "One".into(),
            "\u{200B}".into()
        ]
    );
}

/// The whole full row is a hit target: a press 25px into the empty row (past
/// the old 12px sliver) picks the empty option, not the next one.
#[test]
fn a_press_low_in_the_empty_row_picks_it() {
    let mut m = mount(
        "padding: 20px",
        "width: 200px; height: 30px",
        &[("x", "Ex", false), ("", "", false), ("one", "One", false)],
    );
    let (x, y, w, _) = rows(&m)[1].1;
    let (px, py) = (x + w / 2.0, y + 25.0);
    m.app.handle_event(
        PlatformEvent::MouseDown {
            x: px,
            y: py,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    m.app.handle_event(
        PlatformEvent::MouseUp {
            x: px,
            y: py,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    assert_eq!(*m.picks.borrow(), vec!["".to_string()]);
}
