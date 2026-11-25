/// Whether `node` is not hit itself (#843): `visibility: hidden`/`collapse`,
/// a text node answering from its parent. Paint's answer too
/// (`rinch_dom::visibility::hides_own`).
#[inline]
pub(super) fn hides_own_hit(tree: &rinch_dom::NodeTree, node: &rinch_dom::Node) -> bool {
    rinch_dom::visibility::hides_own(tree, node)
}

/// Whether the glyph under `(lx, ly)` — relative to `node`'s border-box
/// origin — is text a `visibility: visible` element of `node`'s IFC draws.
///
/// An IFC's text is hit as its root (the stretched text nodes are skipped), so
/// a hidden root still answers for the text of a visible inline element in it,
/// which paint draws (#829's per-element mask) and Chrome hits (#843). Asked
/// only of a hidden node under the point whose subtree shows something.
pub(super) fn visible_text_at(
    tree: &rinch_dom::NodeTree,
    node: &rinch_dom::Node,
    lx: f32,
    ly: f32,
) -> bool {
    let Some(inline) = node.text_layout.as_ref() else {
        return false;
    };
    let (ox, oy) = rinch_dom::paint::ifc_root_content_origin(node);
    let rel_x = lx - ox + node.scroll_offset.0 as f32;
    let rel_y = ly - oy + node.scroll_offset.1 as f32;
    let Some(cluster) = rinch_dom::text_query::cluster_range_in_inline(inline, rel_x, rel_y) else {
        return false;
    };
    inline
        .text_ranges
        .iter()
        .find(|r| !r.is_br && (r.flat_start..r.flat_end).contains(&cluster.start))
        .and_then(|r| tree.get(r.node_id))
        .and_then(|t| t.parent)
        .and_then(|p| tree.get(p))
        .is_some_and(|el| !hides_own_hit(tree, el))
}

/// The point `(x, y)` in IFC root `node`'s glyph space: relative to its
/// content origin, its own scroll offset added. `(lx, ly)` is the point
/// relative to its border-box origin.
fn ifc_point(node: &rinch_dom::Node, lx: f32, ly: f32) -> (f32, f32) {
    let (ox, oy) = rinch_dom::paint::ifc_root_content_origin(node);
    (
        lx - ox + node.scroll_offset.0 as f32,
        ly - oy + node.scroll_offset.1 as f32,
    )
}

/// The text node whose glyph is drawn at `(rel_x, rel_y)` ([`ifc_point`])
/// in `inline` (#2404).
fn glyph_text_node(
    inline: &rinch_dom::node::InlineLayout,
    rel_x: f32,
    rel_y: f32,
) -> Option<Option<u64>> {
    let cluster = rinch_dom::text_query::glyph_cluster_in_inline(inline, rel_x, rel_y)?;
    Some(
        inline
            .text_ranges
            .iter()
            .find(|r| !r.is_br && (r.flat_start..r.flat_end).contains(&cluster.start))
            .map(|r| r.node_id),
    )
}

/// Whether `node` takes pointer events: not `pointer-events: none`, and
/// `visibility: visible`.
pub(super) fn takes_pointer(node: &rinch_dom::Node) -> bool {
    use rinch_dom::computed_style::{PointerEventsValue, VisibilityValue};
    node.computed_style.pointer_events != PointerEventsValue::None
        && node.computed_style.visibility == VisibilityValue::Visible
}

/// Whether the point `(lx, ly)`, relative to IFC root `node`'s border-box
/// origin and outside its box, is in the padding or border of one of its
/// inline elements that takes pointer events (#2647). Chrome 157 hits such
/// an element there: a padded span on a line near its block's edge reaches
/// past the block. Not a box that clips: its overflow is not drawn.
pub(super) fn inline_box_outside(
    tree: &rinch_dom::NodeTree,
    node: &rinch_dom::Node,
    lx: f32,
    ly: f32,
) -> bool {
    let Some(inline) = node.text_layout.as_ref() else {
        return false;
    };
    let (rel_x, rel_y) = ifc_point(node, lx, ly);
    rinch_dom::paint::inline_element_at_where(
        |id| tree.get(id),
        inline,
        rel_x,
        rel_y,
        takes_pointer,
    )
    .is_some()
}

/// The text node whose glyph is under the window point `(x, y)` in the IFC
/// root `root_id`, if any: an IFC's text is hit as its root (its text nodes
/// stretch over the whole line box and are skipped), and this says which of
/// its inline elements the point is in. Where no glyph is, the inline
/// element whose padding or border is under the point (#2618; its box where
/// paint draws it, `rinch_dom::paint::inline_element_at`), as Chrome 157's
/// `elementFromPoint` finds it.
pub(crate) fn text_node_at(
    tree: &rinch_dom::NodeTree,
    root_id: u64,
    x: f32,
    y: f32,
) -> Option<u64> {
    let node = tree.get(root_id)?;
    let inline = node.text_layout.as_ref()?;
    let (lx, ly) = pointer_in_node(tree, root_id, x, y);
    let (rel_x, rel_y) = ifc_point(node, lx, ly);
    match glyph_text_node(inline, rel_x, rel_y) {
        Some(text) => text,
        // An inline element's padding or border (#2618).
        None => rinch_dom::paint::inline_element_at(|id| tree.get(id), inline, rel_x, rel_y),
    }
}

