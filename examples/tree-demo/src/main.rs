//! Tree component demo — showcases the reactive Tree with dynamic mutations,
//! programmatic control, state preservation across collapse/expand, custom rendering,
//! and drag-and-drop for reordering and reparenting.

use std::rc::Rc;

use rinch::prelude::*;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

// =============================================================================
// Data helpers
// =============================================================================

fn initial_file_tree() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("src", "src")
            .with_icon(TablerIcon::Folder)
            .with_children(vec![
                TreeNodeData::new("main.rs", "main.rs")
                    .with_icon(TablerIcon::Code)
                    .with_payload(FileInfo {
                        size: "2.4 KB".into(),
                        modified: "2 hours ago".into(),
                    }),
                TreeNodeData::new("lib.rs", "lib.rs")
                    .with_icon(TablerIcon::Code)
                    .with_payload(FileInfo {
                        size: "8.1 KB".into(),
                        modified: "yesterday".into(),
                    }),
                TreeNodeData::new("utils.rs", "utils.rs")
                    .with_icon(TablerIcon::Code)
                    .with_payload(FileInfo {
                        size: "1.2 KB".into(),
                        modified: "3 days ago".into(),
                    }),
            ]),
        TreeNodeData::new("tests", "tests")
            .with_icon(TablerIcon::Folder)
            .with_children(vec![
                TreeNodeData::new("integration.rs", "integration.rs")
                    .with_icon(TablerIcon::Code)
                    .with_payload(FileInfo {
                        size: "4.5 KB".into(),
                        modified: "1 week ago".into(),
                    }),
            ]),
        TreeNodeData::new("Cargo.toml", "Cargo.toml")
            .with_icon(TablerIcon::Settings)
            .with_payload(FileInfo {
                size: "456 B".into(),
                modified: "1 week ago".into(),
            }),
        TreeNodeData::new("README.md", "README.md")
            .with_icon(TablerIcon::File)
            .with_payload(FileInfo {
                size: "3.2 KB".into(),
                modified: "1 month ago".into(),
            }),
    ]
}

#[derive(Clone)]
struct FileInfo {
    size: String,
    modified: String,
}

// =============================================================================
// App
// =============================================================================

