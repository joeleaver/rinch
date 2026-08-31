//! DevTools panel component — rendered in the separate DevTools window.
//!
//! Uses the raw `RenderScope` API directly because `rsx!` generates
//! `::rinch::core::` paths which don't resolve inside the `rinch` crate itself.
//!
//! Pull model: effects watch `doc_version` and read `MainDocRef` directly
//! from context to build the tree view and extract styles.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_dom::node::{NodeKind, RawNodeId};

use super::devtools_store::{
    DevToolsStore, DevToolsTab, DomTreeNode, MainDocRef, StyleCategory, StyleProperty,
};

/// A flattened row for the tree view.
#[derive(Debug, Clone, PartialEq)]
struct FlatTreeRow {
    id: usize,
    depth: usize,
    tag: String,
    id_attr: Option<String>,
    classes: Vec<String>,
    text_preview: Option<String>,
    is_text: bool,
    layout: (f32, f32, f32, f32),
    has_children: bool,
    is_expanded: bool,
}

fn flatten_tree(nodes: &[DomTreeNode], expanded: &HashSet<usize>) -> Vec<FlatTreeRow> {
    let mut rows = Vec::new();
    for node in nodes {
        flatten_node(node, expanded, &mut rows);
    }
    rows
}

fn flatten_node(node: &DomTreeNode, expanded: &HashSet<usize>, rows: &mut Vec<FlatTreeRow>) {
    let is_expanded = expanded.contains(&node.id);
    rows.push(FlatTreeRow {
        id: node.id,
        depth: node.depth,
        tag: node.tag.clone(),
        id_attr: node.id_attr.clone(),
        classes: node.classes.clone(),
        text_preview: node.text_preview.clone(),
        is_text: node.is_text,
        layout: node.layout,
        has_children: !node.children.is_empty(),
        is_expanded,
    });
    if !node.children.is_empty() && is_expanded {
        for child in &node.children {
            flatten_node(child, expanded, rows);
        }
    }
}

/// Helper: create an element via a Weak<RefCell<dyn DomDocument>> ref.
fn make_el(doc: &Weak<RefCell<dyn DomDocument>>, tag: &str) -> NodeHandle {
    let d = doc.upgrade().expect("doc dropped");
    let id = d.borrow_mut().create_element(tag);
    NodeHandle::new(id, doc.clone())
}

/// Helper: create a text node via a Weak<RefCell<dyn DomDocument>> ref.
fn make_text(doc: &Weak<RefCell<dyn DomDocument>>, text: &str) -> NodeHandle {
    let d = doc.upgrade().expect("doc dropped");
    let id = d.borrow_mut().create_text(text);
    NodeHandle::new(id, doc.clone())
}

/// Collect all ancestor node IDs for a given node (excluding the node itself).
fn collect_ancestors(doc: &rinch_dom::RinchDocument, node_id: usize) -> Vec<usize> {
    let mut ancestors = Vec::new();
    let mut current = node_id;
    while let Some(node) = doc.tree.get(current) {
        if let Some(parent) = node.parent {
            ancestors.push(parent);
            current = parent;
        } else {
            break;
        }
    }
    ancestors
}

// ── Tree / style extraction from main document ─────────────────────────────

fn build_tree_nodes_from_doc(doc: &rinch_dom::RinchDocument) -> Vec<DomTreeNode> {
    let tree = &doc.tree;
    let mut result = Vec::new();
    for &child_id in &tree.nodes[tree.body_id].children {
        build_tree_node_recursive(tree, child_id, 0, &mut result);
    }
    result
}

