//! Native `<select>` closed-control model + layout (issue #121, PR1).

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::select::resolve_select_model;

/// One option spec: `(value_attr, label, extra_attrs)`.
type OptSpec<'a> = (Option<&'a str>, &'a str, &'a [(&'a str, &'a str)]);

/// Build a `<select>` with the given options and an optional `value` attribute on
/// the select itself. Returns (doc, select_id).
fn build_select(select_value: Option<&str>, options: &[OptSpec]) -> (RinchDocument, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    if let Some(v) = select_value {
        doc.set_attribute(sel, "value", v);
    }
    doc.append_child(body, sel);
    for (value, label, extra) in options {
        let o = doc.create_element("option");
        if let Some(v) = value {
            doc.set_attribute(o, "value", v);
        }
        for (k, val) in *extra {
            doc.set_attribute(o, k, val);
        }
        doc.append_child(sel, o);
        let t = doc.create_text(label);
        doc.append_child(o, t);
    }
    (doc, sel.0)
}

#[test]
fn model_collects_options_with_value_and_label() {
    let (doc, sel) = build_select(
        None,
        &[
            (Some("a"), "Apple", &[]),
            (Some("b"), "Banana", &[]),
            (None, "Cherry", &[]), // no value attr → value falls back to label
        ],
    );
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.options.len(), 3);
    assert_eq!(m.options[0].value, "a");
    assert_eq!(m.options[0].label, "Apple");
    assert_eq!(
        m.options[2].value, "Cherry",
        "value defaults to text content"
    );
    assert_eq!(m.options[2].label, "Cherry");
}

#[test]
fn first_option_is_selected_by_default() {
    let (doc, sel) = build_select(
        None,
        &[(Some("a"), "Apple", &[]), (Some("b"), "Banana", &[])],
    );
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.selected_index, Some(0));
    assert_eq!(m.selected_label(), Some("Apple"));
}

#[test]
fn selected_attribute_wins_over_default() {
    let (doc, sel) = build_select(
        None,
        &[
            (Some("a"), "Apple", &[]),
            (Some("b"), "Banana", &[("selected", "")]),
            (Some("c"), "Cherry", &[]),
        ],
    );
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.selected_index, Some(1));
    assert_eq!(m.selected_label(), Some("Banana"));
}

#[test]
fn select_value_attribute_wins_over_selected_attribute() {
    // The app writes the chosen value to the select's `value` attribute; it must
    // take precedence over any stale `selected` attribute in the markup.
    let (doc, sel) = build_select(
        Some("c"),
        &[
            (Some("a"), "Apple", &[]),
            (Some("b"), "Banana", &[("selected", "")]),
            (Some("c"), "Cherry", &[]),
        ],
    );
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.selected_index, Some(2));
    assert_eq!(m.selected_label(), Some("Cherry"));
}

#[test]
fn last_selected_attribute_wins_when_several() {
    let (doc, sel) = build_select(
        None,
        &[
            (Some("a"), "Apple", &[("selected", "")]),
            (Some("b"), "Banana", &[("selected", "")]),
        ],
    );
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.selected_index, Some(1));
}

#[test]
fn label_attribute_overrides_text_content() {
    let (doc, sel) = build_select(
        None,
        &[(Some("a"), "Apple text", &[("label", "Apple label")])],
    );
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.options[0].label, "Apple label");
    // value still comes from the value attr (or text) — not the label attr.
    assert_eq!(m.options[0].value, "a");
}

#[test]
fn optgroup_options_are_flattened() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.append_child(body, sel);

    let group = doc.create_element("optgroup");
    doc.set_attribute(group, "label", "Fruit");
    doc.append_child(sel, group);
    for (v, l) in [("a", "Apple"), ("b", "Banana")] {
        let o = doc.create_element("option");
        doc.set_attribute(o, "value", v);
        doc.append_child(group, o);
        let t = doc.create_text(l);
        doc.append_child(o, t);
    }
    // a loose option after the group
    let o = doc.create_element("option");
    doc.set_attribute(o, "value", "c");
    doc.append_child(sel, o);
    let t = doc.create_text("Cherry");
    doc.append_child(o, t);

    let m = resolve_select_model(&doc.tree, sel.0);
    assert_eq!(m.options.len(), 3, "optgroup children flattened in order");
    assert_eq!(m.options[0].value, "a");
    assert_eq!(m.options[2].value, "c");
}

