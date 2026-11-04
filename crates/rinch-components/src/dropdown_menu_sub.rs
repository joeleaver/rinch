//! `DropdownMenuSub`: a cascading submenu row in a `DropdownMenu`,
//! a `ContextMenu` or another submenu (issue #2214).
//!
//! # Defaults, and where they come from
//!
//! Chrome's own menus (`ui/views/controls/menu/`), which its context menu
//! uses on Windows and Linux:
//!
//! - **Open delay 400 ms** — `MenuConfig::show_delay = 400`, the delay
//!   "between when menus are selected or moused over and the menu appears".
//! - **Close delay 400 ms** — Chrome has no separate close delay: hovering a
//!   sibling row starts the same show timer, and when it fires
//!   `MenuController::CommitPendingSelection` closes the open submenu.
//! - **The pointer leaving every menu keeps the submenu open**
//!   (`HandleMouseLocation`: views "keeps the submenu open" for imprecise
//!   movement). Only a move onto another row of the same panel closes it.
//! - **No safe triangle** — Chrome's diagonal tolerance is the 400 ms timer.
//!   [`DropdownMenuSub::safe_triangle`] turns one on.
//! - **3 px overlap** with the parent panel — `submenu_horizontal_overlap =
//!   3`, the default of `--rinch-dropdown-menu-sub-offset` (`-3px`). The
//!   flyout's first row lines up with the row that opened it.
//! - **Flip** to the panel's other side when it would cross the window's
//!   right edge, and **shift** back inside vertically (`CalculateMenuBounds`),
//!   kept [`EDGE_MARGIN`] from the edge as `ContextMenu` keeps its menu.
//! - **Keys** (`OnKeyPressed`): ArrowRight, Enter or Space opens and focuses
//!   the first row; ArrowLeft and Escape close only the innermost submenu and
//!   return the focus to its row.
//! - **A press opens at once** (`OnMousePressed`: "commit the selection
//!   immediately").
//!
//! # How it is built
//!
//! The flyout stays in the DOM under its row, so the CSS cascade reaches it
//! from the menu, any ancestor or the submenu itself, and an item inside it
//! closes the whole menu by the same DOM walk as any other item (a submenu
//! carries no menu-close id of its own). It is `position: fixed` and placed in
//! viewport pixels from the measured row and panel, which takes it out of the
//! parent panel's overflow clip (a capped `DropdownMenu` panel, `ContextMenu`'s
//! scrolling frame) on desktop and in the browser alike. Its origin is
//! measured with `left`/`top` at 0, so a transformed ancestor (which a
//! browser makes the containing block of a fixed box) is accounted for. The
//! offset is a CSS custom property, read through a probe box.
//!
//! **Pointer.** The row's `data-onenter` arms the open timer and gives the
//! parent panel one `data-onmousemove` (only panels holding a submenu get
//! one). That handler tracks the pointer geometrically: off the row, a
//! pending open is cancelled; onto another row of the panel, the open submenu
//! is scheduled to close. When the close timer fires it checks the last
//! pointer position against the submenu's region (row, flyout and its open
//! descendants') once more.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use rinch_core::Component;
use rinch_core::Signal;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events::{EventHandlerId, KeyEventKind};
use rinch_core::reactive::{Effect, ElementBounds};
use rinch_core::{DismissHandle, TimeoutHandle, clear_timeout, set_timeout};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

use crate::menu_nav::{contains, has_class, nearest_panel, panel_items};

/// Chromium `MenuConfig::show_delay`.
pub const DEFAULT_SUB_OPEN_DELAY_MS: u32 = 400;
/// Chromium's sibling switch happens on the same show timer.
pub const DEFAULT_SUB_CLOSE_DELAY_MS: u32 = 400;
/// Space kept between a shifted flyout and the window's edge (`ContextMenu`'s).
pub const EDGE_MARGIN: f32 = 4.0;

const SUB_CLASS: &str = "rinch-dropdown-menu__sub";
const OPENED_CLASS: &str = "rinch-dropdown-menu__sub--opened";
const LEFT_CLASS: &str = "rinch-dropdown-menu__sub--left";
const SUB_ID_ATTR: &str = "data-rinch-menu-sub-id";
/// Marks a panel `data-onmousemove` this module installed.
const PANEL_MARK: &str = "data-rinch-menu-pointer";

