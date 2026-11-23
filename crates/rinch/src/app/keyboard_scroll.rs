//! Keyboard scrolling (#2977): the arrows, Page Up / Page Down, Home / End
//! and Space scroll, as they do in Chrome, when nothing else has the key.
//!
//! The key comes here from `handle_unclaimed_key_fallback`, so after the
//! element's and the app's key handlers, and only while no text field holds
//! the keyboard (the rich-text editor's keys never reach it).
//!
//! # Which scroller
//!
//! The nearest box, from the focused node (else the node the last press
//! landed on, else the page), that overflows on the key's axis, is not locked
//! out by a `lock_scroll` and can still move that way — so a list at its end
//! hands the key to the scroller around it, as the wheel does. At a
//! `position: fixed` box the walk goes straight to the page (no scroller between
//! the box and the viewport moves it, but Chrome hands the key to the viewport,
//! as rinch's wheel does), so a press in a fixed header then Page Down scrolls
//! the page. The page is `<body>`.
//!
//! # How far (measured in Chrome 158, a focused 300 px scroller)
//!
//! - an arrow: 40 px (`kPixelsPerLineStep`);
//! - Page Up / Page Down, Space and Shift+Space: 0.875 of the scrollport
//!   (`kMinFractionToStepWhenPaging`; 262 px of 300);
//! - Home / End (Ctrl too; Cmd+Up / Cmd+Down on macOS): the start or the end.
//!
//! # How
//!
//! On `CurveKind::Keyboard`: 150 ms whatever the distance, `ease-in-out`
//! (Chrome's 40 px, page and 24 700 px scrolls all took about 150 ms and fit
//! it to 0.8% RMS). A key pressed again mid-scroll moves the target on from
//! where the scroll is headed, as auto-repeat does in Chrome. The CSS
//! `scroll-behavior` does not apply (it is for programmatic scrolls); the
//! smooth-scrolling switch the wheel uses does
//! ([`RinchApp::set_smooth_wheel_scroll`]): off, the key scrolls its whole
//! distance at once.

use super::RinchApp;
use super::smooth_wheel::{CurveKind, WheelAxis, offset_bounds};
use super::text_keymap::TextKeymap;
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::ScrollbarAxis;
use rinch_platform::{KeyCode, Modifiers};

/// An arrow key's step, in px (`kPixelsPerLineStep`).
pub(crate) const LINE_STEP: f64 = 40.0;
/// The share of the scrollport a page step moves
/// (`kMinFractionToStepWhenPaging`).
pub(crate) const PAGE_FRACTION: f64 = 0.875;

/// How far a key scrolls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum KeyStep {
    /// An arrow: [`LINE_STEP`] toward the end (`true`) or the start.
    Line(bool),
    /// A page toward the end or the start.
    Page(bool),
    /// To the end or the start.
    Edge(bool),
}