/// Where a press at `(x, y)` in IFC root `root_id` is delivered, as in
/// Chrome 157: the text node under the glyphs, or, when the element around
/// it takes no pointer events (`pointer-events: none`, or not visible), the
/// nearest element above it that does; off the glyphs, the last inline
/// element in document order that takes pointer events and whose padding or
/// border is under the point (#2618). An element that takes none passes the
/// point to one around it only where that one's box holds the point
/// (#2647). `None` past the glyphs and those boxes (the root's own, then).
pub(crate) fn event_target_at(
    tree: &rinch_dom::NodeTree,
    root_id: u64,
    x: f32,
    y: f32,
) -> Option<u64> {
    let node = tree.get(root_id)?;
    let inline = node.text_layout.as_ref()?;
    let (lx, ly) = pointer_in_node(tree, root_id, x, y);
    let (rel_x, rel_y) = ifc_point(node, lx, ly);
    let Some(text) = glyph_text_node(inline, rel_x, rel_y) else {
        return rinch_dom::paint::inline_element_at_where(
            |id| tree.get(id),
            inline,
            rel_x,
            rel_y,
            takes_pointer,
        );
    };
    let target = text?;
    let takes = |id: u64| tree.get(id).is_some_and(takes_pointer);
    let mut element = tree.get(target)?.parent?;
    if takes(element) {
        return Some(target);
    }
    loop {
        if element == root_id {
            return Some(root_id);
        }
        element = tree.get(element)?.parent?;
        if takes(element) {
            return Some(element);
        }
    }
}

/// The element a pointer event targets, given `hit_test`'s answer `hit`
/// and `target`, [`event_target_at`]'s refinement of it (else `hit`): the
/// inline element under the point when `hit` is an IFC root (its glyphs or
/// its padding and border, or the nearest element above it that takes
/// pointer events), else `hit` itself. Never a text node: its parent
/// element, as the DOM's `event.target` and `:hover` are an element (#2648).
///
/// `:hover`, `:active`, the cursor, `mouseenter`/`mouseleave`, the
/// `data-onmouse*` handlers, `oncontextmenu`, `onwheel`, the `title`
/// tooltip and the focus a press takes all start here
/// (`RinchApp::shared_target`), where Chrome starts them; the click and drag
/// walks start at `target` (`RinchApp::shared_event_target`).
pub(crate) fn element_of_target(tree: &rinch_dom::NodeTree, hit: u64, target: u64) -> u64 {
    match tree.get(target) {
        Some(n) if n.is_text() => n.parent.unwrap_or(hit),
        Some(_) => target,
        None => hit,
    }
}

/// Whether some element below `id` computes to `visibility: visible` (#843),
/// i.e. whether a hidden `id`'s subtree can hold anything to hit. One memo with
/// paint and the stacking collector, which skip the same subtrees (#2137):
/// `rinch_dom::visibility::shows_below`.
#[inline]
pub(super) fn shows_below(tree: &rinch_dom::NodeTree, id: rinch_dom::node::RawNodeId) -> bool {
    rinch_dom::visibility::shows_below(tree, id)
}

/// The box a node is *painted* in, in layout pixels, shaped as the four
/// `element_*` fields a [`rinch_core::events::ClickContext`] carries.
///
/// One conversion for every producer of those fields — the click path, the
/// context-menu path, the drag- and mouse-attribute paths, keyboard activation,
/// the ancestor chain and `bounds_signal` — so they cannot report different
/// rects for the same node. It is the `getBoundingClientRect()` answer
/// rinch-web already fills them from, which is what makes the same markup
/// behave the same on both backends (#203).
pub(crate) fn painted_element_box(
    tree: &rinch_dom::NodeTree,
    node_id: u64,
) -> (f32, f32, f32, f32) {
    let r = rinch_dom::paint::painted_border_box(tree, node_id, 1.0);
    (
        r.x0 as f32,
        r.y0 as f32,
        r.width() as f32,
        r.height() as f32,
    )
}

/// A pointer position mapped **into** a node's own space, relative to the
/// node's border-box origin — the inverse of [`painted_element_box`].
///
/// The direction anything consuming a point *inside* a box needs: how far along
/// a scrollbar track a press landed, where in a render surface the pointer is,
/// which character a click chose. Subtracting the painted box's origin is not
/// the same answer — under `transform: scale(2)` a pointer 40px right of the
/// painted left edge is 20px into the box (#203).
///
/// Falls back to the raw point when the composed transform is not invertible
/// (a `scale(0)` subtree paints nothing, so there is no meaningful local
/// position and the caller's clamp is as good an answer as any).
pub(crate) fn pointer_in_node(
    tree: &rinch_dom::NodeTree,
    node_id: u64,
    x: f32,
    y: f32,
) -> (f32, f32) {
    rinch_dom::paint::point_in_painted_box(tree, node_id, 1.0, x as f64, y as f64)
        .map(|(lx, ly)| (lx as f32, ly as f32))
        .unwrap_or((x, y))
}
