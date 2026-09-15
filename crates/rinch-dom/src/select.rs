//! Shared model for the native `<select>` form control.
//!
//! `<select>` on the desktop backend is rendered by rinch-dom (the closed
//! control — the selected option's label plus a dropdown arrow) while the
//! interactive popup, focus and keyboard handling live in the app/shell layer
//! (issue #121), mirroring how native `<input>` is split. Both layers need the
//! same answer to "what are the options and which one is selected", so that
//! resolution lives here, once.
//!
//! Selection follows HTML semantics, resolved in this order:
//! 1. the option whose value equals the select's own `value` attribute, if the
//!    select has one (this is what the app writes back when the user picks an
//!    option, and what the `value:` rsx prop sets — HTML has no `value` content
//!    attribute on `<select>` at all, so this step is rinch's own);
//! 2. otherwise the option whose **selectedness** is set — see
//!    [`set_option_selectedness`];
//! 3. otherwise the first non-disabled option — a single `<select>` always has a
//!    selected option in a browser, defaulting to the first. This is also what
//!    stands in for HTML's "ask for a reset": deselect the selected option and
//!    the select falls back here, exactly as a browser does.
//!
//! Step 2 used to read "the last option carrying a `selected` attribute", which
//! is not what a browser tracks (issue #692). The attribute is only an option's
//! *default* selectedness; the live state moves with the last option **set**, and
//! the two part company as soon as two options carry the attribute at once. See
//! [`set_option_selectedness`] for the rule and the Chrome 150 measurements
//! behind it.
//!
//! `<optgroup>`s are flattened: their `<option>` children are collected in
//! document order as if the group weren't there. (Rendering the group *labels*
//! in the popup is an app-layer concern; the model only needs the options.)

use crate::node::{NodeKind, NodeTree, RawNodeId};

/// One resolved `<option>` of a `<select>`.
#[derive(Debug, Clone)]
pub struct SelectOption {
    /// DOM node id of the `<option>` element.
    pub node_id: RawNodeId,
    /// Submit value: the `value` attribute, or the label if there is no `value`
    /// attribute (HTML falls back to the text content).
    pub value: String,
    /// Display label: the `label` attribute if present, else the trimmed text
    /// content.
    pub label: String,
    /// Whether the option carries a `selected` attribute — its *default*
    /// selectedness (`defaultSelected` in the IDL), which is not in general the
    /// live state. [`SelectOption::selectedness`] is what the selection is
    /// resolved from.
    pub selected_attr: bool,
    /// The option's **live selectedness**: [`Node::selectedness`] where
    /// something has set it, and the attribute's presence where nothing has.
    ///
    /// [`Node::selectedness`]: crate::node::Node::selectedness
    pub selectedness: bool,
    /// Whether the option is `disabled`.
    pub disabled: bool,
}

/// The resolved options of a `<select>` plus which one is currently selected.
#[derive(Debug, Clone, Default)]
pub struct SelectModel {
    pub options: Vec<SelectOption>,
    /// Index into `options` of the selected option, or `None` when the select
    /// has no (enabled) options at all.
    pub selected_index: Option<usize>,
}

impl SelectModel {
    /// The currently selected option, if any.
    pub fn selected(&self) -> Option<&SelectOption> {
        self.selected_index.and_then(|i| self.options.get(i))
    }

    /// The label to show in the closed control, if an option is selected.
    pub fn selected_label(&self) -> Option<&str> {
        self.selected().map(|o| o.label.as_str())
    }
}

/// Resolve a `<select>`'s options and current selection from the DOM.
///
/// Returns an empty model (no options, `selected_index == None`) if `select_id`
/// is not a `<select>` element.
pub fn resolve_select_model(tree: &NodeTree, select_id: RawNodeId) -> SelectModel {
    let mut options = Vec::new();
    let Some(select) = tree.get(select_id) else {
        return SelectModel::default();
    };
    if select.tag() != Some("select") {
        return SelectModel::default();
    }

    collect_options(tree, select_id, &mut options);

    // The select's own `value` attribute is authoritative when present.
    let select_value = select.attributes.get("value").map(|s| s.as_str());

    let selected_index = resolve_selected_index(&options, select_value);

    SelectModel {
        options,
        selected_index,
    }
}

