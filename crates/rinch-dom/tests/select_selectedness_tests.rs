//! An `<option>`'s **live selectedness**, and why it is not its `selected`
//! attribute (issue #692).
//!
//! HTML gives an option two states. The `selected` content attribute is its
//! *default* selectedness; the live state is what the `<select>` reports, and it
//! moves by one rule — "whenever an option's selectedness is set to true, every
//! other option of its select is set to false". The selected option is therefore
//! the last one **set**, which stops being the last one carrying the attribute
//! the moment two of them do.
//!
//! Desktop read the attribute alone (`rposition(|o| o.selected_attr)`), so it
//! answered the *earlier* option wherever the two rules disagreed, while the web
//! answered the later-set one — the divergence #692 reports.
//!
//! Every expectation here was measured in **Chrome 150**, headless, over raw DOM
//! calls on a real `<select>` (no rinch in the loop), reading `selectedIndex`
//! and each option's `.selected` after each step. The measurement script is in
//! the PR. Two of the rows corrected an assumption rather than confirming one:
//!
//! - **Re-writing a `selected` attribute that is already present does nothing**,
//!   whatever string it writes. Blink gates the selectedness update on the
//!   attribute's *transition* (`old_value.IsNull() != new_value.IsNull()`), so
//!   the issue's own "write it again on option 0" sequence leaves option 1
//!   selected, in Chrome as on desktop.
//! - **Removing `selected` from the selected option does not leave it
//!   selected.** It deselects, and the select then falls back to its first
//!   non-disabled option — which is a *different* answer from "option 0"
//!   whenever option 0 is disabled, and that is the case that says the model
//!   has to clear eagerly rather than let an earlier option's attribute
//!   resurface.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::select::resolve_select_model;

/// A `<select>` with `n` options labelled `o0`, `o1`, … Each option is built
/// **detached and then appended**, which is the order `rsx!` and a browser's
/// `createElement` + `setAttribute` + `appendChild` both use.
fn select_with(n: usize, disabled: &[usize]) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.append_child(body, sel);
    let mut opts = Vec::new();
    for i in 0..n {
        let o = doc.create_element("option");
        doc.set_attribute(o, "value", &format!("v{i}"));
        if disabled.contains(&i) {
            doc.set_attribute(o, "disabled", "");
        }
        doc.append_child(sel, o);
        let t = doc.create_text(&format!("o{i}"));
        doc.append_child(o, t);
        opts.push(o);
    }
    (doc, sel, opts)
}

fn selected(doc: &RinchDocument, sel: NodeId) -> Option<usize> {
    resolve_select_model(&doc.tree, sel.0).selected_index
}

/// The reported bug: two options carrying `selected`, and the selection belongs
/// to the one written **last**, not the one later in the document.
///
/// Chrome 150: writing `selected` on option 1 and then option 0 leaves
/// `selectedIndex == 0` with both attributes present. Desktop answered 1.
///
/// Red at `5f16cb0` on the final assertion (`Some(1)`).
///
/// Kills two mutants: restoring `rposition(|o| o.selected_attr)`, and dropping
/// the "clear the others" loop from `set_option_selectedness` — without it both
/// options stay selected and the tie-break answers 1 again.
#[test]
fn the_last_option_set_wins_not_the_last_one_carrying_the_attribute() {
    let (mut doc, sel, o) = select_with(3, &[]);
    assert_eq!(selected(&doc, sel), Some(0), "precondition: the default");

    doc.set_attribute(o[1], "selected", "");
    assert_eq!(selected(&doc, sel), Some(1), "the first write selects it");

    doc.set_attribute(o[0], "selected", "");
    assert_eq!(
        doc.get_attribute(o[1], "selected").as_deref(),
        Some(""),
        "the later option keeps its attribute — the attribute is the option's \
         default and a browser leaves it alone, so the two rules really are \
         looking at different state"
    );
    assert_eq!(
        selected(&doc, sel),
        Some(0),
        "the selection belongs to the option written last (Chrome 150: 0)"
    );
}

/// Writing `selected` on an option that already has it changes nothing —
/// including with a different string, which is what says the gate is the
/// attribute's *presence transition* and not its value.
///
/// Chrome 150 for both spellings: 0, then 1, then still 1.
///
/// Kills the mutant that drops the `!contains_key` gate and re-selects on every
/// write: that answers 0.
#[test]
fn re_writing_a_present_selected_attribute_does_not_move_the_selection() {
    for rewrite in ["", "selected"] {
        let (mut doc, sel, o) = select_with(3, &[]);
        doc.set_attribute(o[0], "selected", "");
        assert_eq!(selected(&doc, sel), Some(0));
        doc.set_attribute(o[1], "selected", "");
        assert_eq!(selected(&doc, sel), Some(1));

        doc.set_attribute(o[0], "selected", rewrite);
        assert_eq!(
            selected(&doc, sel),
            Some(1),
            "re-writing `selected=\"{rewrite}\"` on an option that already \
             carries it must not move the selection (Chrome 150: 1)"
        );
    }
}

