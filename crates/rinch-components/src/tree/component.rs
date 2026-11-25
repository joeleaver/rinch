//! The `Tree` component: its props, its shared render config, and its
//! `Component` impl.

use super::*;

// =============================================================================
// Internal TreeConfig (shared across recursive renders)
// =============================================================================

pub(super) struct TreeConfig {
    pub(super) expand_on_click: bool,
    pub(super) select_on_click: bool,
    pub(super) render_node: Option<RenderTreeNode>,
    pub(super) onselect: Option<ValueCallback<String>>,
    pub(super) onexpand: Option<ValueCallback<String>>,
    pub(super) oncollapse: Option<ValueCallback<String>>,
    pub(super) draggable: bool,
    pub(super) drag_values: Option<DragValues>,
    pub(super) judge_drop: Option<JudgeDrop>,
    pub(super) accepts_children: Option<AcceptsChildren>,
    pub(super) ondropchange: Option<ValueCallback<Option<(TreeDrop, DropVerdict)>>>,
    pub(super) ondrop: Option<ValueCallback<TreeDrop>>,
    pub(super) ondragstart: Option<ValueCallback<Vec<String>>>,
    pub(super) ondragend: Option<ValueCallback<()>>,
    /// Pixels per level, when the tree was given them ([`Tree::indent`]).
    pub(super) indent: Option<f32>,
    pub(super) hover_expand_ms: u32,
    pub(super) after_open_row: AfterOpenRow,
    pub(super) guides: bool,
    pub(super) drag: Rc<DragState>,
    pub(super) keyboard: bool,
    pub(super) onselectionchange: Option<ValueCallback<Vec<String>>>,
    pub(super) handle_key: Option<TreeKeyHandler>,
    pub(super) oncursor: Option<ValueCallback<String>>,
    pub(super) cursor_delay_ms: u32,
    pub(super) label_of: Option<LabelOf>,
    pub(super) keys: KeyState,
    /// Each row's children as the last data drew them, by row value, and a
    /// version bumped whenever the data is read again. A row's subtree reads
    /// its own list from here, so a change under a row reconciles that row's
    /// list in place instead of redrawing every row above it.
    pub(super) rows: RefCell<HashMap<String, Vec<Row>>>,
    pub(super) rows_version: Signal<u64>,
}

/// A row as a list of rows compares it: by what the row itself shows, not by
/// its subtree (which its own list reconciles, [`TreeConfig::rows`]), and by
/// whether it has children at all (a row that gains its first child grows a
/// chevron and a subtree, so it is drawn again).
#[derive(Clone)]
pub(super) struct Row(pub(super) TreeNodeData);

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (&self.0, &other.0);
        a.value == b.value
            && a.label == b.label
            && a.disabled == b.disabled
            && a.icon == b.icon
            && a.has_children() == b.has_children()
            && match (&a.payload, &b.payload) {
                (Some(x), Some(y)) => Rc::ptr_eq(x, y),
                (None, None) => true,
                _ => false,
            }
    }
}

/// Record every row's children under its value, for the rows' own lists.
fn index_rows(rows: &mut HashMap<String, Vec<Row>>, nodes: &[TreeNodeData]) {
    for node in nodes {
        if node.has_children() {
            rows.insert(
                node.value.clone(),
                node.children.iter().cloned().map(Row).collect(),
            );
            index_rows(rows, &node.children);
        }
    }
}

// =============================================================================
// Tree Component
// =============================================================================

