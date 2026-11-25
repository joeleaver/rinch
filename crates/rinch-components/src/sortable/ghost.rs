//! Presentation: the preview that follows the pointer.

use super::*;

/// Attributes a copy leaves behind: handlers, identity, focus, drag and
/// sortable wiring, and form-group membership (a copied checked radio on the
/// page would join its original's group and uncheck it).
///
/// Every `data-` attribute the runtime or a component acts on is left out by
/// prefix or name (handlers, focus traps, viewports and render surfaces, the
/// editor's `data-pm-*`, scroll locks, …); other `data-` attributes stay,
/// as the style hooks they usually are (`[data-state=open]`).
fn copied_attribute(name: &str) -> bool {
    const PREFIXES: [&str; 10] = [
        "data-on",
        "data-rid",
        "data-pm-",
        "data-rinch-",
        "data-sortable-",
        "data-tcm-",
        "data-text-sel",
        "data-cursor-",
        "data-selection-",
        "data-viewport",
    ];
    !(PREFIXES.iter().any(|p| name.starts_with(p))
        || matches!(
            name,
            "id" | "tabindex"
                | "autofocus"
                | "accesskey"
                | "contenteditable"
                | "draggable"
                | "popover"
                | "name"
                | "checked"
                | "form"
                | "data-drag-ghost"
                | "data-drag-window"
                | "data-render-surface"
                | "data-trap-focus"
                | "data-scroll-lock-exempt"
                | "data-nofocus"
                | "data-backdrop"
                | "data-video-player"
                | "data-rtc-view"
                | "data-theme-provider"
                | "data-graph-nowheel"
                | "data-preedit"
                | "data-block-index"
                | "data-user-rid"
        ))
}

/// A class the drag itself put on an item or a list: not part of its look.
fn drag_state_class(class: &str) -> bool {
    class.starts_with("rinch-sortable__item--drop")
        || matches!(
            class,
            DRAGGING_CLASS
                | LIFTED_CLASS
                | HIDDEN_CLASS
                | COLLAPSED_CLASS
                | SHIFTED_CLASS
                | REFLOWING_CLASS
                | ITEM_REFUSED_CLASS
                | DROP_INTO_CLASS
                | LIST_REFUSED_CLASS
                | REASON_SHOWN_CLASS
                | REASON_ABOVE_CLASS
        )
}

/// `node`'s element, empty, with its copyable attributes (drag-state classes
/// left out), built in `scope`. The tag keeps its case (`clipPath`).
fn copy_element(scope: &mut RenderScope, node: &NodeHandle) -> Option<NodeHandle> {
    let copy = scope.create_element(&node.tag_name()?);
    let mut attrs = node.attributes();
    attrs.sort();
    for (name, value) in attrs {
        if !copied_attribute(&name) {
            continue;
        }
        if name == "class" {
            let kept: Vec<&str> = value
                .split_whitespace()
                .filter(|c| !drag_state_class(c))
                .collect();
            copy.set_attribute("class", &kept.join(" "));
        } else {
            copy.set_attribute(&name, &value);
        }
    }
    Some(copy)
}

/// A copy of `node` and everything under it, built in `scope`. A text field
/// carries what it holds now, not only its `value` attribute.
fn copy_tree(scope: &mut RenderScope, node: &NodeHandle) -> Option<NodeHandle> {
    match node.node_type()? {
        3 => Some(scope.create_text(&node.text_content().unwrap_or_default())),
        1 => {
            let copy = copy_element(scope, node)?;
            if let Some(tag) = node.tag_name()
                && (tag.eq_ignore_ascii_case("input") || tag.eq_ignore_ascii_case("textarea"))
                && let Some(value) = node.live_value()
            {
                copy.set_attribute("value", &value);
            }
            for child in node.children() {
                if let Some(c) = copy_tree(scope, &child) {
                    copy.append_child(&c);
                }
            }
            Some(copy)
        }
        _ => None,
    }
}

/// Declarations a copy drops from the item's inline style: where it is
/// placed and how big it is are the preview's. (The app's own element keeps
/// all of its own.)
const PLACEMENT: [&str; 22] = [
    "position",
    "left",
    "top",
    "right",
    "bottom",
    "inset",
    "transform",
    "translate",
    "rotate",
    "scale",
    "transition",
    "z-index",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "width",
    "height",
    "flex",
    "opacity",
    "pointer-events",
];

/// A stand-in for an element above the item: its tag, classes and attributes
/// (inline style included), and `display: contents`, so it draws no box but
/// the cascade runs through it as through the original — inherited values
/// (font, colour, `--rinch-sortable-ghost-*`), descendant selectors
/// (`.column .card`) and classes setting custom properties all reach the
/// preview, while the original's own box (size, padding, a `transform`) does
/// not.
fn mirror(scope: &mut RenderScope, node: &NodeHandle) -> Option<NodeHandle> {
    let m = copy_element(scope, node)?;
    m.set_style("display", "contents");
    Some(m)
}

pub(super) fn px(v: f32) -> String {
    format!("{v}px")
}

