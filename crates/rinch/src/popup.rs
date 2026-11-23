//! Popup windows (#2552): a window the system places against an element of
//! another window, sized by its content, that can leave its parent's
//! surface (`xdg_popup` on Wayland). A menu from a 40 px panel, a tooltip
//! above a bar, a submenu.
//!
//! Open one with [`open_popup`]. Where the platform has none
//! ([`popups_supported`] is `false`: X11, Windows and macOS for now, the web,
//! Android), the call answers [`PopupError::Unsupported`] and the caller
//! keeps drawing its popup inside its own window.
//!
//! - **A popup is a window.** It has its own document, built once from its
//!   content, and a [`WindowHandle`]. It belongs to the owner in scope as a
//!   window from [`crate::open_window`] does.
//! - **Its parent** is the anchor's window. A popup anchored in a popup is
//!   above it in the same **chain**: a submenu.
//! - **Its size** is its content's, up to [`PopupProps::max_size`] (by
//!   default the parent's output), and follows the content.
//! - **Closing.** A window's popups close before it, topmost first, whatever
//!   owns them; a popup the system or the user dismissed tells
//!   [`PopupProps::on_dismissed`] why ([`Dismissal`]).
//! - **Input** for a [`PopupKind::Menu`]: a press in any of the app's windows
//!   outside the popups above it closes them and is delivered; keys reaching
//!   any window of the chain go to the topmost menu; Escape closes it; the
//!   app's menu shortcuts are held while a menu is open.

use std::cell::Cell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::{
    Adjust, ContentSizing, Dismissal, InputRegion, Placement, PopupKind, PopupPlaced, Side,
    WindowProps,
};
use rinch_core::reactive::Signal;

use crate::WindowHandle;
use crate::window_handle;

/// What a popup hangs from. Its window is the popup's parent.
#[derive(Clone, Debug)]
pub enum PopupAnchor {
    /// An element's border box (`NodeHandle::bounding_rect`), in the
    /// element's window, which is the parent. When the element moves while
    /// the popup is open, the popup is placed again.
    Node(NodeHandle),
    /// An element's border box stretched across its window's whole cross
    /// axis: the full height for a popup placed above or below it, the full
    /// width for one placed to its left or right. A menu from a bar's button
    /// anchored this way opens past the bar's edge and never covers the bar;
    /// it is placed again when the element moves, the bar is resized or the
    /// zoom changes, as [`PopupAnchor::Node`] is.
    NodeAcross(NodeHandle),
    /// A rectangle in a window, in CSS px (the app's zoom is applied to it):
    /// a pointer position for a context menu is a 1x1 rectangle.
    /// `window: None` is the window whose code is running
    /// ([`WindowHandle::current`]).
    Rect {
        window: Option<WindowHandle>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
}

impl Default for PopupAnchor {
    /// The top-left 1x1 rectangle of the current window.
    fn default() -> Self {
        PopupAnchor::Rect {
            window: None,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

/// How to open a popup: [`open_popup`]'s props.
#[derive(Clone, Default)]
pub struct PopupProps {
    /// What it hangs from.
    pub anchor: PopupAnchor,
    /// Which side of the anchor it opens on, and how it lines up there.
    pub placement: Placement,
    /// Logical px, applied after the placement: positive `y` is a gap below
    /// the anchor, negative moves it up (a submenu's first item level with
    /// its opener).
    pub offset: (f32, f32),
    /// What the system may do when it does not fit. Default: flip and slide
    /// on both axes. Add [`Adjust::RESIZE_Y`] for a menu that may be taller
    /// than the screen: it is given the height left, and its content
    /// scrolls.
    pub adjust: Adjust,
    /// A menu (a grab, the keyboard) or a tooltip (neither).
    pub kind: PopupKind,
    /// The largest size its content may take, logical px. Default: its
    /// parent's output.
    pub max_size: Option<(f32, f32)>,
    /// Whether its surface is transparent where nothing is painted (rounded
    /// corners, a shadow).
    pub transparent: bool,
    /// Give a menu the keyboard through its parent layer surface (Wayland):
    /// before the popup is made, its parent's keyboard interactivity is
    /// raised to `exclusive` and rinch waits for the keyboard to arrive; the
    /// interactivity the parent had is restored when the chain closes. For a
    /// panel whose interactivity is `none` on a compositor that does not give
    /// a grabbing popup the keyboard (sway, labwc). Off by default: KWin
    /// gives the menu the keyboard with nothing raised, and raising there
    /// leaves the panel holding the keyboard. Ignored for a tooltip and for a
    /// parent that is not a layer surface.
    pub keyboard_via_parent: bool,
    /// Told once why the popup went, for every cause but the app's own
    /// close ([`Dismissal`]).
    pub on_dismissed: Option<Rc<dyn Fn(Dismissal)>>,
}

impl std::fmt::Debug for PopupProps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PopupProps")
            .field("anchor", &self.anchor)
            .field("placement", &self.placement)
            .field("offset", &self.offset)
            .field("adjust", &self.adjust)
            .field("kind", &self.kind)
            .field("max_size", &self.max_size)
            .field("transparent", &self.transparent)
            .field("keyboard_via_parent", &self.keyboard_via_parent)
            .finish_non_exhaustive()
    }
}

/// Why [`open_popup`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PopupError {
    /// Not on this platform or session ([`popups_supported`] is `false`),
    /// or turned off with `App::popup_windows(false)`.
    Unsupported,
    /// A [`PopupKind::Menu`] was asked for, and its parent chain has
    /// received no press to name as the grab's cause (a menu opened at
    /// start-up, or from D-Bus with no input).
    NoUserInput,
    /// The anchor's window is not open, or its node is in no window.
    NoParent,
    /// A [`PopupKind::Menu`] was asked for on a [`PopupKind::Tooltip`].
    MenuOnTooltip,
}

impl std::fmt::Display for PopupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "popup windows are not supported here",
            Self::NoUserInput => "a menu popup needs a press in its parent chain to open from",
            Self::NoParent => "the popup's anchor is in no open window",
            Self::MenuOnTooltip => "a menu popup cannot open on a tooltip popup",
        })
    }
}

