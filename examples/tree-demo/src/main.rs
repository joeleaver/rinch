//! Tree component demo — showcases the reactive Tree with dynamic mutations,
//! programmatic control, state preservation across collapse/expand, and custom rendering.

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
    let icon_file_plus = render_tabler_icon(__scope, TablerIcon::FilePlus, TablerIconStyle::Outline);
    let icon_folder_plus = render_tabler_icon(__scope, TablerIcon::FolderPlus, TablerIconStyle::Outline);
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

    let data_source_closure: Rc<dyn Fn() -> Vec<TreeNodeData>> =
        Rc::new(move || tree_data.get());

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
            let meta_text =
                scope.create_text(&format!("{} — {}", info.size, info.modified));
            meta.append_child(&meta_text);
            wrapper.append_child(&meta);
        }

        wrapper
    });

    let data_source_closure: Rc<dyn Fn() -> Vec<TreeNodeData>> =
        Rc::new(move || tree_data.get());

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
// Entry point
// =============================================================================

fn main() {
    run("Tree Demo", 900, 700, app);
}