fn build_tree_node_recursive(
    tree: &rinch_dom::node::NodeTree,
    id: RawNodeId,
    depth: usize,
    out: &mut Vec<DomTreeNode>,
) {
    let Some(node) = tree.get(id) else { return };

    // The box the node is painted in, so the panel's numbers agree with the
    // inspect overlay and with what a click on the screen would hit (#203).
    let painted = rinch_dom::paint::painted_border_box(tree, id, 1.0);

    let (tag, id_attr, classes, text_preview, is_text) = match &node.kind {
        NodeKind::Document => return,
        NodeKind::Comment(_) => return,
        NodeKind::Element(el) => {
            let id_attr = node.attributes.get("id").cloned();
            let classes: Vec<String> = node
                .attributes
                .get("class")
                .map(|c| c.split_whitespace().map(String::from).collect())
                .unwrap_or_default();
            (el.tag.clone(), id_attr, classes, None, false)
        }
        NodeKind::Text(t) => {
            let preview = t.content.trim().to_string();
            if preview.is_empty() {
                return;
            }
            ("text".to_string(), None, vec![], Some(preview), true)
        }
    };

    let mut children = Vec::new();
    if !is_text {
        for &child_id in &node.children {
            build_tree_node_recursive(tree, child_id, depth + 1, &mut children);
        }
    }

    out.push(DomTreeNode {
        id,
        tag,
        id_attr,
        classes,
        text_preview,
        layout: (
            painted.x0 as f32,
            painted.y0 as f32,
            painted.width() as f32,
            painted.height() as f32,
        ),
        children,
        depth,
        is_text,
    });
}

fn count_nodes_in_doc(doc: &rinch_dom::RinchDocument) -> usize {
    doc.tree.nodes.len()
}

fn fmt_lp(v: &rinch_dom::computed_style::LengthPercentageValue) -> String {
    use rinch_dom::computed_style::LengthPercentageValue;
    match v {
        LengthPercentageValue::Zero => "0".into(),
        LengthPercentageValue::Length(px) => format!("{:.1}px", px),
        LengthPercentageValue::Percent(pct) => format!("{:.1}%", pct),
    }
}

fn fmt_lpa(v: &rinch_dom::computed_style::LengthPercentageAutoValue) -> String {
    use rinch_dom::computed_style::LengthPercentageAutoValue;
    match v {
        LengthPercentageAutoValue::Auto => "auto".into(),
        LengthPercentageAutoValue::Length(px) => format!("{:.1}px", px),
        LengthPercentageAutoValue::Percent(pct) => format!("{:.1}%", pct),
    }
}

fn is_lp_zero(v: &rinch_dom::computed_style::LengthPercentageValue) -> bool {
    matches!(v, rinch_dom::computed_style::LengthPercentageValue::Zero)
}

fn is_lpa_auto(v: &rinch_dom::computed_style::LengthPercentageAutoValue) -> bool {
    matches!(
        v,
        rinch_dom::computed_style::LengthPercentageAutoValue::Auto
    )
}