impl std::error::Error for PopupError {}

/// A popup's `on_dismissed`.
pub(crate) type OnDismissed = Rc<dyn Fn(Dismissal)>;

/// What the registry keeps of a popup beside its window entry.
pub(crate) struct PopupEntry {
    /// The parent's window id (`0`: the harness's mounted app).
    pub(crate) parent: u64,
    pub(crate) props: PopupProps,
    /// Where the system put it ([`WindowHandle::placement`]).
    pub(crate) placed: Option<Signal<Option<PopupPlaced>>>,
    /// Why it goes, once known: the first cause recorded.
    pub(crate) dismissal: Option<Dismissal>,
    /// When the app asked for it: `[PERF]`'s "first frame" is measured
    /// from here.
    pub(crate) asked_at: std::time::Instant,
    /// The anchor rectangle last given to the system, logical px (x, y,
    /// width, height): a different one re-places the popup.
    pub(crate) sent_anchor: Option<[i32; 4]>,
}

impl PopupEntry {
    /// Its placement signal made in `scope`, the window entry's.
    pub(crate) fn in_scope(mut self, scope: &rinch_core::reactive::Scope) -> Self {
        self.placed = Some(scope.run(|| Signal::new(None)));
        self
    }

    /// What to tell, and the cause, when a cause was recorded and someone
    /// listens.
    pub(crate) fn take_dismissed(&mut self) -> Option<(OnDismissed, Dismissal)> {
        let why = self.dismissal.take()?;
        self.props.on_dismissed.clone().map(|f| (f, why))
    }
}

/// How recent a press in a menu's parent chain must be for the menu to
/// open (`Err(NoUserInput)` otherwise): 10 s. A menu opens from a click or a
/// key, at once or after a short delay (a hover timer, a D-Bus round trip
/// for a tray item's menu); a press older than this names nothing the user
/// meant.
pub const PRESS_RECENT: std::time::Duration = std::time::Duration::from_secs(10);