/// Tree view component for displaying hierarchical data.
///
/// # Example
///
/// ```ignore
/// let data = vec![
///     TreeNodeData::new("docs", "Documents")
///         .with_icon(TablerIcon::Folder)
///         .with_children(vec![
///             TreeNodeData::new("readme", "README.md").with_icon(TablerIcon::File),
///             TreeNodeData::new("license", "LICENSE").with_icon(TablerIcon::File),
///         ]),
/// ];
///
/// let tree = UseTreeReturn::new(UseTreeOptions::default());
///
/// rsx! {
///     Tree {
///         data: data,
///         tree: Some(tree),
///         expand_on_click: true,
///         select_on_click: true,
///         onselect: ValueCallback::new(|value| println!("Selected: {}", value)),
///     }
/// }
/// ```
pub struct Tree {
    /// Tree data to display.
    pub data: Vec<TreeNodeData>,
    /// Tree state (from `UseTreeReturn::new()`).
    pub tree: Option<UseTreeReturn>,
    /// Indentation per level (spacing scale: xs, sm, md, lg, xl or custom CSS).
    pub level_offset: String,
    /// Whether clicking a node expands/collapses it.
    pub expand_on_click: bool,
    /// Whether clicking a node selects it.
    pub select_on_click: bool,
    /// Custom render function for node content.
    pub render_node: Option<RenderTreeNode>,
    /// Callback when a node is selected.
    pub onselect: Option<ValueCallback<String>>,
    /// Callback when a node is expanded.
    pub onexpand: Option<ValueCallback<String>>,
    /// Callback when a node is collapsed.
    pub oncollapse: Option<ValueCallback<String>>,
    /// Reactive data source. When provided, root nodes update when this changes.
    pub data_source: Option<Rc<dyn Fn() -> Vec<TreeNodeData>>>,
    /// Indentation per level in pixels. Overrides `level_offset`. A drag at
    /// the end of a group chooses its depth by the pointer's horizontal
    /// position in units of this; without it, the tree measures the indent
    /// of its rendered rows (from `level_offset`) once per drag.
    pub indent: Option<f32>,
    /// Whether rows can be dragged to rearrange the tree (needs `tree`).
    /// While a drag is over the tree it shows where the rows would land: a
    /// line before or after a row, at the depth they would take, or an
    /// outline around a row they would go into.
    pub draggable: bool,
    /// What a drag from a row carries (its own value when not given), for
    /// an app that drags its whole selection or names rows by its own ids.
    /// In a `multiple` tree it is called for each row the drag (or a key
    /// move, [`TreeKey::carried`]) takes — the selected rows when the
    /// pressed row is one of several — and each value is kept once, so one
    /// that returns the whole selection for any row carries it once. It
    /// never decides which rows a key move moves ([`TreeKey::drop_for`]).
    pub drag_values: Option<DragValues>,
    /// The app's judgement of each drop on offer. A refused drop is drawn in
    /// the error color and is not delivered. Drops into a dragged row's own
    /// subtree are always refused. Allowed when not given.
    pub judge_drop: Option<JudgeDrop>,
    /// Which rows can take children by a drop. A row that does not has no
    /// "inside" zone: its top half is before it, its bottom half after it
    /// (and it is never expanded by hovering). Every row takes children when
    /// not given.
    pub accepts_children: Option<AcceptsChildren>,
    /// The drop on offer and its verdict, each time it changes. `None` when
    /// no drop is on offer: over a row that offers none, once the pointer
    /// has left the rows, and when the drag ends. Moving from one row
    /// straight onto the next reports only the next row's drop.
    pub ondropchange: Option<ValueCallback<Option<(TreeDrop, DropVerdict)>>>,
    /// An allowed drop, when the pointer is released.
    pub ondrop: Option<ValueCallback<TreeDrop>>,
    /// A drag from a row started, carrying these values: for an app whose
    /// other drop targets take rows dragged out of the tree.
    pub ondragstart: Option<ValueCallback<Vec<String>>>,
    /// The drag ended, dropped anywhere or cancelled; after any drop.
    pub ondragend: Option<ValueCallback<()>>,
    /// How long a drag rests inside a collapsed row before it expands, in
    /// milliseconds; 0 never expands.
    pub hover_expand_ms: u32,
    /// Where a drop just below an expanded row with children lands (the
    /// bottom of the row's own box): first inside it by default, or, with
    /// [`AfterOpenRow::ByColumn`], inside or after it as the pointer's indent
    /// column says. The indicator starts at the indent of the depth the drop
    /// takes and runs to the row's right edge.
    pub after_open_row: AfterOpenRow,
    /// Draw a faint vertical line down each expanded row's children.
    pub guides: bool,
    /// Keyboard navigation (needs `tree`): while one of the rows has the
    /// keyboard, the arrow keys move a cursor (Right expands or steps in,
    /// Left collapses or steps out), Home and End jump to the first and last
    /// row, `*` expands the cursor row's siblings, and typed letters find the
    /// next row that starts with them. The tree is one Tab stop: the cursor
    /// row (the first row before there is one) is `tabindex="0"`, every other
    /// row `-1`. In a single-select tree the selection follows the cursor as
    /// a click would select (only with `select_on_click`, never a disabled
    /// row, and `onselect` is told). In a `multiple` tree a plain arrow
    /// selects nothing; Space toggles the cursor row; Shift with a move key
    /// (and Shift+Space) selects the run from the anchor (the row last
    /// clicked, Ctrl-clicked or toggled, or the one the app named with
    /// [`TreeController::set_anchor`]) to the cursor row, replacing the
    /// selection; Enter adds the cursor row as a click would, keeping the
    /// rest. All of these only with `select_on_click`, and none selects a
    /// disabled row (see [`Tree::onselectionchange`]).
    ///
    /// The tree takes no key with Ctrl, Cmd or Alt held, and neither PageUp
    /// nor PageDown (#3062): an app that wants Ctrl+A to select, or the page
    /// keys to move the cursor, binds them in [`Tree::handle_key`].
    pub keyboard: bool,
    /// Every key the tree's rows get, before the tree's own; `true` consumes
    /// it. [`TreeKey::drop_for`] turns a key into a structural move.
    pub handle_key: Option<TreeKeyHandler>,
    /// The cursor came to rest on a row (after `cursor_delay_ms`): for an
    /// app that opens what the cursor is on, once, not on every row it
    /// passed.
    pub oncursor: Option<ValueCallback<String>>,
    /// How long the cursor rests on a row before `oncursor`, in
    /// milliseconds (120 by default); 0 reports every move at once.
    pub cursor_delay_ms: u32,
    /// The text type-ahead matches for a row (its label when not given).
    pub label_of: Option<LabelOf>,
    /// The selection, top to bottom, each time a click or a key changes it,
    /// in a `multiple` tree only (`UseTreeOptions::multiple`; a single-select
    /// tree tells `onselect`). There a plain click selects the row alone;
    /// Ctrl- (Cmd-) click adds or removes a row; Shift-click, Shift with a
    /// move key and Shift+Space select the run of drawn rows from the anchor
    /// (the row last clicked, Ctrl-clicked, toggled with Space or entered
    /// with Enter, even when that unselected it, or the one the app named
    /// with [`TreeController::set_anchor`]; a hidden one counts as its
    /// shown ancestor) to that row, replacing the selection; Enter adds the
    /// cursor row, keeping the rest; a collapse by a click or a key unselects
    /// the rows it hides
    /// (only with `select_on_click`: otherwise the selection is the app's).
    /// Disabled rows are never selected, and nothing is reported when the
    /// selection did not change (a click that collapses and selects is one
    /// report).
    /// A selected value whose row left the data is not reported, though it
    /// stays in [`UseTreeReturn::selected`]: the tree does not prune the
    /// selection when the data changes.
    ///
    /// Differences from the WAI-ARIA tree pattern: Ctrl+A (optional in the
    /// pattern) is not the tree's (see [`Tree::keyboard`]); Shift with an
    /// arrow selects the run from the anchor (the desktop file-manager model)
    /// where the pattern's recommended model toggles the next row and keeps
    /// the rest; and Ctrl+Shift-click toggles the row (Ctrl wins) rather than
    /// adding the run.
    pub onselectionchange: Option<ValueCallback<Vec<String>>>,
}