/// The scroll `key` with `modifiers` makes under `keymap`, if any.
pub(crate) fn key_scroll(
    keymap: TextKeymap,
    key: KeyCode,
    modifiers: Modifiers,
) -> Option<(WheelAxis, KeyStep)> {
    use KeyStep::*;
    use WheelAxis::*;
    let Modifiers {
        shift,
        ctrl,
        alt,
        meta,
    } = modifiers;
    let bare = !shift && !ctrl && !alt && !meta;
    match keymap {
        TextKeymap::Standard => match key {
            KeyCode::ArrowUp if bare => Some((Vertical, Line(false))),
            KeyCode::ArrowDown if bare => Some((Vertical, Line(true))),
            KeyCode::ArrowLeft if bare => Some((Horizontal, Line(false))),
            KeyCode::ArrowRight if bare => Some((Horizontal, Line(true))),
            KeyCode::PageUp if !ctrl && !alt && !meta => Some((Vertical, Page(false))),
            KeyCode::PageDown if !ctrl && !alt && !meta => Some((Vertical, Page(true))),
            KeyCode::Home if !shift && !alt && !meta => Some((Vertical, Edge(false))),
            KeyCode::End if !shift && !alt && !meta => Some((Vertical, Edge(true))),
            KeyCode::Space if !ctrl && !alt && !meta => Some((Vertical, Page(!shift))),
            _ => None,
        },
        TextKeymap::MacOs => match key {
            KeyCode::ArrowUp if meta && !shift && !alt && !ctrl => Some((Vertical, Edge(false))),
            KeyCode::ArrowDown if meta && !shift && !alt && !ctrl => Some((Vertical, Edge(true))),
            KeyCode::ArrowUp if alt && !shift && !meta && !ctrl => Some((Vertical, Page(false))),
            KeyCode::ArrowDown if alt && !shift && !meta && !ctrl => Some((Vertical, Page(true))),
            KeyCode::ArrowUp if bare => Some((Vertical, Line(false))),
            KeyCode::ArrowDown if bare => Some((Vertical, Line(true))),
            KeyCode::ArrowLeft if bare => Some((Horizontal, Line(false))),
            KeyCode::ArrowRight if bare => Some((Horizontal, Line(true))),
            KeyCode::PageUp if !ctrl && !alt && !meta => Some((Vertical, Page(false))),
            KeyCode::PageDown if !ctrl && !alt && !meta => Some((Vertical, Page(true))),
            KeyCode::Home if bare => Some((Vertical, Edge(false))),
            KeyCode::End if bare => Some((Vertical, Edge(true))),
            KeyCode::Space if !ctrl && !alt && !meta => Some((Vertical, Page(!shift))),
            _ => None,
        },
    }
}

fn scrollbar_axis(axis: WheelAxis) -> ScrollbarAxis {
    match axis {
        WheelAxis::Vertical => ScrollbarAxis::Vertical,
        WheelAxis::Horizontal => ScrollbarAxis::Horizontal,
    }
}

impl RinchApp {
    /// Whether Space on the focused node activates it rather than scrolling,
    /// as Chrome 158 answers: a `<button>`, a form control (`<input>`,
    /// `<select>`, `<textarea>`), `<summary>` and `<option>` take Space; a link,
    /// `role="button"` or any other focused box does not (Space scrolls the page
    /// there, and no click fires, even with a click handler on an ancestor). A
    /// node with its own click handler (`data-rid`) is rinch's activation target
    /// and takes it too.
    pub(super) fn focused_node_activates(&self) -> bool {
        let super::FocusTarget::Node(id) = self.focus_target else {
            return false;
        };
        let Some(doc) = &self.doc else { return false };
        let d = doc.borrow();
        let Some(node) = d.tree.get(id) else {
            return false;
        };
        node.attributes.contains_key("data-rid")
            || matches!(
                node.tag().unwrap_or(""),
                "button" | "input" | "select" | "textarea" | "summary" | "option"
            )
    }

