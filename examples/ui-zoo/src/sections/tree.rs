//! Tree section - Tree component showcase.
//!
//! Demonstrates hierarchical data display with expand/collapse functionality,
//! selection, custom rendering, icons, and various configurations.

use rinch::prelude::*;
use rinch_tabler_icons::TablerIcon;
use std::rc::Rc;

/// State for the Tree section, stored in context.
#[derive(Clone)]
pub struct TreeSectionState {
    /// Currently selected node value (for selection demo).
    pub selected_value: Signal<Option<String>>,
    // Tree states for each demo
    pub basic_tree: UseTreeReturn,
    pub selection_tree: UseTreeReturn,
    pub expand_tree: UseTreeReturn,
    pub custom_tree: UseTreeReturn,
    pub icons_tree: UseTreeReturn,
    pub disabled_tree: UseTreeReturn,
    pub level_tree_xs: UseTreeReturn,
    pub level_tree_sm: UseTreeReturn,
    pub level_tree_md: UseTreeReturn,
    pub level_tree_lg: UseTreeReturn,
}

/// Initialize the Tree section state. Call this from the main app function.
pub fn init_tree_state() {
    create_context(TreeSectionState {
        selected_value: Signal::new(None),
        basic_tree: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&basic_data(), &["src"]),
            ..Default::default()
        }),
        selection_tree: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&selection_data(), &["documents"]),
            ..Default::default()
        }),
        expand_tree: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&expand_collapse_data(), &["company"]),
            ..Default::default()
        }),
        custom_tree: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&custom_render_data(), &["project", "src"]),
            ..Default::default()
        }),
        icons_tree: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&icons_data(), &["workspace", "src"]),
            ..Default::default()
        }),
        disabled_tree: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(
                &disabled_nodes_data(),
                &["settings", "advanced"],
            ),
            ..Default::default()
        }),
        level_tree_xs: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&level_offset_data(), &["*"]),
            ..Default::default()
        }),
        level_tree_sm: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&level_offset_data(), &["*"]),
            ..Default::default()
        }),
        level_tree_md: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&level_offset_data(), &["*"]),
            ..Default::default()
        }),
        level_tree_lg: UseTreeReturn::new(UseTreeOptions {
            initial_expanded: get_tree_expanded_state(&level_offset_data(), &["*"]),
            ..Default::default()
        }),
    });
}

#[component]
pub fn tree_section() -> NodeHandle {
    let state = use_context::<TreeSectionState>();

    let (
        selected_value,
        basic_tree,
        selection_tree,
        expand_tree,
        custom_tree,
        icons_tree,
        disabled_tree,
        level_tree_xs,
        level_tree_sm,
        level_tree_md,
        level_tree_lg,
    ) = (
        state.selected_value,
        state.basic_tree,
        state.selection_tree,
        state.expand_tree,
        state.custom_tree,
        state.icons_tree,
        state.disabled_tree,
        state.level_tree_xs,
        state.level_tree_sm,
        state.level_tree_md,
        state.level_tree_lg,
    );

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Tree" }
                Text { size: "lg", color: "dimmed",
                    "Display hierarchical data with expand/collapse functionality"
                }
            }
            Space { h: "xl" }

            // ============================================
            // BASIC FILE TREE
            // ============================================
            Title { order: 3, "Basic File Tree" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Simple expand/collapse with default styling. Click on folders to expand or collapse." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                {basic_file_tree_demo(__scope, basic_tree)}
            }

            Space { h: "xl" }

            // ============================================
            // WITH SELECTION
            // ============================================
            Title { order: 3, "With Selection" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Enable selection to highlight clicked nodes. The selected value is shown in the badge." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Group { justify: "between",
                        Text { weight: "600", "Select a file or folder" }
                        Badge { color: "blue", variant: "light",
                            {|| selected_value.get().unwrap_or_else(|| "None".to_string())}
                        }
                    }
                    Divider {}
                    {selection_tree_demo(__scope, selection_tree, selected_value)}
                }
            }

            Space { h: "xl" }

            // ============================================
            // EXPAND/COLLAPSE CONTROLS
            // ============================================
            Title { order: 3, "Expand/Collapse Controls" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Use the tree controller to programmatically expand or collapse all nodes." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                {expand_collapse_demo(__scope, expand_tree)}
            }

            Space { h: "xl" }

            // ============================================
            // CUSTOM RENDERING
            // ============================================
            Title { order: 3, "Custom Rendering" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Use render_node to customize how each node is displayed, showing additional metadata like file sizes." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                {custom_render_demo(__scope, custom_tree)}
            }

            Space { h: "xl" }

            // ============================================
            // WITH ICONS
            // ============================================
            Title { order: 3, "With Icons" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Use with_icon() on TreeNodeData to show folder and file icons." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                {icons_tree_demo(__scope, icons_tree)}
            }

            Space { h: "xl" }

            // ============================================
            // DISABLED NODES
            // ============================================
            Title { order: 3, "Disabled Nodes" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Some nodes can be disabled to prevent interaction." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                {disabled_nodes_demo(__scope, disabled_tree)}
            }

            Space { h: "xl" }

            // ============================================
            // LEVEL OFFSETS
            // ============================================
            Title { order: 3, "Level Offsets" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Compare different indentation levels side by side." }
            Space { h: "md" }

            SimpleGrid { cols: Some(4), spacing: "md",
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "xs" }
                        Divider {}
                        {level_offset_demo(__scope, level_tree_xs, "xs")}
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "sm" }
                        Divider {}
                        {level_offset_demo(__scope, level_tree_sm, "sm")}
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "md" }
                        Divider {}
                        {level_offset_demo(__scope, level_tree_md, "md")}
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "lg" }
                        Divider {}
                        {level_offset_demo(__scope, level_tree_lg, "lg")}
                    }
                }
            }
        }
    }
}

