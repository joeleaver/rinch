//! #1684: a click in a text field never puts the caret inside a grapheme
//! cluster. Parley gives a caret stop to every character of a cluster (one
//! per character of a ligature glyph — an emoji ZWJ sequence, a flag, `❤️` —
//! or of a run the host draws as separate glyphs), and the click used to take
//! whichever one was nearest; it now goes to the nearer edge of the cluster.
//!
//! The sweep presses at every pixel across the field, so it holds whatever
//! the host's emoji face is; the positive control is that it reached both
//! edges of every cluster and an interior stop answers one of them.

use super::*;

const VALUE: &str = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}b\u{2764}\u{FE0F}c\u{1F1EB}\u{1F1F7}d1\u{FE0F}\u{20E3}e\u{301}f";
const STYLE: &str = "display: block; width: 700px; font: 32px/40px sans-serif; padding: 0; border: 0";

fn page(tag: &'static str) -> (RinchApp, usize) {
    let id = Rc::new(Cell::new(0usize));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute("style", STYLE);
        field.set_attribute("value", VALUE);
        root.append_child(&field);
        id_in.set(field.node_id().0);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, id.get())
}

fn sweep(app: &mut RinchApp, field: usize) -> Vec<usize> {
    let (bx, by, bw, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, field)
    };
    let y = by + 20.0;
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let mut out = Vec::new();
    let mut x = bx;
    while x < bx + bw {
        out.push(RinchApp::compute_input_cursor_from_click(
            &d.tree,
            &mut app.hit_test_font_cx,
            &mut app.hit_test_layout_cx,
            field,
            x,
            y,
        ));
        x += 0.5;
    }
    out
}

fn check(tag: &'static str) {
    let (mut app, field) = page(tag);
    let offsets = sweep(&mut app, field);
    let inside: Vec<usize> = offsets
        .iter()
        .copied()
        .filter(|&o| !rinch_words::is_grapheme_boundary(VALUE, o))
        .collect();
    assert!(inside.is_empty(), "{tag}: carets inside a cluster at {inside:?}");
    // Positive control: every boundary of the value was reached.
    let boundaries: Vec<usize> =
        (0..=VALUE.len()).filter(|&b| rinch_words::is_grapheme_boundary(VALUE, b)).collect();
    for b in boundaries {
        assert!(offsets.contains(&b), "{tag}: the sweep never reached boundary {b}");
    }
}

#[test]
fn a_click_in_an_input_lands_on_a_cluster_edge_1684() {
    check("input");
}

#[test]
fn a_click_in_a_textarea_lands_on_a_cluster_edge_1684() {
    check("textarea");
}

/// The snap goes to the NEARER edge: a stop inside the family emoji answers
/// its start from the left half and its end from the right half.
#[test]
fn the_snap_takes_the_nearer_edge_1684() {
    let (mut app, field) = page("input");
    let offsets = sweep(&mut app, field);
    let family = (1, 1 + "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".len());
    let first_end = offsets.iter().position(|&o| o == family.1).unwrap();
    let last_start = offsets.iter().rposition(|&o| o == family.0).unwrap();
    // The answers switch once, from the start to the end.
    assert!(last_start < first_end, "{last_start} {first_end}");
    assert!(offsets[last_start..=first_end].iter().all(|&o| o == family.0 || o == family.1));
}