    /// Scroll for an unclaimed `key`, as Chrome's default action does.
    /// Answers whether it scrolled (or started a scroll); `false` leaves the
    /// key to whatever comes after.
    pub(super) fn keyboard_scroll(&mut self, key: KeyCode, modifiers: Modifiers) -> bool {
        let Some((axis, step)) = key_scroll(self.text_keymap, key, modifiers) else {
            return false;
        };
        let Some(doc) = self.doc.clone() else {
            return false;
        };
        let start = match self.focus_target {
            super::FocusTarget::Node(id) => Some(id),
            _ => self.keyboard_scroll_anchor,
        };
        let forward = match step {
            KeyStep::Line(f) | KeyStep::Page(f) | KeyStep::Edge(f) => f,
        };
        let plan = {
            let d = doc.borrow();
            self.keyboard_scroller(&d, start, axis, forward).map(|id| {
                let (min, max) = offset_bounds(&d, id, axis);
                let delta = match step {
                    KeyStep::Line(f) => {
                        if f {
                            LINE_STEP
                        } else {
                            -LINE_STEP
                        }
                    }
                    KeyStep::Page(f) => {
                        let id_ = rinch_core::dom::NodeId(id);
                        let port = match axis {
                            WheelAxis::Vertical => {
                                rinch_core::dom::DomDocument::client_height(&*d, id_)
                            }
                            WheelAxis::Horizontal => {
                                rinch_core::dom::DomDocument::client_width(&*d, id_)
                            }
                        };
                        let page = (port * PAGE_FRACTION).max(1.0);
                        if f { page } else { -page }
                    }
                    KeyStep::Edge(_) => 0.0,
                };
                (id, delta, if forward { max } else { min })
            })
        };
        let Some((id, delta, edge)) = plan else {
            return false;
        };
        let edge_step = matches!(step, KeyStep::Edge(_));
        if self.smooth_wheel.enabled {
            let d = doc.borrow();
            if edge_step {
                let now = d.animation_now_ms();
                self.smooth_wheel
                    .animate_to(&d, id, axis, edge, CurveKind::Keyboard, now);
            } else {
                self.smooth_wheel
                    .animate_by(&d, id, axis, delta, CurveKind::Keyboard);
            }
            return true;
        }
        // Smooth scrolling off: the whole distance now, through the wheel's
        // scroll write (device grid, damage, `onscroll`).
        let fired = {
            let mut d = doc.borrow_mut();
            let delta = if edge_step {
                let n = d.tree.get(id);
                let offset = n.map_or(0.0, |n| match axis {
                    WheelAxis::Vertical => n.scroll_offset.1 + n.scroll_remainder.1,
                    WheelAxis::Horizontal => n.scroll_offset.0 + n.scroll_remainder.0,
                });
                edge - offset
            } else {
                delta
            };
            let (moved, handler) = match axis {
                WheelAxis::Vertical => Self::scroll_vertically_by(&mut d, id, delta),
                WheelAxis::Horizontal => Self::scroll_horizontally_by(&mut d, id, delta),
            };
            if moved {
                self.scene_dirty = true;
            }
            handler.map(|h| (h, Self::scroll_event_for(&d.tree, id)))
        };
        if let Some((h, event)) = fired {
            rinch_core::events::dispatch_scroll_event(rinch_core::events::EventHandlerId(h), event);
        }
        true
    }

    /// The scroller a key moves on `axis` toward its end (`forward`) or start,
    /// from `start` (see the module documentation).
    fn keyboard_scroller(
        &self,
        d: &RinchDocument,
        start: Option<u64>,
        axis: WheelAxis,
        forward: bool,
    ) -> Option<u64> {
        let tree = &d.tree;
        let usable = |id: u64| {
            if !rinch_dom::paint::scrollbar::overflows(tree, id, scrollbar_axis(axis))
                || tree.scroll_locked_out(id)
            {
                return false;
            }
            // Where it is, or where a running scroll is taking it.
            let at = self.smooth_wheel.heading_to(d, id, axis).or_else(|| {
                tree.get(id).map(|n| match axis {
                    WheelAxis::Vertical => n.scroll_offset.1,
                    WheelAxis::Horizontal => n.scroll_offset.0,
                })
            });
            let Some(at) = at else { return false };
            let (min, max) = offset_bounds(d, id, axis);
            if forward {
                at < max - 0.5
            } else {
                at > min + 0.5
            }
        };
        let mut current = start.filter(|&id| tree.get(id).is_some());
        while let Some(id) = current {
            let node = tree.get(id)?;
            if usable(id) {
                return Some(id);
            }
            // A fixed box hands the key to the page, as Chrome's does.
            if node.box_position() == rinch_dom::computed_style::PositionValue::Fixed {
                break;
            }
            current = node.parent;
        }
        tree.get(tree.body_id)?;
        usable(tree.body_id).then_some(tree.body_id)
    }
}
