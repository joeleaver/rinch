//! Keeping an anchored dropdown inside the viewport (issue #1764).
//!
//! `DropdownMenu` and `Popover` hang their dropdown off their own root with
//! `position: absolute`, so it follows the trigger when the page scrolls and
//! keeps its place in the DOM (nested menus, CSS hooks). What that does not do
//! is keep it on screen: a trigger near the window's bottom or right edge sent
//! the dropdown past it. On each open this module measures and corrects it, the
//! way Floating UI's `flip`, `shift` and `size` middlewares do:
//!
//! - **flip**: the dropdown opens on its preferred side when it fits there
//!   (inside the viewport), else on the opposite side when it fits there;
//! - **size**: one that fits on neither side goes on the side with more room,
//!   capped to that room and scrolling (`max-height` + `overflow-y: auto`; top
//!   and bottom sides only);
//! - **shift**: on the cross axis one that runs past the viewport's edge is
//!   moved back inside it, keeping [`EDGE_MARGIN`] from the edge (to the start
//!   edge when it is wider than the viewport). One that fits is not moved, so a
//!   menu under a trigger flush with the window's edge stays flush.
//!
//! The decision is [`decide`], a pure function. It is written as inline insets
//! on the dropdown element (the side's `top`/`bottom` or `left`/`right` at
//! `100%`, and the cross axis in px) with its measured width pinned, since an
//! absolute box's shrink-to-fit width depends on its `left`. The root's
//! position class is swapped to the side used, as `DropdownMenu`'s old flip
//! did, so a `Popover`'s arrow points the right way. Never a `transform` on a
//! `DropdownMenu` panel: in a browser a transform makes the panel the
//! containing block of a nested menu's `position: fixed` backdrop.
//!
//! **When it is measured.** The browser lays out on demand, so on the web
//! the dropdown is measured and placed in the same task as the open. rinch-dom
//! answers geometry from its last layout, and right after the open's own class
//! write a layout is pending ([`NodeHandle::layout_pending`]): the dropdown is
//! then measured from a [`NodeHandle::bounds_signal`] that wakes after that
//! layout, and the runtime lays the placement out again before the frame is
//! painted (`RinchApp::settle_bounds_writes`). Either way no painted frame
//! shows the dropdown where it does not belong.
//!
//! While open, the dropdown is placed again whenever its root moves (a page
//! scroll, an outer menu placing itself), from the size measured at open; a
//! window resize that does not move the root keeps the placement until the
//! next open, and so does a change of the dropdown's own content (its width is
//! pinned). The viewport is the only boundary (not a clipping ancestor), and a
//! transformed (scaled) ancestor is not accounted for.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::NodeHandle;
use rinch_core::reactive::Effect;

/// Space kept between a dropdown and the viewport's edge.
pub(crate) const EDGE_MARGIN: f32 = 8.0;

/// The side of the anchor a dropdown opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl Side {
    fn opposite(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// Whether the dropdown is above or below the anchor (its cross axis is
    /// horizontal).
    fn vertical(self) -> bool {
        matches!(self, Side::Top | Side::Bottom)
    }
}

/// Where on the cross axis a dropdown is aligned with its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Start,
    Center,
    End,
}

/// A box in viewport-relative logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// What [`decide`] answers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Decision {
    /// The side the dropdown goes on.
    pub side: Side,
    /// Its start on the cross axis: its left edge for a top or bottom
    /// dropdown, its top edge for a left or right one.
    pub cross: f32,
    /// A `max-height` when it fits on neither side (top/bottom only).
    pub cap: Option<f32>,
}

