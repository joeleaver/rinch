//! Selecting several rows: clicks, runs and the hidden-row rules.

use super::*;

// =============================================================================
// Selecting several rows
// =============================================================================

impl Shape {
    /// The rows drawn from `a` to `b` (either way round), top to bottom. An
    /// end hidden inside a collapsed row counts as the nearest row drawn
    /// above it, its shown ancestor.
    pub(super) fn run(&self, expanded: &HashSet<String>, a: usize, b: usize) -> Vec<usize> {
        let visible = self.visible(expanded);
        let at = |mut row: usize| loop {
            if let Some(i) = visible.iter().position(|&r| r == row) {
                return Some(i);
            }
            row = self.rows[row].parent?;
        };
        let (Some(i), Some(j)) = (at(a), at(b)) else {
            return vec![b];
        };
        let (from, to) = if i <= j { (i, j) } else { (j, i) };
        visible[from..=to].to_vec()
    }

    /// What a drag or a move of `row` carries: the selection, top to bottom,
    /// when `row` is one of several selected rows (each selected row inside
    /// another selected one travels inside it, so it is left out); `row`
    /// alone otherwise.
    pub(super) fn carried(
        &self,
        expanded: &HashSet<String>,
        selected: &HashSet<String>,
        row: usize,
        multiple: bool,
    ) -> Vec<usize> {
        if !multiple || selected.len() < 2 || !selected.contains(&self.rows[row].value) {
            return vec![row];
        }
        let chosen: Vec<usize> = self
            .visible(expanded)
            .into_iter()
            .filter(|&r| selected.contains(&self.rows[r].value))
            .collect();
        chosen
            .iter()
            .copied()
            .filter(|&r| {
                !chosen
                    .iter()
                    .any(|&other| other != r && self.is_within(r, other))
            })
            .collect()
    }

    /// The rows drawn right above and right below the block `rows` (and
    /// everything under them).
    pub(super) fn around(
        &self,
        expanded: &HashSet<String>,
        rows: &[usize],
    ) -> (Option<usize>, Option<usize>) {
        let visible = self.visible(expanded);
        let inside = |r: usize| rows.iter().any(|&b| self.is_within(r, b));
        let first = visible.iter().position(|&r| inside(r));
        let above = first.and_then(|i| i.checked_sub(1)).map(|i| visible[i]);
        let last = visible.iter().rposition(|&r| inside(r));
        let below = last.and_then(|i| visible[i + 1..].iter().copied().find(|&r| !inside(r)));
        (above, below)
    }
}

/// What a drag or a key move from the row of `value` carries: the rows it
/// moves (tree order), and the values it carries, through the app's
/// `drag_values` for each of those rows, each value once (an app whose
/// `drag_values` returns the whole selection for any row carries it once).
pub(super) fn carried_from(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    value: &str,
) -> (Vec<usize>, Vec<String>) {
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let rows = match shape.index(value) {
        Some(row) => {
            let expanded = untracked(|| tree_state.expanded.get());
            let selected = untracked(|| tree_state.selected.get());
            shape.carried(&expanded, &selected, row, tree_state.controller.multiple)
        }
        None => Vec::new(),
    };
    let values: Vec<String> = if rows.is_empty() {
        vec![value.to_string()]
    } else {
        rows.iter().map(|&r| shape.rows[r].value.clone()).collect()
    };
    let carried = match &config.drag_values {
        Some(each) => {
            let mut seen = HashSet::new();
            values
                .iter()
                .flat_map(|v| each(v))
                .filter(|v| seen.insert(v.clone()))
                .collect()
        }
        None => values,
    };
    (rows, carried)
}

/// How a click (or Enter) on a row changes the selection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Pick {
    /// A plain click: the row alone.
    Replace,
    /// Ctrl- (or Cmd-) click in a `multiple` tree: the row in or out.
    Toggle,
    /// Shift-click in a `multiple` tree: the run from the anchor.
    Run,
    /// Enter in a `multiple` tree: the row joins the selection, which is
    /// otherwise kept (what a click did there before #1598).
    Add,
}