/// Data for basic file tree demo
fn basic_data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("src", "src").with_children(vec![
            TreeNodeData::new("components", "components").with_children(vec![
                TreeNodeData::new("Button.tsx", "Button.tsx"),
                TreeNodeData::new("Input.tsx", "Input.tsx"),
                TreeNodeData::new("Modal.tsx", "Modal.tsx"),
            ]),
            TreeNodeData::new("hooks", "hooks").with_children(vec![
                TreeNodeData::new("useAuth.ts", "useAuth.ts"),
                TreeNodeData::new("useTheme.ts", "useTheme.ts"),
            ]),
            TreeNodeData::new("utils", "utils").with_children(vec![
                TreeNodeData::new("helpers.ts", "helpers.ts"),
                TreeNodeData::new("constants.ts", "constants.ts"),
            ]),
            TreeNodeData::new("main.tsx", "main.tsx"),
            TreeNodeData::new("App.tsx", "App.tsx"),
        ]),
        TreeNodeData::new("public", "public").with_children(vec![
            TreeNodeData::new("index.html", "index.html"),
            TreeNodeData::new("favicon.ico", "favicon.ico"),
            TreeNodeData::new("assets", "assets").with_children(vec![
                TreeNodeData::new("logo.svg", "logo.svg"),
                TreeNodeData::new("styles.css", "styles.css"),
            ]),
        ]),
        TreeNodeData::new("package.json", "package.json"),
        TreeNodeData::new("tsconfig.json", "tsconfig.json"),
        TreeNodeData::new("README.md", "README.md"),
    ]
}

/// Demo: Basic file tree with expand/collapse
#[component]
fn basic_file_tree_demo(tree: UseTreeReturn) -> NodeHandle {
    let data = basic_data();

    rsx! {
        Tree {
            data: data,
            tree: Some(tree),
            level_offset: "md",
        }
    }
}

/// Data for selection tree demo
fn selection_data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("documents", "Documents").with_children(vec![
            TreeNodeData::new("reports", "Reports").with_children(vec![
                TreeNodeData::new("q1-report", "Q1 2024.pdf"),
                TreeNodeData::new("q2-report", "Q2 2024.pdf"),
                TreeNodeData::new("q3-report", "Q3 2024.pdf"),
                TreeNodeData::new("annual-report", "Annual Summary.pdf"),
            ]),
            TreeNodeData::new("invoices", "Invoices").with_children(vec![
                TreeNodeData::new("inv-001", "INV-001.pdf"),
                TreeNodeData::new("inv-002", "INV-002.pdf"),
                TreeNodeData::new("inv-003", "INV-003.pdf"),
            ]),
            TreeNodeData::new("contracts", "Contracts").with_children(vec![
                TreeNodeData::new("nda", "NDA.docx"),
                TreeNodeData::new("sow", "SOW.docx"),
            ]),
        ]),
        TreeNodeData::new("images", "Images").with_children(vec![
            TreeNodeData::new("vacation", "vacation.jpg"),
            TreeNodeData::new("profile", "profile.png"),
            TreeNodeData::new("screenshot", "screenshot.png"),
        ]),
        TreeNodeData::new("downloads", "Downloads").with_children(vec![
            TreeNodeData::new("installer", "setup.exe"),
            TreeNodeData::new("archive", "backup.zip"),
        ]),
    ]
}

/// Demo: Tree with selection and badge display
#[component]
fn selection_tree_demo(tree: UseTreeReturn, selected_signal: Signal<Option<String>>) -> NodeHandle {
    let data = selection_data();

    rsx! {
        Tree {
            data: data,
            tree: Some(tree),
            level_offset: "md",
            select_on_click: true,
            onselect: ValueCallback::new(move |value: String| {
                selected_signal.set(Some(value));
            }),
        }
    }
}