/// Where a `size` dropdown goes against `anchor` in a `viewport`, `offset`
/// px off the anchor, preferring `side`.
pub(crate) fn decide(
    anchor: Rect,
    size: (f32, f32),
    viewport: (f32, f32),
    side: Side,
    align: Align,
    offset: f32,
    margin: f32,
) -> Decision {
    let (w, h) = size;
    let (vw, vh) = viewport;
    // The room on a side, keeping `keep` from the viewport's edge.
    let room = |s: Side, keep: f32| match s {
        Side::Bottom => vh - keep - (anchor.y + anchor.h + offset),
        Side::Top => anchor.y - offset - keep,
        Side::Right => vw - keep - (anchor.x + anchor.w + offset),
        Side::Left => anchor.x - offset - keep,
    };
    let need = if side.vertical() { h } else { w };
    let opposite = side.opposite();
    // "Fits" is inside the viewport itself: a dropdown that fits is left
    // where the caller put it, margin or not. The margin is kept by what
    // moves one: a cap, and a shift.
    let (side_used, cap) = if need <= room(side, 0.0) {
        (side, None)
    } else if need <= room(opposite, 0.0) {
        (opposite, None)
    } else {
        let s = if room(opposite, margin) > room(side, margin) {
            opposite
        } else {
            side
        };
        (s, side.vertical().then(|| room(s, margin).max(0.0)))
    };
    let (a0, alen, len, vlen) = if side.vertical() {
        (anchor.x, anchor.w, w, vw)
    } else {
        (anchor.y, anchor.h, h, vh)
    };
    let natural = match align {
        Align::Start => a0,
        Align::Center => a0 + alen / 2.0 - len / 2.0,
        Align::End => a0 + alen - len,
    };
    let max = vlen - margin - len;
    let cross = if natural >= 0.0 && natural + len <= vlen {
        natural
    } else if max < margin {
        margin
    } else {
        natural.clamp(margin, max)
    };
    Decision {
        side: side_used,
        cross,
        cap,
    }
}

/// One component's dropdown, as [`arm`] places it.
pub(crate) struct Floating {
    /// The component's root: what the dropdown's insets are relative to, and
    /// what carries the position class.
    pub root: NodeHandle,
    /// The class every root of this component carries (`rinch-popover`): the
    /// dropdown search does not enter a nested one.
    pub root_class: &'static str,
    /// The dropdown's class (`rinch-popover__dropdown`).
    pub dropdown_class: &'static str,
    /// The side and alignment the caller asked for.
    pub side: Side,
    pub align: Align,
    /// The gap between anchor and dropdown, in px.
    pub offset: f32,
    /// The root's position class for the dropdown on a side: the caller's
    /// own class for `side`, its flipped twin for the opposite side.
    pub class_for: Rc<dyn Fn(Side) -> &'static str>,
    /// Whether a centered dropdown is centred by `translate(-50%)` on its
    /// cross axis (`Popover`), so its inset is its centre, not its start.
    pub center_translates: bool,
}

/// The inline properties a placement writes, cleared at the next open.
const PLACED: &[&str] = &[
    "top",
    "bottom",
    "left",
    "right",
    "width",
    "box-sizing",
    "max-height",
    "overflow-y",
];

/// What one open of a dropdown knows.
#[derive(Default)]
struct Open {
    /// The dropdown, waiting for a layout to be measured in.
    waiting: Option<NodeHandle>,
    /// Its own size, measured once per open before anything was written.
    size: Option<(f32, f32)>,
    /// The anchor it was last placed against.
    anchor: Option<Rect>,
}

/// Keep `f`'s dropdown inside the viewport while `opened`.
///
/// Measured and placed on each open; placed again while open whenever the
/// root moves (a page scroll, a menu around it placing itself), from the size
/// measured at open. Call after the effect that shows the dropdown, so it
/// runs after it.
pub(crate) fn arm(f: Floating, opened: Rc<dyn Fn() -> bool>) {
    let f = Rc::new(f);
    let current = Rc::new(Cell::new(f.side));
    let state: Rc<RefCell<Open>> = Rc::new(RefCell::new(Open::default()));
    let was_open = Cell::new(false);
    Effect::new(move || {
        let open = opened();
        let rising = open && !was_open.get();
        was_open.set(open);
        if !rising {
            return;
        }
        let Some(dd) = find_dropdown(&f.root, f.root_class, f.dropdown_class) else {
            return;
        };
        let pending = f.root.layout_pending();
        if !pending && f.root.viewport_size().is_none() {
            // A host with no geometry (the mock document): nothing to place.
            return;
        }
        *state.borrow_mut() = Open::default();
        remove_styles(&dd, PLACED);
        let placed = !pending && measure_and_place(&f, &dd, &current, &state);
        if !placed {
            // Measured after the next layout, which lays out the placement
            // again before that frame is painted (desktop:
            // `RinchApp::settle_bounds_writes`; the browser lays out again
            // after a `ResizeObserver` callback writes).
            state.borrow_mut().waiting = Some(dd.clone());
        }
        // Both owned by this run of the effect, so freed (and unwatched) on
        // close.
        let dd_bounds = dd.bounds_signal();
        let root_bounds = f.root.bounds_signal();
        let f = f.clone();
        let current = current.clone();
        let state = state.clone();
        Effect::new(move || {
            let d = dd_bounds.get();
            let _ = root_bounds.get();
            let waiting = state.borrow().waiting.clone();
            if let Some(dd) = waiting {
                // Not asked whether a layout is pending: this runs as the
                // bounds are published after a layout, which is what they
                // describe, and a write some other subscriber made since
                // would leave the dropdown waiting for a change of bounds
                // that never comes.
                if (d.width <= 0.0 && d.height <= 0.0)
                    || !measure_and_place(&f, &dd, &current, &state)
                {
                    return;
                }
                state.borrow_mut().waiting = None;
                return;
            }
            // Placed: follow the root when it moves.
            let (size, anchor) = {
                let s = state.borrow();
                (s.size, s.anchor)
            };
            let (Some(size), Some(anchor)) = (size, anchor) else {
                return;
            };
            let Some(now) = root_rect(&f.root) else {
                return;
            };
            if (now.x - anchor.x).abs() < 0.5
                && (now.y - anchor.y).abs() < 0.5
                && (now.w - anchor.w).abs() < 0.5
                && (now.h - anchor.h).abs() < 0.5
            {
                return;
            }
            place(&f, &dd, &current, &state, size, now);
        });
    });
}