/// A click on the row of `value`, or Enter on it (`Pick::Add`).
pub(super) fn activate_row(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    value: &str,
    has_children: bool,
    pick: Pick,
) {
    let controller = tree_state.controller;
    let multiple = controller.multiple;
    let select_on_click = config.select_on_click;
    // Ctrl- and Shift-clicks change the selection and nothing else.
    if multiple && select_on_click {
        match pick {
            Pick::Toggle => {
                let mut next = untracked(|| controller.selected.get());
                if !next.remove(value) {
                    next.insert(value.to_string());
                }
                tree_state.controller.set_anchor(Some(value));
                change_selection(config, tree_state, next);
                return;
            }
            Pick::Run => {
                select_run(config, tree_state, value, value);
                return;
            }
            Pick::Replace | Pick::Add => {}
        }
    }
    // A collapse by the click drops the rows it hides from the selection;
    // the selection changes once, below, with the click's pick.
    let mut collapsed = false;
    if config.expand_on_click && has_children {
        let was_expanded = untracked(|| tree_state.expanded.get()).contains(value);
        controller.toggle(value);
        if !was_expanded {
            if let Some(cb) = &config.onexpand {
                cb.invoke(value.to_string());
            }
        } else {
            collapsed = true;
            if let Some(cb) = &config.oncollapse {
                cb.invoke(value.to_string());
            }
        }
    }
    if select_on_click {
        if multiple {
            let next = if pick == Pick::Add {
                let mut next = untracked(|| controller.selected.get());
                if collapsed {
                    next = without_hidden(config, next, value);
                }
                next.insert(value.to_string());
                next
            } else {
                HashSet::from([value.to_string()])
            };
            tree_state.controller.set_anchor(Some(value));
            change_selection(config, tree_state, next);
        } else {
            controller.select(value);
        }
        if let Some(cb) = &config.onselect {
            cb.invoke(value.to_string());
        }
    }
}

/// Make `next` the selection, and tell the app if that changed it.
fn change_selection(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, next: HashSet<String>) {
    if untracked(|| tree_state.selected.get()) == next {
        return;
    }
    tree_state.controller.set_selection(next);
    selection_changed(config, tree_state);
}

/// Select the run of drawn rows from the anchor to `to`, leaving out
/// disabled rows. With no anchor, or one no longer in the data, the run
/// starts at `fallback` (which becomes the anchor).
pub(super) fn select_run(
    config: &Rc<TreeConfig>,
    tree_state: UseTreeReturn,
    fallback: &str,
    to: &str,
) {
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(b) = shape.index(to) else {
        return;
    };
    let anchor = untracked(|| tree_state.controller.anchor());
    let a = match anchor.as_deref().and_then(|v| shape.index(v)) {
        Some(a) => a,
        None => {
            let a = shape.index(fallback).unwrap_or(b);
            tree_state.controller.set_anchor(Some(&shape.rows[a].value));
            a
        }
    };
    let expanded = untracked(|| tree_state.expanded.get());
    let run: HashSet<String> = shape
        .run(&expanded, a, b)
        .into_iter()
        .filter(|&r| !shape.rows[r].disabled)
        .map(|r| shape.rows[r].value.clone())
        .collect();
    change_selection(config, tree_state, run);
}

/// A collapse by the tree's chevron or Left: the collapsed row's
/// descendants leave the selection, so nothing hidden stays selected. Only
/// in a `multiple` tree that selects on click: a tree that does not select
/// leaves the selection to the app, as it was.
pub(super) fn unselect_hidden(config: &Rc<TreeConfig>, tree_state: UseTreeReturn, value: &str) {
    if !tree_state.controller.multiple || !config.select_on_click {
        return;
    }
    let selected = untracked(|| tree_state.selected.get());
    let kept = without_hidden(config, selected, value);
    change_selection(config, tree_state, kept);
}

/// `selected` without the rows inside the row of `value` (the row itself
/// stays).
fn without_hidden(
    config: &Rc<TreeConfig>,
    selected: HashSet<String>,
    value: &str,
) -> HashSet<String> {
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let Some(row) = shape.index(value) else {
        return selected;
    };
    selected
        .into_iter()
        .filter(|v| {
            shape
                .index(v)
                .is_none_or(|r| r == row || !shape.is_within(r, row))
        })
        .collect()
}

/// Tell the app the selection, top to bottom. A selected value with no row
/// in the data (the app deleted it) is not reported, though the selection
/// keeps it: the tree does not prune the selection when the data changes.
pub(super) fn selection_changed(config: &Rc<TreeConfig>, tree_state: UseTreeReturn) {
    let Some(cb) = &config.onselectionchange else {
        return;
    };
    let shape = config.drag.shape(config.accepts_children.as_ref());
    let selected = untracked(|| tree_state.selected.get());
    let mut ordered: Vec<(usize, String)> = selected
        .into_iter()
        .filter_map(|v| Some((shape.index(&v)?, v)))
        .collect();
    ordered.sort();
    cb.invoke(ordered.into_iter().map(|(_, v)| v).collect());
}