/// A cascading submenu row.
///
/// ```ignore
/// DropdownMenuDropdown {
///     DropdownMenuItem { onclick: || open(), "Open" }
///     DropdownMenuSub { label: "New", left_section: TablerIcon::Plus,
///         DropdownMenuItem { onclick: || new_file(), "File" }
///         DropdownMenuSub { label: "From template",
///             DropdownMenuItem { onclick: || from("a"), "Report" }
///         }
///     }
/// }
/// ```
///
/// The children are the flyout's content — `DropdownMenuItem`,
/// `DropdownMenuLabel`, `DropdownMenuDivider` or another `DropdownMenuSub`.
/// It works the same inside `DropdownMenuDropdown`, `ContextMenuDropdown` and
/// another submenu. Choosing an item anywhere in the tree closes the whole
/// menu; a disabled submenu never opens.
#[derive(Debug)]
pub struct DropdownMenuSub {
    /// The row's text.
    pub label: String,
    /// Left section icon (as `DropdownMenuItem`'s).
    pub left_section: Option<TablerIcon>,
    /// Text color (as `DropdownMenuItem`'s).
    pub color: String,
    /// A disabled submenu does not open, by pointer or key.
    pub disabled: bool,
    /// The trailing icon; `None` is the default chevron (`ChevronRight`).
    pub chevron: Option<TablerIcon>,
    /// No trailing icon at all.
    pub hide_chevron: bool,
    /// Hover time before the flyout opens, in ms (Chrome: 400).
    pub open_delay: u32,
    /// Time on another row of the panel before the flyout closes, in ms
    /// (Chrome: 400). With [`safe_triangle`](Self::safe_triangle) on, a short
    /// or zero delay is the usual pairing.
    pub close_delay: u32,
    /// Open on hover (after `open_delay`).
    pub open_on_hover: bool,
    /// Open at once when the row is pressed.
    pub open_on_click: bool,
    /// Keep the flyout open, and keep sibling rows from opening theirs, while
    /// the pointer moves inside the triangle from where it left the row to
    /// the flyout's near edge. Off by default: Chrome has none.
    pub safe_triangle: bool,
    /// Open on the panel's other side when it does not fit on its own.
    pub flip: bool,
    /// Move the flyout back inside the window when it runs past an edge (and
    /// cap its height, scrolling, when it is taller than the window).
    pub shift: bool,
    /// Horizontal gap between the parent panel and the flyout, in px
    /// (negative overlaps). Written as `--rinch-dropdown-menu-sub-offset` on
    /// the submenu; unset, the cascade's value applies (default `-3px`).
    pub offset: Option<i32>,
    /// A window resize while open closes the submenu (Chrome closes its
    /// menus when the window's bounds change). Off: it is placed again.
    pub close_on_resize: bool,
}

impl Default for DropdownMenuSub {
    fn default() -> Self {
        Self {
            label: String::new(),
            left_section: None,
            color: String::new(),
            disabled: false,
            chevron: None,
            hide_chevron: false,
            open_delay: DEFAULT_SUB_OPEN_DELAY_MS,
            close_delay: DEFAULT_SUB_CLOSE_DELAY_MS,
            open_on_hover: true,
            open_on_click: true,
            safe_triangle: false,
            flip: true,
            shift: true,
            offset: None,
            close_on_resize: true,
        }
    }
}

#[derive(Clone, Copy)]
struct Opts {
    disabled: bool,
    open_delay: u32,
    close_delay: u32,
    open_on_hover: bool,
    open_on_click: bool,
    safe_triangle: bool,
    flip: bool,
    shift: bool,
    close_on_resize: bool,
}

/// What one open measured: the flyout's size, the origin of its containing
/// block, the offset and the first row's inset.
#[derive(Clone, Copy, Default)]
struct Measured {
    w: f32,
    h: f32,
    origin: (f32, f32),
    offset: f32,
    inset: f32,
}

#[derive(Default)]
struct Placement {
    waiting: bool,
    measured: Option<Measured>,
    anchor: Option<ElementBounds>,
    /// The size the flyout has as placed: a different one later is its
    /// content changing, and it is measured again (review of #2216).
    placed_size: Option<(f32, f32)>,
    /// The viewport it was placed in: a different one is a window resize.
    viewport: Option<(f32, f32)>,
}

