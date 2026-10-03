//! A user's pick through the desktop `<select>` popup and a later
//! programmatic `selected`/`value` write are two different mechanisms rinch
//! tracks (`crates/rinch-dom/src/select.rs`'s module doc), and the select's
//! own `value` attribute used to win over live selectedness **unconditionally
//! once present** — so a user's pick could never be overtaken by a later
//! script write, where Chrome and rinch-web let the later write win (issue
//! #757). `select_selectedness_tests.rs` (rinch-dom) pins the DOM-level model
//! fix; this file is the part that model-level fixture cannot see: that the
//! real desktop pick path (`commit_select`, reached through the popup's
//! keyboard route) and a live script write race the same way through the
//! whole app, and that a script write made while the popup is still open
//! moves its highlight (the "vice versa" half of the fix).

use super::*;
use rinch_core::dom::NodeId;
use std::cell::Cell;

const VP: (f32, f32) = (800.0, 600.0);

struct Fixture {
    app: RinchApp,
    select: usize,
    options: [usize; 3],
}

/// The select's node id plus its three options', captured at mount.
type MountedIds = (usize, [usize; 3]);

/// A `<select>` with three plain options (values `va`/`vb`/`vc`), no
/// `selected` attribute in the markup — the default option (index 0) starts
/// selected, matching `select_with` in the rinch-dom fixtures.
fn mount() -> Fixture {
    let ids: Rc<Cell<Option<MountedIds>>> = Rc::new(Cell::new(None));
    let ids2 = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let select = scope.create_element("select");
        let mut opts = [0usize; 3];
        for (i, v) in ["va", "vb", "vc"].iter().enumerate() {
            let o = scope.create_element("option");
            o.set_attribute("value", v);
            let t = scope.create_text(&format!("o{i}"));
            o.append_child(&t);
            select.append_child(&o);
            opts[i] = o.node_id().0;
        }
        ids2.set(Some((select.node_id().0, opts)));
        select
    });
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    let (select, options) = ids.get().unwrap();
    Fixture {
        app,
        select,
        options,
    }
}

fn key(app: &mut RinchApp, key: KeyCode) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (VP.0 as u32, VP.1 as u32),
        1.0,
    );
}

fn resolved_index(app: &RinchApp, select: usize) -> Option<usize> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    rinch_dom::select::resolve_select_model(&d.tree, select).selected_index
}

/// The user picks option B (index 1) through the popup; a script then writes
/// `selected` on option C (index 2). The later write — the script — wins,
/// as it would in Chrome or rinch-web.
///
/// Before the fix: the select's `value` attribute (written by the pick) won
/// unconditionally once present, so the resolved index stayed `1` after the
/// script's write — pinned, pre-fix, by
/// `the_selects_value_attribute_still_outranks_selectedness` in
/// `rinch-dom`'s `select_selectedness_tests.rs`.
#[test]
fn a_users_pick_then_a_programmatic_write_the_write_wins() {
    let mut f = mount();
    f.app.open_select_popup(f.select, VP.0, VP.1);
    assert_eq!(f.app.open_select.as_ref().unwrap().highlighted, 0);

    // Arrow down once (index 0 -> 1) and commit: the user picks "b".
    key(&mut f.app, KeyCode::ArrowDown);
    key(&mut f.app, KeyCode::Enter);
    assert!(f.app.open_select.is_none(), "committing closes the popup");
    assert_eq!(
        resolved_index(&f.app, f.select),
        Some(1),
        "the pick is the selection — nothing fresher yet"
    );

    // A script selects option "c".
    {
        let doc = f.app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.set_attribute(NodeId(f.options[2]), "selected", "");
    }
    assert_eq!(
        resolved_index(&f.app, f.select),
        Some(2),
        "the later programmatic write outranks the earlier pick (#757)"
    );
}

/// The reverse order: a script selects option C first, then the user picks
/// option B through the popup. The pick — the later write — wins.
#[test]
fn a_programmatic_write_then_a_users_pick_the_pick_wins() {
    let mut f = mount();

    // A script selects option "c" first.
    {
        let doc = f.app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.set_attribute(NodeId(f.options[2]), "selected", "");
    }
    assert_eq!(resolved_index(&f.app, f.select), Some(2), "precondition");

    f.app.open_select_popup(f.select, VP.0, VP.1);
    assert_eq!(
        f.app.open_select.as_ref().unwrap().highlighted,
        2,
        "the popup opens on the currently resolved selection"
    );

    // Arrow up once (index 2 -> 1) and commit: the user picks "b".
    key(&mut f.app, KeyCode::ArrowUp);
    key(&mut f.app, KeyCode::Enter);
    assert!(f.app.open_select.is_none(), "committing closes the popup");
    assert_eq!(
        resolved_index(&f.app, f.select),
        Some(1),
        "the pick, being the later write, outranks the earlier programmatic write (#757)"
    );
}

/// A script write made while the popup is still open moves the popup's own
/// highlight (issue #757's "vice versa"), not just the model `commit_select`
/// would eventually read. The user's own arrow-key navigation, which never
/// touches the model's resolved selection, is untouched by this (mutant
/// check: comparing against `highlighted` instead of
/// `last_known_selected_index` would snap the highlight right back after the
/// `ArrowDown` below — see that assertion).
#[test]
fn a_programmatic_write_while_the_popup_is_open_moves_the_highlight() {
    let mut f = mount();
    f.app.open_select_popup(f.select, VP.0, VP.1);
    assert_eq!(f.app.open_select.as_ref().unwrap().highlighted, 0);

    // The user arrows down first — this must NOT be undone by the resync.
    key(&mut f.app, KeyCode::ArrowDown);
    assert_eq!(
        f.app.open_select.as_ref().unwrap().highlighted,
        1,
        "plain arrow-key navigation is unaffected by the resync"
    );

    // A script selects option "c" while the popup is still open.
    {
        let doc = f.app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.set_attribute(NodeId(f.options[2]), "selected", "");
    }
    f.app.resolve_and_repaint(VP.0, VP.1);

    let open = f.app.open_select.as_ref().expect("popup still open");
    assert_eq!(
        open.highlighted, 2,
        "the script's write moves the popup's highlight to the new selection"
    );
    let highlighted_id = open.option_ids[2];
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let n = d.tree.get(highlighted_id).unwrap();
    assert!(
        n.attributes.contains_key("data-highlighted"),
        "the DOM row carries the moved highlight marker"
    );
    assert!(
        n.attributes.contains_key("data-selected"),
        "and the moved selected marker"
    );
}