fn extract_style_properties_from_doc(
    doc: &rinch_dom::RinchDocument,
    node_id: usize,
) -> Vec<StyleProperty> {
    let Some(node) = doc.tree.get(node_id) else {
        return vec![];
    };
    let cs = &node.computed_style;
    let mut props = Vec::new();

    // Layout
    props.push(StyleProperty {
        name: "display".into(),
        value: format!("{:?}", cs.display),
        category: StyleCategory::Layout,
    });
    props.push(StyleProperty {
        name: "position".into(),
        value: format!("{:?}", cs.position),
        category: StyleCategory::Layout,
    });
    props.push(StyleProperty {
        name: "computed size".into(),
        value: format!("{:.0} x {:.0}", node.layout.width, node.layout.height),
        category: StyleCategory::Layout,
    });

    // Box model
    if !is_lpa_auto(&cs.margin_top)
        || !is_lpa_auto(&cs.margin_right)
        || !is_lpa_auto(&cs.margin_bottom)
        || !is_lpa_auto(&cs.margin_left)
    {
        props.push(StyleProperty {
            name: "margin".into(),
            value: format!(
                "{} {} {} {}",
                fmt_lpa(&cs.margin_top),
                fmt_lpa(&cs.margin_right),
                fmt_lpa(&cs.margin_bottom),
                fmt_lpa(&cs.margin_left)
            ),
            category: StyleCategory::BoxModel,
        });
    }
    if !is_lp_zero(&cs.padding_top)
        || !is_lp_zero(&cs.padding_right)
        || !is_lp_zero(&cs.padding_bottom)
        || !is_lp_zero(&cs.padding_left)
    {
        props.push(StyleProperty {
            name: "padding".into(),
            value: format!(
                "{} {} {} {}",
                fmt_lp(&cs.padding_top),
                fmt_lp(&cs.padding_right),
                fmt_lp(&cs.padding_bottom),
                fmt_lp(&cs.padding_left)
            ),
            category: StyleCategory::BoxModel,
        });
    }
    if !is_lp_zero(&cs.border_top_width)
        || !is_lp_zero(&cs.border_right_width)
        || !is_lp_zero(&cs.border_bottom_width)
        || !is_lp_zero(&cs.border_left_width)
    {
        props.push(StyleProperty {
            name: "border-width".into(),
            value: format!(
                "{} {} {} {}",
                fmt_lp(&cs.border_top_width),
                fmt_lp(&cs.border_right_width),
                fmt_lp(&cs.border_bottom_width),
                fmt_lp(&cs.border_left_width)
            ),
            category: StyleCategory::BoxModel,
        });
    }

    // Typography
    if cs.font_size != 0.0 {
        props.push(StyleProperty {
            name: "font-size".into(),
            value: format!("{:.1}px", cs.font_size),
            category: StyleCategory::Typography,
        });
    }
    if cs.font_weight != 0.0 {
        props.push(StyleProperty {
            name: "font-weight".into(),
            value: format!("{:.0}", cs.font_weight),
            category: StyleCategory::Typography,
        });
    }
    props.push(StyleProperty {
        name: "line-height".into(),
        value: format!("{:?}", cs.line_height),
        category: StyleCategory::Typography,
    });

    // Colors
    if let Some(c) = &cs.color {
        props.push(StyleProperty {
            name: "color".into(),
            value: format!("{:?}", c),
            category: StyleCategory::Colors,
        });
    }
    match &cs.background {
        rinch_dom::computed_style::BackgroundValue::None => {}
        bg => {
            props.push(StyleProperty {
                name: "background".into(),
                value: format!("{:?}", bg),
                category: StyleCategory::Colors,
            });
        }
    }

    // Flex
    if cs.flex_grow != 0.0 {
        props.push(StyleProperty {
            name: "flex-grow".into(),
            value: format!("{}", cs.flex_grow),
            category: StyleCategory::Flex,
        });
    }
    if cs.flex_shrink != 1.0 {
        props.push(StyleProperty {
            name: "flex-shrink".into(),
            value: format!("{}", cs.flex_shrink),
            category: StyleCategory::Flex,
        });
    }
    if !is_lp_zero(&cs.gap_row) || !is_lp_zero(&cs.gap_column) {
        props.push(StyleProperty {
            name: "gap".into(),
            value: format!("{} {}", fmt_lp(&cs.gap_row), fmt_lp(&cs.gap_column)),
            category: StyleCategory::Flex,
        });
    }

    // Visual
    if cs.opacity < 1.0 {
        props.push(StyleProperty {
            name: "opacity".into(),
            value: format!("{:.2}", cs.opacity),
            category: StyleCategory::Visual,
        });
    }
    if !is_lp_zero(&cs.border_radius_top_left)
        || !is_lp_zero(&cs.border_radius_top_right)
        || !is_lp_zero(&cs.border_radius_bottom_left)
        || !is_lp_zero(&cs.border_radius_bottom_right)
    {
        props.push(StyleProperty {
            name: "border-radius".into(),
            value: format!(
                "{} {} {} {}",
                fmt_lp(&cs.border_radius_top_left),
                fmt_lp(&cs.border_radius_top_right),
                fmt_lp(&cs.border_radius_bottom_right),
                fmt_lp(&cs.border_radius_bottom_left)
            ),
            category: StyleCategory::Visual,
        });
    }

    props
}

// ── Components ──────────────────────────────────────────────────────────────

