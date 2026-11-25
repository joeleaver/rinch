use super::super::ScrollAxis;
use super::scroll::find_scrollbar_hit;
use super::targets::pointer_in_node;

/// Detect whether a mouse position is near a window edge for resize.
///
/// All five arguments are in the same unit: **logical (CSS) pixels** — the
/// pointer position `PlatformEvent` carries, the logical layout viewport, and
/// `WindowProps::resize_inset`, which is documented as matching a CSS margin.
/// The caller used to hand this the physical `window_size` and a
/// `inset * scale_factor`, because the desktop shell fed physical pointer
/// coordinates; #299 moved the pointer into logical space, so the whole
/// comparison moved with it.
///
/// The grab zone is `inset` px deep on every edge, and a corner is where two of
/// those zones meet — an `inset` x `inset` square. Nothing is added to `inset`
/// and no corner is enlarged (see the comment in the body, and #423).
///
/// This answers only "is this position in the zone". Whether the press is
/// actually *given* to a resize is the caller's decision: a visible scrollbar
/// thumb takes an edge press off it (#399/#420, [`pointer_on_scrollbar_thumb`]).
///
/// Returns the resize direction if the cursor is within the grab zone.
pub(crate) fn detect_resize_edge(
    x: f32,
    y: f32,
    window_width: f32,
    window_height: f32,
    inset: f32,
) -> Option<rinch_platform::ResizeDirection> {
    use rinch_platform::ResizeDirection::*;
    // The inset is the whole grab zone, on every edge. There is no additive
    // extension and no enlarged corner: a corner is simply where two edge zones
    // meet, an `inset` x `inset` square.
    //
    // There used to be a second, larger `corner = inset * 2` radius here whose
    // four guards read `corner_top || corner_left` and so on. Every one of them
    // was a tautology — the arm they guarded already required `near_top` or
    // `near_left`, and `near_x` implies `corner_x` for any `corner > edge` — so
    // the enlargement never took effect in either the additive form or the
    // multiplicative one that replaced it. Deleting it changes no behaviour
    // (`the_corner_is_where_two_edge_zones_meet` pins that) and stops the code
    // promising something it does not do; CLAUDE.md promised it too, and no
    // longer does (#423).
    //
    // Making it real was considered and rejected: a 16px corner at the default
    // 8px inset reaches into a `BorderlessWindow`'s window-control buttons,
    // which sit within 10px of the top-right corner. An app that wants bigger
    // handles can raise `resize_inset`.
    let edge = inset;

    let near_left = x < edge;
    let near_right = x > window_width - edge;
    let near_top = y < edge;
    let near_bottom = y > window_height - edge;

    match (near_top, near_bottom, near_left, near_right) {
        (true, _, true, _) => Some(NorthWest),
        (true, _, _, true) => Some(NorthEast),
        (_, true, true, _) => Some(SouthWest),
        (_, true, _, true) => Some(SouthEast),
        (true, _, _, _) => Some(North),
        (_, true, _, _) => Some(South),
        (_, _, true, _) => Some(West),
        (_, _, _, true) => Some(East),
        _ => None,
    }
}

/// Whether a resize direction is a corner (diagonal) rather than an edge.
///
/// The distinction matters because an edge yields to a visible scrollbar thumb
/// and a corner does not — see [`pointer_on_scrollbar_thumb`].
pub(crate) fn is_corner_resize(dir: rinch_platform::ResizeDirection) -> bool {
    use rinch_platform::ResizeDirection::*;
    matches!(dir, NorthEast | NorthWest | SouthEast | SouthWest)
}

