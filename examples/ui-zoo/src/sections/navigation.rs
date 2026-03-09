//! Navigation section - Navigation components with interactive state.

use rinch::prelude::*;

/// State for the Navigation section, stored in a store.
#[derive(Clone)]
pub struct NavigationSectionState {
    pub pagination_page: Signal<u32>,
    pub pagination_with_edges_page: Signal<u32>,
    pub stepper_active: Signal<u32>,
    pub tabs_value: Signal<String>,
    pub tabs_pills_value: Signal<String>,
}

/// Initialize the Navigation section state. Call this from the main app function.
pub fn init_navigation_state() {
    create_store(NavigationSectionState {
        pagination_page: Signal::new(1),
        pagination_with_edges_page: Signal::new(10),
        stepper_active: Signal::new(1),
        tabs_value: Signal::new("gallery".to_string()),
        tabs_pills_value: Signal::new("one".to_string()),
    });
}

#[component]
pub fn navigation_section() -> NodeHandle {
    let state = use_store::<NavigationSectionState>();

    let (pagination_page, pagination_with_edges_page, stepper_active, tabs_value, tabs_pills_value) = (
        state.pagination_page,
        state.pagination_with_edges_page,
        state.stepper_active,
        state.tabs_value,
        state.tabs_pills_value,
    );

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Navigation" }
                Text { size: "lg", color: "dimmed",
                    "Components for navigating between pages and sections"
                }
            }
            Space { h: "xl" }

            // ============================================
            // TABS
            // ============================================
            Title { order: 3, "Tabs" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Switch between different views within the same context." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Interactive tabs
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "Default Tabs" }
                            Badge { color: "blue", variant: "light", {|| tabs_value.get()} }
                        }
                        Divider {}
                        Tabs { value: tabs_value.get(),
                            TabsList {
                                Tab { value: "gallery", onclick: move || tabs_value.set("gallery".to_string()), "Gallery" }
                                Tab { value: "messages", onclick: move || tabs_value.set("messages".to_string()), "Messages" }
                                Tab { value: "settings", onclick: move || tabs_value.set("settings".to_string()), "Settings" }
                            }
                            TabsPanel { value: "gallery",
                                Paper { p: "md", radius: "sm", with_border: true,
                                    Text { size: "sm", color: "dimmed", "Browse and manage your photo gallery." }
                                }
                            }
                            TabsPanel { value: "messages",
                                Paper { p: "md", radius: "sm", with_border: true,
                                    Text { size: "sm", color: "dimmed", "View and respond to your messages." }
                                }
                            }
                            TabsPanel { value: "settings",
                                Paper { p: "md", radius: "sm", with_border: true,
                                    Text { size: "sm", color: "dimmed", "Configure your application preferences." }
                                }
                            }
                        }
                    }
                }

                // Pills variant
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "Pills Variant" }
                            Badge { color: "violet", variant: "light", {|| tabs_pills_value.get()} }
                        }
                        Divider {}
                        Tabs { variant: "pills", value: tabs_pills_value.get(),
                            TabsList {
                                Tab { value: "one", onclick: move || tabs_pills_value.set("one".to_string()), "First" }
                                Tab { value: "two", onclick: move || tabs_pills_value.set("two".to_string()), "Second" }
                                Tab { value: "three", onclick: move || tabs_pills_value.set("three".to_string()), "Third" }
                            }
                        }
                        Text { size: "sm", color: "dimmed", "Pills style is great for filters and toggles." }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // NAV LINKS
            // ============================================
            Title { order: 3, "Navigation Links" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Sidebar and menu navigation items." }
            Space { h: "md" }

            SimpleGrid { cols: Some(3), spacing: "lg",
                // Basic NavLink
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Basic" }
                        Divider {}
                        Stack { gap: "0",
                            NavLink { label: "Home", active: true }
                            NavLink { label: "Dashboard" }
                            NavLink { label: "Settings" }
                            NavLink { label: "Profile" }
                        }
                    }
                }

                // With description
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "With Description" }
                        Divider {}
                        Stack { gap: "0",
                            NavLink { label: "Messages", description: "3 unread" }
                            NavLink { label: "Notifications", description: "12 new" }
                            NavLink { label: "Updates", description: "Available" }
                        }
                    }
                }

                // Colors
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Colors" }
                        Divider {}
                        Stack { gap: "0",
                            NavLink { label: "Blue", color: "blue", active: true }
                            NavLink { label: "Green", color: "green", active: true }
                            NavLink { label: "Red", color: "red", active: true }
                            NavLink { label: "Violet", color: "violet", active: true }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // BREADCRUMBS
            // ============================================
            Title { order: 3, "Breadcrumbs" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Show the current location within a hierarchy." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Default Separator" }
                        Divider {}
                        Breadcrumbs {
                            BreadcrumbsItem { href: "#", "Home" }
                            BreadcrumbsItem { href: "#", "Products" }
                            BreadcrumbsItem { href: "#", "Electronics" }
                            BreadcrumbsItem { "Phones" }
                        }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Custom Separators" }
                        Divider {}
                        Breadcrumbs { separator: ">",
                            BreadcrumbsItem { href: "#", "Home" }
                            BreadcrumbsItem { href: "#", "Library" }
                            BreadcrumbsItem { "Data" }
                        }
                        Space { h: "sm" }
                        Breadcrumbs { separator: "|",
                            BreadcrumbsItem { href: "#", "One" }
                            BreadcrumbsItem { href: "#", "Two" }
                            BreadcrumbsItem { "Three" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // PAGINATION
            // ============================================
            Title { order: 3, "Pagination" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Navigate through pages of content." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Interactive pagination
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "Basic" }
                            Badge { color: "blue", variant: "light", {|| format!("Page {}", pagination_page.get())} }
                        }
                        Divider {}
                        Pagination {
                            total: {10_u32},
                            value: pagination_page.get(),
                            siblings: {1_u32},
                            onchange: move |page| pagination_page.set(page)
                        }
                    }
                }

                // With edges
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "With First/Last" }
                            Badge { color: "cyan", variant: "light", {|| format!("Page {}", pagination_with_edges_page.get())} }
                        }
                        Divider {}
                        Pagination {
                            total: {20_u32},
                            value: pagination_with_edges_page.get(),
                            siblings: {2_u32},
                            with_edges: true,
                            onchange: move |page| pagination_with_edges_page.set(page)
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // STEPPER
            // ============================================
            Title { order: 3, "Stepper" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Guide users through multi-step processes." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "lg",
                    Group { justify: "between",
                        Text { weight: "600", "Interactive Stepper" }
                        Badge { color: "green", variant: "light", {|| format!("Step {}", stepper_active.get() + 1)} }
                    }
                    Stepper { active: stepper_active.get(),
                        StepperStep { label: "Account", description: "Create your account" }
                        StepperStep { label: "Verify", description: "Verify your email" }
                        StepperStep { label: "Complete", description: "Get started" }
                    }
                    Divider {}
                    Group { justify: "center", gap: "sm",
                        Button {
                            variant: "outline",
                            disabled: stepper_active.get() == 0,
                            onclick: move || stepper_active.update(|v| *v = v.saturating_sub(1)),
                            "Previous"
                        }
                        Button {
                            disabled: stepper_active.get() >= 2,
                            onclick: move || stepper_active.update(|v| *v = (*v + 1).min(2)),
                            "Next Step"
                        }
                        Button {
                            variant: "subtle",
                            color: "gray",
                            onclick: move || stepper_active.set(0),
                            "Reset"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // TREE
            // ============================================
            Title { order: 3, "Tree" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Display hierarchical data with expand/collapse functionality." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Basic Tree
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Basic File Tree" }
                        Divider {}
                        {tree_demo(__scope)}
                    }
                }

                // Tree with selection
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "With Selection" }
                        Divider {}
                        {tree_selection_demo(__scope)}
                    }
                }
            }
        }
    }
}

