//! Keyboard navigation and Escape for `DropdownMenu` and `ContextMenu`
//! (issues #2169, #2170, #2214), shared with `DropdownMenuSub`'s flyouts.
//!
//! **What a key does, and where the defaults come from.** Chrome's own menus
//! (`ui/views/controls/menu/menu_controller.cc`, `menu_config.h`):
//!
//! - ArrowDown / ArrowUp move to the next / previous enabled item of the panel
//!   that holds the focus, wrapping at the ends (`arrow_key_selection_wraps =
//!   true`; `MenuKeyOptions::loop_focus`). With the focus outside the menu —
//!   a menu opened with the mouse has no selection — Down picks the root
//!   panel's first item and Up its last.
//! - Home / End pick the first / last (`MoveSelectionToFirstOrLastItem`).
//! - A letter or digit is type-ahead (`SelectByChar`): the items whose label
//!   starts with it, cycling from the focused one; a single match is
//!   activated (a submenu row opens its submenu).
//! - Escape closes the innermost open thing: a submenu's own dismiss entry is
//!   pushed when it opens, so it sits above the menu's (LIFO), and the menu
//!   closes on the next Escape. Focus goes back to the element that held it
//!   when the menu opened.
//!
//! **How the keys get here.** While a menu is open it holds an app key
//! listener ([`rinch_core::on_app_key`]): offered every key press nothing
//! earlier consumed, wherever the focus is — a context menu opened by a right
//! press has no focused element inside it, and Chrome's native one still takes
//! the arrows. A stack of open menus makes the listener of the most recently
//! opened one the only one that answers, so a menu opened inside another's
//! panel gets the keys first. Keys are declined while a text field holds the
//! focus (a filter field in a menu keeps its arrows and its typing), and keys
//! with Ctrl, Alt or Meta are never taken.
//!
//! A submenu row's own keys (ArrowRight / Enter / Space open it, ArrowLeft
//! closes the submenu the focus is in) are an element `onkeydown` on the
//! submenu (`dropdown_menu_sub.rs`), which the dispatch reaches before any app
//! key listener.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events::{EventHandlerId, KeyEventData, KeyEventKind};
use rinch_core::{AppKeyListener, DismissHandle};

/// The classes of a menu panel: the element whose items the arrows walk.
pub(crate) const PANEL_CLASSES: [&str; 3] = [
    "rinch-dropdown-menu__dropdown",
    "rinch-context-menu__dropdown",
    "rinch-dropdown-menu__sub-dropdown",
];

/// The class every menu row carries (`DropdownMenuItem`, a submenu's row).
pub(crate) const ITEM_CLASS: &str = "rinch-dropdown-menu__item";

/// An escape hatch: offered every key the menu would see, before it; `true`
/// consumes the key.
pub type MenuKeyHook = Rc<dyn Fn(&KeyEventData) -> bool>;

/// The keyboard options a menu carries (its props of the same names).
#[derive(Clone)]
pub(crate) struct MenuKeyOptions {
    pub close_on_escape: bool,
    pub keyboard_nav: bool,
    pub loop_focus: bool,
    pub typeahead: bool,
    pub on_key: Option<MenuKeyHook>,
}

pub(crate) fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .is_some_and(|c| c.split_whitespace().any(|t| t == class))
}

pub(crate) fn is_panel(node: &NodeHandle) -> bool {
    PANEL_CLASSES.iter().any(|c| has_class(node, c))
}

/// Whether `ancestor` is `node` or one of its ancestors.
pub(crate) fn contains(ancestor: &NodeHandle, node: &NodeHandle) -> bool {
    let mut cur = Some(node.clone());
    while let Some(n) = cur {
        if n.node_id() == ancestor.node_id() {
            return true;
        }
        cur = n.parent_node();
    }
    false
}

/// The nearest menu panel at or above `node`.
pub(crate) fn nearest_panel(node: &NodeHandle) -> Option<NodeHandle> {
    let mut cur = Some(node.clone());
    while let Some(n) = cur {
        if is_panel(&n) {
            return Some(n);
        }
        cur = n.parent_node();
    }
    None
}

fn is_disabled(node: &NodeHandle) -> bool {
    node.get_attribute("disabled").is_some()
        || has_class(node, "rinch-dropdown-menu__item--disabled")
}

