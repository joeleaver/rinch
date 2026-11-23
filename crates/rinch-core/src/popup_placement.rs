//! What a popup window is (#2552): where it goes against its anchor
//! ([`Placement`], [`Adjust`]), what kind it is ([`PopupKind`]), where the
//! system put it ([`PopupPlaced`]) and why it went ([`Dismissal`]).
//! Re-exported from [`crate::element`], beside [`WindowProps`](crate::element::WindowProps).
//!
//! These are data only. `rinch::popup` opens popups with them; the props
//! that name a window (`PopupProps`, `PopupAnchor`) live there, next to the
//! `WindowHandle` they take.

/// Which side of its anchor a popup opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Side {
    Top,
    /// Below the anchor (the default: a menu under its button).
    #[default]
    Bottom,
    Left,
    Right,
}

/// How a popup's edge lines up with its anchor's, along the side it opens
/// on: `Start` is the left edge for `Top`/`Bottom` and the top edge for
/// `Left`/`Right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// Where a popup goes against its anchor: on `side`, aligned by `align`.
///
/// - [`Placement::BOTTOM_START`]: below, left edges aligned (a menu under its
///   button, a menu from a **top** bar).
/// - [`Placement::TOP_START`]: above, left edges aligned (a menu from a
///   **bottom** bar).
/// - [`Placement::RIGHT_START`]: to the right, top edges aligned (a submenu).
///
/// What the system may do when the popup does not fit there is [`Adjust`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Placement {
    pub side: Side,
    pub align: Align,
}

impl Placement {
    pub const BOTTOM_START: Self = Self::new(Side::Bottom, Align::Start);
    pub const BOTTOM_CENTER: Self = Self::new(Side::Bottom, Align::Center);
    pub const BOTTOM_END: Self = Self::new(Side::Bottom, Align::End);
    pub const TOP_START: Self = Self::new(Side::Top, Align::Start);
    pub const TOP_CENTER: Self = Self::new(Side::Top, Align::Center);
    pub const TOP_END: Self = Self::new(Side::Top, Align::End);
    pub const RIGHT_START: Self = Self::new(Side::Right, Align::Start);
    pub const RIGHT_CENTER: Self = Self::new(Side::Right, Align::Center);
    pub const RIGHT_END: Self = Self::new(Side::Right, Align::End);
    pub const LEFT_START: Self = Self::new(Side::Left, Align::Start);
    pub const LEFT_CENTER: Self = Self::new(Side::Left, Align::Center);
    pub const LEFT_END: Self = Self::new(Side::Left, Align::End);

    /// A placement on `side`, aligned by `align`.
    pub const fn new(side: Side, align: Align) -> Self {
        Self { side, align }
    }
}

/// What the system may do with a popup that would not fit where its
/// [`Placement`] puts it (the area it is kept inside is the compositor's:
/// the work area on KWin, the output on sway and labwc). Bit for bit
/// `xdg_positioner`'s constraint adjustment.
///
/// The default is [`Adjust::FLIP_AND_SLIDE`]. A menu that may be taller than
/// the screen adds [`Adjust::RESIZE_Y`]: it is then given the height left,
/// and its content scrolls. [`Adjust::FIT`] is all six.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Adjust(u8);

impl Adjust {
    pub const NONE: Self = Self(0);
    /// Slide along the x axis, keeping the side.
    pub const SLIDE_X: Self = Self(1 << 0);
    /// Slide along the y axis.
    pub const SLIDE_Y: Self = Self(1 << 1);
    /// Open on the other side horizontally (a submenu at the right edge
    /// opens to the left).
    pub const FLIP_X: Self = Self(1 << 2);
    /// Open on the other side vertically.
    pub const FLIP_Y: Self = Self(1 << 3);
    /// Be made narrower: the content then scrolls.
    pub const RESIZE_X: Self = Self(1 << 4);
    /// Be made shorter: the content then scrolls.
    pub const RESIZE_Y: Self = Self(1 << 5);
    /// Flip and slide on both axes: the default.
    pub const FLIP_AND_SLIDE: Self =
        Self(Self::FLIP_X.0 | Self::FLIP_Y.0 | Self::SLIDE_X.0 | Self::SLIDE_Y.0);
    /// A menu from a bar along the top or bottom edge: slide along the bar,
    /// flip to the other side only when that is better, and be made shorter
    /// (the content scrolls) when taller than the space left. Pair it with
    /// `PopupAnchor::NodeAcross` and `Placement::TOP_START` (a bottom bar) or
    /// `BOTTOM_START` (a top bar).
    pub const BAR_MENU: Self = Self(Self::SLIDE_X.0 | Self::FLIP_Y.0 | Self::RESIZE_Y.0);
    /// Flip, slide and resize on both axes.
    pub const FIT: Self = Self(Self::FLIP_AND_SLIDE.0 | Self::RESIZE_X.0 | Self::RESIZE_Y.0);

    /// The bits, `xdg_positioner`'s values.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every bit of `other` is set here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl Default for Adjust {
    fn default() -> Self {
        Self::FLIP_AND_SLIDE
    }
}

impl std::ops::BitOr for Adjust {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Adjust {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// What a popup is to the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PopupKind {
    /// A grab: opened by a press (the newest press its parent chain
    /// received), it has the keyboard, and the system closes it on a press
    /// outside the app. Escape closes the topmost one, and the app's menu
    /// shortcuts are held while one is open. A menu on a popup must open
    /// from a menu. Calendars, volume sliders and other popups that take
    /// input are menus too in this version; a kind of their own may come
    /// later.
    #[default]
    Menu,
    /// No grab, never the keyboard, and no pointer input: a tooltip
    /// appearing under a resting pointer does not take it from the element
    /// it describes. Nothing closes it but the app and its parent going.
    Tooltip,
}

/// Where the system put a popup, in its parent's logical px (the
/// parent's surface origin at 0, 0), and whether it was flipped to the
/// other side from what its [`Placement`] asked.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PopupPlaced {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// It opened on the other side horizontally (a `Left`/`Right` placement
    /// flipped). Only the side's axis is judged, never the alignment's.
    pub flipped_x: bool,
    /// It opened on the other side vertically (a `Top`/`Bottom` placement
    /// flipped).
    pub flipped_y: bool,
}

/// Why a popup went, told once to its `on_dismissed`. A popup the app
/// closed itself (its handle's `close`, its owner going) is told nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dismissal {
    /// The system dismissed it (`popup_done` on Wayland): a press in
    /// another app, a shortcut that took the focus, the screen locking.
    System,
    /// A press in one of the app's own windows outside the popups above
    /// that press. The press is delivered too. `on_anchor` is whether it
    /// was on the popup's anchor, so a toggle can tell its own press apart.
    PressOutside { on_anchor: bool },
    /// Escape, in this popup when it was the topmost menu.
    Escape,
    /// The window below it closed (or hid): its parent, or a popup below it
    /// in the chain. A bar removed with its output closes its menu and
    /// submenus this way, topmost first.
    ParentClosed,
    /// Another menu chain opened (one menu chain at a time).
    Replaced,
    /// The system refused it after it was asked for (on Wayland, a grab
    /// with no press to name, or a popup dismissed before it was placed).
    Failed(String),
}