/// The preview a pointer drag of `item` shows, mounted on `<body>`: the
/// wrapper and the scope it was built in.
///
/// The wrapper (`display: contents`) holds a stand-in for every element
/// between `<body>` and the item, then the preview's body: the item's copy,
/// or the app's element inside a stand-in for the item. So the preview is
/// styled as if it were still in its list. The body is the one box: placed
/// `position: fixed` by the wrapper's `--rinch-sortable-ghost-x/-y`, at the
/// item's layout size, scaled to its painted size (an item inside a scaled
/// ancestor), and given its look by the `--rinch-sortable-ghost-*`
/// properties it inherits through the stand-ins.
pub(super) fn build_ghost(
    cfg: &ListConfig,
    item: &NodeHandle,
    key: &str,
    grab: Option<&SortPointer>,
) -> Option<(NodeHandle, Option<RenderScope>, (f32, f32))> {
    let modifier = match &cfg.preview {
        SortPreview::Clone => "rinch-sortable__ghost-body--clone",
        SortPreview::Custom(_) => "rinch-sortable__ghost-body--custom",
        SortPreview::None | SortPreview::Snapshot => return None,
    };
    let doc = cfg.doc.upgrade()?;
    let ghost = GHOST_WRAPPERS.with_mut(|w| {
        let key = cfg.body.doc_key();
        if let Some(g) = w.get(key).filter(|g| g.is_valid()) {
            return g.clone();
        }
        let g = RenderScope::new(doc.clone(), cfg.body.node_id()).create_element("div");
        g.set_attribute("class", GHOST_CLASS);
        g.set_attribute(GHOST_ATTR, "");
        g.set_attribute("aria-hidden", "true");
        w.insert(key, g.clone());
        g
    });
    let mut scope = RenderScope::new(doc, cfg.body.node_id());
    // The item's painted box (`grab`) and its layout size: their ratio is the
    // scale of the ancestors, and the body is placed by its centre so that
    // scale does not move it.
    let (pw, ph) = grab.map_or((0.0, 0.0), |g| (g.width, g.height));
    let (lw, lh) = item.layout_size().unwrap_or((pw, ph));
    let fit = if lw > 0.0 { pw / lw } else { 1.0 };
    let fix = ((lw - pw) / 2.0, (lh - ph) / 2.0);
    let (x, y) = grab.map_or((0.0, 0.0), |g| {
        (g.x - g.grab_x - fix.0, g.y - g.grab_y - fix.1)
    });
    ghost.set_styles(&[
        (GHOST_X, &px(x)),
        (GHOST_Y, &px(y)),
        ("--rinch-sortable-ghost-width", &px(lw)),
        ("--rinch-sortable-ghost-height", &px(lh)),
        ("--rinch-sortable-ghost-fit", &fit.to_string()),
    ]);
    // The stand-ins, outermost first.
    let mut chain = Vec::new();
    let mut at = item.parent_node();
    let mut reached = false;
    while let Some(n) = at {
        if n.node_id() == cfg.body.node_id() {
            reached = true;
            break;
        }
        at = n.parent_node();
        chain.push(n);
    }
    if !reached {
        chain.clear();
    }
    let mut parent = ghost.clone();
    for n in chain.iter().rev() {
        if let Some(m) = mirror(&mut scope, n) {
            parent.append_child(&m);
            parent = m;
        }
    }
    let body = match &cfg.preview {
        SortPreview::Clone => copy_tree(&mut scope, item),
        SortPreview::Custom(f) => {
            if let Some(m) = mirror(&mut scope, item) {
                parent.append_child(&m);
                parent = m;
            }
            let f = f.clone();
            let key = key.to_string();
            // Owned by the preview's scope: what the function makes (signals,
            // memos, effects) goes when the drag ends.
            let _owner = scope.push_owner();
            Some(rinch_core::untracked_handler(|| f(&mut scope, &key)))
        }
        _ => None,
    };
    if let Some(body) = body {
        if matches!(cfg.preview, SortPreview::Clone) {
            clear_props(&body, &PLACEMENT);
        }
        body.add_class("rinch-sortable__ghost-body");
        body.add_class(modifier);
        parent.append_child(&body);
    }
    cfg.body.append_child(&ghost);
    Some((ghost, Some(scope), fix))
}

/// The pointer is at (`x`, `y`): move the preview and tell the pointer signal.
pub(super) fn follow_pointer(x: f32, y: f32) {
    let ghost = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let a = a.as_mut().filter(|a| a.lift.is_none())?;
        let fix = a.ghost_fix;
        let g = a.grab.as_mut()?;
        if g.x == x && g.y == y {
            return None;
        }
        g.x = x;
        g.y = y;
        Some((
            a.ghost.as_ref().map(|(n, _)| n.clone()),
            g.grab_x + fix.0,
            g.grab_y + fix.1,
        ))
    });
    let Some((node, gx, gy)) = ghost else {
        return;
    };
    if let Some(node) = node {
        node.set_styles(&[(GHOST_X, &px(x - gx)), (GHOST_Y, &px(y - gy))]);
    }
    publish();
}