thread_local! {
    /// Whether the event loop's display has popups (the shell sets it once
    /// the loop runs).
    static SUPPORTED: Cell<bool> = const { Cell::new(false) };
    /// `App::popup_windows(false)`.
    static TURNED_OFF: Cell<bool> = const { Cell::new(false) };
}

/// The shell: whether its display has popup windows (a Wayland session).
pub(crate) fn set_supported(on: bool) {
    SUPPORTED.with(|s| s.set(on));
}

/// `App::popup_windows`: `false` turns popups off for the app, so every
/// caller takes its in-window form.
pub(crate) fn set_turned_off(off: bool) {
    TURNED_OFF.with(|s| s.set(off));
}

/// Whether this platform and session can show popup windows: a Wayland
/// session (with xdg-shell) whose app did not turn them off with
/// `App::popup_windows(false)`. `false` on X11, Windows and macOS for now,
/// on the web and on Android, and before the event loop runs. Under
/// `rinch::testing`, `true`: the harness stands in a Wayland compositor.
pub fn popups_supported() -> bool {
    if TURNED_OFF.with(Cell::get) {
        return false;
    }
    if SUPPORTED.with(Cell::get) {
        return true;
    }
    #[cfg(feature = "testing")]
    {
        crate::testing::captures()
    }
    #[cfg(not(feature = "testing"))]
    {
        false
    }
}

/// Open a popup: a window placed by the system against `props.anchor`,
/// sized by its content, with its own document built once from `content`.
/// Main thread only. The handle is usable at once; the popup appears on the
/// event loop's next turn.
///
/// It belongs to the owner in scope, as [`crate::open_window`]'s
/// windows do (a popup opened in a menu row's handler closes with the row).
/// Its parent is the anchor's window, whatever owns it, and it always closes
/// before its parent.
///
/// A [`PopupKind::Menu`] takes a grab named by the newest press its parent
/// chain received (`Err(NoUserInput)` with none). Only one menu chain is open
/// at a time: a menu whose parent is not in the open chain replaces it
/// ([`Dismissal::Replaced`]), and is made one turn after the old chain has
/// gone.
///
/// In the popup's document the content sits in a box rinch sizes (see the
/// guide's "A window sized by its content"): pad the content, not `<body>`,
/// and keep viewport units out of it.
pub fn open_popup<F>(props: PopupProps, content: F) -> Result<WindowHandle, PopupError>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    if !popups_supported() {
        return Err(PopupError::Unsupported);
    }
    let parent = parent_of(&props.anchor)?;
    if props.kind == PopupKind::Menu {
        let parent_kind = window_handle::with_popup(parent, |p| p.props.kind);
        if parent_kind == Some(PopupKind::Tooltip) {
            return Err(PopupError::MenuOnTooltip);
        }
        if !window_handle::chain_pressed(parent) {
            return Err(PopupError::NoUserInput);
        }
    }
    let window_props = window_props_for(&props);
    let menu = props.kind == PopupKind::Menu;
    let id = Rc::new(Cell::new(0u64));
    let content_id = id.clone();
    let content: window_handle::WindowContent = Box::new(move |scope: &mut RenderScope| {
        let node = content(scope);
        if menu {
            // Escape closes this menu (and what is above it), as a native
            // menu's does; keys reach the topmost menu's document, so its
            // entry is the one asked.
            let id = content_id.get();
            let handle = rinch_core::push_dismiss_handler_within(&node, move || {
                if WindowHandle::from_id(id).is_open_untracked() {
                    window_handle::dismiss(id, Dismissal::Escape);
                    true
                } else {
                    false
                }
            });
            scope.on_cleanup(move || drop(handle));
        }
        node
    });
    let entry = PopupEntry {
        parent,
        props,
        placed: None,
        dismissal: None,
        asked_at: std::time::Instant::now(),
        sent_anchor: None,
    };
    let handle = window_handle::open(window_props, content, None, Some(entry));
    id.set(handle.id());
    Ok(handle)
}

