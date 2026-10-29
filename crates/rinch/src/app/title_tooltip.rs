//! The `title` tooltip (issue #1646): after the pointer rests
//! [`TITLE_TOOLTIP_DELAY`] over an element whose nearest `title` is not empty,
//! the runtime shows that text in a small box below and to the right of the
//! pointer — the advisory tooltip a browser shows, which `rinch-web` gets from
//! the browser itself.
//!
//! The box is not a DOM node: it is a [`rinch_dom::paint::tooltip::Tooltip`],
//! shaped once and painted after the document by `build_pixels` /
//! `build_scene`, with its framebuffer rect as the frame's damage. Showing,
//! hiding or re-texting it runs no style, layout or IFC pass.
//!
//! The rest pointer is timed by [`RinchApp::title_tooltip_deadline`], which the
//! desktop shell and `RinchContext::next_wake` fold into their timed wake; the
//! show itself is [`RinchApp::tick_title_tooltip`], run from `AboutToWait`.
//! Nothing is timed, walked or painted while the pointer is over no titled
//! element, and a document that never wrote a `title` pays one counter read
//! per pointer move (`NodeTree::title_sources`).
//!
//! It hides when the pointer moves onto content under another `title` (or
//! none), and on any press, wheel, key press, pointer cancel, resize, window
//! blur or the pointer leaving the window. A press, wheel or key also keeps it
//! hidden until the pointer reaches content under another `title`: resting on
//! the button just clicked does not bring it back. One tooltip at a time.

use super::*;
use rinch_dom::paint::tooltip::{Tooltip, advisory_title, own_title, place_tooltip};

/// How long the pointer rests before the tooltip shows.
pub const TITLE_TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// A tooltip waiting for the pointer to rest.
#[derive(Debug)]
struct Pending {
    /// The node whose `title` applies.
    source: u64,
    /// The pointer, logical px, as of its last move.
    pointer: (f32, f32),
    due: Instant,
}

/// A tooltip on screen.
#[derive(Debug)]
pub(crate) struct ShownTooltip {
    /// The node whose `title` it shows.
    pub(crate) source: u64,
    /// The pointer it was placed for, logical px.
    pointer: (f32, f32),
    /// Its box's top-left corner, logical px.
    pub(crate) origin: (f32, f32),
    pub(crate) tip: Tooltip,
}

/// The tooltip's state on a [`RinchApp`].
#[derive(Debug)]
pub(crate) struct TitleTooltipState {
    enabled: bool,
    pending: Option<Pending>,
    pub(crate) shown: Option<ShownTooltip>,
    /// The `title` source a press, wheel or key hid the tooltip over: no
    /// tooltip for it until the pointer reaches another one (or untitled
    /// content).
    suppressed: Option<u64>,
    /// The source the pointer was last over, for [`Self::suppressed`].
    over: Option<u64>,
    /// The framebuffer rect the tooltip covered in the frame on screen, in
    /// device px — the inspect highlight's twin.
    #[cfg(software_shell)]
    pub(crate) last_rect: Option<peniko::kurbo::Rect>,
}