/// Demo: Basic file tree
#[component]
fn tree_demo() -> NodeHandle {
    let data = vec![
        TreeNodeData::new("src", "src").with_children(vec![
            TreeNodeData::new("components", "components").with_children(vec![
                TreeNodeData::new("Button.tsx", "Button.tsx"),
                TreeNodeData::new("Input.tsx", "Input.tsx"),
            ]),
            TreeNodeData::new("hooks", "hooks").with_children(vec![
                TreeNodeData::new("useAuth.ts", "useAuth.ts"),
                TreeNodeData::new("useTheme.ts", "useTheme.ts"),
            ]),
            TreeNodeData::new("main.tsx", "main.tsx"),
        ]),
        TreeNodeData::new("public", "public").with_children(vec![
            TreeNodeData::new("index.html", "index.html"),
            TreeNodeData::new("favicon.ico", "favicon.ico"),
        ]),
        TreeNodeData::new("package.json", "package.json"),
        TreeNodeData::new("README.md", "README.md"),
    ];

    let tree = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: get_tree_expanded_state(&data, &["src"]),
        ..Default::default()
    });

    rsx! {
        Tree {
            data: data,
            tree: Some(tree),
            level_offset: "md",
        }
    }
}

/// Demo: Tree with selection
#[component]
fn tree_selection_demo() -> NodeHandle {
    let data = vec![
        TreeNodeData::new("documents", "Documents").with_children(vec![
            TreeNodeData::new("reports", "Reports").with_children(vec![
                TreeNodeData::new("q1-2024", "Q1 2024.pdf"),
                TreeNodeData::new("q2-2024", "Q2 2024.pdf"),
            ]),
            TreeNodeData::new("invoices", "Invoices").with_children(vec![
                TreeNodeData::new("inv-001", "INV-001.pdf"),
                TreeNodeData::new("inv-002", "INV-002.pdf"),
            ]),
        ]),
        TreeNodeData::new("images", "Images").with_children(vec![
            TreeNodeData::new("photo1", "vacation.jpg"),
            TreeNodeData::new("photo2", "profile.png"),
        ]),
    ];

    let tree = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: get_tree_expanded_state(&data, &["documents"]),
        ..Default::default()
    });

    rsx! {
        Tree {
            data: data,
            tree: Some(tree),
            level_offset: "md",
            select_on_click: true,
        }
    }
}