#[component]
fn app() -> NodeHandle {
    // -- Shared tree state --
    let tree_data = Signal::new(initial_file_tree());
    let tree_state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: get_tree_expanded_state(&initial_file_tree(), &["src"]),
        ..Default::default()
    });

    // Counter for unique IDs when adding files
    let next_id = Signal::new(0u32);

    // -- Per-item editing state demo --
    let renaming_node = Signal::new(Option::<String>::None);

    // Pre-render icons (avoids __scope capture issues inside RSX closures)
    let icon_file_plus =
        render_tabler_icon(__scope, TablerIcon::FilePlus, TablerIconStyle::Outline);
    let icon_folder_plus =
        render_tabler_icon(__scope, TablerIcon::FolderPlus, TablerIconStyle::Outline);
    let icon_trash = render_tabler_icon(__scope, TablerIcon::Trash, TablerIconStyle::Outline);

    rsx! {
        Stack { gap: "xl", p: "xl", maw: "900px",

            // Header
            Title { order: 1, "Tree Component Demo" }
            Text { size: "lg", color: "dimmed",
                "Reactive tree with dynamic mutations, programmatic control, and state preservation."
            }

            // =========================================================
            // Section 1: File Browser with Dynamic Mutations
            // =========================================================
            Title { order: 3, "File Browser — Dynamic Mutations" }
            Text { color: "dimmed", size: "sm",
                "Add and remove nodes dynamically. The tree updates surgically via keyed reconciliation."
            }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",

                    // Toolbar
                    Group { gap: "sm",
                        Button {
                            variant: "light",
                            onclick: move || {
                                let id = next_id.get();
                                next_id.set(id + 1);
                                let name = format!("new_file_{}.rs", id);
                                tree_data.update(|data| {
                                    if let Some(src) = data.iter_mut().find(|n| n.value == "src") {
                                        src.children.push(
                                            TreeNodeData::new(name.clone(), name)
                                                .with_icon(TablerIcon::Code)
                                                .with_payload(FileInfo {
                                                    size: "0 B".into(),
                                                    modified: "just now".into(),
                                                }),
                                        );
                                    }
                                });
                            },
                            {icon_file_plus}
                            " Add File"
                        }
                        Button {
                            variant: "light",
                            onclick: move || {
                                let id = next_id.get();
                                next_id.set(id + 1);
                                let name = format!("folder_{}", id);
                                let value = name.clone();
                                tree_data.update(|data| {
                                    data.push(
                                        TreeNodeData::new(name.clone(), name)
                                            .with_icon(TablerIcon::Folder)
                                            .with_children(vec![
                                                TreeNodeData::new(
                                                    format!("{}/placeholder", value),
                                                    "placeholder.txt",
                                                )
                                                .with_icon(TablerIcon::File),
                                            ]),
                                    );
                                });
                                tree_state.controller.expand(&value);
                            },
                            {icon_folder_plus}
                            " Add Folder"
                        }
                        Button {
                            variant: "light",
                            color: "red",
                            onclick: move || {
                                let sel = tree_state.selected.get();
                                if sel.is_empty() {
                                    return;
                                }
                                tree_data.update(|data| {
                                    fn remove_nodes(
                                        nodes: &mut Vec<TreeNodeData>,
                                        to_remove: &std::collections::HashSet<String>,
                                    ) {
                                        nodes.retain(|n| !to_remove.contains(&n.value));
                                        for node in nodes.iter_mut() {
                                            remove_nodes(&mut node.children, to_remove);
                                        }
                                    }
                                    remove_nodes(data, &sel);
                                });
                                tree_state.controller.clear_selected();
                            },
                            {icon_trash}
                            " Remove Selected"
                        }
                    }

                    // Selection indicator
                    Group { gap: "sm", align: "center",
                        Text { size: "sm", weight: "600", "Selected:" }
                        Badge { color: "blue", variant: "light",
                            {|| {
                                let sel = tree_state.selected.get();
                                if sel.is_empty() {
                                    "None".to_string()
                                } else {
                                    sel.into_iter().collect::<Vec<_>>().join(", ")
                                }
                            }}
                        }
                    }

                    Divider {}

                    // The tree — data_source provides the reactive data source
                    Tree {
                        data: tree_data.get(),
                        tree: Some(tree_state),
                        data_source: Some(Rc::new(move || tree_data.get())),
                        select_on_click: true,
                    }
                }
            }

            // =========================================================
            // Section 2: Programmatic Control
            // =========================================================
            Title { order: 3, "Programmatic Control" }
            Text { color: "dimmed", size: "sm",
                "Expand all, collapse all, or select specific nodes via the controller."
            }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Group { gap: "sm",
                        Button {
                            variant: "light",
                            onclick: move || tree_state.controller.expand_all(&tree_data.get()),
                            "Expand All"
                        }
                        Button {
                            variant: "light",
                            color: "gray",
                            onclick: move || tree_state.controller.collapse_all(),
                            "Collapse All"
                        }
                        Button {
                            variant: "light",
                            color: "teal",
                            onclick: move || tree_state.controller.select("main.rs"),
                            "Select main.rs"
                        }
                        Button {
                            variant: "light",
                            color: "orange",
                            onclick: move || tree_state.controller.clear_selected(),
                            "Clear Selection"
                        }
                    }
                }
            }

            // =========================================================
            // Section 3: State Preservation
            // =========================================================
            Title { order: 3, "State Preservation Across Collapse" }
            Text { color: "dimmed", size: "sm",
                "Click 'Rename' on a file, collapse the parent folder, then re-expand. The rename state is preserved because subtree DOM is hidden, not destroyed."
            }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Group { gap: "sm", align: "center",
                        Text { size: "sm", weight: "600", "Renaming:" }
                        Badge { color: "violet", variant: "light",
                            {|| renaming_node.get().unwrap_or_else(|| "None".to_string())}
                        }
                    }
                    Divider {}
                    {state_preservation_tree(__scope, tree_data, tree_state, renaming_node)}
                }
            }

            // =========================================================
            // Section 4: Custom Render with Reactive Selection Highlight
            // =========================================================
            Title { order: 3, "Custom Renderer — File Info" }
            Text { color: "dimmed", size: "sm",
                "Custom render function showing file sizes and modification dates."
            }

            Paper { p: "xl", radius: "md", with_border: true,
                {custom_render_tree(__scope, tree_data, tree_state)}
            }

            // =========================================================
            // Section 5: Drag-and-Drop Reordering & Reparenting
            // =========================================================
            Title { order: 3, "Drag & Drop — Reorder & Reparent" }
            Text { color: "dimmed", size: "sm",
                "Drag files between folders to reparent them. Drag within a folder to reorder. \
                 Drop on a folder to move the item inside it."
            }

            Paper { p: "xl", radius: "md", with_border: true,
                {dnd_tree_section(__scope)}
            }
        }
    }
}