#[test]
fn empty_select_has_no_selection() {
    let (doc, sel) = build_select(None, &[]);
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.selected_index, None);
    assert_eq!(m.selected_label(), None);
}

#[test]
fn disabled_first_option_is_skipped_for_default() {
    let (doc, sel) = build_select(
        None,
        &[
            (Some("a"), "Pick one", &[("disabled", "")]),
            (Some("b"), "Banana", &[]),
        ],
    );
    let m = resolve_select_model(&doc.tree, sel);
    assert_eq!(m.selected_index, Some(1), "skip the disabled first option");
}

#[test]
fn options_do_not_lay_out_as_stacked_text() {
    // The core bug: option children must not render as visible stacked/inline
    // text. With `option { display: none }` they contribute no layout box.
    let (mut doc, sel) = build_select(
        None,
        &[
            (Some("a"), "Apple", &[]),
            (Some("b"), "Banana Longname", &[]),
            (Some("c"), "Cherry", &[]),
        ],
    );
    doc.resolve_layout(1000.0, 800.0);

    let sel_layout = doc.tree.get(sel).unwrap().layout;
    // The control is a single row tall — not three options stacked.
    assert!(
        sel_layout.height < 40.0,
        "closed select should be one control-height row, got {}",
        sel_layout.height
    );
    for &opt_id in &doc.tree.get(sel).unwrap().children.clone() {
        let o = doc.tree.get(opt_id).unwrap();
        assert_eq!(
            (o.layout.width, o.layout.height),
            (0.0, 0.0),
            "option must not lay out (display:none)"
        );
    }
}

#[test]
fn unstyled_select_does_not_collapse() {
    // A raw <select> with no CSS must still be a visible control (min-width +
    // padding), not collapse the way a raw <input> does.
    let (mut doc, sel) = build_select(None, &[(Some("a"), "Apple", &[])]);
    doc.resolve_layout(1000.0, 800.0);
    let l = doc.tree.get(sel).unwrap().layout;
    assert!(
        l.width >= 60.0,
        "min-width keeps the control visible, got {}",
        l.width
    );
    assert!(l.height > 0.0, "control has height, got {}", l.height);
}

#[test]
fn unstyled_select_width_tracks_widest_option() {
    // The closed control sizes to the widest option (browser behaviour), so a
    // longer set of options yields a wider control — and the width does not
    // depend on which option is selected.
    let (mut narrow, n_sel) = build_select(None, &[(Some("a"), "Hi", &[])]);
    narrow.resolve_layout(1000.0, 800.0);
    let narrow_w = narrow.tree.get(n_sel).unwrap().layout.width;

    let (mut wide, w_sel) = build_select(
        None,
        &[
            (Some("a"), "Hi", &[]),
            (Some("b"), "A much longer option label", &[]),
        ],
    );
    wide.resolve_layout(1000.0, 800.0);
    let wide_w = wide.tree.get(w_sel).unwrap().layout.width;

    assert!(
        wide_w > narrow_w + 50.0,
        "wider options → wider control: narrow={narrow_w}, wide={wide_w}"
    );
}

#[test]
fn explicit_width_is_respected_over_intrinsic() {
    // An author width wins — the label clips rather than forcing the control wide.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.set_attribute(sel, "style", "width: 90px;");
    doc.append_child(body, sel);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let t = doc.create_text("An option label far wider than ninety pixels");
    doc.append_child(o, t);

    doc.resolve_layout(1000.0, 800.0);
    let w = doc.tree.get(sel.0).unwrap().layout.width;
    assert!((w - 90.0).abs() < 1.0, "explicit width respected, got {w}");
}