/// Deselecting the selected option hands the select back to its first
/// **non-disabled** option, not to whichever earlier option still carries a
/// `selected` attribute.
///
/// Chrome 150, two options carrying `selected` with option 0 disabled:
/// `selectedIndex` is 1, and removing option 1's attribute leaves it at 1 —
/// option 1 is re-selected by HTML's "ask for a reset" because option 0 cannot
/// take it.
///
/// This is the fixture that discriminates between clearing selectedness eagerly
/// and letting an earlier option's attribute resurface: the lazy model answers
/// 0 here. The disabled first option is the whole point — with an enabled one
/// both models answer 0 and the test would be sitting on a fixed point.
#[test]
fn deselecting_falls_back_to_the_first_enabled_option_not_to_a_stale_attribute() {
    let (mut doc, sel, o) = select_with(3, &[0]);
    doc.set_attribute(o[0], "selected", "");
    doc.set_attribute(o[1], "selected", "");
    assert_eq!(selected(&doc, sel), Some(1), "precondition");

    doc.remove_attribute(o[1], "selected");
    assert_eq!(
        doc.get_attribute(o[0], "selected").as_deref(),
        Some(""),
        "option 0 still carries the attribute — which is exactly what must not \
         bring it back"
    );
    assert_eq!(
        selected(&doc, sel),
        Some(1),
        "option 0 is disabled, so the fallback is the first enabled option \
         (Chrome 150: 1)"
    );

    // And the enabled twin, for the other direction: with nothing selected and
    // option 0 available, the fallback is option 0.
    let (mut doc, sel, o) = select_with(3, &[]);
    doc.set_attribute(o[1], "selected", "");
    doc.remove_attribute(o[1], "selected");
    assert_eq!(selected(&doc, sel), Some(0), "Chrome 150: 0");
}

/// An option **appended** while it already carries selectedness takes the
/// selection from the option that had it — the order `rsx!` builds in, and a
/// browser's `createElement` + `setAttribute` + `appendChild`.
///
/// Chrome 150, markup `<option disabled selected>a<option selected>b<option>c`:
/// `selectedIndex` is 1, and removing option 1's attribute leaves it at 1.
///
/// The first assertion alone would sit on a fixed point: with no insertion rule
/// at all both options keep their selectedness and the tie-break answers the
/// last one in tree order — the same index. What tells the two apart is
/// deselecting the winner afterwards, and a **disabled** first option, so that
/// "fall back to the first enabled option" and "option 0's stale selectedness
/// resurfaces" are different answers. Measured: without it this fixture passes
/// against a build that applies no insertion rule at all.
///
/// Kills the mutant that drops `options_inserted` from `append_child`.
#[test]
fn an_option_appended_already_selected_takes_the_selection_from_the_one_there() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.append_child(body, sel);
    let mut o = Vec::new();
    for i in 0..3 {
        let opt = doc.create_element("option");
        doc.set_attribute(opt, "value", &format!("v{i}"));
        if i == 0 {
            doc.set_attribute(opt, "disabled", "");
        }
        if i < 2 {
            // Set while the option is still detached, as markup and `rsx!` do.
            doc.set_attribute(opt, "selected", "");
        }
        doc.append_child(sel, opt);
        o.push(opt);
    }
    assert_eq!(
        selected(&doc, sel),
        Some(1),
        "the option appended last with selectedness wins (Chrome 150: 1)"
    );

    doc.remove_attribute(o[1], "selected");
    assert_eq!(
        doc.get_attribute(o[0], "selected").as_deref(),
        Some(""),
        "option 0 still carries its attribute"
    );
    assert_eq!(
        selected(&doc, sel),
        Some(1),
        "but it lost its selectedness when option 1 arrived, so the fallback is          the first enabled option and not option 0 (Chrome 150: 1)"
    );
}