/// Root component for the DevTools window.
pub fn devtools_root(scope: &mut RenderScope) -> NodeHandle {
    let store = rinch_core::use_store::<DevToolsStore>();

    let root = scope.create_element("div");
    root.set_attribute("class", "devtools-root");

    // Tab bar
    let tabbar = render_tab_bar(scope, store);
    root.append_child(&tabbar);

    // Toolbar
    let toolbar = render_toolbar(scope, store);
    root.append_child(&toolbar);

    // Panel content — three panels, show/hide based on tab
    let panel = scope.create_element("div");
    panel.set_attribute("class", "devtools-panel");

    let elements_node = render_elements_panel(scope, store);
    let styles_node = render_styles_panel(scope, store);
    let perf_node = render_performance_panel(scope, store);

    panel.append_child(&elements_node);
    panel.append_child(&styles_node);
    panel.append_child(&perf_node);

    // Show/hide panels based on active tab
    let active_tab = store.active_tab;
    scope.create_effect({
        let elements_node = elements_node.clone();
        let styles_node = styles_node.clone();
        let perf_node = perf_node.clone();
        move || {
            let tab = active_tab.get();
            let (e, s, p) = match tab {
                DevToolsTab::Elements => ("display: block;", "display: none;", "display: none;"),
                DevToolsTab::Styles => ("display: none;", "display: block;", "display: none;"),
                DevToolsTab::Performance => ("display: none;", "display: none;", "display: block;"),
            };
            elements_node.set_attribute("style", e);
            styles_node.set_attribute("style", s);
            perf_node.set_attribute("style", p);
        }
    });

    root.append_child(&panel);
    root
}

fn render_tab_bar(scope: &mut RenderScope, store: DevToolsStore) -> NodeHandle {
    let tabs: &[(&str, DevToolsTab)] = &[
        ("Elements", DevToolsTab::Elements),
        ("Styles", DevToolsTab::Styles),
        ("Performance", DevToolsTab::Performance),
    ];

    let container = scope.create_element("div");
    container.set_attribute("class", "devtools-tabbar");

    for &(label, tab) in tabs {
        let tab_div = scope.create_element("div");

        let active_tab = store.active_tab;
        scope.create_effect({
            let tab_div = tab_div.clone();
            move || {
                let cls = if active_tab.get() == tab {
                    "devtools-tab devtools-tab--active"
                } else {
                    "devtools-tab"
                };
                tab_div.set_attribute("class", cls);
            }
        });

        let handler_id = scope.register_handler({
            let active_tab = store.active_tab;
            move || active_tab.set(tab)
        });
        tab_div.set_attribute("data-rid", &handler_id.0.to_string());

        let text = scope.create_text(label);
        tab_div.append_child(&text);
        container.append_child(&tab_div);
    }

    container
}

fn render_toolbar(scope: &mut RenderScope, store: DevToolsStore) -> NodeHandle {
    let inspect_mode = store.inspect_mode;
    let doc_version = store.doc_version;
    let main_doc = rinch_core::use_context::<MainDocRef>();

    let toolbar = scope.create_element("div");
    toolbar.set_attribute("class", "devtools-toolbar");

    // Inspect button
    let inspect_btn = scope.create_element("div");
    scope.create_effect({
        let inspect_btn = inspect_btn.clone();
        move || {
            let cls = if inspect_mode.get() {
                "devtools-btn devtools-btn--active"
            } else {
                "devtools-btn"
            };
            inspect_btn.set_attribute("class", cls);
        }
    });
    let handler_id = scope.register_handler(move || inspect_mode.update(|v| *v = !*v));
    inspect_btn.set_attribute("data-rid", &handler_id.0.to_string());
    let btn_text = scope.create_text("Inspect");
    inspect_btn.append_child(&btn_text);
    toolbar.append_child(&inspect_btn);

    // Node count — reads directly from main document
    let count_span = scope.create_element("span");
    count_span.set_attribute(
        "style",
        "color: #808080; margin-left: auto; font-size: 11px;",
    );
    scope.create_effect({
        let count_span = count_span.clone();
        let main_doc = main_doc.clone();
        move || {
            let _version = doc_version.get(); // subscribe to changes
            let doc = main_doc.0.borrow();
            let count = count_nodes_in_doc(&doc);
            count_span.set_text(&format!("{} nodes", count));
        }
    });
    toolbar.append_child(&count_span);

    toolbar
}

