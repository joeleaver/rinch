# Layout and style invalidation, text caches, scoped IFC pass

Read this page when your task touches: layout_dirty/ifc_dirty, typography invalidation, atomic inlines, resize restyle, style invalidation, text caches.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

**Layout invalidation: three paths, two flags.** `resolve_layout` early-returns
when `tree.layout_dirty` is false (styles resolve, dirty Parley layouts rebuild,
**no Taffy compute**), and runs the inline-formatting-context setup passes only
when a structural change is pending — **scoped** to where it happened
(`tree.ifc_seeds`), or over the whole document when `tree.ifc_dirty` asks for
the fallback (see **The structural pass is scoped** below). Both gates are
load-bearing for frame cost and both used to be closed on changes that move a
box:

- **A typography change is a layout change (#678).** `font-family`,
  `font-weight`, `font-style`, `line-height`, `letter-spacing`, `word-spacing`,
  `text-transform`, `white-space` and `overflow-wrap` are not Taffy properties —
  and neither is **`font-size`**, for a box whose own sizes are in `px`, since it
  reaches the Taffy style only through a value that *uses* it. Any of them
  re-wraps the text, so `ComputedStyle::same_measured_text_inputs` sets
  `layout_dirty` for them. That predicate is deliberately **narrower** than
  `same_text_layout_inputs` (#654), which decides whether the *glyphs* must be
  re-shaped: `color` is baked into the glyphs and moves no box, so a
  `:hover { color }` on IFC text still takes the cheap path — pinned by
  `frozen_box_remeasure_tests::a_colour_only_restyle_still_skips_taffy`, which
  reads `tree.taffy_computes`, the counter that exists because nothing else
  distinguishes "took the cheap path" from "recomputed and got the same answer".
  A text **leaf** — a flex or grid item's own text, in a block-level container
  or inside an `inline-flex` / `inline-grid` such as a `Button` label — is
  cheaper still: it is painted from the layout its *measure* built
  (`Node::cached_text_parley`), which only a compute rebuilds, so paint draws it
  in its parent's **current** computed colour instead of the brush it was shaped
  with (`render_text`'s `color` override, #904). A colour change or a colour
  transition frame on it re-shapes nothing and runs no compute
  (`perf_regression_scenarios::a_colour_hover_on_*`); it used to leave a
  block-level flex item's label in its old colour. Nor does a flex or grid
  container's own text take its `text-overflow: ellipsis`: that text is an
  anonymous item, which does not clip, and Chrome 153 draws it clipped with no
  "…" (`DisplayValue::is_flex_or_grid_container`, #904).
  Cost, measured on 500 rows: a whole-document typography swap goes 7.4 → 16.1ms,
  a one-row hover 0.60 → 0.70ms.
  **Being in that list is necessary and was not sufficient** (#698):
  `letter-spacing` and `word-spacing` sat in both predicates while reaching no
  Parley producer but `build_parley_layout`, whose only callers are two MCP
  debug tools — so they invalidated correctly and then re-shaped the text
  without themselves. They now reach the IFC root, the per-span properties, the
  `TextMeasure` context and both of its consumers, the `text-overflow:
  ellipsis` rebuild (an IFC root's; a text leaf's was deleted in #982) and
  paint's on-demand fallback. The **form-control** text path (`<input>`,
  `<textarea>`, `<select>`) is deliberately not among them: paint and the two
  hit-test builders there have to move as one piece, which is #320. A
  percentage `letter-spacing`/`word-spacing` resolves against the element's
  own `font-size` (#743, `typography::letter_spacing_from_stylo`/
  `word_spacing_from_stylo`) — measured in Chrome 153, not against either
  property's glyph-level reading of the spec (a space's own advance, for
  `word-spacing`): `50%` at `font-size: 20px` adds 10px per affected cluster
  on either property, and doubling `font-size` doubles it, where 50% of the
  font's own (much narrower) glyph advance would not. `font_size` is already
  one of the fields `same_text_layout_inputs`/`same_measured_text_inputs`
  compare, so a later `font-size` change still re-shapes this text with no
  change needed there.
- **An atomic inline is sized by its own passes and no compute (#661).**
  `inline-block`, `inline-flex` and `inline-grid` boxes are detached from their
  parent's Taffy child list so the enclosing IFC can measure them as Parley
  `InlineBox`es, so the **root compute never reaches one**. All four sizers go
  through `measure_inline_blocks`: `compute_inline_block_layouts` on an
  `ifc_dirty` pass, `measure_scoped_atomic_inlines` — its share of a scoped
  structural pass, sizing only the scope's regions — on every other structural
  pass, `resolve_percentage_inline_blocks` after the root compute
  for any width that needs the containing block's — a percentage, a
  `fit-content`/`stretch` keyword (#691), or `auto`, which is shrink-to-fit
  (#658; an `auto` box measured as `auto` that fits is skipped by one
  comparison) — which is also what makes such a box track a viewport
  resize, on a pass with no `ifc_dirty` and an empty dirty set, and —
  since #661 —
  `remeasure_dirty_atomic_inlines`. (`resolve_percentage_inline_blocks` also
  measures the min-content size of an `auto` box a measure asked for, #1476,
  and runs alternately with the root compute until it changes nothing, at
  most four times — see **Common Issues**, the sizing-keyword entry.) `ifc_dirty` is left false by a style-only
  restyle and by a `set_text_content` alike, so before that last pass the box
  was measured once and frozen at `225x20` while paint drew six lines. It
  re-measures the ones a change actually reached, off a **dirty set**
  (`tree.dirty_atomic_inlines`) rather than a flag: measured, re-measuring the
  whole document instead costs
  +47% on a one-row text edit in a 500-row document carrying 500 chips.
- **A transition or animation writes `computed_style` directly**, so it reaches
  none of the cascade's invalidation; `tick_transitions` and `tick_animations`
  each invalidate the text measure of the nodes they are interpolating
  (`TransitionProperty::changes_text_measure`). `font-size` is the only
  animatable property in that set today, and the `All` arm of that predicate is
  unreachable — a transition map is always keyed by the concrete property — so
  `transition: all`, which `Checkbox` and `Radio` both declare, pays nothing per
  frame. **Their Taffy re-sync marks atomic inlines separately**, because that
  pre-pass fires only for `font-size` while a `transition: width` on a box
  *inside* an `inline-block` is #661's own symptom reached without the cascade
  (found by the review of #694). **A running `color` frame re-shapes nothing, for a leaf or an IFC**
  (the frame that ends the run rebuilds an IFC's paint layout once, below).
  Paint colours a text leaf from the live style (#904). An IFC's `text_layout`
  carries each run's colour as a brush, so paint asks every text range for its
  colour again (#679): `ifc::text_color` — the `color` of the text node's DOM
  parent — recorded at the build as `IfcTextRange::color` (the root's own as
  `InlineLayout::root_color`, which is what a `text-overflow: ellipsis`
  rebuild's text follows), and `paint::text::LiveColours` draws the ranges that
  answer differently in the new colour, glyph by glyph where two ranges share a
  Parley run. An underline or line-through follows only where it was
  `currentcolor` (no `text-decoration-color` from the range's element up to the
  IFC root); a wavy underline and an inline element's background rectangle and border
  (colour, padding, radius; its horizontal extent from its edge boxes, #1426) are read from their owner's style on every paint
  (`ifc::wavy_underline_color`, `ifc::inline_background_span`). Two things a
  tick still rebuilds a paint layout for, with no Taffy compute
  (`settle_ticked_text`): the frame an inline element's background **appears**
  on (the layout holds a span only for an element that had one), and the frame
  a colour **stops** moving — a finished transition, an ended or fill-settled
  animation — so later paints are back on the layout's own brushes rather than
  recolouring stretch by stretch for good; every paint of an IFC still makes
  the comparison, one pass over its text ranges
  (`perf_regression_scenarios::a_colour_transition_frame_on_ifc_text_shapes_nothing`,
  `the_frame_that_ends_a_colour_transition_rebuilds_one_layout`). An animation
  paused by a restyle is re-shaped at its frozen colour by that restyle's
  cascade (`rv_e_paused_animation`). A split inline's (#513) text is drawn by
  its container's boxes, so `compute_damage` names each text child's `ifc_root`
  for a paint-dirty split inline — without it the software renderer kept such
  text in its start colour until the run ended (`rv_m5_damage_split_inline`).
  Pins: `crates/rinch-dom/tests/animated_text_brush_679_tests.rs`; a span on a
  line cut by `text-overflow: ellipsis` follows too, since the cut keeps its
  text ranges (#1451, `rv_b1_span_on_ellipsis_line_mid_run`). Not covered: an ellipsis
  line laid out by an anonymous block box keeps a stale colour, after a plain
  class change too (#1450, older than #679; the four `rv_a*` fixtures are
  `#[ignore]`d on it); a `padding` frame on an inline element still runs a
  Taffy compute and a shape (#1437); and a `color` transition still stops at
  its own node (text in a child that inherits takes the end colour at once).
- **A `display: contents` wrapper's Taffy style is `sync_display_contents`'s,
  not the cascade's.** That pass stores it as `Display::None`
  (`node::display_contents_taffy_style`), while `to_taffy_style` maps `contents`
  to `Display::Flex`; every pass that rebuilds a Taffy style from computed
  values and compares it (`apply_stylo_styles_to_taffy`, both tick re-syncs)
  substitutes the stored form when `Node::taffy_style_owned_by_contents_splice`
  says so. Before that, any re-cascade of a wrapper — `rsx!` emits one per
  reactive text and per if/for/match/component site, and a hover re-cascades a
  whole subtree — read as a display change and ran the whole-document
  `ifc_dirty` pass: 91ms for a colour-only hover over 2000 rows, 0.18ms after.
  `frozen_box_remeasure_tests` pins both halves through `tree.ifc_setup_passes`.

`RinchDocument::invalidate_text_measure_for_node` is the one place that knows
what a typography change owes: the IFC's Parley layout, the box of any atomic
inline above it, and the `NodeContext::Text` a text child is measured through
when it is a flex or grid item — plus the Taffy `mark_dirty` beside that last
one, since Taffy caches a leaf measure per available space and serves the stale
one back otherwise. Such a leaf is painted from the layout its measure built:
the root compute's for a block-level container and the detached atomic
compute's (`measure_inline_blocks`, which keeps them in
`NodeTree::atomic_leaf_layouts` until `copy_cached_text_layouts`, filed under
the width they were wrapped at and picked by the box's **unrounded** width) for
an `inline-flex` / `inline-grid`; before #904 the latter kept none, and paint
re-shaped every such label on every frame.

**A resize restyles only what the size reaches** (`RinchDocument::restyle_for_viewport_change`).
`Stylist::set_device` answers which stylesheet origins' media-query results
flipped; only such a flip rebuilds the cascade data and re-cascades the whole
document (`full_restyle_viewport`). Otherwise the only restyled elements are the
ones whose last cascade resolved a viewport unit — Stylo's
`USES_VIEWPORT_UNITS`, copied onto `Node::uses_viewport_units` by the cascade,
`::before`/`::after` included — each with its subtree
(`viewport_unit_restyles`). Two rinch-side inputs ride along without a cascade:
every `position: fixed`/`absolute` box is queued for a Taffy re-sync, because
`out_of_flow` bakes the viewport size into its Taffy style. The first layout no
longer re-cascades everything either, which is what used to hide another
non-cascade input of `apply_stylo_styles_to_taffy`: an unstyled `<select>`'s
widest-option width (`note_select_content_changed`). (A childless block's
one-line floor was a second, re-synced by `note_first_child`; both are gone
since #296 — see **An empty `<div>` is 0 tall** under Common Issues.) `resolve_styles` walks the whole document
only when a whole-document restyle asks (`NodeTree::full_style_walk`) — an
empty `style_roots` means nothing to do, not "walk everything".
`crates/rinch-dom/tests/viewport_restyle_tests.rs` is the twin oracle.

**Style invalidation: what a change restyles** (`style_resolution/invalidation.rs`,
`resolve.rs`). An attribute, class, id or `:hover`/`:focus`/`:active` change used
to re-cascade the element's whole subtree whatever the selectors said, and never
a sibling — so `+`, `~`, `:nth-child`, `:first-/:last-/:only-child` and `:empty`
went stale on class toggles, insertions and removals. Now:

- **Attributes and state go through Stylo's own invalidator.** `set_attribute`,
  `remove_attribute`, `set_styles` and `update_hover`/`update_focus`/
  `update_active`/`set_focus_visible` take a `ServoElementSnapshot` **before**
  the change (`note_attribute_change` / `note_state_change`, once per element
  per resolve — except that a subtree re-inserted and cascaded on the spot
  mid-frame, `resolve_inserted_subtree`, forgets its elements' snapshots, so a
  later change in the frame snapshots the state that cascade saw: kept, a class
  added, the element moved, the class removed read as no change, #956);
  `resolve_styles` runs Stylo's `StateAndAttrInvalidationProcessor`
  over all of them (`process_snapshots`), which marks exactly the elements a
  selector's answer changed for — self, descendants (e.g. `.card:hover .title`)
  or later siblings — with restyle hints. One change from Stylo's own: the
  descendants of a `display: none` element are processed too
  (`EveryDescendant`), because rinch styles hidden subtrees and a change made
  while hidden must land (#703). `:checked` is element state
  (`stylo_impl::element_state`); `:disabled`/`:enabled`/`:link` are read from
  attributes, so `disabled` and `href` force self + subtree + later siblings.
  `style`, `rows` (textarea), `start` (ol), `value` (li) and any attribute on an
  element with generated content (`content: attr()` now works) force what they
  feed. `:nth-child(… of S)` needs nothing extra: Stylo's maps record the
  selectors inside `of S`.
- **Child-list changes use selector flags.** Matching runs with
  `NeedsSelectorFlags::Yes`; `note_child_list_changed` restyles the siblings the
  parent's `HAS_SLOW_SELECTOR*` / `HAS_EDGE_CHILD_SELECTOR` /
  `HAS_EMPTY_SELECTOR` flags say an insertion or removal can reach, and an
  `<ol>`'s later `<li>` markers. `:empty` follows the spec: no element child
  (generated boxes aside) and no non-empty text.
- **An inserted child stays between the generated boxes** (#1629). A
  `::before` (or list marker) is generated as its element's first child and an
  `::after` as its last when the element is cascaded, and inserting a child
  does not re-cascade the element, so `append_child`, `insert_before` and
  `insert_child` clamp the position they computed between the two
  (`RinchDocument::clamp_between_generated`, which reads the first and last
  child only; `Node::is_generated_after` says which end a generated box
  belongs to). `get_children` and `next_sibling` still list the generated
  boxes, so an index or reference read from them names a real slot: an insert
  before the `::before` box lands after it. An append used to land after the
  `::after` box — every `rsx!` row added after its parent was attached.
  `set_text_content` / `set_inner_html` replace the element's own children
  only (`RinchDocument::own_children_range`) and keep its generated boxes
  (#1630); they used to drop them until the element was next re-cascaded.
- **A move within one parent keeps its style and its own text layout** (#914)
  — a keyed `for` reorder, which moves a live row with `insert_before`. Its
  ancestor chain is unchanged, so only a positional selector can see the move,
  and the two `note_child_list_changed` calls (old index, new index) mark the
  moved node itself whenever the parent's flags say one can
  (`keeps_style_across_move`). Nor does a move drop the node's **own** IFC
  layout, only the one it was a member of (`invalidate_ifc_left_by`): a
  typography change its new position brings arrives through the cascade's
  text-input comparison like any other. A move to **another** parent still
  re-cascades the subtree. Before, every moved row was re-cascaded and
  re-shaped — twice per row on a reversed list.
- **The cascade propagates, it does not blanket.** `resolve_styles_recursive`
  cascades a node that has no style or a hint, then asks `child_cascade(old,
  new)`: children follow when an inherited struct, a custom property, the
  inherited flags, zoom, writing mode or `display` moved; only
  `INHERITS_RESET_STYLE` children when just reset properties moved; none when
  nothing did. It descends elsewhere only along `style_dirty_descendants`. A
  colour-only `:hover` cascades **one** element. The `*_sensitive` flags are no
  longer cleared on re-cascade (a card re-cascaded alone would lose the flag its
  descendants' matching set).
- **Non-cascade inputs of `apply_stylo_styles_to_taffy` are re-synced
  explicitly**: a node that starts or stops being a containing block re-syncs its
  absolute descendants (ICB sizing), alongside the resize/floor/select re-syncs
  above.
- **Per-element cost**: a bloom filter of the ancestors (pushed and popped by
  the walk) rejects descendant combinators early.
- **`query_selector` uses the cascade's matcher, without its flags** (#1796,
  `crate::query`). `NodeHandle::query_selector{,_all}` and
  `DomDocument::query_selector{,_all}` parse with Stylo's `SelectorParser`
  (author origin) and match with `matches_selector_list` over `RinchNode`, so a
  query sees every selector the cascade does. It matches with
  `NeedsSelectorFlags::No` and no bloom filter: a query must not record
  selector flags, which are invalidation state (a flag set by a query would make
  later insertions restyle siblings no stylesheet asks for). It reads the tree
  and nothing else — no cascade, no layout, no counter. Generated pseudo-element
  nodes are skipped with their subtrees. The MCP tool's matcher,
  `testing::query_selector`, is the same matcher from `<body>` (included), with
  the old one-simple-selector matcher as the fallback for a string CSS rejects.

`crates/rinch-dom/tests/style_invalidation_twin_tests.rs` is the oracle (named
scenarios plus a seeded random differential, `RINCH_TWIN_SEEDS=n` for more).

**Two text caches survive across frames, and nothing drops either wholesale.**
Each IFC root keeps its paint layout (`Node::text_layout`) and the sizes the
measure function returned for it (`NodeTree::ifc_measure_cache`, keyed **by
root**, one size per available width, at most 8, removed when
`NodeTree::remove_subtree` frees the node). Exactly two rules drop them, and between them they must cover every
input a shaped layout is built from:

- **A restyle drops only what changed.** The cascade compares each re-cascaded
  node's old and new text inputs (`same_text_layout_inputs`: font size, weight,
  family, style, line height, letter/word spacing, colour — it is baked into the
  glyphs' brush — text-align, decoration, underline offset, transform,
  white-space, overflow-wrap, text-overflow, `overflow-x`, whether `display` is
  a flex or grid container — it decides the ellipsis, #1046 — and an inline
  box's background/padding) and calls `invalidate_text_measure_for_node` for the one
  that moved. An inherited change reaches every descendant that re-cascades,
  and each is compared on its own. There is **no** eager drop any more:
  `invalidate_descendant_styles`, `set_attribute`/`remove_attribute` and
  `set_style` used to drop every text layout under the restyled node, so a
  colour-only hover re-shaped all of it; now it re-shapes nothing and takes
  the paint-only path. The whole-document restyle a theme change runs
  (`recompute_all_styles_full`) is no exception (#913): it re-cascades every
  element and the same comparison decides, so a dark-mode toggle whose
  variables reach no text re-shapes nothing, anonymous block boxes included:
  a structural IFC pass (whole-document or scoped) re-mints a box for the same
  run (same container, same members) into the old box's slab entry and keeps
  its paint layout and measures, as a box outside the pass's scope keeps them;
  a container restyle drops them through `run_boxes`, and the content
  signature drops them for a changed run (#964, `RetiredAnonBoxes`). Generated content that is minted or freed (a `::before`/`::after` that
  starts or stops being generated, or whose text changed, and every list
  marker, which is regenerated on each cascade of its `<li>`) is the one
  restyle that invalidates regardless, because the nodes an inline layout
  names were freed and minted again; a `::before`/`::after` box that still
  generates is kept and restyled in place (#1023), so a hover on its
  originator re-shapes nothing it did not change.
- **A structural pass drops only the roots whose content moved**
  (`refresh_ifc_signatures`, after the atomic inlines are sized; a scoped pass
  signs only its own containers' roots). It hashes
  each root's members — id, parent, sibling index, text, tag, `display` mode,
  `position`, generated-ness, and an atomic inline's just-computed size — and a
  root whose signature is new or different loses its measures and paint layout
  and is marked in Taffy. It used to clear the whole measure cache, which
  re-shaped every paragraph to measure one appended row (41 shapes on a 40-row
  list; now 1). The signature is also a second net under the mutation verbs,
  whose invalidation reaches only the changed node's *parent*: text appended
  inside a `<span>` inside a paragraph, and a chip resized during a structural
  pass, both left the paragraph painting its old line before it.
  **A chip resized outside a structural pass marks its root's signature
  stale** (`IfcRootMeasures::stale`, set by `forget_ifc_measures_and_signature`
  from `remeasure_dirty_atomic_inlines` and `invalidate_ifc_of_atomic_inline`):
  the root is measured again at the new size while its signature still hashes
  the old one, so a chip that went back to that size made the next pass's
  signature match and the sizes measured in between were served. The next
  pass that signs the root treats it as changed (review of #1627). A new path
  that moves an atomic inline's size and drops its root's sizes must use that
  call, not `forget_ifc_measures`.
- **`invalidate_ifc_root` marks the root in Taffy on every route** (and its
  #466 measure leaf). An inline member is detached from Taffy, so a drop that
  came through one — a `span`'s own font change, `set_text_content` on text in a
  `display: contents` wrapper — marked nothing the compute walks, and Taffy
  served the root its old size (**#878**).
- **A text run's IFC root can be a box no element holds.** Beside a
  block-level sibling a run is laid out by an anonymous block box, and inside a
  split inline (#513) by the box around its fragment — neither is in the
  element tree, so no ancestor walk from the restyled element finds it, and a
  text node is never cascaded on its own. `invalidate_text_measure_for_node`
  therefore also drops each **text child's own `ifc_root`**; without it a font
  change on `div > ["t", li]` left the box at its old line height and glyphs.
  Such a box takes its `text-overflow: ellipsis` from its block container —
  its `parent`, for a split span's runs too — because it carries only the
  inherited properties and neither `text-overflow` nor `overflow` is
  (`ellipsis_style_owner`, #1071; Chrome 153 draws the container's "…" on
  text beside a block child). So a restyle of the container also drops every
  box in its **`run_boxes`**: a split span holds its runs' text, and does not
  re-cascade when the container's non-inherited properties change.
  **The "…" is decided per line, not by `white-space`** (#1091): each line
  whose content is wider than the content box after wrapping is cut and ends
  in "…", the others are kept — an unbreakable word under `white-space:
  normal`, a `nowrap` span in a wrapping root and every long line of `pre`
  text, where Chrome 153 draws one too. It used to need the root's own
  `nowrap`/`pre`. An
  inline element's `nowrap`/`pre` reaches parley as `TextWrapMode::NoWrap`
  (it used to wrap), and so does the root's own (#1212): a `nowrap`/`pre`
  root is broken at the container's width whenever the nearest element of
  some text in it wraps (`ifc::wraps_anywhere`), so a `normal` span inside a
  `pre` root wraps, and only an IFC with no wrapping text at all is broken
  unconstrained. Parley takes a break's wrap mode from the cluster before it,
  so `pre` text then a `normal` span has no break at their seam, as in
  Chrome 153. A `pre` element's preserved spaces are not taken into a
  soft-wrap hang by rinch's hang pass, and a line ending in them is aligned
  with them as content (`ifc::align_unhung_trailing` narrows its alignment
  width by the `pre` spaces at its very end), so an overflowing right-aligned
  line starts at 0 as in Chrome. On a left-to-right line aligned `right`,
  `center` or `end`, every space before the hanging run at the line's end
  counts too, whatever its `white-space` (#1042, below). One mixed seam is
  not Chrome's yet, tracked in #1243: parley still hangs the one `pre`
  space that overflows right after `pre-wrap` spaces (and on a
  right-to-left line a mixed run is still narrowed by its final `pre` part
  only). The "…" is drawn by rebuilding the paragraph as flat
  text in the root's style, which would strip every line of it, so the
  per-line cut is taken only where that draws the same glyphs
  (`ellipsis_rebuild_is_faithful`: no inline box, no background or
  decoration span, every run in the root's text style, and no `text-align:
  justify`, which lines ended by the hard breaks the rebuild joins them with
  do not get). The rebuild **keeps its text ranges**, mapped through the cut
  onto the kept prefix of each line (`ifc::map_ranges_through_cut`, #1065;
  the "…" and the line joins are in no range and take the root's style), so
  everything paint reads per range still holds on a cut line: a span's
  `visibility` (#853), its own `text-shadow` (#1065) and its live colour
  (#679, #1451) — which is why those two properties are not in the faithful
  test. Every mapped range records the root's colour, the brush the rebuild
  shapes in, so a span of another colour is recoloured at paint. Caret and
  selection mapping never read the ranges and still go through the flat
  text. Anything else keeps the old behaviour: a `nowrap`/`pre` root
  with no wrapping text is cut whole to one flat prefix (its ranges kept,
  so hidden spans, span shadows and span colours still hold) — losing its
  spans' fonts, backgrounds and decorations and its chips (dropped, left
  where they were) — and any other root (one holding a wrapping span
  included) is clipped with no "…" and
  keeps its styling. Chrome keeps both; that needs the cut made at paint time
  (#1100). The cut walks the clusters already shaped, in **logical** order
  (`ellipsis_cut_ends`; parley's `Line::runs` is visual, and summed that way
  a mixed-direction line is not cut), so `ellipsis_shapes` is 2 per rebuild
  however many lines are cut — one more when a cut line re-wraps and falls
  back to hard breaks, and one more per step back when a cut line shaped
  again overshoots the box by its broken kerning. The whole-text cut still
  shapes each prefix of its binary search.
- **An atomic inline is two roots' business.** An `inline-block` / `-flex` /
  `-grid` holding text is a member of the IFC around it *and* the root of its
  own, so `invalidate_ifc_for_node` drops both: reaching only the outer one
  left a `<button>`'s label in its old colour after a class change. A chip the
  structural pass finds **changed** is queued for `remeasure_dirty_atomic_inlines`
  at the end of that pass, because `compute_inline_block_layouts` sized it
  earlier in the pass with no sign its content had moved — and so is every
  atomic inline *containing* a root the pass found changed (a paragraph in an
  `inline-block`, a flex item's IFC in an `inline-flex`), for the same reason.
- **A node that stops being an IFC root loses its `text_layout`** in the same
  pass. Caret rects, layer bounds and the ancestor walk read that field as "is a
  root".
- `build_ifc_layouts` rebuilds a root that is dirty, has no layout, or has one
  built at another width — **every** time. It used to skip every non-dirty root
  before the width check whenever any root was dirty, so a flex item narrowed by
  a sibling's text edit kept its glyphs broken at the old width.

**The structural pass is scoped** (`crates/rinch-dom/src/ifc_scope.rs`, module
doc). An append, removal, reorder, `if` toggle, component re-render or
`display`/`position` flip no longer re-runs the IFC setup over the whole
document. The mutation verbs record **seeds** — `NodeTree::seed_ifc(node,
IfcSeed::Children | IfcSeed::Subtree)` — and the next layout re-sets-up only the
**formatting containers** they reach, with their **regions**: a formatting
container is any node that is not *transparent* (`display: contents`, or a
non-atomic `display: inline` element — exactly what
`recompute_contributes_in_flow_block`, `collect_run_units` and
`mark_inline_descendants` walk *through*), and its region is everything reached
from its children through transparent nodes, down to and including the **stop
nodes** (the formatting containers directly beneath it). Every decision the
passes make — splices, contributes/hoisting, anonymous boxes, splits, measure
leaves, roots, marks, stale contexts, signatures — is taken by one container
from its region alone, and a container's own role never depends on its content.
So everything outside the scope keeps its state *and its Taffy cache*. The one
fact that crosses a container boundary is an **atomic inline's size**: the pass
queues every atomic inline at or above a scope container, and
`remeasure_dirty_atomic_inlines` sizes them innermost-first, one at a time,
re-queueing the atomic inlines around any IFC whose chip changed size. A
`Subtree` seed sets up the whole subtree (a moved subtree has had its marks
cleared by the verb that moved it). A seed on a node **not connected to the
document** is dropped: detached subtrees are no longer set up at all, and
attaching one re-seeds it (#628's optimisation, half-landed — the
`ifc_classifier_tests` detached-route fixtures now force the whole-document pass,
which still walks detached subtrees). **Nor is a detached subtree measured or
shaped.** Every verb that takes a subtree out of the document clears its
`ifc_root` marks with `clear_ifc_root_recursive` — `set_text_content`'s orphans
included since #1073; it orphans without freeing, and its orphans used to keep
their marks, so their roots were shaped (#1069) and their atomic inlines sized
(#1040) while out. Two filters remain for what the whole-document pass marks
again: the three sizers that read the registries or the dirty set
(`compute_inline_block_layouts`, `resolve_percentage_inline_blocks`,
`remeasure_dirty_atomic_inlines`) skip an entry `depth_if_connected` says is out
of the document — the fourth, `measure_scoped_atomic_inlines`, sizes only its
scope's regions, and a scope holds no disconnected node — and
`build_ifc_layouts` drops a disconnected root's layout and registration instead
of shaping it. Anonymous boxes, splits and measure leaves
whose owner left the slab (`set_inner_html`, pseudo-element churn) are swept as
orphans at a scoped pass's start. **`tree.ifc_dirty` is now "run the
whole-document pass"** — the first layout (`ifc_full_initial`),
`recompute_all_styles_full` (`ifc_full_theme`), or any direct write
(`ifc_full_unattributed`) — so a site that still sets `ifc_dirty` directly is
slow, never wrong; call `NodeTree::request_full_ifc(reason)` to say why. A site
that changes structure and does **neither** is silently wrong: nothing sets the
regions it touched up again (the pseudo-element regeneration in `resolve.rs`
writes `children` directly and seeds its node itself — pinned by
`scoped_ifc_scenario_tests::probe_pseudo_content_removed_from_a_mixed_container`).
Every verb that moves or removes a subtree also takes each of its nodes out of
the anonymous box whose run it was in (`clear_ifc_root_recursive`), because a
scoped pass reaches the old container only while it is still in the document.
The virtualized editor's block collapse/materialise (`virtual_window.rs`) seeds
the toggled block instead of forcing a whole-document pass per scroll. Counters:
`ifc_scoped_passes`, `ifc_scope_containers`, `ifc_scope_nodes`,
`ifc_full_passes` and its reasons. The registries `ifc_root_registry` and
`atomic_inline_registry` replace the slab scans `build_ifc_layouts`,
`inline_block_measure_roots` and `resolve_percentage_inline_blocks` did every
layout; each is a superset filtered at use by the predicate the scan applied.
`copy_cached_text_layouts` walks the measure cache's nodes, not the slab. The
measure function no longer bypasses the IFC measure cache for a dirty root: it
drops that root's old sizes at the start of the compute and reuses what it
shapes itself (one shape for a new row where there were four).
`crates/rinch-dom/tests/scoped_ifc_oracle_tests.rs` is the oracle: every
mutation shape in 17 contexts, plus a lockstep randomized differential, each
comparing the scoped pass with the whole-document pass on the same history and
both with a fresh layout; its module doc lists what it found.

**`<body>`'s flex basis is `0` when its height cannot depend on it** (#1260,
`node::body_taffy_overrides`): `<html>` is a fixed viewport-sized flex column and
`<body>` its only item, so the UA sheet's `overflow-y: auto` body (a scroll
container, auto minimum `0`) with `height`/`flex-basis` auto and `flex-shrink`
above zero ends at the viewport's height less its margins, clamped by
`min-height`/`max-height`, whatever its basis. Measuring that basis was a whole
`ComputeSize` pass over everything in `<body>` per compute — every row of a
500-row list probed a second time per edit, about 22% of `rinch-bench`'s
`append_row`. `tests/body_flex_basis_tests.rs` pins the box.

Still not done, and measured by `perf_counter_baselines`: appending a row still
has Taffy *ask* every sibling row for its size once (a leaf's final-layout cache
entry is keyed on the available space, which changes when the column grows —
Taffy's, answered from rinch's measure cache); the per-pass walks
`read_layout_results`, `clamp_scroll_offsets`, `resolve_layout_calcs` and
`request_background_image_loads` are still O(document). A colour-only restyle
still drops the root's cached measures along with its glyphs. (`set_style` of an
**inherited** property used to reach no descendant with a cached style; the
cascade now re-cascades the children of any element whose inherited style moved
— see **Style invalidation** above.)
`crates/rinch-dom/tests/incremental_text_layout_oracle_tests.rs` is the pin:
each input above against a fresh layout of the final state, with the mutant
that kills it named in the PR.