// =============================================================================
// Section 3: State preservation tree with rename buttons
// =============================================================================

#[component]
fn state_preservation_tree(
    tree_data: Signal<Vec<TreeNodeData>>,
    tree_state: UseTreeReturn,
    renaming_node: Signal<Option<String>>,
) -> NodeHandle {
    // Custom renderer that adds a "Rename" button per leaf node
    let render_fn: RenderTreeNode = Rc::new(move |payload, scope| {
        let wrapper = scope.create_element("div");
        wrapper.set_attribute(
            "style",
            "display: flex; justify-content: space-between; align-items: center; width: 100%; gap: 8px;",
        );

        let label = scope.create_element("span");
        label.set_attribute("class", "rinch-tree__label");
        let text = scope.create_text(&payload.node.label);
        label.append_child(&text);
        wrapper.append_child(&label);

        // Add rename toggle button for leaf nodes
        if !payload.has_children {
            let btn = scope.create_element("button");
            btn.set_attribute(
                "style",
                "font-size: var(--rinch-font-size-xs); padding: 2px 8px; border-radius: var(--rinch-radius-sm); border: 1px solid var(--rinch-color-gray-4); background: transparent; cursor: pointer;",
            );

            let node_value = payload.node_value.to_string();
            let nv_for_handler = node_value.clone();

            // Reactive button text
            let btn_text = scope.create_text("Rename");
            btn.append_child(&btn_text);

            let btn_text_clone = btn_text.clone();
            let nv = node_value.clone();
            scope.create_effect(move || {
                let is_renaming = renaming_node.get().as_deref() == Some(&nv);
                btn_text_clone.set_text(if is_renaming { "Done" } else { "Rename" });
            });

            let handler_id = scope.register_handler(move || {
                let current = renaming_node.get();
                if current.as_deref() == Some(&nv_for_handler) {
                    renaming_node.set(None);
                } else {
                    renaming_node.set(Some(nv_for_handler.clone()));
                }
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());

            wrapper.append_child(&btn);
        }

        wrapper
    });

    let data_source_closure: Rc<dyn Fn() -> Vec<TreeNodeData>> = Rc::new(move || tree_data.get());

    rsx! {
        Tree {
            data: tree_data.get(),
            tree: Some(tree_state),
            data_source: Some(data_source_closure),
            render_node: Some(render_fn),
        }
    }
}

// =============================================================================
// Section 4: Custom renderer with file info
// =============================================================================

#[component]
fn custom_render_tree(
    tree_data: Signal<Vec<TreeNodeData>>,
    tree_state: UseTreeReturn,
) -> NodeHandle {
    let render_fn: RenderTreeNode = Rc::new(move |payload, scope| {
        let wrapper = scope.create_element("div");
        wrapper.set_attribute(
            "style",
            "display: flex; justify-content: space-between; align-items: center; width: 100%; gap: 16px;",
        );

        let label = scope.create_element("span");
        label.set_attribute("class", "rinch-tree__label");
        let text = scope.create_text(&payload.node.label);
        label.append_child(&text);
        wrapper.append_child(&label);

        // Show file info for leaf nodes
        if !payload.has_children
            && let Some(info) = payload.node.downcast_payload::<FileInfo>()
        {
            let meta = scope.create_element("span");
            meta.set_attribute(
                "style",
                "color: var(--rinch-color-dimmed); font-size: var(--rinch-font-size-xs);",
            );
            let meta_text = scope.create_text(&format!("{} — {}", info.size, info.modified));
            meta.append_child(&meta_text);
            wrapper.append_child(&meta);
        }

        wrapper
    });

    let data_source_closure: Rc<dyn Fn() -> Vec<TreeNodeData>> = Rc::new(move || tree_data.get());

    rsx! {
        Tree {
            data: tree_data.get(),
            tree: Some(tree_state),
            data_source: Some(data_source_closure),
            render_node: Some(render_fn),
        }
    }
}

// =============================================================================
// Section 5: Drag-and-drop tree with reordering & reparenting
// =============================================================================

/// Simplified tree node for the DnD demo.
#[derive(Clone, Debug, PartialEq)]
struct DndNode {
    id: String,
    label: String,
    icon: TablerIcon,
    is_folder: bool,
    children: Vec<DndNode>,
}

impl DndNode {
    fn file(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: TablerIcon::Code,
            is_folder: false,
            children: vec![],
        }
    }
    fn folder(id: &str, label: &str, children: Vec<DndNode>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: TablerIcon::Folder,
            is_folder: true,
            children,
        }
    }
}