/// Walk `<option>` children in document order, descending into `<optgroup>`.
fn collect_options(tree: &NodeTree, parent_id: RawNodeId, out: &mut Vec<SelectOption>) {
    let Some(parent) = tree.get(parent_id) else {
        return;
    };
    for &child_id in &parent.children {
        let Some(child) = tree.get(child_id) else {
            continue;
        };
        match child.tag() {
            Some("option") => {
                let label_text = element_text(tree, child_id);
                let label = child
                    .attributes
                    .get("label")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| label_text.trim().to_string());
                // HTML: an option's value defaults to its text content when the
                // `value` attribute is absent.
                let value = child
                    .attributes
                    .get("value")
                    .cloned()
                    .unwrap_or_else(|| label_text.trim().to_string());
                let selected_attr = child.attributes.contains_key("selected");
                out.push(SelectOption {
                    node_id: child_id,
                    value,
                    label,
                    selected_attr,
                    // Nothing has set this option's selectedness, so its
                    // attribute still speaks for it — the state every option is
                    // in until something writes `selected` or the option is
                    // inserted into a select already carrying it.
                    selectedness: child.selectedness.unwrap_or(selected_attr),
                    disabled: child.attributes.contains_key("disabled"),
                });
            }
            Some("optgroup") => collect_options(tree, child_id, out),
            _ => {}
        }
    }
}

/// Concatenate the text of a subtree (an option's visible label).
fn element_text(tree: &NodeTree, id: RawNodeId) -> String {
    let mut buf = String::new();
    fn walk(tree: &NodeTree, id: RawNodeId, buf: &mut String) {
        let Some(node) = tree.get(id) else { return };
        if let NodeKind::Text(t) = &node.kind {
            buf.push_str(&t.content);
        }
        for &child in &node.children {
            walk(tree, child, buf);
        }
    }
    walk(tree, id, &mut buf);
    buf
}

fn resolve_selected_index(options: &[SelectOption], select_value: Option<&str>) -> Option<usize> {
    if options.is_empty() {
        return None;
    }
    // 1. Match the select's `value` attribute.
    if let Some(val) = select_value
        && let Some(i) = options.iter().position(|o| o.value == val)
    {
        return Some(i);
    }
    // 2. The option whose selectedness is set (#692). At most one is, wherever
    //    `set_option_selectedness` has run — it clears the others, as HTML does
    //    — so the `rposition` is a tie-break for the one state it does not
    //    reach: a subtree built straight into `Node::attributes` and never
    //    inserted option-by-option, where several options fall back to their
    //    attributes at once. Last in tree order is the browser's answer there
    //    (`<option selected>a<option selected>b` selects `b`, measured).
    if let Some(i) = options.iter().rposition(|o| o.selectedness) {
        return Some(i);
    }
    // 3. First non-disabled option, else the first option.
    options.iter().position(|o| !o.disabled).or(Some(0))
}

// ── the live half: who is selected, and when that moves ──────────────────────