thread_local! {
    /// The default for a new [`RinchApp`] on this thread: `App::title_tooltips`
    /// (`run` creates the window's app on the thread it is called on).
    static TOOLTIPS_OFF_BY_DEFAULT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a `RinchApp` created from now on, on this thread, shows `title` tooltips
/// ([`App::title_tooltips`](crate::App::title_tooltips) calls this before the
/// window's app is created). Android ignores it: it never shows one.
pub fn set_title_tooltips_default(enabled: bool) {
    TOOLTIPS_OFF_BY_DEFAULT.set(!enabled);
}

impl Default for TitleTooltipState {
    fn default() -> Self {
        Self {
            // A touch screen has no resting pointer: a finger that stays down
            // is a long press (a context menu), and one that lifts leaves no
            // pointer behind.
            enabled: !cfg!(target_os = "android") && !TOOLTIPS_OFF_BY_DEFAULT.get(),
            pending: None,
            shown: None,
            suppressed: None,
            over: None,
            #[cfg(software_shell)]
            last_rect: None,
        }
    }
}

impl RinchApp {
    /// Turn the `title` tooltip on or off (on by default, except on Android).
    /// Turning it off hides one that is showing.
    pub fn set_title_tooltips(&mut self, enabled: bool) {
        self.title_tooltip.enabled = enabled;
        if !enabled {
            self.title_tooltip.pending = None;
            self.hide_title_tooltip_now();
        }
    }

    /// Whether the `title` tooltip is on.
    pub fn title_tooltips_enabled(&self) -> bool {
        self.title_tooltip.enabled
    }

    /// The text of the tooltip on screen, if one is.
    pub fn title_tooltip_text(&self) -> Option<&str> {
        self.title_tooltip.shown.as_ref().map(|s| s.tip.text())
    }

    /// The tooltip's box on screen as `(x, y, w, h)` in logical px, if one is.
    pub fn title_tooltip_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.title_tooltip.shown.as_ref().map(|s| {
            let (w, h) = s.tip.size();
            (s.origin.0, s.origin.1, w, h)
        })
    }

    /// When a pointer at rest will show a tooltip: the instant the timed wake
    /// is for. `None` when nothing is waiting.
    pub fn title_tooltip_deadline(&self) -> Option<Instant> {
        self.title_tooltip.pending.as_ref().map(|p| p.due)
    }

    /// A pointer move to `(x, y)` over `hit` (the move's hit test).
    pub(crate) fn note_title_tooltip_move(
        &mut self,
        hit: Option<u64>,
        x: f32,
        y: f32,
        now: Instant,
    ) {
        let state = &self.title_tooltip;
        if !state.enabled {
            return;
        }
        let titled = self
            .doc
            .as_ref()
            .is_some_and(|doc| doc.borrow().tree.title_sources > 0);
        if !titled && state.pending.is_none() && state.shown.is_none() && state.over.is_none() {
            // No `title` anywhere and nothing to take down: the whole cost of
            // a pointer move in a document without one.
            return;
        }
        let source = if titled {
            hit.and_then(|h| {
                let doc = self.doc.as_ref()?;
                let d = doc.borrow();
                advisory_title(&d.tree, h).map(|(id, _)| id)
            })
        } else {
            None
        };
        let state = &mut self.title_tooltip;
        state.over = source;
        if state.suppressed.is_some() && state.suppressed == source {
            return;
        }
        state.suppressed = None;
        if let Some(shown) = &state.shown {
            if Some(shown.source) == source {
                // Still over what it describes: it stays where it is.
                return;
            }
            self.hide_title_tooltip_now();
        }
        self.title_tooltip.pending = source.map(|source| Pending {
            source,
            pointer: (x, y),
            due: now + TITLE_TOOLTIP_DELAY,
        });
    }

    /// Hide the tooltip (and forget a pending one) because the user did
    /// something — a press, a wheel, a key. It stays hidden until the pointer
    /// reaches content under another `title`.
    pub(crate) fn dismiss_title_tooltip(&mut self) {
        let state = &mut self.title_tooltip;
        state.pending = None;
        state.suppressed = state.over;
        self.hide_title_tooltip_now();
    }

    /// Hide the tooltip and forget a pending one, with no suppression: the
    /// pointer left the window, the window lost focus or was resized.
    pub fn hide_title_tooltip(&mut self) {
        self.title_tooltip.pending = None;
        self.title_tooltip.over = None;
        self.title_tooltip.suppressed = None;
        self.hide_title_tooltip_now();
    }

    /// Take the tooltip off the screen, if it is on it.
    fn hide_title_tooltip_now(&mut self) {
        if self.title_tooltip.shown.take().is_some() {
            // Its last rect is the damage (`build_pixels`).
            self.request_repaint();
        }
    }

    /// Show a tooltip whose pointer has rested, and follow a shown tooltip's
    /// `title` as it changes. Run from `AboutToWait`. Answers whether the
    /// frame on screen is out of date.
    pub(crate) fn tick_title_tooltip(&mut self, now: Instant, vp_w: f32, vp_h: f32) -> bool {
        let state = &self.title_tooltip;
        if state.pending.is_none() && state.shown.is_none() {
            return false;
        }
        // No tooltip during a drag of any kind.
        if rinch_core::Drag::is_active() || self.active_dnd.is_some() || self.pending_drag.is_some()
        {
            let had = self.title_tooltip.shown.is_some();
            self.title_tooltip.pending = None;
            self.hide_title_tooltip_now();
            return had;
        }
        let Some(doc) = self.doc.clone() else {
            return false;
        };

        if let Some(shown) = &self.title_tooltip.shown {
            let source = shown.source;
            let pointer = shown.pointer;
            let d = doc.borrow();
            let now_text = (Self::in_document(&d.tree, source))
                .then(|| own_title(&d.tree, source).filter(|t| !t.trim().is_empty()))
                .flatten();
            drop(d);
            match now_text {
                None => {
                    // The attribute went (or the element did): so does the tooltip.
                    self.title_tooltip.shown = None;
                    self.request_repaint();
                    return true;
                }
                Some(text) if text != shown.tip.text() => {
                    let tip = {
                        let mut d = doc.borrow_mut();
                        let d = &mut *d;
                        Tooltip::new(&d.tree, &mut d.font_cx, &mut d.layout_cx, &text)
                    };
                    let origin = place_tooltip(pointer, tip.size(), (vp_w, vp_h));
                    self.title_tooltip.shown = Some(ShownTooltip {
                        source,
                        pointer,
                        origin,
                        tip,
                    });
                    self.request_repaint();
                    return true;
                }
                Some(_) => return false,
            }
        }

        let Some(pending) = &self.title_tooltip.pending else {
            return false;
        };
        if now < pending.due {
            return false;
        }
        let Pending {
            source, pointer, ..
        } = self.title_tooltip.pending.take().expect("checked above");
        let tip = {
            let mut d = doc.borrow_mut();
            let d = &mut *d;
            // Read again: the attribute may have changed or gone while the
            // pointer rested.
            let text = Self::in_document(&d.tree, source)
                .then(|| own_title(&d.tree, source).filter(|t| !t.trim().is_empty()))
                .flatten();
            let Some(text) = text else {
                return false;
            };
            Tooltip::new(&d.tree, &mut d.font_cx, &mut d.layout_cx, &text)
        };
        let origin = place_tooltip(pointer, tip.size(), (vp_w, vp_h));
        self.title_tooltip.shown = Some(ShownTooltip {
            source,
            pointer,
            origin,
            tip,
        });
        self.request_repaint();
        true
    }

    fn in_document(tree: &rinch_dom::NodeTree, id: u64) -> bool {
        let mut cur = Some(id);
        while let Some(nid) = cur {
            if nid == tree.root_id {
                return true;
            }
            cur = tree.nodes.get(nid).and_then(|n| n.parent);
        }
        false
    }

    /// Paint the tooltip on screen, if one is, after the document.
    pub(crate) fn paint_title_tooltip(
        shown: Option<&ShownTooltip>,
        painter: &mut dyn rinch_dom::paint::painter::Painter,
        scale: f64,
    ) {
        if let Some(s) = shown {
            s.tip.paint(painter, scale, s.origin);
        }
    }
}
