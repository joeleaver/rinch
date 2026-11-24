//! Caret motion the browser has to measure: page and vertical steps, line
//! edges, visual lines, and where the keyboard's context menu opens.

use super::context_menu::PARK_SIZE;
use super::*;

/// Page Up (`down = false`) or Page Down in the focused editor (#2020), as on
/// desktop (`RinchApp::page_editor`): the page is 0.875 of the nearest
/// scroller's `clientHeight` (Chrome's step), at most the window's — the
/// scroller a scroll lock leaves alone, not past a `position: fixed` box; the
/// scroller moves a page, and the caret goes to the point a page below (above)
/// where it is, at the goal column, so with the caret in view it keeps its
/// place on screen. With the caret scrolled out of view the point is brought
/// into the scrollport first: the page is from the caret, not the view. A page
/// that reaches the document's last (first) line, or runs past the editor, puts
/// the caret at the document's end (start), as Chrome does. `scroll_only`
/// (macOS's plain key), a read-only editor (Chrome's read-only `<textarea>`)
/// and a caret with no rect scroll and leave the selection. Always consumed:
/// the browser's own Page Down would scroll the capture textarea's page.
pub(super) fn page_step(
    handle: &EditorHandle,
    container_nid: u64,
    doc: &web_sys::Document,
    down: bool,
    extend: bool,
    scroll_only: bool,
) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Some(container) =
        node_by_nid(container_nid).and_then(|n| n.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    // The nearest scroller that overflows, the editor itself included, and
    // that no `lock_scroll` overlay locks out (desktop's `page_scroller`, the
    // drag auto-scroll's rule) — else the page's scrolling element, on the
    // same terms. The walk stops at a `position: fixed` box (a modal): the
    // viewport holds it, so nothing outside it moves it.
    let mut scroller = None;
    let mut fixed = false;
    let mut cur = Some(container.clone());
    while let Some(el) = cur {
        let cs = window.get_computed_style(&el).ok().flatten();
        let prop = |name: &str| {
            cs.as_ref()
                .and_then(|cs| cs.get_property_value(name).ok())
                .unwrap_or_default()
        };
        let overflow = prop("overflow-y");
        if (overflow == "auto" || overflow == "scroll")
            && el.scroll_height() > el.client_height()
            && !crate::web_document::scroll_locked_out(&el)
        {
            scroller = Some(el);
            break;
        }
        if prop("position") == "fixed" {
            fixed = true;
            break;
        }
        cur = el.parent_element();
    }
    let page_el = doc.scrolling_element();
    let page_locked = page_el
        .as_ref()
        .is_none_or(crate::web_document::scroll_locked_out);
    if scroller.is_none() && !fixed && !page_locked {
        scroller = page_el.clone();
    }
    let view = window
        .inner_height()
        .ok()
        .and_then(|h| h.as_f64())
        .unwrap_or(f64::MAX);
    let port = scroller
        .as_ref()
        .map_or(view, |s| f64::from(s.client_height()))
        .min(view);
    let page = (port * 0.875).round().max(1.0);
    let step = if down { page } else { -page };
    let head = handle.selection().head();
    let from = if scroll_only || handle.refuses_edits() {
        None
    } else {
        head_screen_rect(handle, head)
    };
    let scroll_by = |s: &web_sys::Element, dy: f64| {
        let before = f64::from(s.scroll_top());
        s.set_scroll_top((before + dy).round() as i32);
        f64::from(s.scroll_top()) - before
    };
    let scrolled = scroller.as_ref().map_or(0.0, |s| scroll_by(s, step));
    let Some((hx, hy, hh)) = from else {
        return true;
    };
    let gx = goal_x().unwrap_or(hx);
    set_goal_x(Some(gx));
    // A page from the caret's middle, where the caret is drawn now —
    // measured, so a scroller that did not carry the editor moves nothing.
    let now_y =
        head_screen_rect(handle, head).map_or(f64::from(hy) - scrolled, |(_, y, _)| f64::from(y));
    let mut ty = (now_y + f64::from(hh * 0.5) + step) as f32;
    let view_h = view.min(f64::from(i32::MAX)) as f32;
    // The caret scrolled out of the scroller's view (the wheel): the point a
    // page from it is outside the scrollport too, where
    // `caretPositionFromPoint` finds nothing or another element. Chrome 155
    // and Firefox 155 move the caret a page from where it is, wherever the
    // view is: bring the point into the scrollport first, half a caret in
    // from the edge it is past.
    if let Some(s) = &scroller {
        let band = if page_el.as_ref().is_some_and(|p| p.is_same_node(Some(s))) {
            (0.0, view_h)
        } else {
            let r = s.get_bounding_client_rect();
            let top = r.top() as f32 + s.client_top() as f32;
            (top.max(0.0), (top + s.client_height() as f32).min(view_h))
        };
        let (lo, hi) = (band.0 + hh * 0.5, band.1 - hh * 0.5);
        let over = if ty < lo {
            ty - lo
        } else if ty > hi {
            ty - hi
        } else {
            0.0
        };
        if over != 0.0 && lo <= hi {
            ty -= scroll_by(s, f64::from(over)) as f32;
        }
    }
    // A point still off the window resolves to nothing: with the scroller
    // running past the window's edge (and stopped at its end), bring the
    // point in by the window, as the caret's reveal would have afterwards —
    // unless a scroll lock holds the page, or the editor is in a fixed box,
    // which the window does not move.
    let off = if ty < 0.0 {
        ty - hh
    } else if ty >= view_h {
        ty - view_h + hh
    } else {
        0.0
    };
    if off != 0.0 && !page_locked && !fixed {
        let before = window.scroll_y().unwrap_or(0.0);
        window.scroll_by_with_x_and_y(0.0, f64::from(off));
        ty -= (window.scroll_y().unwrap_or(0.0) - before) as f32;
    }
    let edge = || {
        let doc = handle.doc();
        if down {
            Selection::at_end(&doc).head()
        } else {
            Selection::at_start(&doc).head()
        }
    };
    let target = resolve_editor_point(doc, gx, ty)
        .filter(|hit| hit.container_nid == container_nid)
        .and_then(|hit| hit.pos(handle))
        .map(|p| {
            let (affinity, rect) = hit_affinity(handle, p, ty);
            (p, affinity, rect)
        })
        // On the document's last (first) line: Chrome goes to its end.
        .filter(|&(_, _, rect)| {
            let end = head_screen_rect(handle, edge());
            !matches!((rect, end), (Some((_, py, ph)), Some((_, ey, _)))
                if (py - ey).abs() < ph * 0.5)
        });
    let Some((p, affinity, _)) = target else {
        let motion = if down {
            CursorMotion::DocEnd
        } else {
            CursorMotion::DocStart
        };
        handle.move_cursor(motion, extend);
        return true;
    };
    let sel = if extend {
        Selection::text(handle.selection().anchor(), p)
    } else {
        Selection::cursor(p)
    };
    handle.set_selection_with_affinity(sel, affinity);
    true
}