fn root_rect(root: &NodeHandle) -> Option<Rect> {
    let a = root.bounding_rect()?;
    Some(Rect {
        x: a.x,
        y: a.y,
        w: a.width,
        h: a.height,
    })
}

/// Measure `dd` (with nothing of a placement on it) and place it; `false`
/// when there is nothing to measure yet.
fn measure_and_place(
    f: &Floating,
    dd: &NodeHandle,
    current: &Cell<Side>,
    state: &RefCell<Open>,
) -> bool {
    let Some(anchor) = root_rect(&f.root) else {
        return false;
    };
    let Some(d) = dd.bounding_rect() else {
        return false;
    };
    if d.width <= 0.0 && d.height <= 0.0 {
        return false;
    }
    state.borrow_mut().size = Some((d.width, d.height));
    place(f, dd, current, state, (d.width, d.height), anchor)
}

/// Place a `size` dropdown against `anchor`.
fn place(
    f: &Floating,
    dd: &NodeHandle,
    current: &Cell<Side>,
    state: &RefCell<Open>,
    size: (f32, f32),
    anchor: Rect,
) -> bool {
    let Some(viewport) = f.root.viewport_size() else {
        return false;
    };
    state.borrow_mut().anchor = Some(anchor);
    let dec = decide(
        anchor,
        size,
        viewport,
        f.side,
        f.align,
        f.offset,
        EDGE_MARGIN,
    );
    let was = current.get();
    if dec.side != was {
        f.root.remove_class((f.class_for)(was));
        f.root.add_class((f.class_for)(dec.side));
        current.set(dec.side);
    }
    let centred = f.center_translates && f.align == Align::Center;
    let (w, h) = size;
    let width = px(w);
    if dec.side.vertical() {
        let shift = if centred { w / 2.0 } else { 0.0 };
        let left = px(dec.cross - anchor.x + shift);
        let (top, bottom) = if dec.side == Side::Bottom {
            ("100%", "auto")
        } else {
            ("auto", "100%")
        };
        dd.set_styles(&[
            ("top", top),
            ("bottom", bottom),
            ("left", &left),
            ("right", "auto"),
            ("width", &width),
            ("box-sizing", "border-box"),
        ]);
    } else {
        let shift = if centred { h / 2.0 } else { 0.0 };
        let top = px(dec.cross - anchor.y + shift);
        let (left, right) = if dec.side == Side::Right {
            ("100%", "auto")
        } else {
            ("auto", "100%")
        };
        dd.set_styles(&[
            ("left", left),
            ("right", right),
            ("top", &top),
            ("bottom", "auto"),
            ("width", &width),
            ("box-sizing", "border-box"),
        ]);
    }
    match dec.cap {
        Some(cap) => dd.set_styles(&[("max-height", &px(cap)), ("overflow-y", "auto")]),
        None => remove_styles(dd, &["max-height", "overflow-y"]),
    }
    true
}

fn px(v: f32) -> String {
    format!("{}px", (v * 100.0).round() / 100.0)
}

fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .is_some_and(|c| c.split_whitespace().any(|t| t == class))
}