/// Data for expand/collapse demo
fn expand_collapse_data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("company", "Acme Corporation").with_children(vec![
            TreeNodeData::new("engineering", "Engineering").with_children(vec![
                TreeNodeData::new("frontend", "Frontend Team").with_children(vec![
                    TreeNodeData::new("alice", "Alice Johnson"),
                    TreeNodeData::new("bob", "Bob Smith"),
                ]),
                TreeNodeData::new("backend", "Backend Team").with_children(vec![
                    TreeNodeData::new("charlie", "Charlie Brown"),
                    TreeNodeData::new("diana", "Diana Prince"),
                ]),
                TreeNodeData::new("devops", "DevOps Team")
                    .with_children(vec![TreeNodeData::new("eve", "Eve Williams")]),
            ]),
            TreeNodeData::new("marketing", "Marketing").with_children(vec![
                TreeNodeData::new("content", "Content Team")
                    .with_children(vec![TreeNodeData::new("frank", "Frank Miller")]),
                TreeNodeData::new("seo", "SEO Team")
                    .with_children(vec![TreeNodeData::new("grace", "Grace Lee")]),
            ]),
            TreeNodeData::new("sales", "Sales").with_children(vec![
                TreeNodeData::new("henry", "Henry Ford"),
                TreeNodeData::new("iris", "Iris Chen"),
            ]),
        ]),
    ]
}

/// Demo: Programmatic expand/collapse with controller buttons
#[component]
fn expand_collapse_demo(tree: UseTreeReturn) -> NodeHandle {
    let data = expand_collapse_data();

    // Clone data for the controller buttons
    let data_for_expand = data.clone();

    let controller_expand = tree.controller;
    let controller_collapse = tree.controller;

    rsx! {
        Stack { gap: "md",
            Group { justify: "center", gap: "sm",
                Button {
                    variant: "light",
                    onclick: move || controller_expand.expand_all(&data_for_expand),
                    "Expand All"
                }
                Button {
                    variant: "light",
                    color: "gray",
                    onclick: move || controller_collapse.collapse_all(),
                    "Collapse All"
                }
            }
            Divider {}
            Tree {
                data: data,
                tree: Some(tree),
                level_offset: "md",
            }
        }
    }
}

// File size payload for custom render demo
#[derive(Clone)]
struct FileInfo {
    size: String,
    modified: String,
}

/// Data for custom render demo
fn custom_render_data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("project", "my-project").with_children(vec![
            TreeNodeData::new("src", "src").with_children(vec![
                TreeNodeData::new("main.rs", "main.rs").with_payload(FileInfo {
                    size: "2.4 KB".into(),
                    modified: "2 hours ago".into(),
                }),
                TreeNodeData::new("lib.rs", "lib.rs").with_payload(FileInfo {
                    size: "8.1 KB".into(),
                    modified: "yesterday".into(),
                }),
                TreeNodeData::new("utils.rs", "utils.rs").with_payload(FileInfo {
                    size: "1.2 KB".into(),
                    modified: "3 days ago".into(),
                }),
            ]),
            TreeNodeData::new("Cargo.toml", "Cargo.toml").with_payload(FileInfo {
                size: "456 B".into(),
                modified: "1 week ago".into(),
            }),
            TreeNodeData::new("Cargo.lock", "Cargo.lock").with_payload(FileInfo {
                size: "42 KB".into(),
                modified: "2 hours ago".into(),
            }),
            TreeNodeData::new("README.md", "README.md").with_payload(FileInfo {
                size: "3.2 KB".into(),
                modified: "1 month ago".into(),
            }),
        ]),
    ]
}

/// Demo: Custom rendering with file sizes
#[component]
fn custom_render_demo(tree: UseTreeReturn) -> NodeHandle {
    let data = custom_render_data();

    // Custom render function to show file info
    let render_node: RenderTreeNode = Rc::new(|payload, scope| {
        let label_wrapper = scope.create_element("div");
        label_wrapper.set_attribute("class", "rinch-tree__custom-label");
        label_wrapper.set_attribute("style", "display: flex; justify-content: space-between; align-items: center; width: 100%; gap: 16px;");

        let label = scope.create_element("span");
        label.set_attribute("class", "rinch-tree__label");
        let label_text = scope.create_text(&payload.node.label);
        label.append_child(&label_text);
        label_wrapper.append_child(&label);

        // Show file info if available (only for leaf nodes)
        if !payload.has_children
            && let Some(info) = payload.node.downcast_payload::<FileInfo>()
        {
            let meta = scope.create_element("span");
            meta.set_attribute(
                "style",
                "color: var(--rinch-color-dimmed); font-size: var(--rinch-font-size-xs);",
            );
            let meta_text = scope.create_text(&format!("{} - {}", info.size, info.modified));
            meta.append_child(&meta_text);
            label_wrapper.append_child(&meta);
        }

        label_wrapper
    });

    rsx! {
        Tree {
            data: data,
            tree: Some(tree),
            level_offset: "md",
            render_node: Some(render_node),
        }
    }
}