/// Vertical caret movement: hit-test at the goal column one line above/below the head's
/// current screen position. Mirrors the desktop `vertical_step` with browser geometry.
pub(super) fn vertical_step(
    handle: &EditorHandle,
    container_nid: u64,
    doc: &web_sys::Document,
    down: bool,
    extend: bool,
) -> bool {
    let dir = if down { 1 } else { -1 };
    // Shift+Up/Down over a cell selection steps its head cell (#1847).
    if extend && matches!(handle.selection(), Selection::Cell(_)) {
        return handle.shift_arrow_cells(rinch_editor_core::Axis::Vert, dir, None);
    }
    let head = handle.selection().head();
    let Some((hx, hy, hh)) = head_screen_rect(handle, head) else {
        return false;
    };
    let gx = goal_x().unwrap_or(hx);
    set_goal_x(Some(gx));
    let ty = if down { hy + hh * 1.5 } else { hy - hh * 0.5 };

    // First, the geometry probe at the goal column. Accept it only if the caret
    // genuinely changed visual line: a move to a *different* textblock, or — within a
    // wrapped block — a caret whose new screen y actually crossed the half-line
    // threshold. Otherwise the probe snapped back to the current line (the target line
    // is an empty block with no text to hit-test, or a block atom is in the way).
    let head_tb = handle.caret_address(head).map(|(t, _)| t);
    // The hit and the side of a soft wrap it belongs to (#301): a goal column
    // past the target line's end lands on its wrap point, which is drawn on
    // that line only upstream.
    let geo_head = resolve_editor_point(doc, gx, ty)
        .filter(|hit| hit.container_nid == container_nid)
        .and_then(|hit| hit.pos(handle))
        .map(|p| (p, hit_affinity(handle, p, ty)))
        .filter(|&(p, (_, rect))| {
            if handle.caret_address(p).map(|(t, _)| t) != head_tb {
                return true; // different textblock — a real line change
            }
            match rect {
                Some((_, py, _)) if down => py > hy + hh * 0.5,
                Some((_, py, _)) => py < hy - hh * 0.5,
                None => false,
            }
        })
        .map(|(p, (affinity, _))| (p, affinity));

    // Stuck — step to the adjacent textblock in the model so the caret can still land
    // on a blank line above/below (or past a block atom). Mirrors the desktop fallback.
    // Leaving the caret's textblock never steps over a node an app draws with a
    // node view: it is node-selected and offered the caret, as on desktop.
    let leaves = match &geo_head {
        Some((p, _)) => handle.caret_address(*p).map(|(t, _)| t) != head_tb,
        None => true,
    };
    if !extend
        && leaves
        && let Some(sel) = handle.vertical_node_view(down)
    {
        let from = if down {
            rinch_editor_view::CaretFrom::Above
        } else {
            rinch_editor_view::CaretFrom::Below
        };
        handle.select_from_arrow(sel, from);
        return true;
    }
    let step = geo_head.or_else(|| {
        let target = handle.vertical_block_fallback(down)?.head();
        // Into a table from outside it: the cell under the goal column, not
        // the table's last (first) cell in document order (#1847).
        let entry = handle.vertical_table_entry(target, down, gx, |p| {
            head_screen_rect(handle, p).map(|(x, _, _)| x)
        });
        Some((entry.unwrap_or(target), CaretAffinity::Downstream))
    });
    // Shift+Up/Down at a cell's top or bottom line selects cells, as
    // prosemirror-tables' `shiftArrow` does (#1847).
    if extend && handle.shift_arrow_cells(rinch_editor_core::Axis::Vert, dir, step.map(|(p, _)| p))
    {
        return true;
    }
    let Some((new_head, affinity)) = step else {
        return false;
    };

    let sel = if extend {
        Selection::text(handle.selection().anchor(), new_head)
    } else {
        Selection::cursor(new_head)
    };
    handle.set_selection_with_affinity(sel, affinity);
    true
}

