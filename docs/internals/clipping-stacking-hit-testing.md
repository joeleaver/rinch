# Overflow clipping, stacking contexts, hit testing

Read this page when your task touches: clip predicate/shape, clip chains, stacking order, transforms, hit cache, scroll offsets.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

**A `0x0` box paints its descendants** (#1696): `paint_node` walks a box zero in both dimensions as it walks one zero in one (#142) — its origin, transform, opacity and stacking context are kept, its own inline content (text, atomic inlines such as an unpositioned `inline-block` marker) and its children are drawn, its own `overflow` clip is pushed, and only its background, border, shadow and outline are not drawn (nor is its `filter` applied to what overflows it, #1701). It used to return without visiting the children, so a `0x0` stacking context (the `Editor` overlay slot) drew none of its descendants and a `0x0` box's in-flow content was never drawn; only a positioned descendant of a non-stacking `0x0` box, hoisted to an outer sequence, was. A `0x0` box that clips on both axes clips to nothing, so its subtree is not walked (`zero_clip_hides_subtree`) unless it is a stacking context holding a `position: fixed` descendant, or an absolute one it does not contain. `display: none` is decided by `display`, not size; an `<svg>` or `<img>` is not walked (`zero_box_paints_children`). Pins: `rinch-dom/tests/zero_box_descendants_1696_tests.rs`, `review_1699_probes.rs` (Chrome 155 ink counts), `rinch/src/app/zero_box_overlay_1696_tests.rs`.

**Overflow clipping.** One predicate — `Node::clips_overflow()`, "either axis is
not `visible`, and the box is neither a non-atomic `display: inline` element nor
`display: contents`" (which generates no box to clip to, #1038) — and
one shape, `paint::clip_shape` (the rounded **padding** box, #536 — CSS clips
overflowing content to the padding edge, not the border box; `paint::
padding_box_insets`/`padding_box_radii` are the one place the four border
widths are measured and the radii reduced for it, and every cross-crate
consumer below asks one of those three functions rather than re-deriving the
inset).
Everything that needs either asks those: paint's clip bracket, its dirty-region
subtree prune, the layer-bounds walk, a hoisted entry's clip chain, hit
testing's `check_children` gate, and `RinchApp`'s two viewport clip walks.
(`creates_stacking_context` was on that list until stage B and is deliberately
not any more — see **Stacking contexts and the clip chain** below.) Those seven
sites held **four** different predicates before #324 stage A, and the
disagreement was real rather than cosmetic: paint, `layer_bounds` and
`creates_stacking_context` matched `overflow_y` against `Hidden | Scroll | Auto`
and so **missed `overflow: clip` entirely**, while hit testing clipped it —
content drawn and not clickable.

**But "this node clips, therefore a bracket is open" is no longer true** (card
K43). A Vello clip layer is two extra passes over the clipped area, paid whether
or not it removes a pixel, so `paint_node` may *decline* to push one for a box
that answers `clips_overflow` — when the clip covers the render target, or when
`layer_bounds::clip_cuts_nothing` says nothing inside reaches past it. Both cases
require **square corners**: a rounded clip cuts the corners of its own box, so a
subtree fitting the box does not mean the shape cuts nothing.

"Reaches past it" counts an inline element's background, not only the line boxes: an
IFC root's extent includes the rectangles `paint_inline_backgrounds` fills for
an inline element's background (`paint::text::inline_background_rects`, one
function for both), which reach past the line by the element's padding (#1762).
Before that, a clip was elided whenever the text alone fitted, so in a column of
identical `overflow: hidden` cells a padded pill was clipped only in the cells
whose height rounded below the line. It also counts what the root draws past
its unshifted line boxes otherwise (#1817): text `vertical-align` moves off its
line (#724; a shifted line is measured as its whole line box moved by the
shift), and a wavy underline's wave. All of it comes from one function,
`paint::text::inline_ink_past_lines`, which the damage ink reads too
(paint-and-damage.md, #1815). A shifted element's background and border move
with its text since #1817 (`InlineLayout::element_shift`: the shift of the
nearest element from the owner up whose `vertical-align` moved it, read from
`InlineVerticalAlignSpan::owner`); they stayed on the line before. Those rectangles are scaled by the device
scale like the glyphs since #1762 (they were drawn in CSS px). Glyph ink past
the line box (an italic's overhang) is still not measured.

This weakens no answer above, and consumers must not assume it does — the
predicate and the shape are unchanged, and everything reasoning about *where
content ends up* (the clip chain, hit testing, `layer_bounds`' own intersection)
still applies the clip, because the elision only ever drops a restriction that
was already vacuous. What it breaks is the **reverse inference**, which is why
`paint_children_with_stacking` is *told* whether a bracket is open
(`clip_elided`) rather than deducing it. Anything new that needs to know must be
told too, not infer it from the predicate.

**The clips the painter has open are a cull** (#910). `paint_document` and
`paint_subtree` wrap the painter in `ClipTrackingPainter`, which mirrors every
`push_clip` / `push_layer` / `pop_layer` onto a stack of cull rects (each
clip's transformed bounding box grown by `INK_MARGIN_CSS_PX`), and
`intersects_dirty_region` tests against its top as well as the damage and the
window. Mirroring the painter rather than the call sites is what keeps it
exact: a hoisted entry's clip chain, the #545 lift around a `position: fixed`
entry and an elided bracket are whatever the painter was told. A box the cull
would draw nothing for is dismissed by its parent's loop
(`paints_nothing_without_visit`) without entering `paint_node`, so a 500-row
scroller costs its ~20 visible rows. Since #1719 that question is bounded by a
count of nodes (`PAINTS_NOTHING_BUDGET`, 128 per question), not a depth of 3,
which a `Tree` row (`li > content > icon > svg > path`) and every open folder
around it exceeded; it looks through a `display: contents` wrapper (rsx's
`for`/`if`/reactive text) instead of answering "visit"; a box with no area
and nothing to walk answers "paints nothing", as `paint_node` returns for one;
and a **stacking-context** entry outside the cull is answered by the walk
`paint_node` would make (`subtree_is_entirely_outside`), so rows that are
stacking contexts, and `z-index` children of hoisted `position: relative`
rows, are culled too. A box with nothing in its box tree builds no paint
sequence. Pins: `rinch-dom/tests/positioned_row_cull_1719_tests.rs` (pixels
against an oracle, scrolled four ways, full and partial repaints),
`perf_regression_scenarios::*rows_paint_the_rows_on_screen`,
`perf_regression_tests::a_scrolled_open_tree_paints_the_rows_on_screen` (the
real `Tree`, 363 open rows: 604 → 172 visits). The ink margin is the window cull's
policy, with its known loss: ink cast more than 64 CSS px from a box that sits
outside a clip is not drawn back into it.

`clip` is the only value that ever reached that gap, and that is measured rather
than reasoned: css-overflow-3 §3 makes a `visible` compute to `auto` when the
other axis is neither `visible` nor `clip`, Stylo's style adjuster implements
it, so `overflow-x: hidden; overflow-y: visible` is **unreachable** — pinned in
`crates/rinch-dom/tests/clip_predicate_tests.rs`, which is also what fails if a
Stylo bump ever changes that. `clip` beside `visible` is the pair the spec
allows, so it stays asymmetric.

One deviation from CSS remains **in the predicate and the shape** — this is not
an inventory of everything rinch gets wrong about `overflow`, which also covers
scrolling, `text-overflow` and scrollbars. rinch clips both axes with one rect,
so `overflow-x: clip; overflow-y: visible` clips vertically too (#535).

**#536 is fixed: the rect is the padding box**, not the border box. A clipping
container with a non-zero `border-width` used to let its content paint over its
own border — border paints *before* the clip bracket opens
(`paint_borders`, then `push_clip`, then children), so nothing else caught it.
With no border the two boxes coincide, which is every clipping box in the
component library, so this was correctness debt rather than a live visual bug
until something actually set a border. `paint::padding_box_insets(node)`
returns `(left, top, right, bottom)` in logical px, clamped so a border wider
than the box cannot invert the rect; `paint::padding_box_radii(node, scale)` is
`border_radii` with each corner's horizontal radius reduced by the left or
right border width and its vertical radius by the top or bottom one
(css-backgrounds-3 §5.5; it took the *average* of the two widths off one
circular radius until #977). **The radii are a `paint::radii::BorderRadii`**
(#977): each corner a `(horizontal, vertical)` pair, a percentage resolved per
axis (width for the horizontal radius, height for the vertical), a corner
square when either radius is zero, and all eight scaled by CSS's overlap
factor `min(Lᵢ / Sᵢ)` when the radii along a side add up to more than it
(`border-radius: 100px 0 0 0` on 200x60 is a 60px corner; kurbo's per-corner
clamp to half the shorter side drew 30). `ComputedStyle` carries the vertical
radii as `border_radius_*_y` beside the horizontal `border_radius_*`, and a
`border-radius` transition moves both (`AnimatableValue::CornerRadius`).
`BorderRadii::shape(rect)` is what every rounded box is drawn and clipped
with — a kurbo `RoundedRect` when the corners are circular and fit in half the
shorter side (the same shape every correct box had before; a solid rounded
border's pixels still move, see below), a `BezPath` of elliptical arcs
otherwise. Background, clip (and a hoisted entry's clip chain), filter
overlays, the uniform and per-side borders, the outline and the outer and
inset shadows use it; a solid rounded uniform border is now filled as the ring
between the border-box and padding-box shapes (it was a centre-line stroke
with the outer radius, half a width too round). Still circular: the
compositor's viewport-hole mask (`viewport_rect_with_radius`, each corner its
smaller radius), an inline element's background (`border-top-left-radius`'s
horizontal radius only), and hit testing, which ignores radii altogether
(#1639). Every consumer of the shape
agrees: `clip_shape` itself, `layer_bounds`' children-clip intersection and the
`covers_target`/`clip_cuts_nothing` elision check in `paint_node` (both now
read `clip`'s own rect rather than the node's plain border-box `rect`), hit
testing's `check_children` gate (reads `padding_box_insets` directly, in
`crates/rinch/src/app/hit_testing.rs` — a click on a clipping container's own
border strip resolves to the container, never to content that paint no longer
draws there), `RinchApp::viewport_rect_with_radius`/`viewport_clip_rect` (the
`GameViewport`/video compositor hole, inset and its radii reduced the same
way), and `paint::clip_chain_bounds` (#909's damage clip chain, below —
`padding_box_insets_for_size` is the one difference from every other caller:
a damage-chain ancestor can be asked about its *painted* (`Frame::Painted`,
`prev_layout`) size rather than its current `node.layout`, and the border
widths — read from current computed style either way, since nothing records
a painted one — have to clamp against whichever size is actually in use).

A third one is gone: **clipping no longer forms a stacking context** — see
**Stacking contexts and the clip chain** below.

**A non-atomic `display: inline` element never clips** (#591 PR 1), whatever its
`overflow` computes to — `overflow` applies to block, flex and grid containers
(css-overflow-3 §3), and an inline *box* is none of those; an `inline-block` is a
block container and still clips. So does a **replaced element or form control at
`display: inline`** (`<img>`, `<svg>`, `<input>`, `<button>`, …): it is an atomic
inline whatever its `display` says, as in Chrome, and its `display_mode` is
`InlineBlock` (`node::is_atomic_at_display_inline`, #1089 — it used to be a flowed
inline with a `0x0` box). The same rule covers `video`, `canvas`, `iframe`, `meter`
and `progress`, which the UA sheet leaves `display: inline`, so their **default**
rendering changed too: an atomic box. `canvas`, `video` and `iframe` take Chrome's
**300x150** default object size (a canvas's own `width`/`height` attributes, with
their aspect ratio), are not stretched by a block container (Taffy's
`item_is_replaced`), and lay out no fallback content (`replaced.rs`, #1173; a grid item is still
stretched, #1280). Their element children — and an `<input>`'s or `<textarea>`'s —
are `display: none !important` by the UA sheet, and the element is always a hollow
IFC root, so no child is laid out, painted or hit and none takes the element's
measure away (#1178, #1288); its own `flex`/`grid` is laid out as block
(`replaced::ignores_inner_display`). Chrome lets canvas fallback content take Tab
focus; rinch does not (#1294); `meter` and `progress` still have no default
size and lay out their fallback content. The predicate says so, not `clip_shape`, so the
bracket, the chain, hit testing's gate and the dirty-region prune all agree. The
rinch-specific reason it had to be said: a *flowed* inline element owns no box
(`Node::is_flowed_inline_element` — its `layout` is zeroed and `E ghost box`
enforces that), so a clip derived from it was a `0x0` rect that the stacking
collector pushed onto the chain of a positioned box hoisted out from under it
whose entry carries the live chain — a `relative` box, or an `absolute` whose
containing block is the span itself; an `absolute` truncated at a containing
block above the span (#591's own child) and a `fixed` box never carried it. An
`inline-block` button inside an `overflow: hidden` span could not be tapped. **The
guard is deliberately wider than the boxless set**: a split inline and an
*unmarked* inline element — one no IFC has claimed, which therefore keeps a real
Taffy box — are inline boxes too, and `overflow` applies to neither — narrowing
the guard to the flowed predicate silently restores the clip on both. The
unmarked case used to be produced by the inner `<span>` of a re-measured
`inline-block` (#630); since #592 an `inline-block` is a block container and that
span is its IFC content with a `0x0` box. A second route — an inline flex or
grid item behind a `display: contents` wrapper, which rinch did not blockify —
was reachable from ordinary markup until #998 closed it; no route but
construction in a fixture is known now. A future component that puts `overflow: hidden` on an inline-level
element loses its clip silently, exactly as in a browser; there is no warning.
An inline element with `overflow: auto` is still found as a *scroll container*
(those walks read `overflow` against `Scroll | Auto` directly) while clipping
nothing — pre-existing, and no ink is at stake on a `0x0` box.

Anything new that asks "does this clip" must call the predicate, not re-derive
it. Code looking for the nearest *scroll* container (sticky positioning, wheel
routing, the scrollbar overlays) wants a different question and must not borrow
this one: `visible` and `clip` are the two non-scrollable values and only one of
them clips. That question is `Node::scrolls_y()` / `scrolls_x()`.

**Stacking contexts and the clip chain.** `Node::creates_stacking_context()`
answers: a positioned box with an explicit `z-index`, `position: fixed` or
`sticky` whatever the `z-index`, `opacity < 1`, a `mask-image` with a gradient
layer (#1798), a `transform` other than `none`
— an identity such as `translateX(0)` included (#415, `Node::has_transform`).
**None of them on a `display: contents` element** (#1038): it generates no box,
so it is neither a stacking context nor a positioned layer (`is_positioned_z_auto`),
its `opacity`/`transform` do not reach its children and its `z-index` scopes
nothing (all measured in Chrome 153). **Nor does a `transform` on a non-atomic
`display: inline` element** (#1080, CSS Transforms 1 §1): a plain span is not
transformable, so it is no stacking context and no containing block, and its
transform moves nothing — the same set `clips_overflow` excludes. The computed
value is kept (Chrome's `getComputedStyle` reports the matrix); every consumer
of the *effect* asks a `Node` method, not the raw value: both predicates ask
`Node::has_transform()` (not `none`, #415), and `paint::compose_node_transform`,
`PaintedState`, hit testing's inverse and its subtree prune ask
`Node::has_applied_transform()` (a non-identity matrix). The one reader
left on the raw value is the hit cache's `HitStyleKey::transformed`
(`!transform.is_none`), which sees only `ComputedStyle`; on such a span it only
costs an extra (harmless) cache invalidation when the transform toggles. Where a box is *anchored* depends on the
same fact, so every coordinate walk that can meet a contents node asks
`Node::box_position()` — the computed `position`, `static` for `display:
contents` — not `computed_style.position` (the readers left on the raw value are
ones a contents node never reaches: a stacking entry, a `0x0`-guarded layout
patch, the scroll-extent predicate behind the contents splice): a
`display: contents; position: fixed` wrapper's `0,0` layout used to be read as
a viewport position, and the walks that stop at a fixed ancestor
(`position_and_transform_in`, hit testing's `descend`, the damage clip chain,
`layer_bounds`) put its subtree at the window's origin. And it is no **scroll
container**: every scroll finder asks `Node::scrolls_y()` / `scrolls_x()`
(`overflow` `auto | scroll` on an element that generates a box) — the wheel's
finders, the scrollbar geometry, scroll-into-view and `scroll_to_fraction`,
`clamp_scroll_offsets`; the sticky walk skips a contents ancestor. A finder that
picked a `contents; overflow: auto` wrapper wrote a `scroll_offset` onto a
box-less node, so nothing on screen moved and `compute_absolute_position` did.

That is **not** the whole CSS list, and the shortfall is not only about what
`ComputedStyle` can hold. `clip-path`, `isolation`, `mix-blend-mode`,
`contain: paint` and `will-change` are absent from `ComputedStyle` altogether
and need new style plumbing per property — tracked separately from #542 (below),
which closed the two creators that needed none.

**#542 (closed): two creators were representable and simply missing, and now
aren't.**

- a non-`none` **`filter`** (CSS Filter Effects §2.1) — `filter: brightness(0.5)`
  reaches `ComputedStyle::filter_brightness` and paint consumes it.
  `Node::has_non_identity_filter()` answers `true` when any of the four scalars
  (`filter_brightness`/`filter_grayscale`/`filter_saturate`/`filter_hue_rotate`)
  differs from that filter function's identity value, and
  `creates_stacking_context` now asks it. It is an approximation, not the CSS
  rule, and stays one: `filter: brightness(1)` is a genuine non-`none` filter
  that is numerically identical to "no filter" in this storage (`ComputedStyle`
  has no separate "a filter was declared" bit), so it is still indistinguishable
  from absent — Chrome creates a stacking context for it anyway. `blur()`
  really is unexpressed; only the four scalars survive `from_stylo`, so a
  `blur()`-only filter still answers `false` too. Both are the accepted,
  pinned boundary of what #542 closed, not a regression.
- a **flex or grid item with a `z-index`** other than `auto`, even at
  `position: static` (css-flexbox-1 §5.4, css-grid-1 §6) — the `z_index` was
  already in `ComputedStyle`, but the item's own style can't answer "is my
  layout parent a flex/grid container". `Node::is_flex_or_grid_item: Cell<bool>`
  carries that fact, set at every cascade of the node
  (`style_resolution/resolve.rs`, and the pseudo-element cascade in
  `style_resolution/pseudo.rs`) from the same `layout_parent_style` Stylo's own
  adjuster blockifies against (#998's "layout parent": skips a
  `display: contents` wrapper to the nearest real ancestor) — so it tracks
  blockification for free, since a container's `display` moving to/from
  flex/grid already re-cascades its children (`child_cascade` follows a
  `display` change).

**#415** was the related one, in this same function and the same class, and is
closed: a `transform` that composes to the identity (`translateX(0)`,
`rotate(0deg)`, `scale(1)`) is a stacking context and an absolute containing
block, because CSS keys both on the computed value not being `none`
(`TransformValue::is_none`, read through `Node::has_transform`). The
paint and hit-test fast paths still ask the other question —
`Node::has_applied_transform`, a non-identity matrix — so an identity
transform creates the context without paying a transform at paint. A
transition or animation frame is never `none` (`AnimatableTransform::to_style`),
so an element is a stacking context for the whole run; a transition that
*finishes* at `none` writes `none` and stops being one, and the document's tick
re-syncs the absolute descendants it stopped containing
(`tests/identity_transform_tests.rs`, each fixture against Chrome 153). Two
ends are **not** Chrome's: `rotate(0deg)` → `none` under a declared transition
starts no run in rinch (`lists_equivalent`), so the context goes at once where
Chrome holds it for the run. (An animation with no fill that finishes used to
keep its last sample, and so stay a context; since #783 its finishing tick
restyles the node, and the cascade's own flip re-sync releases the absolute
descendants.) The `HitStyleKey` keys
`is_none`, not identity.

**A `GameViewport` is the base layer at its place in paint order** (#1866). What paints
before the viewport (ancestor backgrounds, an earlier or lower-`z-index` sibling) does not show
inside its box; what paints after it (a later sibling, a higher `z-index`, a hoisted popup)
covers it; the box is clipped by the viewport's own clip chain and moved by its transform. On
the GPU window and the embed overlay paint splits the viewport out there as a base slice and the
split compositor replaces what is beneath it; elsewhere (software, `RinchContext::scene()`, a
refused split) paint erases the box in the scene. A group the viewport sits in breaks the rule —
`opacity < 1`, `mask-image` or `filter` on an ancestor (or the viewport), whose layer is
composited over what is beneath it — and on the GPU it refuses the split (the frame falls back
to the layered path); a debug build says so once per viewport. In the layered fallback the
compositor's hole mask (`viewport_rect_with_radius`, above) rounds the *layer* by a clipping
ancestor's radius, and the viewport's own radius is the erase's.
Detail: `paint-and-damage.md`, "Viewport holes".

**An identity transform costs what any stacking context costs**, the same as
`opacity: 0.99`: it is hoisted, and until #1719 neither the off-screen paint
cull nor hit testing's flow-extent prune skipped one — 300 rows in a 400px
scroller cost `PaintNodesVisited` 24 → 303 and `StackingOrderBuilds` 2 → 313
on a wheel notch, `HitTestNodesVisited` 4 → 285 on a warm pointer move,
against the `transform: none` twin. #1719 culls and prunes such rows (24, 2,
2: `perf_regression_tests::identity_transform_rows_cost_what_any_stacking_context_costs`);
a stacking context whose subtree hoists something, or a non-identity
transform (inverted on entry), is still entered by the hit test. So a component's
resting state says `transform: none`, not `scale(1)` / `translateY(0)` /
`rotate(0deg)`: `Checkbox`'s icon, `Radio`'s dot, `Slider`'s label and
`Select`'s chevron were changed for it (`none` interpolates with the other
state's function as the identity does, in rinch and in Chrome 153, so the
transition is the same; 300 checked checkboxes went back from 603 to 303 nodes
per warm move). `none` ↔ `rotate(0deg)` is the one pair that starts nothing.

Stage B slightly **widened** that exposure rather than leaving it untouched: a
box declaring **both** a filter (or a static flex/grid item's `z-index`) and a
clipping `overflow` used to get a stacking context by accident through the
`overflow` arm, and lost it when that arm went — its clipping survived (the
chain carries that) but its ordering did not. #542 closes that: such a box now
gets a real stacking context from its own declared creator, so the ordering is
correct again rather than merely latent. (Neither was folded into stage B
itself, because adding a creator changes which boxes hoist — the axis stage B
was re-founding — and landing both at once with the rest of that change would
have made a regression impossible to attribute.)

**`overflow` is not on that list** (#324 stage B). It used to be, so that a
descendant hoisted to an ancestor's paint sequence stayed inside the bracket
paint opened around it — clipping was made a stacking question because the clip
was implemented as a per-sequence bracket. That cost the same user-visible bug
twice, because it made rinch compare two `z-index` values from different
stacking contexts, which CSS never does: #317's `DropdownMenu`/`Select`
backdrops and #534's menu-bar overlay, each worked around by respelling a
`fixed` overlay as `absolute`, and each reverted in stage C.

Clipping is decoupled from stacking now. Every hoisted
`stacking::PaintEntry` carries a **clip chain** — the clipping ancestors between
it and the collecting root, as a `ClipSpan` into `PaintOrder::clips` — and its
consumer re-applies them: `paint_children_with_stacking` pushes them around the
entry, `hit_test_node` rejects a probe point outside them. Four rules make it
correct, and each has a fixture in
`crates/rinch-dom/tests/clip_chain_tests.rs`:

- **A link's rect is the clipping box's own `clip_shape`, at its PRE-scroll
  painted origin.** A container's box does not move when its content scrolls.
  Invisible at scroll offset 0, which is why every fixture is scrolled.
- **`position: absolute` truncates its chain at its containing block.** CSS does
  not clip an absolute by an `overflow` ancestor below its containing block, so
  `descend` tracks `live.len()` as of the nearest
  `establishes_abs_containing_block()` and an absolute entry takes only that
  prefix. This is also what keeps #204's initial-containing-block correction
  correct. `position: fixed` takes an **empty** chain; everything else takes the
  whole one — an in-flow or `relative` box is clipped by every clipping
  ancestor, containing block or not.
- **The collecting root's own clip is in no chain.** Paint opens that bracket
  before it walks the sequence, and hit testing gates the whole walk on it —
  **except around a `position: fixed` entry** (#545), which the root does not
  contain: paint lifts the bracket for the length of that entry and puts the very
  same shape back, and hit testing exempts it from the bounds gate per entry.
  That was stage B's documented "Known gap", and #545 is what made it live.
  **And every clip above the root too (#549).** The chain on the *root's own*
  entry, in whatever sequence hoisted the root, is pushed around the root's
  whole subtree, so lifting the root's bracket alone left `plain clipper >
  stacking context > fixed` cut to the clipper, where Chrome 155 paints and
  hits the fixed box. Paint now lifts every clip the painter has open down to
  the fixed box's containing block: `ClipTrackingPainter` mirrors each layer it
  forwards onto `OPEN_LAYERS` (beside `CLIP_CULL`, #910), `paint_node` records
  a floor (`FIXED_FLOOR`) when it enters a box that contains fixed descendants
  (`paint::contains_fixed_descendants`: a `transform`, a `filter`, a
  `mask-image`), and `paint_sequence_entry`'s `LiftedClips` pops everything
  from the lowest clip above that floor, pushes back the non-clip layers among
  them (an ancestor context's `opacity` applies to a fixed descendant), paints
  the entry, and restores the exact stack. The common case is the old one: the
  root's bracket on top, popped and pushed back alone. The one deviation is
  group opacity: when a clip sits *under* an ancestor's `opacity` layer, the
  fixed box is composited as a group of its own at that opacity rather than
  with the rest of the context (visible only where the two overlap). Hit
  testing carries the same fact down as `hit_test_node`'s `clipped` flag —
  "the probe is outside a clip a fixed descendant escapes" — which a fixed
  entry clears, a containing box turns into a miss, and which only stacking
  contexts holding such a box are entered under (`holds_escaping_fixed`, a
  per-generation bit in the hit cache). Under a transformed, filtered or
  masked ancestor the clips at and above it still apply, as in Chrome — rinch
  still *places* the box against the viewport there (#1372). Pins:
  `rinch-dom/tests/fixed_escape_549_tests.rs` (pixels),
  `rinch/src/app/fixed_escape_hit_549_tests.rs`, `damage_clip_chain_tests`'
  `a_fixed_box_under_a_context_inside_a_clipper_is_damaged_outside_it`, and
  `rinch/tests/gpu_fixed_escape_549.rs` (Vello, `test-gpu`). An absolute's
  "Known gap" in `stacking.rs` is untouched: it is the root's bracket applying
  to an absolute the root does not contain, a different escape. A third consumer has to know the same thing from the other
  side: `paint::layer_bounds` walks the tree rather than the sequence, so it sees
  clippers a fixed descendant escapes, and its `Extent::Escapes` case stops one
  narrowing a translucent layer to less than it paints — which tiny-skia ignores
  and Vello enforces, i.e. a software/GPU divergence no pixel test can see.
  **`position: absolute` has the same hole and it is NOT handled for the two
  *bounds* callers** (#550), but it is a *different shape*. `Collector::span`
  truncates an absolute's chain at its containing block (`Absolute =>
  self.cb_depth`, set at **any** `establishes_abs_containing_block()` ancestor —
  any non-`static` position or a transform on a box that is not
  `display: contents`, #994), so it escapes the clippers *below*
  that block while staying clipped by the ones above. `opacity_layer_bounds` and
  `clip_cuts_nothing` narrow it at **every** clipping ancestor regardless, so a
  layer holding one can come back too small with the same GPU-only symptom. It
  needs a **partial** escape, which `Escapes` cannot express — that is why only
  the `fixed` case is covered there, and it is pre-existing rather than something
  #204 introduced. The module's **third** caller,
  `subtree_is_entirely_outside` (the off-window cull, #562), does **not** accept
  that hole and does not close it either: it answers `Escapes` for an absolute
  whenever a clipping ancestor **below the walk root** sits below the containing
  block, which is whole-escape where partial would be exact — conservative in the
  direction that costs an unpruned subtree rather than a deleted box. *Below the
  walk root* is the whole qualification and not pedantry: the root's own clip is
  one no absolute inside escapes, because paint opens that bracket around the
  entire sequence (#549), so a clipping root that is not itself a containing
  block answers `Within` and prunes —
  `offscreen_cull_tests::a_clipping_non_containing_block_root_still_prunes`. The asymmetry is
  deliberate: a bounds under-measure costs content on the Vello path only, a
  cull under-measure costs it on every path. `layer_bounds.rs`'s module doc names
  all of this explicitly; read it before touching this.
- **No transform composition is needed.** A transform creates a stacking
  context, so `descend` never crosses one and every link lives in the collecting
  root's own untransformed space — the same space the entries' offsets are in.

Hit testing tests a link's **rect only**, ignoring its radii, matching the
`check_children` gate it stands in for; paint pushes the rounded shape. That is
the pre-existing rounded-corner divergence, not a new one.

**Hit testing prunes and caches** (update-path audit F2.1). A pointer move used
to run two full hit tests, each O(document): every stacking root rebuilt its
`PaintOrder`, and a non-clipping box probed all its children because they may
overflow it, so every off-screen row of a scroller was visited. Now one hit test
per move is shared by `data-onmousemove`, the drop-target search and hover, and
one per press by everything a press asks (#908: the `data-onmousedown`
dispatch, the editor, the focus claim, the draggable search and each phase of
the click — nine for a toolbar press over a focused editor). Both go through
`RinchApp::shared_hit` (`move_hit` is it against the app's own document), which
remembers the last answer with the hit cache's generation and the point, and
re-tests once the generation has moved — a handler that mutated the document in
between. The memo is dropped at the top of every `handle_event`, because a
transition or animation tick moves a `transform` without moving the generation
(see below). The `data-onmousemove` walk is skipped outright while
`NodeTree::mousemove_handlers` is 0. The walk skips a child whose **flow
extent** misses the point — the union of every box `hit_test_node` can test when
it enters that child as an ordinary node (its own box, plus its non-hoisted
children's extents unless it clips). In-flow and positioned `z-index: auto`
entries are pruned on it, and so is a stacking context whose flow walk passes
over no box painted at a stacking root (#1719: its sequence is then exactly its
flow walk; that bit is a separate memo, `HitCache::hoists`, computed only for a
stacking context the hit test asks about, and a box that clips answers it
conservatively, "hoists", without walking). A stacking context
that hoists something is always entered, since its hoisted descendants can be
anywhere; one with nothing in its box tree builds no sequence. Pin:
`prune_tests::stacking_context_rows_hit_identically` (against the unpruned
walk). Extents and hit-test stacking sequences live in
`NodeTree::hit_cache` (`rinch_dom::hit_cache`), valid until its generation
moves: every mutating `DomDocument` method that changes something (an identical
attribute or text write returns first), `resolve_styles`, `resolve_layout`
(except on its paint-only path, which moves no box — #911),
`NodeTree::push_dirty` and `remove_subtree` bump it, and so does a transition
or animation tick — **only** for a node whose `HitStyleKey` it changed (the
`computed_style` inputs the cache depends on: `display`, `position`,
`overflow`, `opacity < 1`, `visibility`, whether `transform` is `none` (#415),
`z-index`, `pointer-events`, left/top padding and border). `AboutToWait` ticks
after every batch, so an unconditional tick invalidation made every real move
cold, and a colour animation (a `Loader`) must not either. The transform
**value** is deliberately not keyed — a transformed box is a stacking context,
so no flow extent or cached sequence offset holds it, and `descend` reads it
live — which keeps a `Drawer`/`Popover` slide warm. Only opacity, transform,
padding and border widths are animatable, so only those key fields have tick
fixtures (`prune_tests::a_*_tick_*`, each killing the mutant that drops its
field). **A write straight into `tree.nodes[..]` that changes
`layout`, `scroll_offset` or `computed_style` without going through one of
those must call `tree.hit_cache.invalidate()`**, or the next hit test answers
from the old geometry. `app::hit_testing::prune_tests` is the oracle: the walk
as it was before, verbatim, compared point by point with the pruned one over 60
generated documents (positioned, fixed, transformed, clipped and scrolled
boxes, `z-index`, `pointer-events: none`, `display: contents`) before and after
a scroll, a restyle and a removal. Measured: a warm move over a 500-row
scroller went from 2 hit tests visiting 1986 nodes to 1 visiting 4, including
with `AboutToWait` between moves (`a_move_after_about_to_wait_is_warm`,
`a_colour_animation_keeps_moves_warm`).

**`visibility: hidden` is per node, not per subtree** (#843). A hidden box is
not hit itself and offers no scrollbar (paint draws neither, #829), but
`visibility` is inherited and a descendant may set it back to `visible`; paint
draws that descendant and Chrome's `elementFromPoint` answers it, so
`hit_test_node` and `find_scrollbar_hit_node` walk below a hidden node. They
skip its whole subtree, as they always did, when nothing below it is visible:
`shows_below`, a third memo (`HitCache::shows`), answered over the box-tree
children (an anonymous block has no DOM children; its wrapped inline content is
what it shows) and asked only of a hidden node, so a visible tree pays one enum
test per node and an all-hidden one (a closed `Drawer`) one walk per
generation. A text node takes its parent's visibility. An IFC's text is hit as
its root, so a hidden IFC root still answers a point over the glyphs of a
visible inline element in it (`visible_text_at`: the cluster under the point,
its `IfcTextRange`, that text node's parent element); Chrome answers the span
itself, which desktop never does for inline text, visible root or not.
Before #843 the walks returned `None` at the first hidden node, with a comment
fearing Stylo would leave a child's inherited `visible` stale when an ancestor's
class changed; `hidden_ancestor_hit_843_tests::toggling_an_ancestors_visibility_reaches_the_inheriting_child`
measures that it does not (class and `style` toggles both reach the child).
The prune oracle walks hidden subtrees unconditionally.

**A scroll is the one partial invalidation** (#911). Every scroll-offset write
in the shell — the wheel (touch scrolls arrive as wheel events), a scrollbar
drag, `apply_scroll_into_view` (every `scroll_into_view` and
`scroll_to_fraction`), `set_scroll_top`/`set_scroll_left` — goes through
`NodeTree::mark_scrolled`, which calls `HitCache::invalidate_scroll`: the
generation still moves (so `move_hit` re-tests) and every stacking sequence is
dropped, but extents are kept. An extent is relative to its own node and reads
only its **own** scroll offset, never an ancestor's, and a box that clips — every
scroll container but an inline span — does not fold its children in at all, so scrolling one
changes no extent anywhere. A box that does not clip and has a scroll offset
anyway (`set_scroll_top` can give one that, and so can the wheel, which finds a
non-atomic inline `overflow: auto` span as a scroll container) drops every
extent, as a full invalidation does: keeping the rest would mean recording each
extent's parent as it is computed, and that bookkeeping cost the first pointer
move after every layout 5% of its instructions (`shell::pointer_move_cold`,
measured after #962 merged with it). A new
input to `flow_extent` that is not relative to its node breaks this. (Layout's
own clamp and the `display: none` reset are inside passes that invalidate in
full.) A new scroll-offset writer that changes nothing else should use
`mark_scrolled`, not `push_dirty`, or every notch rebuilds every row's extent.
The wheel arm runs **one** hit test for both the render-surface check and the
scroll routing. Measured on 500 rows: the second and later notches went from 2
hit tests and 493 extents to 1 and 0
(`perf_regression_tests::a_second_wheel_notch_recomputes_no_extent`); the first
notch after a layout still finds the cache cold (498).
`prune_tests::nested_scrollers_scrolled_one_at_a_time_hit_identically` and
`a_resize_that_moves_percentage_boxes_drops_the_memo` are its oracles.

**A scroll offset is kept on the device pixel grid** (#1543). `mark_scrolled`
floors the offset just written to a multiple of `1 / NodeTree::device_scale`
(`NodeTree::snap_scroll_offset`; the scale is set beside the Stylo device pixel
ratio, `RinchApp::set_device_pixel_ratio`), before anything reads it, and
layout's clamp writes the floored range end. So every reader — paint on both
backends, hit testing, damage, `out_of_flow::replace_after_scroll`,
`scroll_top()` — sees one offset, and scrolled content moves rigidly by whole
device pixels, as in Chrome. Before, the software painter's per-glyph
nearest-pixel placement (`skia_painter.rs`'s `CachedGlyph`) rounded each glyph
and each baseline on its own at every fractional device position (125%/150%,
or a fractional wheel delta at 100%), so text re-spaced while it scrolled. The
wheel keeps what a delta leaves below the grid (`Node::scroll_remainder`), or a
slow touchpad's sub-pixel deltas would floor back to where they started; every
other writer zeroes it (`mark_scrolled`, the clamp, the `display: none` reset); a
reveal that scrolls **down** (`apply_scroll_into_view`) rounds up instead,
within the range, so the box is not left a fraction under the edge. The cost:
`scroll_top()` answers the snapped value (a `set_scroll_top(10.33)` at 125%
reads back 9.6), the last fraction of a device pixel of a range that is not on
the grid is unreachable, and a display-scale change re-snaps every offset
(`NodeTree::resnap_scroll_offsets`, from `RinchApp::set_device_pixel_ratio`). Pins:
`rinch-dom/tests/scroll_device_snap_1543_tests.rs` (rigid shift, pixel by
pixel, at 100/125/150%), `app/scroll_device_snap_1543_tests.rs`.

**Two behaviour changes that follow from the rules, are CSS-correct, and will
still surprise someone.**

- **An `absolute` with no positioned ancestor now escapes an intervening
  *static* `overflow: hidden` entirely.** It resolves against the initial
  containing block (#204), so no `overflow` box between it and the root is in
  its containing-block chain and its chain is empty. Before, that box was a
  stacking context and its bracket caught everything hoisted into its sequence,
  so the absolute was clipped. **Measured against Chromium**, not reasoned:
  `elementFromPoint` 100px outside a static `overflow: hidden` 200x200 box
  answers the absolute, and its `offsetParent` is `BODY`. On a transparent
  borderless window the visible form is an absolute painting into the rounded
  corners; the remedy is the browser's — put `position: relative` on the wrapper
  you meant to clip with.
- **A `z-index: -1` descendant of a scroller now sorts into the *body's* step 1**
  rather than the scroller's, so it paints behind the scroller's own background
  instead of merely behind its content. Still clipped by the chain. This one
  follows from the same rule rather than being separately measured — the
  scroller is not a stacking context, so it cannot hold a negative-`z` layer of
  its own.

**Cost.** `TinySkiaPainter::push_clip` fills and intersects a mask over the
clip's bounds (it allocated a full-surface `Mask` per push until the painter
pooled them — see **The software painter's caches** above), so a
push is not free — but consecutive entries that share a `ClipSpan` share one
push, which is exact rather than a heuristic (identical spans name identical
clips), and it collapses the shape that motivated the worry: a scroller with 200
positioned rows is **one** push, not 200. Measured at 1200x800, software
painter, best of 40: 200 positioned rows in one scroller 1.82 → 1.90ms (+4.5%);
an adversarial 50 scrollers x 4 rows — one push per scroller, so **50**, since
each scroller's own entry carries no chain and interrupts the run — 2.73 →
3.08ms (+13%). Building the body's sequence went 1 → 3us.

`clip_chain_tests::consecutive_entries_that_share_a_chain_share_one_clip_push`
is the pin, and **the 50-scroller assertion is the load-bearing half of it**.
The headline "200 rows = 1 push" is a *fixed point*: one scroller means the clip
table only ever holds one chain, so a collector that reused far less still
answers 1. Reusing the table's head instead of its tail leaves every pixel and
every tap identical — pushing the same geometry twice is idempotent — and takes
the 50-scroller case to **197** pushes and a 197-entry table. Do not simplify
that fixture down to one scroller: the reuse would then have no test at all, and
a refactor could quietly 4x the mask cost in the exact workload benchmarked
above. The
containment skip the scoping proposed (don't push a clip the entry is entirely
inside) is **not** implemented: a sound version needs a subtree extent per entry,
which is a new per-frame walk for every positioned box, and the run reuse already
took the realistic case.

**A `position: fixed` box is hoisted to its NEAREST ancestor stacking context**,
not to the body (#545). It is viewport-*positioned*, not viewport-*stacked*: its
entry keeps zeroed offsets, an empty clip chain and the body's transform, but it
is entered in the sequence CSS says owns it. Its empty clip chain is not what
makes it escape the clips above that context — paint's lift is (#549, the
clip-chain rules above). It used to be pulled out to the body
whatever lay between, which compared two `z-index` values across two stacking
contexts — the same fault `overflow` caused before stage B. A `z-index: 99`
dismiss backdrop escaped a wrapper its `z-index: 100` panel could not and covered
it; and under a wrapper whose own `z` was *above* 99 (`Modal` and `Drawer` at 201,
`Notification` at 300) the backdrop sank beneath the whole wrapper, so a popup
inside a modal could not be dismissed at all. Chromium does neither. There is no
`is_body` flag in `stacking.rs` any more: the body is simply the outermost
stacking context.

rinch still models no **containment** — CSS makes a *transformed* ancestor the
containing block of a fixed descendant (and an `opacity` one not), while
`out_of_flow::out_of_flow_kind` answers "the viewport" for every fixed box. That
was already wrong before #545 and is exactly as wrong after; what #545 preserves
is that it is wrong *consistently*, since paint keeps handing a fixed entry the
body's transform. Tracked with **#1372**; the non-parent containing-block
machinery it needs exists for `absolute` since #386
(`OutOfFlowKind::AncestorAbsolute`).

**Stage C reverted both workarounds.** `.rinch-dropdown-menu__backdrop`,
`.rinch-select__backdrop` and `.rinch-app-menu-bar__overlay` are `position:
fixed` again, and `build_overlay`'s compensating offset is gone with them. That
buys back two behaviours, each pinned by a pair of tests in
`popup_backdrop_hit_tests` — one asserting the behaviour, one asserting the
`absolute` spelling did not have it: a tap **outside the popup's clipping
ancestor** dismisses, and a tap on the **app's own fixed chrome** dismisses
rather than being taken by the chrome.

The overlay's `position` and its offset had to move **together**, and the test
that says so fails from *both* sides: flipping only the `position` fails
`the_below_titlebar_overlay_covers_the_whole_window` at the window's bottom edge,
flipping only the offset fails the same test at the title bar. Stage C also
needed **#545** as much as stage B — the reverted overlay lands in
`.rinch-app-menu-bar__inline-layer`'s own sequence at 199 beside the row's 201,
measured, so the "199 and 201 in one sequence" its comments describe is literally
what happens. Hoisted to the body it would have sat at 199 against the *layer's*
200, which orders the same way: every test would have stayed green with the
comments' mechanism wrong.

**Key files:**
- `crates/rinch-dom/src/stacking.rs` — the paint sequence and the clip chain
- `crates/rinch-dom/src/paint/clip.rs` — the clip predicate and shape
- `crates/rinch-dom/src/paint/painter.rs` — Abstract `Painter` trait
- `crates/rinch-dom/src/paint/vello_painter.rs` — GPU backend
- `crates/rinch-dom/src/paint/skia_painter.rs` — Software backend
- `crates/rinch-dom/src/paint/mod.rs` — `paint_document()`, dirty region computation, subtree pruning
- `crates/rinch/src/app/mod.rs` — `build_scene()` (GPU) / `build_pixels()` (software)