/// The enabled rows of `panel` in document order — not the rows of a
/// panel nested in it (a submenu's flyout, a nested `DropdownMenu`).
pub(crate) fn panel_items(panel: &NodeHandle) -> Vec<NodeHandle> {
    fn walk(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
        for child in node.children() {
            if child.tag_name().is_none() {
                continue;
            }
            if is_panel(&child) {
                continue;
            }
            if has_class(&child, ITEM_CLASS) {
                // Not asked `is_rendered`: a flyout opened in this same
                // handler is not laid out yet on desktop, which answers from
                // the last resolve.
                if !is_disabled(&child) {
                    out.push(child);
                }
                continue;
            }
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(panel, &mut out);
    out
}

/// Whether `node` takes typed text (its arrows and letters are its own).
fn takes_text(node: &NodeHandle) -> bool {
    match node.tag_name().map(|t| t.to_ascii_lowercase()).as_deref() {
        Some("textarea") => true,
        Some("input") => {
            let ty = node
                .get_attribute("type")
                .unwrap_or_default()
                .to_ascii_lowercase();
            !matches!(
                ty.as_str(),
                "button"
                    | "checkbox"
                    | "radio"
                    | "submit"
                    | "reset"
                    | "image"
                    | "range"
                    | "color"
                    | "file"
                    | "hidden"
            )
        }
        _ => node
            .get_attribute("contenteditable")
            .is_some_and(|v| v != "false"),
    }
}

/// The label a row is matched against by type-ahead.
fn label_of(item: &NodeHandle) -> String {
    fn find_label(node: &NodeHandle) -> Option<NodeHandle> {
        for child in node.children() {
            if has_class(&child, "rinch-dropdown-menu__item-label") {
                return Some(child);
            }
            if let Some(found) = find_label(&child) {
                return Some(found);
            }
        }
        None
    }
    find_label(item)
        .unwrap_or_else(|| item.clone())
        .text_content()
        .unwrap_or_default()
}

/// Activate a row as a click would: its handler, or, for a submenu row, open
/// the submenu and focus its first row.
fn activate(item: &NodeHandle) {
    if crate::dropdown_menu_sub::open_from_trigger(item, true) {
        return;
    }
    item.focus();
    if let Some(rid) = item
        .get_attribute("data-rid")
        .and_then(|v| v.parse::<usize>().ok())
    {
        rinch_core::events::dispatch_event(EventHandlerId(rid));
    }
}

/// Answer a key for the menu whose root panel is `root_panel`. `true` when
/// the key was used.
pub(crate) fn handle_key(root_panel: &NodeHandle, k: &KeyEventData, opts: &MenuKeyOptions) -> bool {
    if k.kind != KeyEventKind::Down {
        return false;
    }
    if let Some(hook) = &opts.on_key
        && hook(k)
    {
        return true;
    }
    if !opts.keyboard_nav || k.ctrl || k.alt || k.meta {
        return false;
    }
    let focused = root_panel.active_element();
    if focused.as_ref().is_some_and(takes_text) {
        return false;
    }
    // The panel whose rows the key walks: the focus's own, or the root one.
    let panel = focused
        .as_ref()
        .filter(|f| contains(root_panel, f))
        .and_then(nearest_panel)
        .unwrap_or_else(|| root_panel.clone());
    let items = panel_items(&panel);
    if items.is_empty() {
        return false;
    }
    let cur = focused
        .as_ref()
        .and_then(|f| items.iter().position(|i| i.node_id() == f.node_id()));
    let last = items.len() - 1;
    let target = match k.key.as_str() {
        "ArrowDown" => Some(match cur {
            None => 0,
            Some(i) if i == last => {
                if opts.loop_focus {
                    0
                } else {
                    last
                }
            }
            Some(i) => i + 1,
        }),
        "ArrowUp" => Some(match cur {
            None => last,
            Some(0) => {
                if opts.loop_focus {
                    last
                } else {
                    0
                }
            }
            Some(i) => i - 1,
        }),
        "Home" => Some(0),
        "End" => Some(last),
        key if opts.typeahead && key.chars().count() == 1 => {
            let ch = key.chars().next().unwrap();
            if !ch.is_alphanumeric() {
                return false;
            }
            let ch = ch.to_lowercase().collect::<String>();
            let matches: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| label_of(it).trim_start().to_lowercase().starts_with(&ch))
                .map(|(i, _)| i)
                .collect();
            match matches.len() {
                0 => return true,
                1 => {
                    activate(&items[matches[0]]);
                    return true;
                }
                _ => Some(
                    cur.and_then(|c| matches.iter().copied().find(|&m| m > c))
                        .unwrap_or(matches[0]),
                ),
            }
        }
        _ => None,
    };
    match target {
        Some(i) => {
            items[i].focus();
            true
        }
        None => false,
    }
}

thread_local! {
    /// The open menus, most recently opened last. Only the last one's
    /// listener answers a key.
    static OPEN_MENUS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static NEXT_MENU: Cell<u64> = const { Cell::new(1) };
}

/// Hold the menu's keys and its Escape entry while `is_open`.
///
/// - `menu_root`: everything that is "the menu" for the focus return (its
///   panel and, for `DropdownMenu`, its target).
/// - `root_panel`: found when the menu opens (a caller may wrap it).
/// - `on_escape`: closes the menu; `None` leaves Escape to the app.
pub(crate) fn arm_menu_keys(
    scope: &mut RenderScope,
    menu_root: NodeHandle,
    root_panel: Rc<dyn Fn() -> Option<NodeHandle>>,
    is_open: Rc<dyn Fn() -> bool>,
    opts: MenuKeyOptions,
    on_escape: Option<Rc<dyn Fn()>>,
) {
    struct Held {
        id: u64,
        listener: Option<AppKeyListener>,
        dismiss: Option<DismissHandle>,
        opener: Option<NodeHandle>,
        panel: Option<NodeHandle>,
    }
    let held: Rc<RefCell<Option<Held>>> = Rc::new(RefCell::new(None));

    fn release(h: Held) {
        OPEN_MENUS.with(|m| m.borrow_mut().retain(|&x| x != h.id));
        if let Some(l) = h.listener {
            l.remove();
        }
        drop(h.dismiss);
        // Back to the opener when the focus was left inside the menu, which
        // is closing (Chrome returns it there).
        if let (Some(panel), Some(opener)) = (h.panel, h.opener)
            && let Some(f) = panel.active_element()
            && contains(&panel, &f)
            && opener.is_connected()
        {
            opener.focus();
        }
    }

    let held_e = held.clone();
    scope.create_effect(move || {
        let open = is_open();
        let holding = held_e.borrow().is_some();
        if open && !holding {
            let id = NEXT_MENU.with(|n| {
                let id = n.get();
                n.set(id + 1);
                id
            });
            OPEN_MENUS.with(|m| m.borrow_mut().push(id));
            let panel = root_panel();
            let opener = menu_root.active_element();
            let listener = panel.clone().map(|panel| {
                let opts = opts.clone();
                rinch_core::reactive::unowned(|| {
                    rinch_core::on_app_key(move |k| {
                        let top = OPEN_MENUS.with(|m| m.borrow().last().copied());
                        top == Some(id) && panel.is_rendered() && handle_key(&panel, k, &opts)
                    })
                })
            });
            let dismiss = match (&on_escape, &panel) {
                (Some(close), Some(panel)) if opts.close_on_escape => {
                    let close = close.clone();
                    let shown = panel.clone();
                    Some(rinch_core::reactive::unowned(|| {
                        rinch_core::push_dismiss_handler(menu_root.doc_key(), move || {
                            if !shown.is_rendered() {
                                return false;
                            }
                            close();
                            true
                        })
                    }))
                }
                _ => None,
            };
            *held_e.borrow_mut() = Some(Held {
                id,
                listener,
                dismiss,
                opener,
                panel,
            });
        } else if !open && holding {
            let h = held_e.borrow_mut().take();
            if let Some(h) = h {
                release(h);
            }
        }
    });
    scope.on_cleanup(move || {
        let h = held.borrow_mut().take();
        if let Some(h) = h {
            OPEN_MENUS.with(|m| m.borrow_mut().retain(|&x| x != h.id));
            if let Some(l) = h.listener {
                l.remove();
            }
            drop(h.dismiss);
        }
    });
}