/// The window a popup anchored at `anchor` opens on.
fn parent_of(anchor: &PopupAnchor) -> Result<u64, PopupError> {
    let parent = match anchor {
        PopupAnchor::Node(node) | PopupAnchor::NodeAcross(node) => {
            let doc = node.doc_key();
            if doc == 0 {
                return Err(PopupError::NoParent);
            }
            window_handle::window_of_doc(doc)
        }
        PopupAnchor::Rect { window, .. } => match window.or_else(WindowHandle::current) {
            Some(w) => w.id(),
            None => {
                let doc = rinch_core::current_dispatching_doc().ok_or(PopupError::NoParent)?;
                window_handle::window_of_doc(doc)
            }
        },
    };
    if window_handle::can_parent(parent) {
        Ok(parent)
    } else {
        Err(PopupError::NoParent)
    }
}

/// The window props a popup's window is made with: sized by its content up
/// to `max_size` (the shell fills in the parent's output when that is
/// `None`), never a layer surface, and, for a tooltip, taking no pointer
/// input.
pub(crate) fn window_props_for(props: &PopupProps) -> WindowProps {
    let mut sizing = ContentSizing::new();
    if let Some((w, h)) = props.max_size {
        sizing = sizing
            .with_max_width(w.max(1.0).ceil() as u32)
            .with_max_height(h.max(1.0).ceil() as u32);
    }
    let (width, height) = props.max_size.map_or((1920, 1080), |(w, h)| {
        (w.max(1.0) as u32, h.max(1.0) as u32)
    });
    WindowProps {
        title: match props.kind {
            PopupKind::Menu => "popup menu".to_string(),
            PopupKind::Tooltip => "popup tooltip".to_string(),
        },
        width,
        height,
        size_to_content: Some(sizing),
        wayland_layer_shell: Some(false),
        borderless: true,
        resizable: false,
        transparent: props.transparent,
        input_region: match props.kind {
            PopupKind::Menu => InputRegion::All,
            PopupKind::Tooltip => InputRegion::None,
        },
        ..WindowProps::default()
    }
}

impl WindowHandle {
    /// Where the system put this popup, relative to its parent, and whether
    /// it flipped. Reactive; `None` until it is placed, and for a window
    /// that is not a popup.
    pub fn placement(self) -> Option<PopupPlaced> {
        let placed = window_handle::with_popup(self.id(), |p| p.placed).flatten()?;
        placed.get()
    }

    /// Whether this window is a popup ([`open_popup`]).
    pub fn is_popup(self) -> bool {
        window_handle::with_popup(self.id(), |_| ()).is_some()
    }

    /// Close the popup chain this window is in: every popup of it, topmost
    /// first. From a menu item's handler, in a menu or any of its submenus,
    /// this closes the whole menu. On a window that is not a popup, its
    /// popups. Nothing is told to their `on_dismissed`: the app closed them.
    pub fn close_popup_chain(self) {
        let links = window_handle::popups();
        let root = chain::root(self.id(), &links);
        for id in chain::children_of(root, &links) {
            WindowHandle::from_id(id).close();
        }
    }
}

/// The pure chain rules, over `(id, parent, kind)` links (oldest first).
pub(crate) mod chain {
    use rinch_core::element::PopupKind;

    pub(crate) type Links = [(u64, u64, PopupKind)];

    fn parent(id: u64, links: &Links) -> Option<u64> {
        links.iter().find(|l| l.0 == id).map(|l| l.1)
    }

    fn kind(id: u64, links: &Links) -> Option<PopupKind> {
        links.iter().find(|l| l.0 == id).map(|l| l.2)
    }

    /// `id`, then its parent while that is a popup, and so on to the
    /// window at the chain's foot (included).
    pub(crate) fn ancestors_and_self(id: u64, links: &Links) -> Vec<u64> {
        let mut out = vec![id];
        let mut at = id;
        while let Some(p) = parent(at, links) {
            if out.contains(&p) {
                break;
            }
            out.push(p);
            at = p;
        }
        out
    }

    /// The window at the foot of `id`'s chain: `id` itself when it is no
    /// popup.
    pub(crate) fn root(id: u64, links: &Links) -> u64 {
        *ancestors_and_self(id, links).last().expect("never empty")
    }