/// Which side of a soft wrap a caret placed at `pos` by a hit at viewport `y`
/// belongs to (#301), and its rect on that side: `Upstream` when the upstream
/// caret is **nearer** `y` than the downstream one — a hit past a wrapped
/// line's end, which lands on the wrap point — else `Downstream`. Anywhere but
/// a wrap point both sides are one caret and the answer is `Downstream`.
///
/// Nearness, not containment: a caret rect covers the glyphs, not the line
/// box, so a hit in the leading above or below them lies in neither rect.
/// Vertical motion probes 1.5 caret heights below the caret's top, which on a
/// face with a short caret in a tall line is exactly there.
pub(super) fn hit_affinity(
    handle: &EditorHandle,
    pos: Pos,
    y: f32,
) -> (CaretAffinity, Option<(f32, f32, f32)>) {
    let rect = |affinity| {
        handle
            .caret_rect_with_affinity(pos, affinity)
            .map(|r| (r.x, r.y, r.height))
    };
    // Nearness, not containment: a caret rect covers the glyphs, not the
    // line box, so a hit in the leading above or below them lies in neither.
    // Distance from `y` to the rect's vertical span, 0 inside it.
    let distance = |r: Option<(f32, f32, f32)>| {
        r.map_or(f32::INFINITY, |(_, ry, rh)| {
            (ry - y).max(y - (ry + rh)).max(0.0)
        })
    };
    let down = rect(CaretAffinity::Downstream);
    if distance(down) > 0.0 {
        let up = rect(CaretAffinity::Upstream);
        if distance(up) < distance(down) {
            return (CaretAffinity::Upstream, up);
        }
    }
    (CaretAffinity::Downstream, down)
}