struct Sub {
    root: NodeHandle,
    trigger: NodeHandle,
    flyout: NodeHandle,
    probe: NodeHandle,
    /// A hidden `position: fixed` box the size of the viewport, shown while
    /// open: its bounds change when the window is resized.
    sentinel: NodeHandle,
    opened: Signal<bool>,
    is_open: Cell<bool>,
    opts: Opts,
    open_timer: Cell<Option<TimeoutHandle>>,
    close_timer: Cell<Option<TimeoutHandle>>,
    /// Whether the pointer is on the row, from its enter and leave. A
    /// pending open fires only while it is: a pointer that crossed the row
    /// into another flyout, or out of the menu, opens nothing (Chrome drops
    /// the pending selection).
    on_row: Cell<bool>,
    /// Where the pointer last was on the row (the safe triangle's apex).
    exit_point: Cell<Option<(f32, f32)>>,
    dismiss: RefCell<Option<DismissHandle>>,
    placement: RefCell<Placement>,
}

thread_local! {
    static SUBS: RefCell<HashMap<u64, Rc<Sub>>> = RefCell::new(HashMap::new());
    static NEXT_SUB: Cell<u64> = const { Cell::new(1) };
    /// The pointer's last position over a menu panel (viewport px).
    static LAST_POINTER: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
}

fn sub_for(node: &NodeHandle) -> Option<Rc<Sub>> {
    let id = node.get_attribute(SUB_ID_ATTR)?.parse::<u64>().ok()?;
    SUBS.with(|s| s.borrow().get(&id).cloned())
}

fn all_subs() -> Vec<Rc<Sub>> {
    SUBS.with(|s| s.borrow().values().cloned().collect())
}

/// The panel the submenu's row sits in.
fn parent_panel(sub: &Sub) -> Option<NodeHandle> {
    nearest_panel(&sub.root.parent_node()?)
}

fn rect(node: &NodeHandle) -> Option<ElementBounds> {
    node.bounding_rect()
}