fn render_elements_panel(scope: &mut RenderScope, store: DevToolsStore) -> NodeHandle {
    // Use Rc<RefCell> for expanded state to avoid re-entrancy panics.
    // A version signal triggers the effect to rebuild when expanded changes.
    let expanded: Rc<RefCell<HashSet<usize>>> = Rc::new(RefCell::new({
        let mut s = HashSet::new();
        s.insert(0);
        s
    }));
    let tree_version = Signal::new(0u64);

    let doc_version = store.doc_version;
    let selected_node_id = store.selected_node_id;
    let hovered_node_id = store.hovered_node_id;
    let main_doc = rinch_core::use_context::<MainDocRef>();

    let container = scope.create_element("div");
    container.set_attribute("style", "overflow: auto; flex: 1;");

    // Stash doc ref for creating nodes inside effects (DevTools document, not main)
    let doc_weak = scope.doc_weak();

    // Track the previous selected_node_id so we only auto-expand on changes.
    let prev_selected: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));

    scope.create_effect({
        let container = container.clone();
        let doc = doc_weak.clone();
        let main_doc = main_doc.clone();
        let prev_selected = prev_selected.clone();
        let expanded = expanded.clone();
        move || {
            let _version = doc_version.get(); // subscribe to main doc changes
            let _tree_ver = tree_version.get(); // subscribe to expand/collapse
            let selected = selected_node_id.get();

            // Auto-expand ancestors when selection changes (e.g. from inspect click)
            if selected != *prev_selected.borrow() {
                *prev_selected.borrow_mut() = selected;
                if let Some(nid) = selected {
                    let ancestors = {
                        let d = main_doc.0.borrow();
                        collect_ancestors(&d, nid)
                    };
                    let mut exp = expanded.borrow_mut();
                    for ancestor in &ancestors {
                        exp.insert(*ancestor);
                    }
                }
            }

            let exp = expanded.borrow();

            // Read tree directly from main document
            let tree_data = {
                let d = main_doc.0.borrow();
                build_tree_nodes_from_doc(&d)
            };
            let rows = flatten_tree(&tree_data, &exp);
            drop(exp);

            // Remove all existing children
            for child in container.children() {
                child.remove();
            }

            // Build rows
            for row in &rows {
                let row_div = make_el(&doc, "div");

                if selected == Some(row.id) {
                    row_div.set_attribute("class", "tree-row tree-row--selected");
                } else {
                    row_div.set_attribute("class", "tree-row");
                }

                row_div.set_attribute("style", &format!("padding-left: {}px;", row.depth * 16));

                // Click to select
                let row_id = row.id;
                let click_handler = rinch_core::events::register_handler(Rc::new(move || {
                    selected_node_id.set(Some(row_id));
                }));
                row_div.set_attribute("data-rid", &click_handler.0.to_string());

                // Hover
                let enter_handler = rinch_core::events::register_handler(Rc::new(move || {
                    hovered_node_id.set(Some(row_id));
                }));
                row_div.set_attribute("data-onenter", &enter_handler.0.to_string());

                let leave_handler = rinch_core::events::register_handler(Rc::new(move || {
                    hovered_node_id.set(None);
                }));
                row_div.set_attribute("data-onleave", &leave_handler.0.to_string());

                // Chevron
                let chevron = make_el(&doc, "span");
                chevron.set_attribute("class", "tree-chevron");
                if row.has_children {
                    let ch_text = if row.is_expanded {
                        "\u{25BC}"
                    } else {
                        "\u{25B6}"
                    };
                    let ch_label = make_text(&doc, ch_text);
                    chevron.append_child(&ch_label);

                    let node_id = row.id;
                    let expanded_ref = expanded.clone();
                    let ch_handler = rinch_core::events::register_handler(Rc::new(move || {
                        {
                            let mut exp = expanded_ref.borrow_mut();
                            if exp.contains(&node_id) {
                                exp.remove(&node_id);
                            } else {
                                exp.insert(node_id);
                            }
                        }
                        tree_version.update(|v| *v += 1);
                    }));
                    chevron.set_attribute("data-rid", &ch_handler.0.to_string());
                }
                row_div.append_child(&chevron);

                // Node label
                if row.is_text {
                    let text_span = make_el(&doc, "span");
                    text_span.set_attribute("class", "tree-text");
                    let preview: String = row
                        .text_preview
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect();
                    let t = make_text(&doc, &format!("\"{}\"", preview));
                    text_span.append_child(&t);
                    row_div.append_child(&text_span);
                } else {
                    let tag_span = make_el(&doc, "span");
                    tag_span.set_attribute("class", "tree-tag");
                    let t = make_text(&doc, &format!("<{}", row.tag));
                    tag_span.append_child(&t);
                    row_div.append_child(&tag_span);

                    if let Some(id) = &row.id_attr {
                        let id_span = make_el(&doc, "span");
                        id_span.set_attribute("class", "tree-id");
                        let t = make_text(&doc, &format!(" id=\"{}\"", id));
                        id_span.append_child(&t);
                        row_div.append_child(&id_span);
                    }

                    if !row.classes.is_empty() {
                        let cls_span = make_el(&doc, "span");
                        cls_span.set_attribute("class", "tree-class");
                        let t = make_text(&doc, &format!(" .{}", row.classes.join(".")));
                        cls_span.append_child(&t);
                        row_div.append_child(&cls_span);
                    }

                    let close = make_el(&doc, "span");
                    close.set_attribute("class", "tree-tag");
                    let t = make_text(&doc, ">");
                    close.append_child(&t);
                    row_div.append_child(&close);

                    let layout_span = make_el(&doc, "span");
                    layout_span.set_attribute("class", "tree-layout");
                    let t = make_text(
                        &doc,
                        &format!(
                            "{:.0}x{:.0} @ ({:.0},{:.0})",
                            row.layout.2, row.layout.3, row.layout.0, row.layout.1
                        ),
                    );
                    layout_span.append_child(&t);
                    row_div.append_child(&layout_span);
                }

                container.append_child(&row_div);
            }

            // Scroll to the selected row
            if let Some(sel) = selected {
                if let Some(idx) = rows.iter().position(|r| r.id == sel) {
                    let row_height = 22.0;
                    let scroll_to = (idx as f64) * row_height;
                    container.set_scroll_top(scroll_to);
                }
            }
        }
    });

    container
}

