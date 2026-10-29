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
  `InlineLayout::root_color`), and `paint::text::LiveColours` draws the ranges that
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
  lines and text ranges (#1451, #1100, `rv_b1_span_on_ellipsis_line_mid_run`). Not covered: the text of an ellipsis
  line laid out by an anonymous block box keeps a stale colour, after a plain
  class change too (#1450, older than #679; the four `rv_a*` fixtures are
  `#[ignore]`d on it; its "…" takes the container's colour since #1100); a `padding` frame on an inline element still runs a
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
empty `style_roots` means nothing to do, not "walk everything". **Nor does "no
layout has completed yet" ask for one** (#1247): `resolve_styles` and the
insertion fast path `resolve_inserted_subtree` used to read
`!transitions_enabled` that way, so every insertion of an app's initial render
walked the whole document — N rows visited about N²/2 nodes, and 36,000 took
about 15.5 s to build in release (150 ms after). Only `full_style_walk` asks now
(it is set at construction, so the very first resolve still walks everything
once). **The behaviour change that follows**: before the first layout, an
append no longer applies pending style changes elsewhere in the document on
the spot; they wait for the next resolve or layout, exactly as after the first
layout. (An append *under* an element with a pending change still resolves it
first.) So a spinner hidden and shown again before the first layout, with only
unrelated appends in between, keeps its animation clock — never having been
cascaded hidden — which is what a browser does, since it styles nothing
between the two writes. `style_invalidation_twin_tests.rs` checks the
pre-layout targeted walk against a fresh document, a full re-cascade and a
forced-full-walk twin (`random_pre_layout_*`, `named_pre_layout_shapes_*`).
`crates/rinch-dom/tests/sibling_scaling_tests.rs` asserts that no counter grows
faster than the row count between 1,000 and 4,000 rows.
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
- **Stylesheet changes use Stylo's stylesheet invalidation** (#884,
  `style_resolution/sheets.rs`). `load_css`, `set_theme_css` and a `<style>`
  element's sheet coming, going or changing text record the sheet on the
  Stylist without forcing the origin dirty, so Stylo keeps the
  `StylesheetInvalidationSet` it collects (each selector's rightmost
  id/class/tag, or the outermost one left of a descendant combinator for a
  subtree). `flush_stylist` (both flush sites: `resolve_styles` and
  `resolve_inserted_subtree`, which flushes before cascading the inserted
  subtree so a later flush never invalidates it; what the flush queues reaches
  that function's existing declines — `full_style_walk`, or an ancestor with a
  restyle hint or `style_dirty_descendants` — so it needs no check of its own)
  passes
  `<html>` to `Stylist::flush` while `sheet_invalidations_pending` is set, so
  the matching elements get restyle hints and `<html>` is pushed as a style
  root for the walk to follow them. A rule matching nothing restyles nothing
  and does not relayout. A selector Stylo cannot bucket (`*`, `:root`, a bare
  pseudo-class), a `@keyframes` (the servo `Device` says every name may be
  referenced), `@property` or an `@layer a, b;` statement mark `<html>`'s
  subtree: that is the old whole-document restyle (`full_restyle_stylesheet`).
  An `@layer x { … }` block is bucketed by its inner rules. The viewport, DPR and
  theme full restyles still force origins dirty and flush with no document
  element, so they never take this path. Forcing `Author` dirty in
  `load_stylo_css` is what made a `load_css` after the first layout restyle
  nothing: it emptied the invalidation set, and the flush had no element to
  apply it to anyway. **A targeted late sheet starts CSS transitions**, as in browsers:
  the elements it restyles keep their old style data, so a changed
  transitioned property animates (a late `.leaf { width: 300px }` under
  `.leaf { transition: width 10s }` transitions). Before #884 a `<style>`
  append cleared all style data, so nothing transitioned; a whole-document
  late sheet (`:root`, `*`, …) still clears it and starts none. Theme switches still
  go through `recompute_all_styles_full` with transitions off.
- **`<style>` elements apply while connected, in tree order, after every
  `load_css` sheet** (which are the web's `<head>` sheets: `inject_style`).
  `create_element("style")` registers the id; `note_style_elements_moved`
  (called from `note_child_list_changed_with`, `note_child_inserted` and after
  `remove_node` unlinks — it ran while still linked) marks the sheets stale when
  a registered element's connectedness differs from what is installed or a
  `<style>`'s own children changed, and `maybe_load_style_css` on a text edit.
  `sync_style_element_sheets`, at the next flush, keeps the installed prefix
  that still matches (element and text), removes the rest and appends what is
  wanted, re-using a parsed sheet whose element and text are unchanged. A
  document with no `<style>` element pays nothing. Before #884 a `<style>`'s
  text was loaded the moment it was appended, connected or not, never unloaded,
  and every text edit appended another sheet.
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
  root is broken at the container's width whenever some element holding a
  text run or an atomic inline in it wraps, however deep
  (`ifc::wraps_anywhere`; the boxes and the depth since #1238), so a `normal` span
  inside a `pre` root wraps, a `normal` span of inline-blocks breaks between
  them, and only an IFC with no wrapping content at all is broken
  unconstrained. Such a paragraph is still **aligned within the content box**
  (#1237): with no width parley aligns each line within the paragraph's own
  widest line (a right-aligned `a` in a 50px `nowrap` box sat at 0, Chrome
  41.02), so its paint layout is broken line by line with the content width
  set as each line's alignment width (`ifc::break_unwrapped`, through
  `set_prior_line_width`; the content box itself, not the pixel of auto-width
  slack `ifc_paint_max_width` breaks with — except that an auto-width root
  whose paragraph is within a pixel of its content width, a shrink-to-fit
  box rounded from that paragraph, is aligned within the paragraph's own
  width, `ifc::AlignWidth`), and an overflowing line keeps parley's start
  alignment, as CSS does. Since a line's `inline_max_coord` is then the
  box's width, an overflowing line's content passes it: what reads a line's
  horizontal extent for ink takes its placed content too
  (`paint::text::line_reach`), and the paint layout is rebuilt when the
  alignment width changes as well as the break width
  (`InlineLayout::align_width`; an auto-width root and a declared one a pixel
  wider break at the same width). It is not broken *at* that width:
  parley marks a break opportunity after every inline box whatever the wrap
  mode, so a `nowrap` paragraph with a chip would wrap after it. **Soft wraps
  next to atomic inlines follow `white-space`** (#1238): parley allows
  a break after every inline box, and before one whenever the character
  before it wraps. Chrome 155 asks the `white-space` of the element the box
  (or text) is in — not the inline-block's own, which styles its contents —
  and, at an element's end, of the element it closes into: the opportunity
  after a leaf (a text run or a box) is allowed when any element from its
  parent up to the nearest one that also holds the next leaf wraps
  (`ifc::WrapSeams`). So `aa<span normal>[1][2][3]</span>cc` in a `nowrap`
  root breaks after each box but not between `aa` and `[1]`, a chip in
  `nowrap` text does not wrap after itself, and boxes in a `nowrap` span
  inside a wrapping root stay together. **A run of spaces is decided by the
  spaces' own `white-space`** (`ifc::wrap_allowed_after`, review of #1878):
  `<span nowrap>abc def </span>[box]` has no break after its space, though
  the span closes into a wrapping root. Spaces that do not wrap, in an
  element a wrapping one closes into right before them, break *before* the
  run and start the next line with it (`aa<span normal>hello</span> ccc` in a
  `nowrap` or `pre` root: Chrome keeps the space at the start of line 2;
  `ifc::Seam::closes_into_wrap`); one that opens there does not
  (`<span normal>ccc<span nowrap> mm</span>`). A wrapping space followed by a
  non-wrapping one breaks between them. An element's end padding right
  after hanging spaces hangs with them (Chrome
  fits `<span padding:0 3px>[B]q-r </span>` before a `<br>` in 71.5px, its
  content less the space and the end padding). A paragraph broken at a width that
  holds atomic inlines and any element that does not wrap gets the seams
  (`UnhungSpaces::seams`), and the hang pass re-breaks a line of it where
  CSS ends it (`ifc::css_line`, the #1426 edge-box re-break generalised:
  `LineUnit::wrap_after`) when parley's break follows a leaf `white-space`
  allows no wrap after (`wrap_allowed_after`), in a paragraph with `pre`
  seams (#1243, below) at every soft wrap that can reach a `pre` space
  (`ifc::NearPre`), when the line overflows (`line_overflows`: parley appends a
  box after text that does not wrap where CSS breaks before it), or when
  parley took its emergency break after a box too wide for the line that
  `white-space` glues to what follows (Chrome: `[B]cc` stays together) — not
  an `overflow-wrap` line. Inside text a re-broken line asks
  `ifc::TextBreaks`, the segmenter parley uses (`LineSegmenter::new_dictionary`,
  so Thai, Lao, Khmer and Burmese keep their word breaks), less a break
  between a complex-context (`SA`) character and a letter or digit, which
  ICU4X reports and UAX #14 (and Chrome) does not. ICU4X's dictionaries
  still split some words Chrome's keep whole (`ខ្មែរ|ភាសា`, `ລາວ|ພາສາ`), so a
  re-broken line can break inside them. Pins:
  `tests/atomic_inline_soft_wrap_1238_tests.rs` (Chrome 155) and
  `tests/atomic_inline_soft_wrap_1238_differential.rs`, a checked-in Chrome
  155 corpus of 437 random trees (`tests/soft_wrap_1238/chrome155.tsv`, its
  known differences pinned). Only for `right`,
  `center` and `end` in a left-to-right block, and only in the paint layout
  (`build_inline_layout`'s `align_width`; a measure takes only sizes from its layout).
  Parley takes a break's wrap mode from the cluster before it,
  so `pre` text then a `normal` span has no break at their seam, as in
  Chrome 153. A `pre` element's preserved spaces at a line's end are
  aligned as content: parley 0.12 hangs them only as far as they overflow,
  so an overflowing right-aligned line starts at 0 as in Chrome (rinch
  narrowed the alignment width for them until then, `align_unhung_trailing`). On a left-to-right line aligned `right`,
  `center` or `end`, every space before the hanging run at the line's end
  counts too, whatever its `white-space` (#1042, below). A paragraph
  broken at a width that holds a `pre` element's spaces or tabs
  (`RinchDocument::any_pre_space`) takes the seams path above as well when
  parley's breaks can be wrong there (#1243; `ifc::pre_seams_needed`, asked
  once parley has broken it, `UnhungSpaces::pre_seams`): a `pre` run that
  starts with a space right after text that wraps, or a soft-wrapped line
  parley ended in a `pre` space or right before `pre` text. parley has no
  break between a wrapping run of spaces and a `pre` one (UAX #14 has none
  between two spaces), and it hung the one `pre` space that overflowed,
  breaking the `pre` run after it. A paragraph whose `pre` spaces nothing
  wrapping touches pays no pass for them
  (`perf_regression_scenarios::pre_spaces_nothing_wrapping_touches_pay_no_pass`),
  and one that takes the seams scans only the lines that can reach a `pre`
  space (`ifc_css_line_scans`,
  `a_pre_wrap_paragraph_with_pre_spaces_scans_only_lines_near_them`).
  Chrome 155 breaks between the two runs — the first hangs, the `pre`
  spaces start the next line — but how depends on the text nodes the
  wrapping run lies in (`ifc::pre_seam`): an ordinary break when the
  wrapping spaces' element closes there into one that does not wrap, when
  the run starts in the word's text node and goes on into another, or when
  the run is collapsible spaces right after an element that closes there
  (`<span>dd</span> <span pre>    dd</span>` in a `normal` root); only when
  what is before it fits when the run ends the word's text node
  (`aaaa  <span pre>  </span>bbbb` breaks in 50px,
  `hhhhhhh  <span pre>  </span>bbbb` does not); and only when the line
  overflows before the `pre` spaces when the run is a text node of its own
  (`aaaa<span pre-wrap>  </span><span pre>  dd</span>` does not break,
  `hhhhhhh<span pre-wrap>  </span>…` does). In such a paragraph only, a
  wrapping space then a `break-spaces` one break between them too, in
  `css_line`'s scan; a paragraph with no `pre` spaces is laid out as before
  #1243. A line whose trailing run of spaces has `pre` spaces before the
  part that hangs is aligned with them as content (parley's since 0.12). A text node whose spaces collapsed away
  between the wrapping run and the `pre` spaces counts as the run going on
  into another text node when the run's element closes into it
  (`<span>dd </span> <span pre>`; `UnhungSpaces::collapsed`), not when it
  is a second text node of the same element, a sibling element's, or after
  a comment (rsx makes adjacent text nodes; Chrome 155 does not break
  there). The seams' break
  opportunities (`ifc::TextBreaks`, ICU4X's segmenter, the one parley uses)
  are Chrome's where they differ for ASCII: no break between `!`, `/`, `|`
  or `}` and a letter or digit after it (`ifc::chrome_glues_ascii`;
  `<span pre>cccc!</span>dddd` stays whole), on the #1238 box seams and the
  #1426 edge-box path too (the review's no-`pre` corpora: 128 of 4,500
  cases now match Chrome that did not, none stopped matching). parley's own breaks elsewhere still split there
  (#1910). Pins: `tests/pre_space_seams_1243_tests.rs` and
  `tests/pre_space_seams_1243_differential.rs`, a checked-in Chrome 155
  corpus (`tests/pre_space_seams_1243/chrome155.tsv`, its known differences
  pinned). Not Chrome's yet (#1898): in the review's family corpus,
  8 of 78 `pre-wrap` and 11 of 42 `break-spaces` base cases, a word of a
  `pre` element before the run, six random cases with an inline-block whose
  breaks differ and three whose line after an inline box starts at the
  box line's far end, and on a right-to-left line a
  mixed run is still narrowed by its final `pre` part only. **The "…" is cut at paint time, over the lines as laid out**
  (#1100; `ifc::ellipsis_cut`, `node::EllipsisCut`, `InlineLayout::ellipsis`):
  on each overflowing line everything whose visual extent passes the point
  where the "…" still fits before the line's end edge is hidden — each
  grapheme whole, a ligature with every cluster its glyph covers (paint
  draws a glyph with its first cluster, so a cut inside one drew the whole
  glyph under the "…"), through every `TextMask` an IFC paint draws with
  (text, decorations, shadows, wavy underlines: `TextMask::cut`), and each
  atomic inline whole — and the "…" is drawn right after what is kept, in the
  block container's font and current colour, undecorated, casting its
  `text-shadow` (an anonymous block box's container's, #1071). Inline
  backgrounds and borders are not cut: a pill keeps its extent and the
  root's clip cuts it at the padding edge, where Chrome 155 draws it, with
  the "…" over it. The end edge is the side the line leaves the box by: the
  right, or the left for an overflowing right-to-left line (parley infers
  the direction from the text; `direction` is not read, #1666), which parley
  aligns left of the box; an unconstrained (`nowrap`/`pre`) right-to-left
  line is not aligned against the box (it starts at the box's left edge)
  and leaves by the right, so its "…" goes there and what shows is its
  visual left, where Chrome under `direction: rtl` shows its logical start.
  Nothing but the "…" is shaped, so `ellipsis_shapes` is 1 per layout
  however many lines are cut. A paint walks a line cut at its right only up
  to the cut (`EllipsisMark::hidden_past`, `paint::text::shown_clusters`:
  the glyph collection, the hidden-glyph cursor and the shadow extent stop
  there; the limits are by line index, since parley's text ranges wrap past
  65,535 bytes, #1267), so the glyphs collected do not grow with the hidden
  text (`paint::take_text_glyphs_collected`); parley's own `Line::items`
  still walks each run's glyphs once per collection, and a line cut at its
  left is walked whole. Text offsets, carets, hit testing and span
  fragments read the lines, so they are those of the uncut text: a press
  past the cut maps to the hidden text under it, and a hidden atomic
  inline keeps its box, which hit testing is not told is hidden (#1864). The clip proof
  measures an ellipsised root's text no wider than its box
  (`InlineLayout::painted_width`), so a plain ellipsised title still elides
  its clip (`clip_elision_tests`). Until #1100 the "…" was drawn by
  rebuilding the paragraph as flat text in the root's style, which lost
  every span's font, background and decoration and every chip, so a
  paragraph whose rebuild would not have been the same picture was clipped
  with no "…". Pins: `tests/ellipsis_paint_cut_1100_tests.rs` (Chrome 155
  pixels), `ellipsis_overflowing_line_tests.rs`.
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