fn hit(r: Option<ElementBounds>, (x, y): (f32, f32)) -> bool {
    r.is_some_and(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
}

/// The submenus whose row sits in `sub`'s flyout, at any depth.
fn descendants(sub: &Sub) -> Vec<Rc<Sub>> {
    all_subs()
        .into_iter()
        .filter(|d| d.root.node_id() != sub.root.node_id() && contains(&sub.flyout, &d.root))
        .collect()
}

/// Row, flyout and open descendants' flyouts.
fn region_contains(sub: &Sub, p: (f32, f32)) -> bool {
    if hit(rect(&sub.trigger), p) {
        return true;
    }
    if !sub.is_open.get() {
        return false;
    }
    hit(rect(&sub.flyout), p)
        || descendants(sub)
            .iter()
            .any(|d| d.is_open.get() && hit(rect(&d.flyout), p))
}

fn in_triangle(sub: &Sub, p: (f32, f32)) -> bool {
    let (Some(a), Some(f)) = (sub.exit_point.get(), rect(&sub.flyout)) else {
        return false;
    };
    let edge = if has_class(&sub.root, LEFT_CLASS) {
        f.x + f.width
    } else {
        f.x
    };
    let b = (edge, f.y);
    let c = (edge, f.y + f.height);
    let sign = |p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)| {
        (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
    };
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn cancel(slot: &Cell<Option<TimeoutHandle>>) {
    if let Some(t) = slot.take() {
        clear_timeout(t);
    }
}

/// Open `sub`, closing its open siblings first.
fn open(sub: &Rc<Sub>, focus_first: bool) {
    if sub.opts.disabled {
        return;
    }
    cancel(&sub.open_timer);
    cancel(&sub.close_timer);
    if !sub.is_open.get() {
        let panel = parent_panel(sub);
        for other in all_subs() {
            if other.root.node_id() != sub.root.node_id()
                && other.is_open.get()
                && parent_panel(&other).map(|p| p.node_id()) == panel.as_ref().map(|p| p.node_id())
            {
                close(&other, true);
            }
        }
        sub.is_open.set(true);
        sub.root.add_class(OPENED_CLASS);
        sub.trigger.set_attribute("aria-expanded", "true");
        install_panel_pointer(sub);
        let weak = Rc::downgrade(sub);
        let handle = rinch_core::reactive::unowned(|| {
            rinch_core::push_dismiss_handler(sub.root.doc_key(), move || {
                let Some(sub) = weak.upgrade() else {
                    return false;
                };
                if !sub.trigger.is_rendered() {
                    return false;
                }
                close(&sub, true);
                true
            })
        });
        *sub.dismiss.borrow_mut() = Some(handle);
        if sub.opened.is_alive() {
            sub.opened.set(true);
        }
    }
    if focus_first && let Some(first) = panel_items(&sub.flyout).into_iter().next() {
        first.focus();
    }
}

/// Close `sub` and every submenu open inside it. With `refocus`, a focus left
/// inside the flyout goes back to the row.
fn close(sub: &Rc<Sub>, refocus: bool) {
    cancel(&sub.open_timer);
    cancel(&sub.close_timer);
    if !sub.is_open.get() {
        return;
    }
    for d in descendants(sub) {
        close(&d, refocus);
    }
    let focus_inside = refocus
        && sub
            .flyout
            .active_element()
            .is_some_and(|f| contains(&sub.flyout, &f));
    sub.is_open.set(false);
    sub.root.remove_class(OPENED_CLASS);
    sub.trigger.set_attribute("aria-expanded", "false");
    let released = sub.dismiss.borrow_mut().take();
    drop(released);
    if sub.opened.is_alive() {
        sub.opened.set(false);
    }
    if focus_inside {
        sub.trigger.focus();
    }
}

/// Close every open submenu inside `node` (a menu that is closing).
pub(crate) fn close_subs_within(node: &NodeHandle) {
    for sub in all_subs() {
        if sub.is_open.get() && contains(node, &sub.root) {
            close(&sub, false);
        }
    }
}

/// When `trigger` is a submenu's row, open it (focusing its first row when
/// `focus_first`) and answer `true`.
pub(crate) fn open_from_trigger(trigger: &NodeHandle, focus_first: bool) -> bool {
    let Some(root) = trigger.parent_node() else {
        return false;
    };
    match sub_for(&root) {
        Some(sub) if sub.trigger.node_id() == trigger.node_id() => {
            open(&sub, focus_first);
            true
        }
        _ => false,
    }
}

fn arm_open(sub: &Rc<Sub>) {
    if sub.is_open.get() || sub.opts.disabled || !sub.opts.open_on_hover {
        return;
    }
    if sub.open_timer.get().is_some() {
        return;
    }
    if sub.opts.open_delay == 0 {
        open(sub, false);
        return;
    }
    let weak = Rc::downgrade(sub);
    let t = rinch_core::reactive::unowned(|| {
        set_timeout(sub.opts.open_delay, move || {
            if let Some(sub) = weak.upgrade() {
                sub.open_timer.set(None);
                if sub.on_row.get() {
                    open(&sub, false);
                }
            }
        })
    });
    sub.open_timer.set(Some(t));
}

fn arm_close(sub: &Rc<Sub>) {
    if !sub.is_open.get() || sub.close_timer.get().is_some() {
        return;
    }
    let check = |sub: &Rc<Sub>| {
        let still_here = LAST_POINTER
            .with(|p| p.get())
            .is_some_and(|p| region_contains(sub, p));
        if !still_here {
            close(sub, true);
        }
    };
    if sub.opts.close_delay == 0 {
        check(sub);
        return;
    }
    let weak = Rc::downgrade(sub);
    let t = rinch_core::reactive::unowned(|| {
        set_timeout(sub.opts.close_delay, move || {
            if let Some(sub) = weak.upgrade() {
                sub.close_timer.set(None);
                check(&sub);
            }
        })
    });
    sub.close_timer.set(Some(t));
}

/// The parent panel's pointer tracking, once per panel.
fn install_panel_pointer(sub: &Sub) {
    let Some(panel) = parent_panel(sub) else {
        return;
    };
    let live = panel.get_attribute(PANEL_MARK).is_some()
        && panel
            .get_attribute("data-onmousemove")
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|id| rinch_core::events::has_click_handler(EventHandlerId(id)));
    if live {
        return;
    }
    let p = panel.clone();
    let id = rinch_core::events::register_handler(Rc::new(move || panel_pointer_moved(&p)));
    panel.set_attribute("data-onmousemove", &id.0.to_string());
    panel.set_attribute(PANEL_MARK, "");
}

fn panel_pointer_moved(panel: &NodeHandle) {
    let ctx = rinch_core::events::get_click_context();
    let p = (ctx.mouse_x, ctx.mouse_y);
    LAST_POINTER.with(|l| l.set(Some(p)));
    let subs: Vec<Rc<Sub>> = all_subs()
        .into_iter()
        .filter(|s| parent_panel(s).is_some_and(|pp| pp.node_id() == panel.node_id()))
        .collect();
    // An open submenu's triangle holds its siblings back too.
    let guarded = subs.iter().any(|s| {
        s.is_open.get() && s.opts.safe_triangle && !region_contains(s, p) && in_triangle(s, p)
    });
    for sub in &subs {
        let on_row = hit(rect(&sub.trigger), p);
        if on_row {
            sub.exit_point.set(Some(p));
        }
        if sub.is_open.get() {
            if region_contains(sub, p) || (sub.opts.safe_triangle && in_triangle(sub, p)) {
                cancel(&sub.close_timer);
            } else {
                arm_close(sub);
            }
        } else if on_row && !guarded {
            arm_open(sub);
        } else {
            cancel(&sub.open_timer);
        }
    }
}

/// Measure the flyout as it stands at `left`/`top` 0 and place it.
fn measure_and_place(sub: &Sub) -> bool {
    let Some(f) = rect(&sub.flyout) else {
        return false;
    };
    if f.width <= 0.0 && f.height <= 0.0 {
        return false;
    }
    let (Some(probe), Some(root)) = (rect(&sub.probe), rect(&sub.root)) else {
        return false;
    };
    let inset = sub
        .flyout
        .children()
        .into_iter()
        .filter(|c| c.tag_name().is_some())
        .find_map(|c| rect(&c).filter(|r| r.width > 0.0 || r.height > 0.0))
        .map(|r| r.y - f.y)
        .unwrap_or(0.0);
    let m = Measured {
        w: f.width,
        h: f.height,
        origin: (f.x, f.y),
        offset: probe.x - root.x,
        inset,
    };
    sub.placement.borrow_mut().measured = Some(m);
    place(sub, m)
}

/// Where the flyout goes, in viewport px: `(x, y, flipped, cap)`. A flyout
/// wider than the window is placed at the margin; [`place`] narrows it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide(
    row: ElementBounds,
    panel: ElementBounds,
    size: (f32, f32),
    viewport: (f32, f32),
    offset: f32,
    inset: f32,
    flip: bool,
    shift: bool,
) -> (f32, f32, bool, Option<f32>) {
    let (w, h) = size;
    let (vw, vh) = viewport;
    let right = panel.x + panel.width + offset;
    let left = panel.x - offset - w;
    let fits_right = right + w <= vw;
    let fits_left = left >= 0.0;
    let flipped = flip && !fits_right && (fits_left || panel.x > vw - (panel.x + panel.width));
    let mut x = if flipped { left } else { right };
    let mut y = row.y - inset;
    let mut cap = None;
    if shift {
        if w > vw - 2.0 * EDGE_MARGIN {
            x = EDGE_MARGIN;
        } else if x + w > vw {
            x = vw - EDGE_MARGIN - w;
        } else if x < 0.0 {
            x = EDGE_MARGIN;
        }
        if h > vh - 2.0 * EDGE_MARGIN {
            y = EDGE_MARGIN;
            cap = Some(vh - 2.0 * EDGE_MARGIN);
        } else if y + h > vh {
            y = vh - EDGE_MARGIN - h;
        } else if y < 0.0 {
            y = EDGE_MARGIN;
        }
    }
    (x, y, flipped, cap)
}