fn render_styles_panel(scope: &mut RenderScope, store: DevToolsStore) -> NodeHandle {
    let selected_node_id = store.selected_node_id;
    let doc_version = store.doc_version;
    let main_doc = rinch_core::use_context::<MainDocRef>();

    let container = scope.create_element("div");
    let doc_weak = scope.doc_weak();

    scope.create_effect({
        let container = container.clone();
        let doc = doc_weak.clone();
        let main_doc = main_doc.clone();
        move || {
            let _version = doc_version.get(); // subscribe to doc changes
            for child in container.children() {
                child.remove();
            }

            let node_id = selected_node_id.get();

            if node_id.is_none() {
                let empty = make_el(&doc, "div");
                empty.set_attribute("class", "styles-empty");
                let t = make_text(&doc, "Select an element to view styles");
                empty.append_child(&t);
                container.append_child(&empty);
                return;
            }

            let nid = node_id.unwrap();

            // Extract styles directly from main document
            let styles = {
                let d = main_doc.0.borrow();
                extract_style_properties_from_doc(&d, nid)
            };

            let header = make_el(&doc, "div");
            header.set_attribute(
                "style",
                "padding: 4px 0; margin-bottom: 8px; color: #dcdcaa; font-weight: bold;",
            );
            let t = make_text(&doc, &format!("Node #{}", nid));
            header.append_child(&t);
            container.append_child(&header);

            let mut current_category = None;
            let mut cat_section: Option<NodeHandle> = None;

            for prop in &styles {
                if current_category != Some(prop.category) {
                    current_category = Some(prop.category);

                    let section = make_el(&doc, "div");
                    section.set_attribute("class", "styles-category");

                    let cat_header = make_el(&doc, "div");
                    cat_header.set_attribute("class", "styles-category-header");
                    let t = make_text(&doc, &prop.category.to_string());
                    cat_header.append_child(&t);
                    section.append_child(&cat_header);

                    container.append_child(&section);
                    cat_section = Some(section);
                }

                if let Some(ref parent_section) = cat_section {
                    let prop_row = make_el(&doc, "div");
                    prop_row.set_attribute("class", "styles-prop");

                    let name_span = make_el(&doc, "span");
                    name_span.set_attribute("class", "styles-prop-name");
                    let t = make_text(&doc, &format!("{}:", prop.name));
                    name_span.append_child(&t);
                    prop_row.append_child(&name_span);

                    let val_span = make_el(&doc, "span");
                    val_span.set_attribute("class", "styles-prop-value");
                    let t = make_text(&doc, &format!(" {}", prop.value));
                    val_span.append_child(&t);
                    prop_row.append_child(&val_span);

                    parent_section.append_child(&prop_row);
                }
            }
        }
    });

    container
}