/// The first `dropdown_class` element under `root`, not entering a nested
/// `root_class` (another menu's own dropdown).
pub(crate) fn find_dropdown(
    root: &NodeHandle,
    root_class: &str,
    dropdown_class: &str,
) -> Option<NodeHandle> {
    for child in root.children() {
        if has_class(&child, dropdown_class) {
            return Some(child);
        }
        if has_class(&child, root_class) {
            continue;
        }
        if let Some(found) = find_dropdown(&child, root_class, dropdown_class) {
            return Some(found);
        }
    }
    None
}

/// Remove `props` from `node`'s inline style, leaving the rest as written.
fn remove_styles(node: &NodeHandle, props: &[&str]) {
    let Some(style) = node.get_attribute("style") else {
        return;
    };
    let decls = rinch_core::dom::split_declarations(&style);
    let kept: Vec<(String, String)> = decls
        .iter()
        .filter(|(k, _)| !props.contains(&k.as_str()))
        .cloned()
        .collect();
    if kept.len() != decls.len() {
        node.set_attribute("style", &rinch_core::dom::serialize_declarations(&kept));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: (f32, f32) = (800.0, 480.0);

    fn trigger(x: f32, y: f32) -> Rect {
        Rect {
            x,
            y,
            w: 40.0,
            h: 20.0,
        }
    }

    #[test]
    fn it_stays_below_when_it_fits() {
        let d = decide(
            trigger(100.0, 50.0),
            (222.0, 322.0),
            VP,
            Side::Bottom,
            Align::Start,
            4.0,
            8.0,
        );
        assert_eq!(
            d,
            Decision {
                side: Side::Bottom,
                cross: 100.0,
                cap: None
            }
        );
    }

    #[test]
    fn it_flips_up_and_shifts_left_at_the_bottom_right_corner() {
        let d = decide(
            trigger(700.0, 430.0),
            (222.0, 322.0),
            VP,
            Side::Bottom,
            Align::Start,
            4.0,
            8.0,
        );
        assert_eq!(
            d,
            Decision {
                side: Side::Top,
                cross: 800.0 - 8.0 - 222.0,
                cap: None
            }
        );
    }

    #[test]
    fn a_preferred_top_that_does_not_fit_flips_down() {
        let d = decide(
            trigger(300.0, 30.0),
            (200.0, 100.0),
            VP,
            Side::Top,
            Align::Center,
            8.0,
            8.0,
        );
        assert_eq!(d.side, Side::Bottom);
        assert_eq!(d.cross, 300.0 + 20.0 - 100.0);
    }

    #[test]
    fn taller_than_both_sides_takes_the_larger_and_is_capped() {
        // 150 above (minus offset and margin), 290 below.
        let d = decide(
            trigger(10.0, 162.0),
            (200.0, 400.0),
            VP,
            Side::Top,
            Align::Start,
            4.0,
            8.0,
        );
        assert_eq!(d.side, Side::Bottom);
        assert_eq!(d.cap, Some(480.0 - 8.0 - (162.0 + 20.0 + 4.0)));
        assert_eq!(d.cross, 10.0);
    }

    #[test]
    fn a_left_edge_shifts_right_and_a_wide_one_starts_at_the_margin() {
        let d = decide(
            trigger(2.0, 40.0),
            (200.0, 50.0),
            VP,
            Side::Bottom,
            Align::Center,
            8.0,
            8.0,
        );
        assert_eq!(d.cross, 8.0);
        let d = decide(
            trigger(400.0, 40.0),
            (900.0, 50.0),
            VP,
            Side::Bottom,
            Align::End,
            8.0,
            8.0,
        );
        assert_eq!(d.cross, 8.0);
    }

    #[test]
    fn one_that_fits_within_the_margin_is_not_moved() {
        // Flush with the left edge, and 2px from the bottom: it fits.
        let d = decide(
            trigger(0.0, 340.0),
            (200.0, 114.0),
            VP,
            Side::Bottom,
            Align::Start,
            4.0,
            8.0,
        );
        assert_eq!(
            d,
            Decision {
                side: Side::Bottom,
                cross: 0.0,
                cap: None
            }
        );
    }

    #[test]
    fn a_right_side_flips_left_and_shifts_vertically() {
        let d = decide(
            trigger(700.0, 460.0),
            (150.0, 60.0),
            VP,
            Side::Right,
            Align::Start,
            8.0,
            8.0,
        );
        assert_eq!(d.side, Side::Left);
        assert_eq!(d.cross, 480.0 - 8.0 - 60.0);
        assert_eq!(d.cap, None);
    }
}