fn place(sub: &Sub, m: Measured) -> bool {
    let (Some(viewport), Some(row)) = (sub.root.viewport_size(), rect(&sub.trigger)) else {
        return false;
    };
    let panel = parent_panel(sub).and_then(|p| rect(&p)).unwrap_or(row);
    sub.placement.borrow_mut().anchor = Some(row);
    let (x, y, flipped, cap) = decide(
        row,
        panel,
        (m.w, m.h),
        viewport,
        m.offset,
        m.inset,
        sub.opts.flip,
        sub.opts.shift,
    );
    if flipped {
        sub.root.add_class(LEFT_CLASS);
    } else {
        sub.root.remove_class(LEFT_CLASS);
    }
    // No width is pinned: placed where it fits, a fixed box's shrink-to-fit
    // width is its content's, and a row that grows widens it (and is then
    // measured again). Only one wider than the window is narrowed.
    let mut style = format!(
        "left: {}px; top: {}px;",
        px(x - m.origin.0),
        px(y - m.origin.1)
    );
    let mut placed = (m.w, m.h);
    if sub.opts.shift && m.w > viewport.0 - 2.0 * EDGE_MARGIN {
        let w = (viewport.0 - 2.0 * EDGE_MARGIN).max(0.0);
        style.push_str(&format!(" width: {}px;", px(w)));
        placed.0 = w;
    }
    if let Some(cap) = cap {
        style.push_str(&format!(" max-height: {}px; overflow-y: auto;", px(cap)));
        placed.1 = cap;
    }
    sub.flyout.set_attribute("style", &style);
    {
        let mut p = sub.placement.borrow_mut();
        p.placed_size = Some(placed);
        p.viewport = Some(viewport);
    }
    true
}