fn render_performance_panel(scope: &mut RenderScope, store: DevToolsStore) -> NodeHandle {
    let fps_signal = store.fps;
    let frame_time_signal = store.frame_time_ms;
    let doc_version = store.doc_version;
    let main_doc = rinch_core::use_context::<MainDocRef>();

    let container = scope.create_element("div");

    // FPS
    let fps_div = scope.create_element("div");
    fps_div.set_attribute("class", "perf-fps");
    scope.create_effect({
        let fps_div = fps_div.clone();
        move || {
            fps_div.set_text(&format!("{:.0}", fps_signal.get()));
        }
    });
    container.append_child(&fps_div);

    let fps_label = scope.create_element("div");
    fps_label.set_attribute(
        "style",
        "text-align: center; color: #808080; margin-bottom: 16px;",
    );
    let fps_text = scope.create_text("FPS");
    fps_label.append_child(&fps_text);
    container.append_child(&fps_label);

    // Stats
    let stats = scope.create_element("div");
    stats.set_attribute("style", "padding: 0 8px;");

    // Frame time
    let ft_row = scope.create_element("div");
    ft_row.set_attribute("class", "perf-stat");
    let ft_label = scope.create_element("span");
    ft_label.set_attribute("class", "perf-stat-label");
    let t = scope.create_text("Frame time");
    ft_label.append_child(&t);
    ft_row.append_child(&ft_label);
    let ft_value = scope.create_element("span");
    ft_value.set_attribute("class", "perf-stat-value");
    scope.create_effect({
        let ft_value = ft_value.clone();
        move || {
            ft_value.set_text(&format!("{:.2} ms", frame_time_signal.get()));
        }
    });
    ft_row.append_child(&ft_value);
    stats.append_child(&ft_row);

    // Node count — reads directly from main document
    let nc_row = scope.create_element("div");
    nc_row.set_attribute("class", "perf-stat");
    let nc_label = scope.create_element("span");
    nc_label.set_attribute("class", "perf-stat-label");
    let t = scope.create_text("DOM nodes");
    nc_label.append_child(&t);
    nc_row.append_child(&nc_label);
    let nc_value = scope.create_element("span");
    nc_value.set_attribute("class", "perf-stat-value");
    scope.create_effect({
        let nc_value = nc_value.clone();
        let main_doc = main_doc.clone();
        move || {
            let _version = doc_version.get(); // subscribe to changes
            let doc = main_doc.0.borrow();
            let count = count_nodes_in_doc(&doc);
            nc_value.set_text(&format!("{}", count));
        }
    });
    nc_row.append_child(&nc_value);
    stats.append_child(&nc_row);

    container.append_child(&stats);
    container
}