    /// The popups directly on `id`, newest first.
    pub(crate) fn children_of(id: u64, links: &Links) -> Vec<u64> {
        let mut out: Vec<u64> = links.iter().filter(|l| l.1 == id).map(|l| l.0).collect();
        out.sort_unstable_by(|a, b| b.cmp(a));
        out
    }

    /// Every popup above `id` (not `id`), in the order they close: newest
    /// first, the reverse of their opening, so each popup goes before the
    /// one it is on (a popup always opens after its parent). wlroots ends
    /// the connection of a client that destroys a popup that is not the
    /// topmost.
    pub(crate) fn close_order(id: u64, links: &Links) -> Vec<u64> {
        let mut out = Vec::new();
        let mut todo = vec![id];
        while let Some(at) = todo.pop() {
            for child in children_of(at, links) {
                if !out.contains(&child) {
                    out.push(child);
                    todo.push(child);
                }
            }
        }
        out.sort_unstable_by(|a, b| b.cmp(a));
        out
    }

    /// The topmost menu of the chain whose foot is `root`: the newest menu
    /// above it.
    pub(crate) fn topmost_menu(root: u64, links: &Links) -> Option<u64> {
        close_order(root, links)
            .into_iter()
            .filter(|&p| kind(p, links) == Some(PopupKind::Menu))
            .max()
    }

    /// Where a key that reached window `id` goes while a menu chain is
    /// open: the topmost menu of `id`'s chain. `None` when its chain has no
    /// menu (the key stays where it arrived).
    pub(crate) fn key_target(id: u64, links: &Links) -> Option<u64> {
        topmost_menu(root(id, links), links)
    }

    /// The menus a press in window `id` closes: every open menu that is not
    /// `id` or below it in its chain. A press in a menu closes only the
    /// popups above it; a press anywhere else in the app closes the chain.
    pub(crate) fn press_closes(id: u64, links: &Links) -> Vec<u64> {
        let keep = ancestors_and_self(id, links);
        links
            .iter()
            .filter(|l| l.2 == PopupKind::Menu && !keep.contains(&l.0))
            .map(|l| l.0)
            .collect()
    }

    /// Whether a menu `id` (registered already) on `parent` may raise
    /// `parent`'s keyboard (`keyboard_via_parent`): only as the first menu of
    /// its chain, from a parent that is no popup. Never while another menu
    /// of the chain holds a grab (raising then fails on wlroots and
    /// dismisses on KWin). `id` itself is not counted.
    pub(crate) fn may_raise_keyboard(id: u64, parent: u64, links: &Links) -> bool {
        let others: Vec<_> = links.iter().copied().filter(|l| l.0 != id).collect();
        topmost_menu(parent, &others).is_none() && !others.iter().any(|l| l.0 == parent)
    }

    /// Whether any menu popup is open.
    pub(crate) fn menu_open(links: &Links) -> bool {
        links.iter().any(|l| l.2 == PopupKind::Menu)
    }
}

/// Where a popup was put, read against what it asked for: `pos` and `size`
/// in its parent's logical px, `anchor` the rectangle it was placed against.
pub(crate) fn placed_from(
    anchor: [i32; 4],
    placement: Placement,
    pos: (f64, f64),
    size: (f64, f64),
) -> PopupPlaced {
    let [ax, ay, aw, ah] = anchor.map(f64::from);
    let (x, y) = pos;
    let (w, h) = size;
    // Half a pixel of slack for rounding at fractional scales.
    let flipped_y = match placement.side {
        Side::Bottom => y + h <= ay + 0.5,
        Side::Top => y >= ay + ah - 0.5,
        _ => false,
    };
    let flipped_x = match placement.side {
        Side::Right => x + w <= ax + 0.5,
        Side::Left => x >= ax + aw - 0.5,
        _ => false,
    };
    PopupPlaced {
        x,
        y,
        width: w,
        height: h,
        flipped_x,
        flipped_y,
    }
}