/// An option carrying selectedness takes the selection with it when it is
/// **inserted**, even ahead of an option that already has it.
///
/// Chrome 150: a select whose option 1 is selected, given a `selected` option
/// inserted at the front, answers `selectedIndex == 0`. That is what says the
/// rule is "last inserted" rather than "last in tree order" — the two agree for
/// every append, and part company here.
///
/// Kills the mutant that drops `options_inserted` from `insert_child`.
#[test]
fn an_option_inserted_before_an_already_selected_one_takes_the_selection() {
    let (mut doc, sel, o) = select_with(2, &[]);
    doc.set_attribute(o[1], "selected", "");
    assert_eq!(selected(&doc, sel), Some(1), "precondition");

    let fresh = doc.create_element("option");
    doc.set_attribute(fresh, "value", "vz");
    doc.set_attribute(fresh, "selected", "");
    doc.insert_child(sel, fresh, 0);

    let model = resolve_select_model(&doc.tree, sel.0);
    assert_eq!(model.options.len(), 3);
    assert_eq!(
        model.selected_index,
        Some(0),
        "the inserted option is first in tree order AND the last inserted; \
         Chrome 150 gives it the selection"
    );
}

/// A whole `<optgroup>` arriving at once: the **last** selected option in it
/// takes the selection, which is what each of its options running its own
/// insertion steps in tree order leaves.
///
/// Chrome 150, appending an `<optgroup>` holding two `selected` options to a
/// select whose only option was selected: `selectedIndex == 2`, the group's
/// second option, with `.selected` true on it alone.
///
/// Kills the mutant that walks the inserted subtree's options in reverse
/// (which answers 1) — the single-option fixtures cannot, since reversing a
/// one-element list changes nothing.
#[test]
fn the_last_selected_option_of_an_inserted_optgroup_takes_the_selection() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.append_child(body, sel);
    let first = doc.create_element("option");
    doc.set_attribute(first, "value", "a");
    doc.append_child(sel, first);
    assert_eq!(selected(&doc, sel), Some(0), "precondition");

    // The group is built whole, detached, and appended in one go.
    let group = doc.create_element("optgroup");
    for v in ["b", "c"] {
        let o = doc.create_element("option");
        doc.set_attribute(o, "value", v);
        doc.set_attribute(o, "selected", "");
        doc.append_child(group, o);
    }
    doc.append_child(sel, group);

    let model = resolve_select_model(&doc.tree, sel.0);
    assert_eq!(model.options.len(), 3, "the group is flattened");
    assert_eq!(
        model.selected_index,
        Some(2),
        "the group's last selected option wins (Chrome 150: 2)"
    );
}

/// The same rule reaches an option inside an `<optgroup>`, whose select is its
/// grandparent.
///
/// Without the `<optgroup>` hop in `owning_select` the "clear the others" step
/// finds no select and both options stay selected, so the tie-break answers the
/// later one — the pre-#692 answer, one level down.
#[test]
fn the_rule_reaches_an_option_inside_an_optgroup() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.append_child(body, sel);
    let group = doc.create_element("optgroup");
    doc.append_child(sel, group);
    let mut opts = Vec::new();
    for i in 0..3 {
        let o = doc.create_element("option");
        doc.set_attribute(o, "value", &format!("v{i}"));
        doc.append_child(group, o);
        opts.push(o);
    }

    doc.set_attribute(opts[2], "selected", "");
    assert_eq!(
        selected(&doc, sel),
        Some(2),
        "grouped options are flattened"
    );
    doc.set_attribute(opts[1], "selected", "");
    assert_eq!(
        selected(&doc, sel),
        Some(1),
        "the option set last wins inside an <optgroup> too"
    );
}

/// The select's own `value` attribute still outranks selectedness.
///
/// That ordering is rinch's, not HTML's — a browser has no `value` content
/// attribute on `<select>` at all (Chrome 150 ignores one: `selectedIndex`
/// stays 0 and `.value` stays the first option's). It is how the desktop popup
/// records a user's pick (`app/select_widget.rs` writes the select's `value`),
/// so a programmatic `selected` write afterwards must not silently undo it.
#[test]
fn the_selects_value_attribute_still_outranks_selectedness() {
    let (mut doc, sel, o) = select_with(3, &[]);
    doc.set_attribute(o[2], "selected", "");
    assert_eq!(selected(&doc, sel), Some(2), "precondition");

    // The user picks option 1 through the popup.
    doc.set_attribute(sel, "value", "v1");
    assert_eq!(selected(&doc, sel), Some(1), "the pick wins");

    // A later programmatic `selected` write moves selectedness but not the pick.
    doc.set_attribute(o[0], "selected", "");
    assert_eq!(
        selected(&doc, sel),
        Some(1),
        "the select's `value` still answers first"
    );

    // Clear the pick and the selectedness underneath it shows through.
    doc.remove_attribute(sel, "value");
    assert_eq!(selected(&doc, sel), Some(0));
}