impl std::fmt::Debug for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tree")
            .field("data", &format!("{} nodes", self.data.len()))
            .field("level_offset", &self.level_offset)
            .field("expand_on_click", &self.expand_on_click)
            .field("select_on_click", &self.select_on_click)
            .finish()
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            tree: None,
            level_offset: "md".into(),
            expand_on_click: true,
            select_on_click: false,
            render_node: None,
            onselect: None,
            onexpand: None,
            oncollapse: None,
            data_source: None,
            indent: None,
            draggable: false,
            drag_values: None,
            judge_drop: None,
            accepts_children: None,
            ondropchange: None,
            ondrop: None,
            ondragstart: None,
            ondragend: None,
            hover_expand_ms: 600,
            after_open_row: AfterOpenRow::FirstChild,
            guides: false,
            keyboard: false,
            handle_key: None,
            oncursor: None,
            cursor_delay_ms: 120,
            label_of: None,
            onselectionchange: None,
        }
    }
}

impl Component for Tree {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { ul { class: "rinch-tree" } };
        container.set_attribute("role", "tree");
        if self.tree.is_some_and(|tree| tree.controller.multiple) {
            container.set_attribute("aria-multiselectable", "true");
        }
        if self.draggable && self.tree.is_some() {
            container.add_class("rinch-tree--draggable");
        }