fn px(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// Put the flyout back at `left`/`top` 0 and measure it again: now when the
/// layout is current (web), else after the next one.
fn remeasure(sub: &Sub) {
    {
        let mut p = sub.placement.borrow_mut();
        p.placed_size = None;
        p.waiting = false;
    }
    sub.flyout.set_attribute("style", "left: 0px; top: 0px;");
    sub.root.remove_class(LEFT_CLASS);
    let placed = !sub.root.layout_pending() && measure_and_place(sub);
    if !placed {
        sub.placement.borrow_mut().waiting = true;
    }
}

/// Measure and place the flyout on each open; follow the row while open;
/// measure again when its content changes size; close (or place again) on a
/// window resize.
fn arm_placement(sub: &Rc<Sub>) {
    let weak: Weak<Sub> = Rc::downgrade(sub);
    let opened = sub.opened;
    let was_open = Cell::new(false);
    Effect::new(move || {
        let open = opened.get();
        let rising = open && !was_open.get();
        was_open.set(open);
        if !rising {
            return;
        }
        let Some(sub) = weak.upgrade() else {
            return;
        };
        let pending = sub.root.layout_pending();
        if !pending && sub.root.viewport_size().is_none() {
            // No geometry (the mock document): nothing to place.
            return;
        }
        *sub.placement.borrow_mut() = Placement::default();
        remeasure(&sub);
        let flyout_bounds = sub.flyout.bounds_signal();
        let row_bounds = sub.trigger.bounds_signal();
        let viewport_bounds = sub.sentinel.bounds_signal();
        let weak = Rc::downgrade(&sub);
        Effect::new(move || {
            let b = flyout_bounds.get();
            let _ = row_bounds.get();
            let _ = viewport_bounds.get();
            let Some(sub) = weak.upgrade() else {
                return;
            };
            if !sub.is_open.get() {
                return;
            }
            let waiting = sub.placement.borrow().waiting;
            if waiting {
                if (b.width <= 0.0 && b.height <= 0.0) || !measure_and_place(&sub) {
                    return;
                }
                sub.placement.borrow_mut().waiting = false;
                return;
            }
            let (m, anchor, placed_size, placed_in) = {
                let p = sub.placement.borrow();
                (p.measured, p.anchor, p.placed_size, p.viewport)
            };
            // A window resize: closed (Chrome), or placed again.
            if let (Some(was), Some(now)) = (placed_in, sub.root.viewport_size())
                && ((was.0 - now.0).abs() > 0.5 || (was.1 - now.1).abs() > 0.5)
            {
                if sub.opts.close_on_resize {
                    close(&sub, true);
                } else {
                    remeasure(&sub);
                }
                return;
            }
            // Its content changed size: measure it again.
            if let Some((w, h)) = placed_size
                && (b.width > 0.0 || b.height > 0.0)
                && ((b.width - w).abs() > 0.5 || (b.height - h).abs() > 0.5)
            {
                remeasure(&sub);
                return;
            }
            let (Some(m), Some(anchor), Some(now)) = (m, anchor, rect(&sub.trigger)) else {
                return;
            };
            if (now.x - anchor.x).abs() < 0.5 && (now.y - anchor.y).abs() < 0.5 {
                return;
            }
            place(&sub, m);
        });
    });
}

impl Component for DropdownMenuSub {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let root = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__sub" } };
        root.set_attribute("class", SUB_CLASS);
        if let Some(offset) = self.offset {
            root.set_attribute(
                "style",
                &format!("--rinch-dropdown-menu-sub-offset: {offset}px"),
            );
        }

        // The row: a menu item, so it looks and navigates like one.
        let trigger = rinch_macros::rsx! { button { class: "rinch-dropdown-menu__item" } };
        let mut classes = vec![
            crate::menu_nav::ITEM_CLASS,
            "rinch-dropdown-menu__sub-trigger",
        ];
        trigger.set_attribute("role", "menuitem");
        if self.disabled {
            classes.push("rinch-dropdown-menu__item--disabled");
            trigger.set_attribute("disabled", "");
        }
        trigger.set_attribute("class", &classes.join(" "));
        trigger.set_attribute("aria-haspopup", "menu");
        trigger.set_attribute("aria-expanded", "false");
        if !self.color.is_empty() {
            let c = &self.color;
            let value = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                c.clone()
            } else {
                format!("var(--rinch-color-{c}-6)")
            };
            trigger.set_attribute(
                "style",
                &format!("--rinch-dropdown-menu-item-color: {value}"),
            );
        }
        if let Some(icon) = self.left_section {
            let span = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-left" } };
            span.append_child(&render_tabler_icon(__scope, icon, TablerIconStyle::Outline));
            trigger.append_child(&span);
        }
        let label = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-label" } };
        label.append_child(&__scope.create_text(&self.label));
        trigger.append_child(&label);
        if !self.hide_chevron {
            let span = __scope.create_element("span");
            span.set_attribute(
                "class",
                "rinch-dropdown-menu__item-right rinch-dropdown-menu__sub-chevron",
            );
            let icon = self.chevron.unwrap_or(TablerIcon::ChevronRight);
            span.append_child(&render_tabler_icon(__scope, icon, TablerIconStyle::Outline));
            trigger.append_child(&span);
        }
        root.append_child(&trigger);

        // The offset probe: `left: var(--rinch-dropdown-menu-sub-offset)` in
        // the submenu (a positioned box), so its x less the submenu's is the
        // offset the cascade gives this submenu.
        let probe = __scope.create_element("span");
        probe.set_attribute("class", "rinch-dropdown-menu__sub-offset");
        probe.set_attribute("aria-hidden", "true");
        root.append_child(&probe);

        let sentinel = __scope.create_element("span");
        sentinel.set_attribute("class", "rinch-dropdown-menu__sub-viewport");
        sentinel.set_attribute("aria-hidden", "true");
        root.append_child(&sentinel);

        let flyout = __scope.create_element("div");
        flyout.set_attribute("class", "rinch-dropdown-menu__sub-dropdown");
        flyout.set_attribute("role", "menu");
        for child in children {
            flyout.append_child(child);
        }
        root.append_child(&flyout);

        let id = NEXT_SUB.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        root.set_attribute(SUB_ID_ATTR, &id.to_string());
        let sub = Rc::new(Sub {
            root: root.clone(),
            trigger: trigger.clone(),
            flyout,
            probe,
            sentinel,
            opened: Signal::new(false),
            is_open: Cell::new(false),
            opts: Opts {
                disabled: self.disabled,
                open_delay: self.open_delay,
                close_delay: self.close_delay,
                open_on_hover: self.open_on_hover,
                open_on_click: self.open_on_click,
                safe_triangle: self.safe_triangle,
                flip: self.flip,
                shift: self.shift,
                close_on_resize: self.close_on_resize,
            },
            open_timer: Cell::new(None),
            close_timer: Cell::new(None),
            on_row: Cell::new(false),
            exit_point: Cell::new(None),
            dismiss: RefCell::new(None),
            placement: RefCell::new(Placement::default()),
        });
        SUBS.with(|s| s.borrow_mut().insert(id, sub.clone()));
        {
            let weak = Rc::downgrade(&sub);
            __scope.on_cleanup(move || {
                if let Some(sub) = weak.upgrade() {
                    cancel(&sub.open_timer);
                    cancel(&sub.close_timer);
                    let released = sub.dismiss.borrow_mut().take();
                    drop(released);
                }
                SUBS.with(|s| s.borrow_mut().remove(&id));
            });
        }

        arm_placement(&sub);

        // Hover: arm the open timer and start tracking the panel's pointer.
        {
            let weak = Rc::downgrade(&sub);
            let enter = __scope.register_handler(move || {
                let Some(sub) = weak.upgrade() else {
                    return;
                };
                install_panel_pointer(&sub);
                sub.on_row.set(true);
                if sub.is_open.get() {
                    cancel(&sub.close_timer);
                } else {
                    arm_open(&sub);
                }
            });
            trigger.set_attribute("data-onenter", &enter.0.to_string());
            // Desktop fires a leave and an enter on every change of the node
            // under the pointer, so moving inside the row sets this false and
            // true again in one event; a pending open is not restarted.
            let weak = Rc::downgrade(&sub);
            let leave = __scope.register_handler(move || {
                if let Some(sub) = weak.upgrade() {
                    sub.on_row.set(false);
                }
            });
            trigger.set_attribute("data-onleave", &leave.0.to_string());
        }

        // Press: open at once.
        {
            let weak = Rc::downgrade(&sub);
            let click = __scope.register_handler(move || {
                let Some(sub) = weak.upgrade() else {
                    return;
                };
                if sub.opts.open_on_click {
                    open(&sub, false);
                }
            });
            trigger.set_attribute("data-rid", &click.0.to_string());
        }

        // Keys: the row's ArrowRight/Enter/Space, the flyout's ArrowLeft.
        {
            let weak = Rc::downgrade(&sub);
            let keys = __scope.register_key_handler(move |k| {
                let Some(sub) = weak.upgrade() else {
                    return false;
                };
                if k.kind != KeyEventKind::Down || k.ctrl || k.alt || k.meta {
                    return false;
                }
                let Some(focused) = sub.root.active_element() else {
                    return false;
                };
                if focused.node_id() == sub.trigger.node_id() {
                    return match k.key.as_str() {
                        "ArrowRight" | "Enter" | " " => {
                            if !sub.opts.disabled {
                                open(&sub, true);
                            }
                            true
                        }
                        _ => false,
                    };
                }
                let in_flyout =
                    nearest_panel(&focused).is_some_and(|p| p.node_id() == sub.flyout.node_id());
                if in_flyout && matches!(k.key.as_str(), "ArrowLeft" | "Escape") {
                    close(&sub, true);
                    return true;
                }
                false
            });
            root.set_attribute("data-onkeydown", &keys.0.to_string());
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::decide;
    use rinch_core::reactive::ElementBounds;

    fn b(x: f32, y: f32, w: f32, h: f32) -> ElementBounds {
        ElementBounds {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn it_opens_right_of_the_panel_overlapping_by_the_offset() {
        let (x, y, flipped, cap) = decide(
            b(110.0, 57.0, 180.0, 30.0),
            b(100.0, 40.0, 200.0, 300.0),
            (150.0, 120.0),
            (800.0, 600.0),
            -3.0,
            11.0,
            true,
            true,
        );
        assert_eq!((x, y, flipped, cap), (297.0, 46.0, false, None));
    }

    #[test]
    fn it_flips_left_at_the_right_edge_and_shifts_up_at_the_bottom() {
        let (x, y, flipped, _) = decide(
            b(610.0, 540.0, 180.0, 30.0),
            b(600.0, 400.0, 200.0, 190.0),
            (150.0, 120.0),
            (800.0, 600.0),
            -3.0,
            11.0,
            true,
            true,
        );
        assert!(flipped);
        assert_eq!(x, 600.0 + 3.0 - 150.0);
        assert_eq!(y, 600.0 - 4.0 - 120.0);
    }

    #[test]
    fn without_flip_or_shift_it_stays_where_it_was_put() {
        let (x, y, flipped, cap) = decide(
            b(610.0, 540.0, 180.0, 30.0),
            b(600.0, 400.0, 200.0, 190.0),
            (150.0, 120.0),
            (800.0, 600.0),
            -3.0,
            11.0,
            false,
            false,
        );
        assert_eq!((x, y, flipped, cap), (797.0, 529.0, false, None));
    }

    #[test]
    fn taller_than_the_window_is_capped() {
        let (_, y, _, cap) = decide(
            b(110.0, 57.0, 180.0, 30.0),
            b(100.0, 40.0, 200.0, 300.0),
            (150.0, 900.0),
            (800.0, 600.0),
            -3.0,
            0.0,
            true,
            true,
        );
        assert_eq!((y, cap), (4.0, Some(592.0)));
    }
}