/// Data for icons tree demo
fn icons_data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("workspace", "Workspace")
            .with_icon(TablerIcon::Folder)
            .with_children(vec![
                TreeNodeData::new("src", "Source Files")
                    .with_icon(TablerIcon::Folder)
                    .with_children(vec![
                        TreeNodeData::new("main.rs", "main.rs").with_icon(TablerIcon::File),
                        TreeNodeData::new("config.rs", "config.rs").with_icon(TablerIcon::Settings),
                        TreeNodeData::new("user.rs", "user.rs").with_icon(TablerIcon::User),
                    ]),
                TreeNodeData::new("docs", "Documentation")
                    .with_icon(TablerIcon::Folder)
                    .with_children(vec![
                        TreeNodeData::new("readme", "README.md").with_icon(TablerIcon::File),
                        TreeNodeData::new("api", "API Reference").with_icon(TablerIcon::Link),
                    ]),
                TreeNodeData::new("assets", "Assets")
                    .with_icon(TablerIcon::Folder)
                    .with_children(vec![
                        TreeNodeData::new("logo", "logo.png").with_icon(TablerIcon::Photo),
                        TreeNodeData::new("banner", "banner.jpg").with_icon(TablerIcon::Photo),
                    ]),
                TreeNodeData::new("contacts", "Contacts")
                    .with_icon(TablerIcon::Mail)
                    .with_children(vec![
                        TreeNodeData::new("support", "support@example.com")
                            .with_icon(TablerIcon::Mail),
                        TreeNodeData::new("phone", "+1-555-0123").with_icon(TablerIcon::Phone),
                    ]),
            ]),
    ]
}

/// Demo: Tree with icons for different node types
#[component]
fn icons_tree_demo(tree: UseTreeReturn) -> NodeHandle {
    let data = icons_data();

    rsx! {
        Tree {
            data: data,
            tree: Some(tree),
            level_offset: "md",
        }
    }
}

/// Data for disabled nodes demo
fn disabled_nodes_data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("settings", "Settings").with_children(vec![
            TreeNodeData::new("general", "General").with_children(vec![
                TreeNodeData::new("language", "Language"),
                TreeNodeData::new("timezone", "Timezone"),
            ]),
            TreeNodeData::new("security", "Security").with_children(vec![
                TreeNodeData::new("password", "Change Password"),
                TreeNodeData::new("2fa", "Two-Factor Auth"),
            ]),
            TreeNodeData::new("billing", "Billing")
                .disabled(true)
                .with_children(vec![
                    TreeNodeData::new("payment", "Payment Methods").disabled(true),
                    TreeNodeData::new("invoices", "Invoices").disabled(true),
                ]),
            TreeNodeData::new("advanced", "Advanced").with_children(vec![
                TreeNodeData::new("developer", "Developer Mode"),
                TreeNodeData::new("experimental", "Experimental Features").disabled(true),
                TreeNodeData::new("debug", "Debug Options").disabled(true),
            ]),
        ]),
    ]
}

/// Demo: Tree with some disabled nodes
#[component]
fn disabled_nodes_demo(tree: UseTreeReturn) -> NodeHandle {
    let data = disabled_nodes_data();

    rsx! {
        Stack { gap: "sm",
            Text { size: "sm", color: "dimmed", "Billing and some Advanced options are disabled." }
            Tree {
                data: data,
                tree: Some(tree),
                level_offset: "md",
            }
        }
    }
}

/// Data for level offset demo
fn level_offset_data() -> Vec<TreeNodeData> {
    vec![TreeNodeData::new("root", "Root").with_children(vec![
        TreeNodeData::new("level1", "Level 1").with_children(vec![
                        TreeNodeData::new("level2", "Level 2")
                            .with_children(vec![
                                TreeNodeData::new("level3", "Level 3"),
                            ]),
                    ]),
    ])]
}

/// Demo: Tree with configurable level offset
#[component]
fn level_offset_demo(tree: UseTreeReturn, offset: &str) -> NodeHandle {
    let data = level_offset_data();

    rsx! {
        Tree {
            data: data,
            tree: Some(tree),
            level_offset: offset.to_string(),
        }
    }
}