/// The anchor rectangle to give the system: `bounds` (CSS px) times `zoom`
/// (logical px), kept inside the parent's `parent` logical size and rounded
/// outward to whole px, at least 1x1. `xdg_positioner` refuses a rectangle
/// that leaves the parent.
pub(crate) fn anchor_rect(bounds: (f32, f32, f32, f32), zoom: f64, parent: (f64, f64)) -> [i32; 4] {
    let (x, y, w, h) = bounds;
    let z = zoom;
    let (pw, ph) = (parent.0.max(1.0), parent.1.max(1.0));
    let x0 = (f64::from(x) * z).clamp(0.0, pw - 1.0).floor();
    let y0 = (f64::from(y) * z).clamp(0.0, ph - 1.0).floor();
    let x1 = (f64::from(x + w) * z).clamp(0.0, pw).ceil().max(x0 + 1.0);
    let y1 = (f64::from(y + h) * z).clamp(0.0, ph).ceil().max(y0 + 1.0);
    [x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32]
}

/// The anchor rectangle to give the system for `props` now ([`anchor_rect`]
/// of the anchor's bounds), stretched across the parent's cross axis for a
/// [`PopupAnchor::NodeAcross`]. `None` for a node that is not laid out.
pub(crate) fn anchor_rect_for(
    props: &PopupProps,
    zoom: f64,
    parent: (f64, f64),
) -> Option<[i32; 4]> {
    let rect = anchor_rect(anchor_bounds(&props.anchor)?, zoom, parent);
    Some(match props.anchor {
        PopupAnchor::NodeAcross(_) => across(rect, props.placement.side, parent),
        _ => rect,
    })
}

/// `rect` stretched across the parent (`parent`, logical px) on the axis
/// across `side`: the whole height for `Top`/`Bottom`, the whole width for
/// `Left`/`Right`.
pub(crate) fn across(rect: [i32; 4], side: Side, parent: (f64, f64)) -> [i32; 4] {
    let [x, y, w, h] = rect;
    match side {
        Side::Top | Side::Bottom => [x, 0, w, (parent.1.ceil() as i32).max(1)],
        Side::Left | Side::Right => [0, y, (parent.0.ceil() as i32).max(1), h],
    }
}

/// The anchor's rectangle in its window now, CSS px: the node's border
/// box, or the given rectangle. `None` for a node that is not laid out.
pub(crate) fn anchor_bounds(anchor: &PopupAnchor) -> Option<(f32, f32, f32, f32)> {
    match anchor {
        PopupAnchor::Node(node) | PopupAnchor::NodeAcross(node) => {
            node.bounding_rect().map(|b| (b.x, b.y, b.width, b.height))
        }
        PopupAnchor::Rect {
            x,
            y,
            width,
            height,
            ..
        } => Some((*x, *y, *width, *height)),
    }
}

/// Whether a press at `point` (CSS px) in window `window` is on popup
/// `id`'s anchor.
pub(crate) fn press_on_anchor(id: u64, window: u64, point: Option<(f32, f32)>) -> bool {
    let Some((px, py)) = point else {
        return false;
    };
    let Some((parent, anchor)) =
        window_handle::with_popup(id, |p| (p.parent, p.props.anchor.clone()))
    else {
        return false;
    };
    if parent != window {
        return false;
    }
    anchor_bounds(&anchor)
        .is_some_and(|(x, y, w, h)| px >= x && px < x + w && py >= y && py < y + h)
}

/// A press reached window `window` at `point` (CSS px; `None` for a touch
/// with no position): every menu it closes is recorded as
/// [`Dismissal::PressOutside`]. Answers the menus to close, which the
/// caller closes (the shell at the end of the handler, so the press is
/// delivered first).
pub(crate) fn light_dismiss(window: u64, point: Option<(f32, f32)>) -> Vec<u64> {
    let links = window_handle::popups();
    let closes = chain::press_closes(window, &links);
    for &id in &closes {
        let on_anchor = press_on_anchor(id, window, point);
        window_handle::note_dismissal(id, Dismissal::PressOutside { on_anchor });
    }
    closes
}

#[cfg(test)]
#[path = "popup_tests.rs"]
mod tests;