/// Whether the pointer is on a scroll container's thumb: its **painted**
/// extent along the track, over [`find_scrollbar_hit`]'s full strip across it.
///
/// A borderless window's resize inset overlaps whatever content is flush with
/// the window edge, and for a `BorderlessWindow` — whose root is `100vw` x
/// `100vh` with no margin — that is the scroll container filling it. At the
/// default 8px inset the East zone is `x > width - 8` and the thumb is painted
/// in `[width - 8, width - 2)`, so *every pixel of the thumb you can see* was a
/// resize handle and a press dead-centre of it resized the window (#399, #420).
///
/// The rule that resolves it is "what you can see, you can grab", applied per
/// axis. **Along** the track, a press at the thumb's extent goes to the
/// scrollbar, and the empty track past it — like every edge with no bar on it
/// — still resizes. **Across** the bar the test is [`find_scrollbar_hit`]'s
/// whole hit strip, so the 2px margin between the thumb and the window edge
/// grabs the thumb too: edge-forgiving, the way a browser's bar in a maximised
/// window is grabbable at the very last pixel. A browser has no equivalent
/// *resize* conflict, because its resize border lives in the window frame
/// *outside* the client area; a borderless window has no frame to put it in,
/// so something has to give, and the visible target is the one the user is
/// aiming at.
///
/// Corners are excluded by the caller ([`is_corner_resize`]), so a diagonal
/// resize is always reachable however tall the thumb grows.
pub(crate) fn pointer_on_scrollbar_thumb(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> bool {
    let Some(hit) = find_scrollbar_hit(tree, x, y) else {
        return false;
    };
    let Some(node) = tree.get(hit.node_id) else {
        return false;
    };
    let scroll = match hit.axis {
        ScrollAxis::Vertical => node.scroll_offset.1,
        ScrollAxis::Horizontal => node.scroll_offset.0,
    };
    // The thumb's extent along the track, in the container's own space — the
    // same space the pointer is mapped into, so a scroller under a
    // `transform: scale()` answers about the thumb you can see (#203).
    let local = pointer_in_node(tree, hit.node_id, x, y);
    hit.track
        .thumb_contains(scroll, hit.axis.along(local.0, local.1) as f64)
}

/// Map a resize direction to the appropriate cursor style.
pub(crate) fn resize_direction_to_cursor(
    dir: &rinch_platform::ResizeDirection,
) -> rinch_platform::CursorStyle {
    use rinch_platform::CursorStyle as CS;
    use rinch_platform::ResizeDirection as RD;
    match dir {
        RD::North => CS::NResize,
        RD::South => CS::SResize,
        RD::East => CS::EResize,
        RD::West => CS::WResize,
        RD::NorthEast => CS::NeResize,
        RD::NorthWest => CS::NwResize,
        RD::SouthEast => CS::SeResize,
        RD::SouthWest => CS::SwResize,
    }
}

/// Convert a CursorValue from computed style to a platform CursorStyle.
pub(crate) fn cursor_value_to_style(
    cursor: &rinch_dom::computed_style::CursorValue,
) -> rinch_platform::CursorStyle {
    use rinch_dom::computed_style::CursorValue as CV;
    use rinch_platform::CursorStyle as CS;
    match cursor {
        CV::Auto => CS::Auto,
        CV::Default => CS::Default,
        CV::Pointer => CS::Pointer,
        CV::Text => CS::Text,
        CV::Move => CS::Move,
        CV::NotAllowed => CS::NotAllowed,
        CV::Crosshair => CS::Crosshair,
        CV::Grab => CS::Grab,
        CV::Grabbing => CS::Grabbing,
        CV::ColResize => CS::ColResize,
        CV::RowResize => CS::RowResize,
        CV::NResize => CS::NResize,
        CV::SResize => CS::SResize,
        CV::EResize => CS::EResize,
        CV::WResize => CS::WResize,
        CV::NeResize => CS::NeResize,
        CV::NwResize => CS::NwResize,
        CV::SeResize => CS::SeResize,
        CV::SwResize => CS::SwResize,
        CV::EwResize => CS::EwResize,
        CV::NsResize => CS::NsResize,
        CV::ZoomIn => CS::ZoomIn,
        CV::ZoomOut => CS::ZoomOut,
        CV::Wait => CS::Wait,
        CV::Progress => CS::Progress,
        CV::Help => CS::Help,
        CV::None => CS::None,
    }
}
