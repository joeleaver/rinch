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
//!    option, and what the `value:` rsx prop sets);
//! 2. otherwise the last option carrying a `selected` attribute;
//! 3. otherwise the first non-disabled option — a single `<select>` always has a
//!    selected option in a browser, defaulting to the first.
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
    /// Whether the option carries a `selected` attribute.
    pub selected_attr: bool,
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
                out.push(SelectOption {
                    node_id: child_id,
                    value,
                    label,
                    selected_attr: child.attributes.contains_key("selected"),
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
    // 2. Last option with a `selected` attribute (a single select keeps the last
    //    one when markup mistakenly marks several).
    if let Some(i) = options.iter().rposition(|o| o.selected_attr) {
        return Some(i);
    }
    // 3. First non-disabled option, else the first option.
    options.iter().position(|o| !o.disabled).or(Some(0))
}