/// What's being dragged: a node ID.
#[derive(Clone, Debug)]
struct TreeDragData {
    node_id: String,
}

/// Remove a node by ID from the tree, returning it if found.
fn remove_node(nodes: &mut Vec<DndNode>, id: &str) -> Option<DndNode> {
    if let Some(pos) = nodes.iter().position(|n| n.id == id) {
        return Some(nodes.remove(pos));
    }
    for node in nodes.iter_mut() {
        if let Some(found) = remove_node(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

/// Check if `target_id` is a descendant of the node with `ancestor_id`.
fn is_descendant(nodes: &[DndNode], ancestor_id: &str, target_id: &str) -> bool {
    for node in nodes {
        if node.id == ancestor_id {
            return contains_id(&node.children, target_id);
        }
        if is_descendant(&node.children, ancestor_id, target_id) {
            return true;
        }
    }
    false
}

fn contains_id(nodes: &[DndNode], id: &str) -> bool {
    for node in nodes {
        if node.id == id || contains_id(&node.children, id) {
            return true;
        }
    }
    false
}

/// Insert a node before `before_id` in the tree. Returns true if inserted.
fn insert_before(nodes: &mut Vec<DndNode>, before_id: &str, to_insert: DndNode) -> bool {
    if let Some(pos) = nodes.iter().position(|n| n.id == before_id) {
        nodes.insert(pos, to_insert);
        return true;
    }
    for node in nodes.iter_mut() {
        if insert_before(&mut node.children, before_id, to_insert.clone()) {
            return true;
        }
    }
    false
}

/// Insert a node as a child of `parent_id` (appended at end).
fn insert_into_folder(nodes: &mut [DndNode], parent_id: &str, to_insert: DndNode) -> bool {
    for node in nodes.iter_mut() {
        if node.id == parent_id && node.is_folder {
            node.children.push(to_insert);
            return true;
        }
        if insert_into_folder(&mut node.children, parent_id, to_insert.clone()) {
            return true;
        }
    }
    false
}

fn initial_dnd_tree() -> Vec<DndNode> {
    vec![
        DndNode::folder(
            "components",
            "components",
            vec![
                DndNode::file("button.rs", "button.rs"),
                DndNode::file("input.rs", "input.rs"),
                DndNode::file("modal.rs", "modal.rs"),
            ],
        ),
        DndNode::folder(
            "utils",
            "utils",
            vec![
                DndNode::file("helpers.rs", "helpers.rs"),
                DndNode::file("format.rs", "format.rs"),
            ],
        ),
        DndNode::file("app.rs", "app.rs"),
        DndNode::file("main.rs", "main.rs"),
    ]
}

#[component]
fn dnd_tree_section() -> NodeHandle {
    let tree = Signal::new(initial_dnd_tree());
    let drag_ctx = DragContext::<TreeDragData>::new();
    let drop_target = Signal::new(Option::<String>::None);
    let expanded = Signal::new(
        vec!["components".to_string(), "utils".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<String>>(),
    );

    // Status line showing last action
    let status = Signal::new(String::new());

    let icon_reset = render_tabler_icon(__scope, TablerIcon::Refresh, TablerIconStyle::Outline);

    rsx! {
        Stack { gap: "md",

            // Toolbar
            Group { gap: "sm", align: "center",
                Button {
                    variant: "light",
                    size: "xs",
                    onclick: move || {
                        tree.set(initial_dnd_tree());
                        status.set("Reset to initial state".into());
                    },
                    {icon_reset}
                    " Reset"
                }
                Text { size: "xs", color: "dimmed",
                    {|| status.get()}
                }
            }

            Divider {}

            // The tree
            div { style: "min-height: 100px;",
                {dnd_tree_nodes(__scope, tree, drag_ctx, drop_target, expanded, status, 0)}
            }
        }
    }
}

/// Render a list of DnD tree nodes at a given nesting level.
#[component]
fn dnd_tree_nodes(
    tree: Signal<Vec<DndNode>>,
    drag_ctx: DragContext<TreeDragData>,
    drop_target: Signal<Option<String>>,
    expanded: Signal<std::collections::HashSet<String>>,
    status: Signal<String>,
    level: usize,
) -> NodeHandle {
    rsx! {
        div { style: "display: flex; flex-direction: column;",
            for node in tree.get() {
                {dnd_tree_node(
                    __scope, tree, drag_ctx, drop_target, expanded, status,
                    node.clone(), level,
                )}
            }
        }
    }
}

/// Render a single DnD tree node with drag source + drop target.
/// Uses imperative DOM construction to avoid issues with reactive `if` closures
/// capturing non-Copy types like String and NodeHandle.
#[allow(clippy::too_many_arguments)]
#[component]
fn dnd_tree_node(
    tree: Signal<Vec<DndNode>>,
    drag_ctx: DragContext<TreeDragData>,
    drop_target: Signal<Option<String>>,
    expanded: Signal<std::collections::HashSet<String>>,
    status: Signal<String>,
    node: DndNode,
    level: usize,
) -> NodeHandle {
    let node_id = node.id.clone();
    let is_folder = node.is_folder;

    // -- The outer wrapper <div> --
    let wrapper = __scope.create_element("div");
    wrapper.set_attribute("data-value", &node_id);

    // -- The node row: draggable + drop target --
    let row = __scope.create_element("div");
    row.set_attribute("draggable", "true");

    // ondragstart
    {
        let nid = node_id.clone();
        let handler_id = rinch::core::register_handler(std::rc::Rc::new(move || {
            drag_ctx.set(TreeDragData {
                node_id: nid.clone(),
            });
        }));
        row.set_attribute("data-ondragstart", &handler_id.0.to_string());
    }

    // ondragend
    {
        let handler_id = rinch::core::register_handler(std::rc::Rc::new(move || {
            drag_ctx.clear();
            drop_target.set(None);
        }));
        row.set_attribute("data-ondragend", &handler_id.0.to_string());
    }

    // ondrop
    {
        let nid = node_id.clone();
        let label = node.label.clone();
        let handler_id = rinch::core::register_handler(std::rc::Rc::new(move || {
            drop_target.set(None);
            if let Some(drag_data) = drag_ctx.take() {
                if drag_data.node_id == nid {
                    return;
                }
                tree.update(|data| {
                    if is_descendant(data, &drag_data.node_id, &nid) {
                        return;
                    }
                    if let Some(dragged) = remove_node(data, &drag_data.node_id) {
                        let dragged_label = dragged.label.clone();
                        if is_folder {
                            if insert_into_folder(data, &nid, dragged) {
                                expanded.update(|set| {
                                    set.insert(nid.clone());
                                });
                                status.set(format!("Moved '{}' into '{}'", dragged_label, label));
                            }
                        } else if insert_before(data, &nid, dragged) {
                            status.set(format!("Moved '{}' before '{}'", dragged_label, label));
                        }
                    }
                });
            }
        }));
        row.set_attribute("data-ondrop", &handler_id.0.to_string());
    }

    // ondragenter
    {
        let nid = node_id.clone();
        let handler_id = rinch::core::register_handler(std::rc::Rc::new(move || {
            drop_target.set(Some(nid.clone()));
        }));
        row.set_attribute("data-ondragenter", &handler_id.0.to_string());
    }

    // ondragleave
    {
        let nid = node_id.clone();
        let handler_id = rinch::core::register_handler(std::rc::Rc::new(move || {
            if drop_target.get().as_deref() == Some(nid.as_str()) {
                drop_target.set(None);
            }
        }));
        row.set_attribute("data-ondragleave", &handler_id.0.to_string());
    }

    // Reactive row style (drag/drop highlighting)
    {
        let row_clone = row.clone();
        let nid = node_id.clone();
        __scope.create_effect(move || {
            let is_target = drop_target.get().as_deref() == Some(nid.as_str());
            let is_dragged =
                drag_ctx.is_active() && drag_ctx.get().map(|d| d.node_id) == Some(nid.clone());
            let pad_left = level as u32 * 20;
            let bg = if is_target && is_folder {
                "var(--rinch-color-blue-1)"
            } else {
                "transparent"
            };
            let border_top = if is_target && !is_folder {
                "2px solid var(--rinch-color-blue-5)"
            } else {
                "2px solid transparent"
            };
            let opacity = if is_dragged { "0.4" } else { "1.0" };
            row_clone.set_attribute(
                "style",
                &format!(
                    "display: flex; align-items: center; gap: 4px; padding: 4px 8px 4px {}px; \
                     cursor: grab; opacity: {}; background: {}; border-top: {}; \
                     border-radius: var(--rinch-radius-xs); transition: background 0.1s;",
                    pad_left, opacity, bg, border_top,
                ),
            );
        });
    }

    // Chevron or spacer
    if is_folder {
        let chevron_span = __scope.create_element("span");
        let chevron_icon =
            render_tabler_icon(__scope, TablerIcon::ChevronRight, TablerIconStyle::Outline);
        chevron_span.append_child(&chevron_icon);

        // Reactive chevron rotation
        let chevron_clone = chevron_span.clone();
        let nid = node_id.clone();
        __scope.create_effect(move || {
            let is_expanded = expanded.get().contains(&nid);
            let rotation = if is_expanded { "90" } else { "0" };
            chevron_clone.set_attribute(
                "style",
                &format!(
                    "display: flex; width: 18px; height: 18px; cursor: pointer; \
                     transform: rotate({}deg); transition: transform 0.15s;",
                    rotation,
                ),
            );
        });

        // Chevron click → toggle expand
        let nid = node_id.clone();
        let handler_id = __scope.register_handler(move || {
            expanded.update(|set| {
                if set.contains(&nid) {
                    set.remove(&nid);
                } else {
                    set.insert(nid.clone());
                }
            });
        });
        chevron_span.set_attribute("data-rid", &handler_id.0.to_string());

        row.append_child(&chevron_span);
    } else {
        let spacer = __scope.create_element("span");
        spacer.set_attribute("style", "width: 18px; height: 18px;");
        row.append_child(&spacer);
    }

    // Icon
    let icon_span = __scope.create_element("span");
    let icon_color = if is_folder {
        "var(--rinch-color-yellow-6)"
    } else {
        "var(--rinch-color-blue-6)"
    };
    icon_span.set_attribute("style", &format!("display: flex; color: {};", icon_color));
    let icon_el = render_tabler_icon(__scope, node.icon, TablerIconStyle::Outline);
    icon_span.append_child(&icon_el);
    row.append_child(&icon_span);

    // Label
    let label_span = __scope.create_element("span");
    label_span.set_attribute("style", "font-size: var(--rinch-font-size-sm);");
    let label_text = __scope.create_text(&node.label);
    label_span.append_child(&label_text);
    row.append_child(&label_span);

    wrapper.append_child(&row);

    // Children container (folders only)
    if is_folder && !node.children.is_empty() {
        let children_container = __scope.create_element("div");

        // Reactive visibility
        let children_vis = children_container.clone();
        let nid = node_id.clone();
        __scope.create_effect(move || {
            if expanded.get().contains(&nid) {
                children_vis.set_attribute("style", "display: flex; flex-direction: column;");
            } else {
                children_vis.set_attribute("style", "display: none;");
            }
        });

        // Build children signal and keep in sync with tree changes
        let children_data = Signal::new(node.children.clone());
        let nid = node_id.clone();
        __scope.create_effect(move || {
            fn find_children(nodes: &[DndNode], id: &str) -> Vec<DndNode> {
                for n in nodes {
                    if n.id == id {
                        return n.children.clone();
                    }
                    let result = find_children(&n.children, id);
                    if !result.is_empty() {
                        return result;
                    }
                }
                vec![]
            }
            children_data.set(find_children(&tree.get(), &nid));
        });

        // Render children with for_each_dom_typed
        rinch::core::for_each_dom_typed(
            __scope,
            &children_container,
            move || children_data.get(),
            |child: &DndNode| child.id.clone(),
            move |child: DndNode, scope: &mut RenderScope| {
                dnd_tree_node(
                    scope,
                    tree,
                    drag_ctx,
                    drop_target,
                    expanded,
                    status,
                    child,
                    level + 1,
                )
            },
        );

        wrapper.append_child(&children_container);
    }

    wrapper
}

// =============================================================================
// Entry point
// =============================================================================

fn main() {
    run("Tree Demo", 900, 800, app);
}