/// Home / End: move the head to the edge of its visual line, collapsing or
/// (`extend`, Shift) extending. The textblock's edge without geometry — an empty
/// block, or a caret with no rect.
pub(super) fn move_to_line_edge(handle: &EditorHandle, end: bool, extend: bool) -> bool {
    let sel = handle.selection();
    let head = sel.head();
    let Some(edge) = visual_line_bound(handle, head, end) else {
        return handle.move_cursor(
            if end {
                CursorMotion::LineEnd
            } else {
                CursorMotion::LineStart
            },
            extend,
        );
    };
    // At a soft wrap the end of a line and the start of the next are one model
    // position. End's is the upper line's, so it is drawn there (upstream);
    // Home's is the lower line's (downstream, the default) (#301).
    let affinity = if end && is_wrap_below(handle, head, edge) {
        CaretAffinity::Upstream
    } else {
        CaretAffinity::Downstream
    };
    handle.set_selection_with_affinity(
        if extend {
            Selection::text(sel.anchor(), edge)
        } else {
            Selection::cursor(edge)
        },
        affinity,
    );
    true
}

/// Whether `edge`, the end of the visual line the caret at `head` is drawn on,
/// is a soft-wrap point: a downstream caret there is drawn on a later line.
fn is_wrap_below(handle: &EditorHandle, head: Pos, edge: Pos) -> bool {
    let (Some((_, hy, hh)), Some(down)) = (
        head_screen_rect(handle, head),
        handle.caret_rect_with_affinity(edge, CaretAffinity::Downstream),
    ) else {
        return false;
    };
    down.y > hy + hh * 0.5
}