/// Set `option_id`'s selectedness, applying HTML's exclusivity rule (#692).
///
/// > Whenever an option element's selectedness is set to true, if its nearest
/// > ancestor `select` element has the `multiple` attribute absent, the user
/// > agent must set the selectedness of all the other option elements in its
/// > list of options to false.
///
/// So the selected option is the last one **set**, not the last one carrying a
/// `selected` attribute — the attribute is only the option's default. The two
/// agree until two options carry it at once, which is the whole of #692.
///
/// Measured in Chrome 150 (`selectedIndex` after each step, three options, none
/// selected in the markup):
///
/// | sequence | Chrome | desktop before #692 |
/// |---|---|---|
/// | write `selected` on option 1, then option 0 | **0** | 1 |
/// | write on 0, then 1, then 0 again | 1 | 1 |
/// | remove `selected` from the selected option 1 | 0 | 0 |
///
/// Row 2 is why this runs on the **transition** into the attribute rather than
/// on every write: re-writing an attribute that is already present changes
/// nothing in a browser (Blink gates on `old_value.IsNull() != new_value.IsNull()`),
/// and it changed nothing here before either. Row 3 needs no rule of its own —
/// deselecting the last selected option leaves `resolve_selected_index` at step
/// 3, which is what HTML's "ask for a reset" does.
///
/// `multiple` is not honoured: rinch's `<select>` model is single-selection
/// throughout (`selected_index` is one `Option<usize>`), so a `multiple` select
/// already collapsed to one selected option before this existed.
pub(crate) fn set_option_selectedness(tree: &mut NodeTree, option_id: RawNodeId, on: bool) {
    let Some(node) = tree.get_mut(option_id) else {
        return;
    };
    node.selectedness = Some(on);
    if !on {
        return;
    }
    // An option set while it is detached — which is every option `rsx!` builds,
    // since it writes the attributes before appending — has no select to clear.
    // `options_inserted` runs the same rule when it arrives in one.
    let Some(select_id) = owning_select(tree, option_id) else {
        return;
    };
    let mut others = Vec::new();
    collect_option_ids(tree, select_id, &mut others);
    for id in others {
        if id != option_id
            && let Some(other) = tree.get_mut(id)
        {
            other.selectedness = Some(false);
        }
    }
}

/// Apply the same rule for a subtree that has just been inserted (#692).
///
/// An option carrying selectedness joins its select's list when it is inserted,
/// and takes the selection with it — Chrome 150, measured: appending a
/// `selected` option to a select that already has one selects the appended one,
/// and so does *inserting it first*, which is what says the rule is "last
/// inserted" rather than "last in tree order".
///
/// Applied per option in tree order, so the last selected option of an inserted
/// `<optgroup>` wins, the way each of them running its own insertion steps would
/// leave it — Chrome 150, appending a group of two `selected` options to a
/// select whose only option was selected: `selectedIndex == 2`, the group's
/// second option.
pub(crate) fn options_inserted(tree: &mut NodeTree, inserted: RawNodeId) {
    // The cheap gate: `append_child` runs for every node in the document, and
    // only an `<option>` or a group of them can carry selectedness.
    match tree.get(inserted).and_then(|n| n.tag()) {
        Some("option") | Some("optgroup") => {}
        _ => return,
    }
    let mut ids = Vec::new();
    if tree.get(inserted).and_then(|n| n.tag()) == Some("option") {
        ids.push(inserted);
    } else {
        collect_option_ids(tree, inserted, &mut ids);
    }
    // Read the whole subtree's selectedness **first**. Applying the rule to one
    // option clears every other option of the select, its own not-yet-applied
    // siblings included, so a loop that re-read the flag as it went would let
    // the first selected option of an inserted `<optgroup>` wipe the rest and
    // then keep the selection itself — the reverse of what a browser does.
    let pending: Vec<RawNodeId> = ids
        .into_iter()
        .filter(|&id| tree.get(id).and_then(|n| n.selectedness) == Some(true))
        .collect();
    for id in pending {
        set_option_selectedness(tree, id, true);
    }
}

/// The `<select>` whose list of options `option_id` belongs to: its parent, or
/// its `<optgroup>`'s parent. `None` while the option is detached, or parented
/// anywhere else.
fn owning_select(tree: &NodeTree, option_id: RawNodeId) -> Option<RawNodeId> {
    let parent = tree.get(option_id)?.parent?;
    match tree.get(parent)?.tag() {
        Some("select") => Some(parent),
        Some("optgroup") => {
            let grandparent = tree.get(parent)?.parent?;
            (tree.get(grandparent)?.tag() == Some("select")).then_some(grandparent)
        }
        _ => None,
    }
}

/// The ids of `parent`'s `<option>` descendants in document order, flattening
/// `<optgroup>` exactly as [`collect_options`] does.
fn collect_option_ids(tree: &NodeTree, parent_id: RawNodeId, out: &mut Vec<RawNodeId>) {
    let Some(parent) = tree.get(parent_id) else {
        return;
    };
    for &child_id in &parent.children {
        let Some(child) = tree.get(child_id) else {
            continue;
        };
        match child.tag() {
            Some("option") => out.push(child_id),
            Some("optgroup") => collect_option_ids(tree, child_id, out),
            _ => {}
        }
    }
}