        // Apply level offset CSS variable
        let offset = if self.level_offset.is_empty() {
            "md"
        } else {
            &self.level_offset
        };
        let offset_value = match offset {
            "xs" => "var(--rinch-spacing-xs)",
            "sm" => "var(--rinch-spacing-sm)",
            "md" => "var(--rinch-spacing-md)",
            "lg" => "var(--rinch-spacing-lg)",
            "xl" => "var(--rinch-spacing-xl)",
            custom => custom,
        };
        let offset_value = match self.indent {
            Some(px) => format!("{px}px"),
            None => offset_value.to_string(),
        };
        container.set_attribute("style", &format!("--tree-level-offset: {}", offset_value));

        // Build shared config
        let config = Rc::new(TreeConfig {
            expand_on_click: self.expand_on_click,
            select_on_click: self.select_on_click,
            render_node: self.render_node.clone(),
            onselect: self.onselect.clone(),
            onexpand: self.onexpand.clone(),
            oncollapse: self.oncollapse.clone(),
            draggable: self.draggable && self.tree.is_some(),
            drag_values: self.drag_values.clone(),
            judge_drop: self.judge_drop.clone(),
            accepts_children: self.accepts_children.clone(),
            ondropchange: self.ondropchange.clone(),
            ondrop: self.ondrop.clone(),
            ondragstart: self.ondragstart.clone(),
            ondragend: self.ondragend.clone(),
            indent: self.indent,
            hover_expand_ms: self.hover_expand_ms,
            after_open_row: self.after_open_row,
            guides: self.guides,
            drag: Rc::new(DragState::default()),
            keyboard: self.keyboard && self.tree.is_some(),
            onselectionchange: self.onselectionchange.clone(),
            handle_key: self.handle_key.clone(),
            oncursor: self.oncursor.clone(),
            cursor_delay_ms: self.cursor_delay_ms,
            label_of: self.label_of.clone(),
            keys: KeyState::default(),
            rows: RefCell::new(HashMap::new()),
            rows_version: Signal::new(0),
        });

        if let Some(ref tree_state) = self.tree {
            let tree_state = *tree_state;
            let cfg = config.clone();
            if config.keyboard {
                wire_keyboard(__scope, &container, tree_state, &config);
            }

            // Use for_each_dom_typed for reactive root-level rendering
            let collection: Rc<dyn Fn() -> Vec<TreeNodeData>> =
                if let Some(ref data_source) = self.data_source {
                    data_source.clone()
                } else {
                    let data = self.data.clone();
                    Rc::new(move || data.clone())
                };
            // A drag resolves its drop against the data the rows were drawn
            // from, so the tree keeps the last of it.
            let drag = config.drag.clone();
            let draggable = config.draggable || config.keyboard || tree_state.controller.multiple;
            let keys_cfg = config.keyboard.then(|| Rc::downgrade(&config));

            let rows_cfg = Rc::downgrade(&config);
            for_each_dom_typed(
                __scope,
                &container,
                move || {
                    let data = collection();
                    if draggable {
                        let old = std::mem::replace(&mut *drag.data.borrow_mut(), data.clone());
                        drag.shape.borrow_mut().take();
                        if let Some(cfg) = keys_cfg.as_ref().and_then(|cfg| cfg.upgrade()) {
                            untracked(|| data_changed(&cfg, tree_state, &old));
                        }
                    }
                    // Every row's list first, then the rows: a row drawn below
                    // reads its children from here.
                    if let Some(cfg) = rows_cfg.upgrade() {
                        let mut rows = HashMap::new();
                        index_rows(&mut rows, &data);
                        *cfg.rows.borrow_mut() = rows;
                        let version = cfg.rows_version;
                        untracked(|| version.update(|v| *v += 1));
                    }
                    data.into_iter().map(Row).collect::<Vec<_>>()
                },
                |row: &Row| row.0.value.clone(),
                move |row: Row, __scope: &mut RenderScope| {
                    render_tree_node(__scope, &row.0, 0, tree_state, cfg.clone())
                },
            );
        } else {
            // No tree state — render statically
            for node in &self.data {
                let node_handle = render_tree_node_static(__scope, node, 0, &config);
                container.append_child(&node_handle);
            }
        }

        container
    }
}