/// The model position at the start (`end == false`) or end of the visual line
/// the caret at `head` is drawn on — the web twin of desktop's
/// `RinchApp::visual_line_bound`.
///
/// Found from caret geometry alone: a visual line holds a contiguous run of
/// logical positions, so the positions whose caret is drawn above, on and below
/// the caret's line are three consecutive runs, and a binary search over the
/// caret's hard line (the textblock, cut at its hard breaks — [`hard_line`])
/// finds the edge in a handful of `Range` rects. That holds for the rects only
/// as long as each says where its position is drawn: beside an image it reads
/// the image's own box ([`leaf_rect`]), and the downstream rect after a
/// soft-hyphen break is the glyph's, not the range's (#1115). End takes the last position
/// whose **upstream** caret is not below the line, so the end of a wrapped
/// line is the wrap point (the next line's start, drawn upstream); Home the
/// first whose **downstream** caret is not above it. Because nothing is hit
/// tested, a caret line scrolled out of the viewport or its scroller, or
/// covered by another box, answers the same as a visible one (#1026 — the
/// previous `caretRangeFromPoint` probes answered nothing off screen, and the
/// callers fell back to the textblock's edge; desktop's twin reads the line
/// from its text layout, #1107). What it answers are the line's *logical* edges: a right-to-left
/// line's start is at its right edge, and a line that mixes directions answers
/// its first and last logical positions wherever they are drawn.
///
/// `None` without geometry: no caret rect for `head` or for a position the
/// search asks about. The callers then fall back to the textblock's edge.
pub(super) fn visual_line_bound(handle: &EditorHandle, head: Pos, end: bool) -> Option<Pos> {
    use std::cmp::Ordering;
    let (_, hy, hh) = head_screen_rect(handle, head)?;
    let (lo, hi) = hard_line(handle, head)?;
    // Where the caret at `p` is drawn against the caret's own line: on it when
    // the two rects share more than half the shorter one's height (adjacent
    // lines at `line-height: 1` overlap by a sliver, a taller span on the same
    // line contains the caret), else above or below by their middles.
    let doc = handle.doc();
    let side = |p: usize, affinity: CaretAffinity| -> Option<Ordering> {
        let r = match leaf_rect(handle, &doc, Pos(p), affinity) {
            Some(r) => r,
            None => handle.caret_rect_with_affinity(Pos(p), affinity)?,
        };
        let overlap = (r.y + r.height).min(hy + hh) - r.y.max(hy);
        Some(if overlap > 0.5 * r.height.min(hh) {
            Ordering::Equal
        } else {
            (r.y + r.height * 0.5).total_cmp(&(hy + hh * 0.5))
        })
    };
    if end {
        // The last p in [head, hi] not drawn below the line; `head` itself is not.
        let (mut ok, mut bad) = (head.0, hi + 1);
        while bad - ok > 1 {
            let mid = ok + (bad - ok) / 2;
            if side(mid, CaretAffinity::Upstream)? == Ordering::Greater {
                bad = mid;
            } else {
                ok = mid;
            }
        }
        Some(Pos(ok))
    } else {
        // The first p in [lo, head] not drawn above the line; `head` itself is not.
        let (mut bad, mut ok) = (lo as isize - 1, head.0 as isize);
        while ok - bad > 1 {
            let mid = bad + (ok - bad) / 2;
            if side(mid as usize, CaretAffinity::Downstream)? == Ordering::Less {
                bad = mid;
            } else {
                ok = mid;
            }
        }
        Some(Pos(ok as usize))
    }
}

/// The box of the inline leaf — an image — on `affinity`'s side of `p`: the
/// one just before `p` upstream, just after it downstream. `None` when that
/// side holds text, a hard break (which [`hard_line`] already cuts at) or
/// nothing.
///
/// The view gives an inline leaf no bytes, so the caret rects of the positions
/// on its two sides are one rect — the text before it upstream, the character
/// after it downstream — and neither says which line the leaf itself is on.
/// Read from them, [`visual_line_bound`]'s End stepped past an image that
/// starts the next line and Home stopped before one that ends the previous
/// line. The leaf's own box answers that. Found as the nth `[data-pm-type]`
/// element under the textblock, which is the nth leaf child in the model:
/// marks wrap leaves without carrying the attribute.
fn leaf_rect(
    handle: &EditorHandle,
    doc: &rinch_editor_core::Node,
    p: Pos,
    affinity: CaretAffinity,
) -> Option<rinch_core::reactive::ElementBounds> {
    let r = doc.resolve(p).ok()?;
    if r.text_offset() > 0 {
        return None;
    }
    let parent = r.parent();
    let after = r.index(r.depth());
    let i = match affinity {
        CaretAffinity::Upstream => after.checked_sub(1)?,
        CaretAffinity::Downstream => after,
    };
    if i >= parent.child_count() {
        return None;
    }
    let leaf = parent.child(i);
    if leaf.is_text() || leaf.type_name() == "hard_break" {
        return None;
    }
    let nth = (0..i).filter(|&j| !parent.child(j).is_text()).count();
    let (tb, _) = handle.caret_address(p)?;
    let el = node_by_nid(tb)?.dyn_into::<web_sys::Element>().ok()?;
    let b = el
        .query_selector_all("[data-pm-type]")
        .ok()?
        .item(nth as u32)?
        .dyn_into::<web_sys::Element>()
        .ok()?
        .get_bounding_client_rect();
    Some(rinch_core::reactive::ElementBounds {
        x: b.x() as f32,
        y: b.y() as f32,
        width: b.width() as f32,
        height: b.height() as f32,
    })
}

