# Absolute/fixed positioning and static position

Read this page when your task touches: containing blocks, out_of_flow, static position, shrink-to-fit absolute boxes.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

**Absolute positioning.** Taffy resolves an out-of-flow box against its **direct
parent**, always. CSS resolves an absolute box against the **padding box of the
nearest ancestor that establishes a containing block**
(`Node::establishes_abs_containing_block`: positioned, or transformed) — or,
when it has none, against the initial containing block. rinch corrects both
(`crates/rinch-dom/src/out_of_flow.rs`, whose module doc has the detail):

- **No such ancestor** (#204, `OutOfFlowKind::IcbAbsolute`): the viewport.
  `inset: 0` inside an unpositioned 300x200 div gives an 800x600 box.
- **An ancestor that is not the box Taffy lays it out in** (#386,
  `OutOfFlowKind::AncestorAbsolute(cb)`): that ancestor's padding box.
  `relative 400x300 > static 300x200 > abs(inset: 0)` gives 400x300 at the
  grandparent's origin, where it gave 300x200 in the parent. The layout parent
  is the nearest ancestor that generates a box — a `display: contents` wrapper
  and a non-atomic inline element are stepped over — so `relative > contents >
  abs` is still Taffy's own answer and costs nothing.

Both match the browser and therefore `rinch-web`. The correction is a pre-layout
**size** bake into the Taffy style (so the box's own children lay out inside the
right box) plus a **position** written as a *parent-relative* value
(`out_of_flow::place_absolute`), so `LayoutResult` keeps its meaning and no
coordinate consumer — paint, stacking, hit testing, `ClickContext`, the MCP
`absolute` contract — needs an exception. An axis with both insets `auto` keeps
the box's static position (below).

**An ancestor's size is not known before the compute**, so that case is a
fixpoint in `resolve_layout`, sharing `calc_layout`'s loop and its cap of 8:
`out_of_flow::bake_at_style_site` bakes from the ancestor's **last** laid-out
size when a style is (re)built, `RinchDocument::resolve_ancestor_absolutes`
compares every such box after each root compute and rewrites the ones whose
ancestor came out at another size, and the compute re-runs while it rewrote any
(`abs_containing_block_passes`). An out-of-flow box sizes nothing above it, so
this converges in **one** extra compute — plus one per level when a containing
block is itself an absolute resolved against a non-parent. What it costs in
**computes**, pinned in `perf_regression_scenarios`: a box whose size does not
depend on its containing block (`right: 5px; bottom: 5px; width: 40px`) is
placed and never re-sized, so it never pays one; a size-dependent one
(`inset: 0`, `width: 50%`) pays one extra compute on the document's first
layout and one per resize of its containing block — not on a later pass, a
restyle of the box, its insertion under a block that already has a size, or an
inline inset write (the #280 fast path bakes the size for the new insets
itself, `out_of_flow::rebake_after_inset_write`).

**The passes look only at the boxes that need them** (review of #1409; they
iterated every `position: absolute` node at first, which cost a list of
ordinary badges +12% per relayout). An absolute child of its own positioned
parent — Taffy's own answer — is in no index and no pass on a scroll, and on
a layout in none unless its width is `auto`/`fit-content` with no inline inset
(a `top: 100%` dropdown) or it sits in a grid container with an inset or margin
that takes room: those are in `NodeTree::abs_static_fits` and visited once per
layout after the lines (#1404; one `abs_boxes_visited` each, no compute when
nothing moved). `NodeTree::ancestor_absolutes` holds the ancestor-resolved boxes (kept
by the style sites; `read_layout_results` classifies every absolute box anyway,
and one it finds the sites missed — a box *between* it and its containing block
stopped being `display: contents` with no restyle of the absolute one — is
added and the layout resolved again in the same pass,
`out_of_flow::note_kind_at_read`); the size check after each compute iterates
that. `NodeTree::placed_absolutes` holds what the read-back placed this layout
(ancestor and ICB boxes); the second placement and a scroll iterate that. The
counter is `abs_boxes_visited`, one per box per pass: 0 for a document of
direct-child badges, 2 per layout per ancestor-resolved box (size check,
placement), 1 per ICB box. Measured (Callgrind, 500 rows with one badge each,
one leaf restyled and laid out, against `main`, which did not resolve these
boxes at all): direct-child badges 1.78M → 1.79M instructions, ICB badges
2.01M → 2.02M, ancestor-resolved badges 2.31M → 2.69–2.75M — about 0.8k
instructions per ancestor-resolved box per layout, which is the size compare
and the placement.

**A change of containing block owes a layout whether or not a Taffy style
changes.** A node that starts or stops establishing a containing block — in the
cascade, or on the `tick_transitions` frame a `transform` transition finishes
at `none` — re-syncs the absolute descendants that resolve against it (below;
the walk stops below a nearer containing block, whose boxes the flip cannot
reach, so a hover `transform` on a card of `position: relative` rows runs no
compute) *and*, when there is one, sets `layout_dirty`: a box whose size
does not depend on its containing block (`right: 5px; width: 40px`) has the
same Taffy style under either, and only the read-back places it. Without that
it stayed where the old containing block put it until something else laid out
(`abs_containing_block_tests::a_position_only_box_follows_a_change_of_containing_block`;
the initial-containing-block half was already so before #386).

**Every site that rebuilds a Taffy style from `computed_style` calls
`bake_at_style_site`** — the cascade's sync and both tick re-syncs — which is
also what stops `tick_transitions`/`tick_animations` from dropping a `fixed`
box's viewport size on a transition frame. A new rebuild site that skips it
costs an ancestor case one compute (the pass after the compute puts the bake
back) and loses the viewport cases until the next cascade.

**The containing block's rules, for both absolute cases** (Chrome 153's numbers
are in `crates/rinch-dom/tests/abs_containing_block_tests.rs`): insets and a
percentage `width`/`height`/`min-*`/`max-*` resolve against the padding box, a
percentage `padding` or `margin` against its **width** on every side; margins
come out of the space between two insets; `margin: auto` between two insets
centres a sized box (one `auto` takes all the free space; a box larger than the
space still centres on the block axis and starts at the inset on the inline
one); a mixed `calc()` in any of those resolves against the same box
(`calc_layout`) — except for a box inside an atomic inline, where no `calc()`
is resolved at all, in flow or out of it (#1412, older than #386: a `width: calc(50%
+ 10px)` child of a 200px `inline-block` is 10px wide; the plain-percentage
rules above do hold there). That holds for a box whose containing block is
its own parent too, however the parent is sized: `top: calc(100% + 4px)` under
an auto-height `position: relative` block, a content-sized flex or grid
container, or a grid item resolves its `100%` against the parent's laid-out
padding box. `calc_layout` judged that basis by the in-flow rule
(`calc_axis_definite`: an auto height is indefinite, so the percentage is 0)
until #1789/#1974, and such a dropdown came out at `top: 4px`, over its own
trigger, while a plain `top: 100%` (Taffy's) was right; an absolute box sizes
nothing above it, so its basis is definite whatever sizes the block
(`tests/abs_calc_inset_auto_cb_1789_tests.rs`, Chrome 155's numbers). A real
0x0 box is placed; one in a `display: none`
subtree is not (`out_of_flow::is_laid_out`).

**Scrolling.** A scroller *between* a box and its containing block does not
carry the box — it is the containing block's content, not the scroller's — on
an inset axis and a static-position one alike (Chrome: `top: 6px` and a
static box both stay put at `scrollTop = 40`). The containing block's **own**
scroll does carry it, and for the initial containing block the `<body>`'s
scroll is that scroll (rinch's page scroll). Since `layout` is parent-relative,
the position carries the scroll offsets of the boxes between
(`+ Σ chain.scroll`), and a scroll runs no layout — so `NodeTree::mark_scrolled`
calls `out_of_flow::replace_after_scroll`, which writes those boxes again.
It costs one flag read unless the scrolled box lies between a placed box and
its containing block (`Node::on_abs_chain`, set as the read-back walks each
chain): a scroller of rows whose badges resolve against their own row, direct
child or not, re-places nothing, and a scroller holding 500 ICB boxes re-places
all 500 (about 0.6k instructions each, 0.3M a notch — each really has to be
written again to stay still). It drops no hit-test extent: an absolute box is
positioned, so it is in no `flow_extent`, and the stacking sequences that hold
its offset are the ones a scroll drops anyway
(`perf_regression_tests::a_second_wheel_notch_with_an_escaping_absolute_recomputes_no_extent`).
The re-placed box is pushed paint-dirty, so a notch's damage includes a box
that did not move on screen. **A new scroll-offset writer that
bypasses `mark_scrolled` lets such a box ride the scroller until the next
layout.** Before #386 an ICB box rode every scroller it was written in, and
jumped back to the viewport on the next layout while the page was scrolled.
`resolve_layout` also places every placed box a second time
(`out_of_flow::replace_all`) when something on the way to a containing block
was written after the read-back: an anonymous block box's own read-back, the
line position an inline formatting context gives an `inline-block`, a scroll
offset the clamp pulled in. Each of those three writers sets
`NodeTree::abs_late_moves` when it actually moved something, and a layout in
which none did skips the second placement. The `!layout_dirty` text-only path
runs the second of them with no read-back at all — a `text-align` change moves
an `inline-block` along its line there — so it calls `replace_all` too, after
its `build_ifc_layouts`; `replace_all` consumes the flag
(`review2_1409_tests::c1_*`, `c3_*`); **a new writer of a box's position
or scroll offset that runs after `read_layout_results` must set it too.**

**The static position** (#633, #632, #634; CSS 2.1 §10.3.7, §10.6.4). An axis
with both insets `auto` keeps the box where it would have been in the flow —
for `absolute` and for `fixed` (which went to the viewport's origin until #633).
Taffy's answer, after the preceding in-flow sibling in the layout parent, is
that place except among **inline content**, which Taffy sees as one leaf. There
the lines answer (Chrome 153, pinned with the bundled Inter in
`tests/static_position_tests.rs`): a **block-level** box goes at the content
box's left edge (whatever the `text-align`) below the line holding the content
before it, or at the first line's top when nothing precedes it; a box that was
**inline-level before `position` blockified it** — a `<span style="position:
absolute">`, an `inline-block` one — sits in its line where that content ends,
at the line box's top, and moves with `text-align`. At a soft wrap the box stays
on the line the content before it is on (a space hung there is not counted);
after a forced break it starts the next line. Its own margins are added.
The mechanism: `walk_inline_children` notes each out-of-flow box it passes
(`IfcText::note_out_of_flow`; a direct child standing inside an anonymous box's
run — or before its first member, with no block between: an inline-level box
there starts the first line, which `text-align` moves — joins no run, #406, so
it is noted from `Node::run_out_of_flow`, recorded where the runs are
grouped); `ifc::resolve_out_of_flow_marks` turns each note
into both positions once the lines are broken (`InlineLayout::out_of_flow`; a
`text-overflow: ellipsis` cut keeps the lines, #1100); `Node::static_ifc_root` names the root and
the note's index on the box, so `out_of_flow::static_location` costs one flag
read for a box outside inline content and one indexed look for one inside
(`abs_static_lookup_steps`: N looks for N boxes in one paragraph, pinned at two
sizes by `perf_regression_scenarios::static_boxes_in_one_paragraph_are_looked_up_by_index`
— a search per box was quadratic, 8000 boxes 1.9 → 10.4 ms per relayout); and which position is the box's is
`ComputedStyle::inline_level_before_blockify` (Stylo's `original_display`),
read at placement — a change of it alone moves no Taffy value, and neither
does `absolute` ↔ `fixed`, so the cascade sets `layout_dirty` for either on an
out-of-flow box. The lines are built after the read-back, so
`shape_ifc_root_paint_layout` calls `out_of_flow::place_static_after_lines` for
each box its new lines hold, and so does `read_layout_results_for_box` when an anonymous box moved: that
writes an absolute box whose layout parent is its containing block directly —
it is in no list — and sets `abs_late_moves` for the rest. **A new writer of an
IFC root's position that runs after the read-back owes the same call.** The
position is measured with nothing scrolled (a box shown in a scroller already
at `scrollTop = 40` sits where it would at 0): for a fixed box, whose `layout`
is a viewport position, that is the sum of the layouts above it
(`out_of_flow::place_fixed_static`; the sum ends at a fixed ancestor and takes
the baked scroll back off an absolute one), and no scroll moves it afterwards. A
fixed box with a static axis is pushed to `placed_absolutes` (one
`abs_boxes_visited` per layout); one with insets on both axes, and an absolute
box placed by Taffy, static or not, are in no pass. In a **flex** container
the static position is the box as its sole flex item: `justify-content` on the
main axis and `align-self`, else `align-items`, on the cross (`stretch` as the
start; #1492 — `to_taffy_style` used to force `align-self: flex-start` on every
out-of-flow box, so the cross axis ignored `align-items`; pins:
`static_position_flex_1492_tests.rs`, Chrome 153). In a **grid** container
that is its containing block, Taffy aligns the box in the container's padding
box by its own `justify-self`/`align-self`, else the container's
`justify-items`/`align-items` (#1511 carried the `justify-*` pair, which used to
be dropped; pins: `justify_items_1511_tests.rs`, Chrome 156). Under an
unpositioned grid Chrome aligns it in the grid's content box and rinch in its
padding box (#1986). Not Chrome's, each pinned in
`known_differences_from_chrome`: the static position
follows rinch's line boxes, so below a line holding a 26px `inline-block` and
text it is 4px high (rinch's line is 26px, Chrome's 30 — #663).

**A positioned inline span is a containing block too** (#631, CSS 2.1 §10.1):
`out_of_flow_kind` answers `AncestorAbsolute(span)` for a `position: relative`
or `sticky` non-atomic inline, which has no box at all, and the block is read
out of the lines of the inline formatting context that flows the span: from
the top-left of its first fragment to the bottom-right of its last, zero wide
when that is left of the first (a wrapped span). A fragment's left and right
are the **visual** extent of its content (a span of right-to-left text is as
wide as its text), its top and bottom the span's **own font's** rounded ascent
and descent around the baseline whatever its children are set in (the face its
stack resolves an `x` to, `out_of_flow::font_box`), moved by the span's **own**
`vertical-align` shift (the nearest one among the span and the inline elements
around it, as its text is drawn — a shifted child at its start or end moves
nothing); an empty span is a zero-width fragment where it sits
(Chrome 153, pinned with the bundled Inter in
`tests/abs_inline_containing_block_tests.rs` and `review_1434_tests.rs`).
**The spans of one context are measured together, once per set of lines**
(`RinchDocument::measure_inline_containing_blocks` →
`out_of_flow::measure_span_fragments`): one walk of the lines' clusters and one
of the context's entries, kept beside the lines in
`InlineLayout::span_fragments`, so each look at a box is a lookup and rebuilt
lines (which come back with none) are measured again. It was one walk of the
whole context per box per pass, quadratic in the boxes of one paragraph (review
of #1434: a relayout of 800 badges in one paragraph took 1.36 s). The counters
are `abs_inline_measures` and `abs_inline_measure_steps`, pinned at two sizes
by `perf_regression_scenarios::spans_in_one_paragraph_are_measured_in_one_linear_walk`
(854 steps for 50 badges, 1,708 for 100; spans nested in one another each
walk their own clusters, so that case grows with the nesting depth). The chain of such a box ends at the
block container **element** holding the line (`out_of_flow::inline_host` — an
anonymous block box is stepped through), so that element's scroll carries the
box and a scroller between the two does not. The position is snapped to the
pixel grid in that element's frame, since a fragment starts between pixels (a
host that is itself an `inline-block` stands between pixels on its own line,
and the box with it). **Lines are built after the read-back**, so both halves
run late, and only when the read-back met such a box
(`NodeTree::abs_inline_cb_seen`): after `build_ifc_layouts` the spans are
measured, `replace_all` places the boxes (on the text-only path too, where a
`text-align` change moves the span), and `resolve_ancestor_absolutes(true)`
compares each bake with the fragment the new lines give — a rewrite sends
`resolve_layout` round again (compute, read-back, lines), so a box **sized**
from a fragment whose size changed costs one more compute in that layout (the
first one, a text edit inside the span) and a position-only one costs none.
Not Chrome's, each pinned: a **split** inline (one holding a block-level child,
#513) is not measured and keeps the block container (**#1424**) (a span in a
block that draws a `text-overflow: ellipsis` "…" is measured since #1100, which
keeps the lines and cuts them at paint); a relative span's own `left`/`top` moves neither its
text nor the box (**#1425**); a
raised span's fragment (`<sup>`, any positive `vertical-align`) follows glyphs
rinch draws higher than Chrome, whose line grows for them (#1357: a `<sup>` 5px
higher, `vertical-align: 10px` 10px, and a raised span that wraps is shorter
by the growth of each line); a span in a larger font with **no text of its
own** (empty, or holding only an atomic inline or smaller text) is the right
size and sits 5px high, because rinch's line gets no strut from it (**#1463**);
a wrapped span in a font taller than its `line-height` allows spans lines
rinch keeps at the line height (three 22px lines of `16px/20px`: 66 tall
against Chrome's 70); a wrapped **right-to-left** span's right edge is 4.5px
off Chrome's, since parley puts a right-to-left line's trailing space at the
line's left end where Chrome hangs it out of the line, and an empty span at
the very end of a right-to-left run is 5px right of Chrome's; a span that ends in a `<br>` has no empty last fragment
on the next line; a line of nothing but atomic inlines has no strut (#624,
#1258), so a span on one hangs from the boxes' bottom edge. And **paint does
not clip such a box by a static scroller around its span** (**#1438**, older
than #631: `stacking::Collector` descends the box tree, never meets the span,
and ends the box's clip chain at the next positioned box), so a badge whose
line is scrolled out of a static `overflow: auto` div is drawn outside it;
layout, the scroll range and damage do see the span.

**An auto-width absolute box is shrunk to fit in what its containing block
leaves it** (#1404, CSS 2.1 §10.3.7): that block's padding-box width less the
insets and margins, and with both inline insets `auto` the room its static
position leaves — not its Taffy parent's whole width. Where the static position
is (`out_of_flow::StaticAnchor`, on the node as `Node::abs_static_anchor`): a
box among block or inline content or in a grid starts there and may run to the
block's end; in a flex container it is placed as the sole item, so an
end-aligned box gets the room from the block's start to the container's
content end, and a centred one twice the shorter distance from the content
box's centre to a block edge (Chrome 155; the inline-axis alignment is
`justify-content` in a row, `align-self`/`align-items` in a column — so
`Stack align="center"` holding a tooltip-like box). Three routes, all to
`fit-content`: a box whose layout parent is its containing block gets the
`fit-content` **keyword** (`out_of_flow::fit_in_layout_parent`), which Taffy's
block and flex algorithms measure at the block less the insets and margins —
only when an inset or margin takes room, so a plain box keeps `auto` and one
measure; an ancestor-resolved or ICB box is baked `fit-content(<px>)` with the
room (`apply_out_of_flow_size_overrides`); and the anchor, which Taffy does not
know before the compute, is measured after the lines are built
(`RinchDocument::resolve_static_shrink_to_fit`, over `NodeTree::abs_static_fits`)
and read by every bake; a grid parent (whose keyword measure ignores insets)
and an anchor away from the edge of a box's own parent take a length from the
parent's last layout, checked by the same pass. **Cost, in root computes**
(pinned by `what_shrink_to_fit_costs_in_root_computes`): a box whose size does
not depend on its containing block's, or one with a single inset in its own
parent, pays nothing; one shrunk to fit from a static anchor away from its
block's edge pays **one** more on its first layout and on each resize of the
block it is measured against; an ancestor-resolved box (#386) of any auto
width — newly including one with a single inset — pays one more on its first
layout and one per resize of its containing block, as `inset: 0` always has,
and **two** more on its first layout with a static anchor too; none pays
anything on a relayout that moves nothing. An ICB box re-bakes (and is
re-measured) on every viewport resize, with no extra compute. Taffy's leaf
algorithm took an absolute leaf's own margins off a second time;
`out_of_flow::absolute_leaf_inputs` puts them back at both leaf measure sites.
An `auto` atomic inline capped by #1476 answers an IFC measure at its capped
width **rounded**, as its layout will be (`round(width)`) — the answer it gives
once laid out there — so a container sized from it does not move by the
rounding between an incremental and a fresh layout, nor come out a pixel
narrower than the box it holds. Pins:
`tests/abs_shrink_to_fit_1404_tests.rs` (74 Chrome 155 rows) and
`review_1510_1010_fixtures.rs`. Not Chrome's: content that wraps answers its
widest line (**#1276**: the issue's own example is 390x40, was 130x80, Chrome
394x40), and the static position in a grid container that is not the
containing block (its content edge in Chrome).
**Not covered:** a box whose containing block is a non-parent **scroll
container** is placed in it but still counted in no scroll range (**#770**);
`position: fixed` takes none of the margin and padding rules above (its
margins take nothing out of its room and are not added to its position),
keeps the Taffy parent as its `calc()` basis (**#1980**), and is not contained
by a transformed ancestor (**#1372**) — the `AncestorAbsolute` machinery is
what that would use.

**A fixed box is shrunk to fit too** (#893): the same bake, against the
viewport (`ContainingBox::viewport`). An `auto` or `fit-content` width with
fewer than two horizontal insets is `fit-content(<px>)` of the viewport less
the insets — with both `auto`, of the room its static position leaves, a
`StaticAnchor` measured from the unscrolled parent origin that
`place_fixed_static` uses, so the read-back pushes such a box to
`abs_static_fits` beside the absolute ones. An `auto` height with fewer than
two vertical insets is left to Taffy, i.e. the content's; a percentage
`min-`/`max-` size is baked against the viewport. It used to fill the viewport
on both axes, `max-width` the only cap (a tooltip was drawn at its
`max-width`), and the read-back replaced a one-inset height by a sum of the
children's heights (`compute_content_height`, removed), which dropped text
lines, `aspect-ratio` and the replaced measure (#1569). Pins:
`tests/fixed_shrink_to_fit_893_tests.rs` (Chrome 156),
`aspect_ratio_1286_tests::a_fixed_box_takes_the_ratio`. **A component whose overlay must cover its parent (e.g.
`LoadingOverlay`) needs that parent to declare `position: relative`** — without
it the overlay covers whatever positioned ancestor it does have, or the
window, as it always has on the web.