/// The model range `(start, end)` of the hard line `head` is on: its
/// textblock's content, cut at the hard breaks on either side of it. A hard
/// break ends a visual line, and the view gives it no bytes, so the positions
/// on its two sides share one byte offset and one upstream caret rect —
/// [`visual_line_bound`]'s search must not be offered the far side. `None`
/// outside a textblock.
fn hard_line(handle: &EditorHandle, head: Pos) -> Option<(usize, usize)> {
    let doc = handle.doc();
    let r = doc.resolve(head).ok()?;
    let parent = r.parent();
    if !parent.is_textblock() {
        return None;
    }
    let start = r.start(r.depth());
    let off = r.parent_offset();
    let (mut lo, mut hi) = (0, parent.content_size());
    let mut at = 0;
    for i in 0..parent.child_count() {
        let child = parent.child(i);
        let size = child.node_size();
        if child.type_name() == "hard_break" {
            if at + size <= off {
                lo = at + size;
            } else if at >= off {
                hi = at;
                break;
            }
        }
        at += size;
    }
    Some((start + lo, start + hi))
}

/// The viewport `(x, y, height)` of the caret at model `pos`:
/// [`EditorHandle::caret_rect`], the same answer an app gets (a collapsed `Range`
/// at the host text node; an *empty* block's own box, so vertical motion off a
/// blank line still works).
fn head_screen_rect(handle: &EditorHandle, pos: Pos) -> Option<(f32, f32, f32)> {
    handle.caret_rect(pos).map(|r| (r.x, r.y, r.height))
}

/// Where a key-invoked menu opens, in viewport coordinates: at the caret, on its
/// line's middle. Without a caret rect — a selected horizontal rule has no text
/// position, and nor does Select All in a document ending with one — at the
/// selected node's outline, and failing that at the editor's own box, so a park
/// there still carries the selection the menu's Copy acts on.
pub(super) fn key_menu_point(handle: &EditorHandle, container_nid: u64) -> Option<(f32, f32)> {
    if let Some((x, y, h)) = head_screen_rect(handle, handle.selection().head()) {
        return Some((x, y + h / 2.0));
    }
    let editor = node_by_nid(container_nid)?
        .dyn_into::<web_sys::Element>()
        .ok()?;
    // Only a live node selection has an outline to park at. The overlay stays
    // in the DOM (hidden with `visibility`, which keeps its box) once the
    // selection moves on, so its client rect alone cannot say it is live.
    let outline = matches!(handle.selection(), Selection::Node(_))
        .then(|| editor.query_selector("[data-pm-selected]").ok().flatten())
        .flatten()
        .map(|el| el.get_bounding_client_rect())
        .filter(|r| r.width() > 0.0 && r.height() > 0.0);
    Some(match outline {
        Some(r) => (r.x() as f32, (r.y() + r.height() / 2.0) as f32),
        None => {
            let r = editor.get_bounding_client_rect();
            (r.x() as f32, r.y() as f32 + PARK_SIZE / 2.0)
        }
    })
}

/// The app handler that claims the context menu the menu key opens in the focused
/// editor, if any: the lookup a right-click makes, from the caret's block (or the
/// editor itself when the caret has none).
pub(super) fn key_menu_app_handler(
    handle: &EditorHandle,
    container_nid: u64,
) -> Option<(web_sys::Element, rinch_core::events::EventHandlerId)> {
    handle
        .caret_address(handle.selection().head())
        .and_then(|(block, _)| node_by_nid(block))
        .or_else(|| node_by_nid(container_nid))
        .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| nearest_handler(&el, "data-oncontextmenu"))
        .filter(|(_, id)| rinch_core::events::has_click_handler(*id))
}
