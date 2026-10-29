# Rendering backends, damage, scrollbars, viewport holes

Read this page when your task touches: the painters, damage/dirty regions, software caches, GPU partial frames, scrollbars, viewport holes.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Features

### Rendering Backends

Rinch has two rendering backends. Every desktop build carries the software one;
the `gpu` feature adds the GPU one, and the window picks between them at run time:

| Backend | Feature | Renderer | Presentation |
|---------|---------|----------|--------------|
| **GPU** | `"gpu"` | Vello + wgpu | GPU compositing |
| **Software** | (always on desktop) | tiny-skia | softbuffer |

Set in `Cargo.toml`:
```toml
# GPU, with software as the fallback and on request:
rinch = { workspace = true, features = ["desktop", "gpu"] }

# Software only:
rinch = { workspace = true, features = ["desktop"] }
```

On a `gpu` build, `App::renderer(Renderer)` chooses (`shell/renderer.rs`):
`Renderer::Auto` (the default) creates the GPU renderer when the window opens
and presents with software, logging a warning, when the GPU will not start (no
surface, adapter or device, or a validation error while configuring them);
`Renderer::Gpu` panics instead; `Renderer::Software` never touches wgpu. The
`RINCH_RENDERER` environment variable (`auto`, `gpu`, `software`, or `cpu` as
an alias of `software`; trimmed, any case) overrides the app's choice; an empty
value is ignored and an unrecognised one is logged and ignored. **An app that
configured the device itself (`gpu_config`, `external_gpu`) always presents on
the GPU**: it gets no fallback (a GPU that will not start panics), and a
`Software` choice — the app's own or a user's `RINCH_RENDERER=cpu` — is logged
as a warning and ignored (`renderer::tries_gpu`), because its pipelines need
the device `gpu_handle()` hands it. Everywhere else `gpu_handle()` is `None`
for the whole session while software presents. Whether the GPU compositor is
presenting is a run-time question on a `gpu` build
(`renderer::gpu_presenting()`), never `cfg!(not(software_shell))`.

Both use the same `Painter` trait (`crates/rinch-dom/src/paint/painter.rs`):
- `VelloPainter` — records commands into `vello::Scene`
- `TinySkiaPainter` — rasterizes directly to RGBA pixmap

The software renderer includes **dirty region caching**: when only a few nodes change, only the affected rectangular area is cleared and repainted. Subtrees outside the dirty region are skipped during paint traversal.

**The damage is named, and it is a short list of rects** (`paint::compute_damage` → `paint::DamageRegion`). A software frame repaints what its damage names: the paint-dirty nodes, removed nodes' rects, and the drag ghost's and inspect highlight's old *and* new rects (`last_ghost_rect`, `last_inspect_rect` — both translucent, so their rect is repainted every frame they are up). **A frame in which nothing was marked paints nothing** (`repaint_none`): an empty damage used to mean "repaint everything", which made every alt-tab, focus move, selection clear, IME preedit and scroll into view a full repaint and hid sites that forgot to name their node. So a change that reaches pixels without a `DomDocument` write — a shell-written attribute such as `data-text-sel`, `data-focused`, `data-preedit`, a `scroll_offset` — must call `NodeTree::mark_paint_dirty` on its node. **A marked node that names no rect of its own still damages something**: `compute_damage` falls back to the ancestor that paints it (`boxed_owner`: the `<select>` for an `<option>` or its text, else the nearest ancestor with a box), and names nothing only for a node that is detached or has a `display: none` *ancestor* (its own `display` is not asked: a span that has just become `display: none` still has last frame's glyphs on screen). An IFC root whose text layout is rebuilt, or that stops being a root, is pushed paint-dirty too: new glyphs are a paint change whether or not its box moved. "Stops being a root" is read from the root's IFC signature, not from its `text_layout`, which a move verb has usually dropped already — a block whose only span was moved to another parent kept its glyphs on screen until #1064. `<style>`, `<script>`, `<head>`, `<meta>`, `<link>` and `<title>` compute `display: none` (the UA sheet's value; `apply_stylo_styles_to_taffy` skips them and used to leave the default `Flex`, which made a `<style>`'s CSS text an IFC line that flipped its box every structural pass and dragged the fallback to the page root). A box-less element with children (a `display: contents` wrapper, a 0x0 positioned anchor) names its subtree's painted bounds now *and* where they were last painted, so a moved tooltip anchor takes its child's old pixels along. `RinchApp::request_repaint` schedules a frame whose damage is named; `RinchApp::mark_scene_dirty` says "something changed, I don't know what" and repaints in full when nothing else is damaged (`repaint_full_unattributed` — the framework's own caller is window re-creation; the software `GameViewport` frames were one until #361 painted them inline with the viewport's box as their damage). A whole-document restyle (`NodeTree::note_full_restyle`: stylesheet, viewport, DPR, root font-size, theme) sets `whole_document_damaged` and repaints in full (`repaint_full_restyle`). The rects are up to `MAX_DAMAGE_RECTS` (8), pairwise disjoint, merged only where they overlap or one rect wastes little; each is cleared on its own, paint prunes subtrees touching none of them, and draws through one clip — their union as a path, opened with `TinySkiaPainter::push_damage_clip`, not a plain `push_clip`. That clip is not a mask to what it contains (#1007): tiny-skia draws an opaque fill through a different pipeline with a mask than without one (`Source` + lerp against scale + `SourceOver`), and the two round an anti-aliased edge pixel one LSB apart, so while it is the only clip open, a fill or stroke lying inside one damage rect is drawn unmasked, as the full repaint draws it. Any clip pushed above it — one `push_clip` skips as covering the damage included — ends that until its pop; an opacity layer does not. A box that straddles a damage rect's edge is still masked, so its opaque edge pixels inside the damage can still be one LSB from a full repaint's. Pins: `crates/rinch-dom/tests/damage_clip_pipeline_tests.rs`, `damage_clip_escape_tests.rs` (nothing drawn unmasked leaves the damage, miter spikes included), and `r2d-round` in `game_viewport_inline_tests::fresh_frames_match_the_reference_draw`. "Too big" is the **sum** of their areas against `FULL_REPAINT_FRACTION`. Pins: `crates/rinch/src/app/named_damage_tests.rs` (a local pixel oracle per converted site, each killed by removing that site's damage) and `rinch_dom::paint::damage`'s unit tests.

**The desktop GPU window repaints its damage too** (#1718). `RinchApp::build_gpu_frame` makes the same decision `build_pixels` does (`frame_damage`, shared) against the renderer's own state (`WgpuRenderer::ui_invalid_reason`: the UI texture has never been rendered, was resized, or was rendered over another base colour — composite layers came or went), and returns `GpuFrame::Full`, `Unchanged` (nothing rendered, the UI texture presented again) or `Partial`: a scene encoded with the damage's prune (`set_dirty_rects`, no clip) that `desktop::render_partial` renders **untranslated** into a target reaching from the surface origin to the damage's far corner and copies each damage rect from into the UI texture. Untranslated because Vello's output depends on where the target puts a pixel: a scene translated by 31px drew a glyph edge 7 levels off. Vello renders before the surface is acquired, so a skipped present loses no damage. Inline `RenderSurface` frames mark their nodes paint-dirty on GPU as on software. `build_scene` (embed, Android, DevTools) still encodes everything; after a partial frame it re-encodes (`scene_is_full`). Measured (1200x800, RADV 890M, a 170-row `Tree` beside a long document, hover sweep): full encode 0.3–0.7 ms + Vello 1.2–2.3 ms per frame, partial 0.17–0.32 ms + 0.54–0.92 ms; about 0.4–1 ms of Vello's cost is fixed per render whatever the target. Pins: `app/gpu_partial_frame_1718_tests.rs` (GPU-free frame decisions; `#[ignore]`d pixel identity with a full render over a hover and scroll sweep, and the probe).

**A dirty region clears what the last paint drew, not what the tree says now.** Each node keeps what it was last *painted* with — `prev_layout` (its box) and `Node::painted` (its own `box-shadow`/`outline`/`text-shadow` reach — the last is one and a half blur radii (three standard deviations, where the blurred mask of `paint/text_shadow.rs` stops) around its offset, measured from the border box, so text that overflows its box casts past it; an IFC root's reach also holds the `text-shadow` lists its inline elements declare, since it draws their text (#1048; on a line `text-overflow: ellipsis` cuts too, #1065), and what it draws past its unshifted line boxes — `vertical-align`-shifted text, its inline elements' backgrounds and borders (which reach past the line by their padding and border), wavy underlines — from `paint::text::inline_ink_past_lines`, the function the clip proof reads (#1815, #1817; a root that clips its overflow counts none of it, and shifted text casts its shadows from where it is drawn) — its own transform, and whether it clipped and how it was positioned — #909) — all **parent-relative**, and only the paint that consumes the region writes them (`NodeTree::consume_paint_dirty`, O(1) per dirty node — O(text runs) for an IFC root, and a box-less inline's consumption re-reads its root's ink — always, not only while an inline element casts its own `text-shadow`, since a transition writes an inline's padding with no re-shape and the root would keep a pill's old reach (#1815) — once per root per consume, however many of its members were dirty, and not when the root was dirty itself; each span walks only the lines its byte range is on (`inline_span_line_visits`, pinned in `perf_regression_scenarios`); the software dirty-region frame, a full software repaint and `build_scene` all call it). `paint::previous_painted_rect` sums them up the box-tree chain, so a node's old rect is exact however many resolves ran since, when an *ancestor* moved rather than the node, and when its ink or transform changed in the same frame. `compute_dirty_region` takes, per dirty node, that painted rect and its current box grown by its current own ink (a box-less inline names its IFC root's rect, grown by that root's ink, where it is and where it was painted); a **positioned** box (`absolute`, `fixed`, `relative`) whose position changed is also grown by its subtree's reach (`opacity_layer_bounds`), which is what carries an untouched overflowing child along with a dragged panel or a reflow-shifted `relative` row (a subtree that walk cannot bound repaints in full). Static boxes are not walked — a reflow shifts every row — so a static box's in-flow child whose own ink reaches past a reflow-shifted parent is not covered (#880's probe J2, also on main); **a box whose place in the paint order changed is walked whatever its `position`** (#1384): `PaintedState::order` keeps a `stacking::PaintOrderKey` — whether the box was an entry of an ancestor's sequence (`paints_at_stacking_root`), the `z-index` that entry sorted by, and whether it was a stacking context — and `paint::reorders_subtree` compares it with the node's answer now, which is how a restyle and a transition or animation tick (which writes `computed_style` with no cascade) are caught alike. **Any** difference owes the subtree's reach, now and where it was painted. There is deliberately no shortcut for a positioned `z-index: auto` box that becomes a context at `z` 0 with its own entry staying put: besides trapping descendants that sort away from 0, becoming a context starts group opacity over everything under it and puts its entry's clip chain around absolute descendants that otherwise escape it (a fixed one escapes it either way, #549) — a shortcut on "no `z-index` descendant" left all of that stale (review of #1399). The rects are clipped by the box's own clip chain, unless something under it is absolute or fixed against a box above it (`subtree_has_escaping_absolute`; skipped when the box is and was a containing block) — wherever that descendant sits, inside the box's own rect included: then they take the chain an absolute box at the node would have, so the clippers such a descendant escapes are dropped and the ones above its containing block still clip. A flip scrolled wholly out of a clipper repaints nothing unless it holds such a descendant. **Three things repaint the whole window on a flip frame.** A subtree the reach walk cannot bound: a `position: fixed` descendant, `sticky`, or **more than its 512-node budget** — so a flip (a fade crossing opacity 1, a transform starting) on a box holding more than 512 nodes repaints in full, where `main` repainted the box's rect (a 200x200 clipped list of 300 two-node rows: 240000 px against 41616; the first and last frame of a transition, not the ones between). An absolute with a clipper between it and its containing block, that clipper being under the flipping box (`subtree_has_partially_escaping_absolute`): the reach walk narrows it at that clipper (#550), so the reach does not hold it. And a flipping box that itself clips, or clipped when painted, is not a containing block both now and then, and has an absolute under it with no containing block in between (#1408: the box started or stopped clipping that descendant in). The two escape walks are linear in the subtree and **not** bounded by the 512-node budget: the reach walk answers for an `opacity: 0` or collapsed (`estimated_height`) child without entering it, and the escape walks skip only `display: none` (20000 rows under an `opacity: 0` child: 4.1 ms against 1.0 ms for a colour change, debug build). A node whose order held pays one compare and no walk. Before it, opacity crossing 1, a `z-index` or `position` change, or a transform starting left a `z-index` descendant that overflows the box in its old order over a sibling (10000 of 10000 px). Not covered: an `opacity`, `filter` or `transform` **value** change on a box that stays a stacking context still damages only the box's own rect, so in-flow content overflowing it keeps its old pixels (#1395, pre-existing); and the region stops being measured once it reaches `paint::FULL_REPAINT_FRACTION` of the surface. A removal records each removed node's painted rect, children before the parent's state is forgotten. This is what let the editor overlays (caret, selection, node outline) and the inset fast path (#280) stop forcing a full repaint per keystroke and per drag move: a second resolve used to overwrite the old rect, and the forced full repaint also hid a move and an ink change landing in one frame (#880's review). An IFC's direct write of an atomic inline's position pushes the box paint-dirty when it moves, so a chip moved by the text before it is repainted, overflowing children and all. `build_scene` also sets `has_previous_frame = false`, so a software frame after a GPU one is full. The paint prune (`paint_node`, `paints_nothing_without_visit`) tests each box grown by the same own ink, so damage over a shadow's or an outline's bleed and not its box repaints it there (#889 item 1) — except an atomic inline's (`inline-block`/`-flex`/`-grid`): it is drawn by its IFC root, which is culled by its own box, so damage over only an inline-block's shadow bleed below its line is still not repainted (pre-existing; #889). Not covered: the `text-shadow` of text that overflows its box, a `filter` reach, and the #550 partial escape of an absolute from an `overflow` box inside a subtree that **moved** without changing its paint order (a flip repaints in full for it, above). Pins: `crates/rinch/src/app/repaint_old_rect_tests.rs` (and `build_scene_consume_tests.rs`, `--features embed`), `stacking_flip_damage_1384_tests.rs`, `stacking_flip_damage_1384_review_tests.rs`.

**Damage is clipped by what clips the damaged box** (#909). Every rect `compute_damage` names for a node — where it paints now, where it was painted, its IFC root's or owner's box — is intersected with that node's clip chain (`paint::clip_chain_bounds`) before it joins the region, and a removed node's rect is intersected when it is recorded; so a row a scroller moved below its viewport names nothing, and a keyed `for` change in a 300x400 scroller repaints 304 x 404, not the column down to the window's edge. The walk mirrors what clips a box in paint and errs wide in three places: a `position: fixed` box takes no clip above it (paint still applies the ones above a transformed, filtered or masked ancestor, and a transformed or filtered one's own, #549), an `absolute` box skips every clipper below its containing block (whose own clip it takes, as `Collector::span` does), and a `display: contents` or degenerate clipper is not taken. The **current** chain answers for where a box paints now — it is exactly what the next paint clips with. The **painted** chain answers for its old pixels from each node's `PaintedState` (`clips`, `position`, transform) and `prev_layout`, walked along the **current** parents. Two things make that true. Every style change pushes its node, so a painted state holds between two consumptions; and the paint consuming a whole-document restyle, which pushes nothing, re-reads **every** painted node's state (`NodeTree::consume_paint_dirty` — O(n) on a frame that has just restyled and repainted all n, and it also stops ink and transform going stale after a theme change). And a move to another parent (`append_child`, `insert_before`, `insert_child`, `replace_node`'s incoming node) records the subtree's old rects under the old chain and forgets its painted state first (`record_pixels_left_by_move`), exactly as a removal does, so a moved node is not placed along parents it was not painted under; a reorder within one parent and a never-painted or parentless node pay nothing. A node never painted gives no clip. Every detach does the same before the subtree leaves (#966): `remove_node`, `remove_child`, `replace_node`'s displaced node, `set_text_content`'s orphans, and what `set_inner_html` and the pseudo-element pruning free — they used to repaint only the parent's box, leaving an overflowing child or a shadow on screen, and an orphan appended elsewhere had its old rect placed (and clipped) along its new parents. `Frame::Painted` places a box by the `position` it was **painted** with (`PaintedState::position`, #961), so a box that goes static↔fixed in one frame clears where it was — except that the painted frame still reads the **current** IFC content-box offset (`ifc_content_box_offset`), so an atomic inline that goes out of flow in a padded IFC root keeps its old pixels (#1621); a box that becomes or stops being a containing block changes its paint order, which the #1384 rules above already answer (in full when it clips, #1408). A rect clipped away entirely still counts as *named*, so the `boxed_owner` fallback does not repaint the clipper's box in its place. The contents-wrapper subtree rects are not clipped (one chain cannot speak for a subtree whose boxes may escape it). Pins: `crates/rinch/src/app/damage_clip_chain_tests.rs`, `damage_position_change_961_tests.rs`, `detach_damage_966_tests.rs`.

**The software painter's caches.** `TinySkiaPainter` keeps three things across draws and frames (`paint/skia_painter.rs`). **Rasterised glyphs**, keyed by font blob id, face index, exact physical size, hinting and variation coords — and deliberately **no sub-pixel offset**, because swash rasterises each glyph at the origin and the image is placed by nearest-neighbour sampling, so no pixel depends on it; a change that renders at the fractional offset must add a bucketed offset to the key. Bounded by clear-on-threshold at about 16 MB of *accounted* bytes (image bytes plus 128 per entry for the map slot, spare capacity and allocation; measured on small glyphs the real overhead is about 118, so the estimate errs high) or 512 runs; swash also gets one stable `CacheKey` per font file, where `FontRef::from_index` minted a fresh one per call and defeated swash's own scaler cache. A colour **bitmap** glyph (CBDT/CBLC — Noto Color Emoji, Android's emoji — or sbix) is cached the same way (#804; it painted nothing before): read at its strike's own ppem (the smallest at least the font size, else the largest), premultiplied, then area-averaged to the font size (`color_bitmap_glyph`, `scale_strike`) — premultiplied first because swash's own resize filters straight alpha and drags the colour under transparent pixels into the edge; a face with no colour strikes takes the one swash `Render` it always did. The GPU path (Vello) draws bitmap strikes itself. **Clip masks and layer pixmaps** stay surface-sized (tiny-skia takes a mask only at the pixmap's size) but come from a pool and carry the rect that can be non-zero: a clip is filled, intersected and later zeroed over its own bounds — and not at all when its shape (a rect or rounded rect, kept axis-aligned by its transform) covers every pixel the enclosing clip can let through, since the intersection would be the enclosing mask byte for byte, and filled but not intersected when the enclosing clip is 255 wherever it can be non-zero: a partial repaint inside a scroller no longer fills the scroller's whole box, and one around a scroller no longer intersects it (#907; the painter tracks each mask's fully covered rect, and debug builds check every such claim against the mask itself) — and a layer composites back — and is cleared — only over the area its draws touched (the painter's own bookkeeping; it still ignores `push_layer`'s bounds, so tiny-skia stays blind to `layer_bounds` bugs). **Premultiplied images** live on `DecodedImage::premultiplied()`, made on the first software paint; a live frame source is still premultiplied per draw — unless `SurfaceWriter::submit_frame` found every pixel opaque (checked on the submitting thread, `SurfacePixelData::opaque` / `PaintImage::opaque`), in which case it is never premultiplied, and a draw with no rotation or skew, a positive scale (scaled draws included), whole-pixel destination edges and no clip partial where it lands is a row copy (`TinySkiaPainter::copy_opaque_image`, #361; byte-identical to `draw_pixmap`, pinned by `opaque_image_copy_tests` and the randomized `opaque_image_copy_differential_tests`; counter `opaque_image_copies`). The frame's pixels are an `Arc` shared between the surface and paint (`SurfacePixelData::data`), so collecting a frame for a paint copies nothing. **What stays resident:** the glyph cache (above); each translucent cached image's premultiplied copy, beside its straight-alpha pixels for as long as the image is cached (an opaque image keeps none); and the pooled surface buffers — at most 8 masks (1 byte/px: 8.3 MB each at 4K, 2.6 MB at 1080x2460) and 8 layer pixmaps (4 bytes/px: 33 MB each at 4K, 10.6 MB at 1080x2460). The pool is trimmed at the end of every software frame (`TinySkiaPainter::end_frame`, called by `build_pixels`) to the most either kind had open at once in the last 8 frames, so a deep nesting is given back 8 frames later; an app that goes idle straight after one keeps what that frame needed until it paints again, is resized, or is dropped (`paint_surface_trims` counts releases). Measured at 1200x800, release, `skia_painter_perf_tests` (before → after): a 40-paragraph text page 116 → 5.4 ms, 40 rounded scrollers 112 → 6.1 ms, 30 opacity layers 205 → 7.9 ms, 60 images 5.6 → 2.4 ms, a 200x40 partial repaint 5.6 → 0.23 ms. What remains of a text frame is the per-glyph `draw_pixmap` (about 0.85 µs a glyph). `TinySkiaPainter::set_reference_mode` turns all of it off, and `skia_painter_oracle_tests` demands byte-identical pixels against it, cold and warm; counters `glyph_cache_*`, `clip_mask*`, `paint_layers`/`layer_px`, `paint_surface_allocs`/`paint_surface_trims`, `image_premultiplies`.

**Scrollbars.** A scroll container paints an overlay thumb on each axis that is scrollable (`overflow-{x,y}: scroll | auto`) *and* overflowing — 6px thick, 2px margin, 20px minimum thumb (but never longer than its track: a track shorter than 20px is filled by a thumb that cannot travel, #1141), fully rounded, and a neutral 40% that follows the container's palette (see **Styling the bar** below). Both bars are hit-tested over a wider 16px strip along their edge and can be dragged (issue #178). A press on the **thumb** grabs it where it is and only the motion after it scrolls; a press on the empty **track** jumps, mapping its position along the track linearly onto the scroll range (#443 — it used to jump on the thumb too, so grabbing a thumb moved it before the drag began). `ScrollbarTrack::thumb_contains` is the one "is this on the thumb" question, asked by the press and by the resize-zone rule below. Where both are present the bottom-right **corner belongs to neither**: a 16px square (`scrollbar::HIT_THICKNESS`, the hit strip's width) that each painted track gives up at its far end and each hit strip stops at (`Scrollbars::hit_strip`), so nothing paints there, clicking it falls through to the container, and every thumb pixel is grabbable, a short track's included (#1141: the minimum thumb used to be drawn past a track shorter than 20px — into the corner on a box under about 38px with both bars up). Paint used to give up only the other bar's 8px drawn footprint, which left the last ~6px of a thumb at full scroll painted in the square no strip claims (#444). A press that computes no new offset — a grab, or a track press whose jump clamps to where it already is — writes, dirties and fires nothing. **The bars are drawn above the scroller's own positioned descendants** (#1717): a scroller that is no stacking context has its `relative`/`z-index`/contained-absolute rows hoisted to the enclosing stacking context's sequence, so `stacking_paint_order` records it in `PaintOrder::bars` (`DeferredBars`: after the last entry at `z >= 0` whose clip chain runs through the scroller) and `paint_children_with_stacking` draws its bars there, under the scroller's own clip chain, while its `paint_node` skips them (`bars_deferred`). Chrome 155's order, measured with overlay scrollbars: a later positioned sibling, a box above every row's `z-index`, and an absolute that escapes the scroller still cover the bar. Hit testing needs nothing: `find_scrollbar_hit` is asked before any click and does not consult paint order. Desktop only — on the web the browser draws its own.

That geometry lives in **one place**, `crates/rinch-dom/src/paint/scrollbar.rs`
(`scrollbars(tree, node_id, scale)` → a `Scrollbars` holding an `Option<ScrollbarTrack>` per axis — `None` where that axis has no bar): paint draws
the thumb from it, and `find_scrollbar_hit` plus the `MouseDown`/`MouseMove`
arms in `crates/rinch/src/app/` press and drag it by it. They used to derive it
separately and had drifted (#400) — paint measured the track across the
container's **border** box and input across its **content** box, and input never
knew about the 20px minimum thumb — so a drag did not move the thumb the
distance the pointer moved. A drag now converts pointer distance to scroll
distance through `ScrollbarTrack::scroll_for_drag`, whose denominator is the
thumb's **travel** (`track_len - thumb_len`), not the track length. Anything new
that needs to know where a bar is should ask that module rather than re-derive
it.

**What counts as scrollable content (#765, #396).** `content_extents` measures
the scroll range from the container's **child boxes**, and only from those the
container is the **containing block** of — `out_of_flow::contributes_to_scrollable_overflow`,
which mirrors `out_of_flow_kind`'s walk. The two **must not drift apart**, and
nothing enforces it — `scrollable_overflow_tests` pins this predicate against
Chrome, but no test compares it with `out_of_flow_kind`. They share
`Node::establishes_abs_containing_block`, but each spells its own `match` on
`position` and its own initial-containing-block test, so a change to either —
a transformed ancestor containing a fixed box (#1372), say — has to be made
in both. A `position:
fixed` child resolves against the viewport, so it is no part of any scroll
range below it; an `absolute` child counts only where its containing block is
the container or below it — walking its DOM ancestors up to the container, one
is positioned or transformed (`Node::establishes_abs_containing_block`), or the
container *is* the initial containing block, which in rinch is the `<html>` box.
Below the container that walk crosses `display: contents` wrappers and, for a
box hoisted into its host out of a flowed inline element (#591), the inline
elements it was hoisted out of — a `position: relative` span among them is its
containing block and the container's content, so the box counts (#1049). A
`display: contents` element is neither, whatever its `position` or `transform`
compute to: it generates no box, so the predicate answers `false` for it and an
absolute under a `display: contents; position: relative` wrapper resolves
against whatever contains the wrapper (#994, measured in Chrome 153). Taffy lays
every out-of-flow box out against its direct parent whatever CSS says, so
without that filter a closed `Drawer` — `position: fixed`, and still rendered
since #751/#761 — made an `overflow: auto` ancestor report 800x600 of content
and paint **both** bars over a div holding 100x50. `visibility: hidden` is not
the rule and must not become it: a hidden box still has a box and still counts,
in rinch as in Chrome. `display: none` generates none and its zeroed rect
contributes nothing without a special case.

A child box is a direct child, or a box reached **through** `display: contents`
children, at any depth (#396): a wrapper generates no box, its children are laid
out in the container's own frame, and the containing-block rule is asked of the
container for each of them — a wrapper contains nothing. `rsx!` emits such a
wrapper for a `for`/`if`/`match` that is an rsx root or a branch's lone node,
for a branch of several nodes, for an embedded `Vec<NodeHandle>` and around
every reactive `{|| text}` (a `for` written straight into an element is *not*
wrapped — its rows are the element's own children), so before
#396 a scroll region whose rows came from one reported no content, painted no
bar and ignored the wheel. An **IFC root's** own inline content is measured from
its inline layout (`Node::text_layout`, at the content origin), not from its
text nodes' boxes: a text node under a contents wrapper or an inline `<span>`
has no box at all, and a direct one's proxy box is border-box-sized, which
over-reported a padded text scroller. Text beside a block child is laid out
by anonymous block boxes (and inside a split inline, #513, by the box around its
fragment), which are in no `children` list — so the walk reads the **box
tree** (`RinchDocument::box_tree_children`), not `children` (#995): each such
box is measured by its rect and by its own lines, so a `nowrap` line wider than
the container counts. The same list holds an out-of-flow box hoisted out of a
flowed inline element into its host (#591), which the element walk never
reached (it saw the inline's `0x0` box); the containing-block rule is asked of
it as of any child, walking its DOM ancestors up to the host: under a
`position: relative` span the span is the containing block and the host's
content, so the box counts when the scroller **is** the block holding the
span's line (#1049), at the span's fragment (#631); with another block between
the scroller and the line it counts nowhere (#770).
`DomDocument::scroll_height` /
`scroll_width` — what the wheel and `scroll_into_view` clamp to — **are**
`content_extents`, and so is layout's own clamp of a scrolled container
(`clamp_scroll_offsets`, which runs after `copy_cached_text_layouts`, since the
range reads this pass's `text_layout`s); each used to be a copy of the walk over `children`
alone, and the clamp copy took back on every layout pass the range the others
granted (a scroller at the bottom of #995's shape snapped from 150 to 50, a
text scroller from 100 to 0). **The walk reads `children` in one pass** where
the box tree is `children` (no anonymous box, no hoist, no split inline child)
and takes `box_tree_children` whole otherwise: `box_tree_children`'s own
borrow test is a second pass over every child, +37% per call on 500 rows.

Limits, all pre-existing and none introduced by that filter. The walk
stops at the first **box**, so an absolute whose containing block is a **non-parent**
ancestor contributes to no box's range at all — unless everything between it
and a block container above is non-positioned flowed inline elements and
`display: contents` wrappers, which hoist it into that block container's box
list (#591, reached since #995; under a positioned span it counts there too, #1049) — Chrome gives it to that
ancestor (measured: a 700x1500 absolute under a static `overflow: auto` div
lands on `documentElement.scrollHeight`), and rinch used to give it to the
wrong box, which is what grew the phantom bar (**#770**). **#769 is fixed**:
the wheel's finders — `find_scroll_container` / `find_horizontal_scroll_container`,
their `_at_point` twins, and the body fallback the two walks share — used to
compare the content extent against the container's **border**-box size where
`scrollbars` compares it against the content box, so a padded or bordered
container whose overflow was smaller than its `padding + border` painted and
let you drag a thumb the wheel routed straight past. That window is narrow,
which is why it was rarely seen — not because components avoided it: the
`Drawer` body, `FloatingPanel` body, `Select` dropdown, the menu-bar dropdowns
and `Code` are all padded or bordered scroll regions, and had the bug.
`paint::scrollbar::visible_extents` is now the one content-box computation:
`scrollbars`, `DomDocument::client_height`/`client_width`, layout's
`clamp_scroll_offsets` and the four wheel-routing walks all call it, the walks
through `paint::scrollbar::overflows` — the same move #400 made for
`content_extents`. `overflows` is the scrollable-and-overflowing half of
`scrollbars`' test and deliberately **not** its `--rinch-scrollbar-width: none`
half: that removes the bar, not the scrolling (as `scrollbar-width: none` does
in Chrome). The body fallback now also asks `scrolls_y()`/`scrolls_x()`, so an
`overflow: hidden` body is no fallback wheel target (CSS-correct). Still open
beside it: `clamp_scroll_offsets` never clamps the x offset (**#1368**).
(`apply_scroll_into_view` measured a scroller's band from its border-box top
against its content-box height until #842 made it the padding box, #1367.)

**How each box counts (#776, #775).** `extend_with_child` gives every box two
contributions, as Chrome 156 measures them. An **in-flow** box (anything but
`absolute`) counts its **margin box** at its *static* position — the layout
rect less a `relative`/`sticky` offset (Taffy places the box at the offset),
grown by the *declared* end margins (a percentage against the container's
content width, `auto` as 0, negative never below the border box; an
over-constrained block's used `margin-right` is negative and Chrome still
counts the declared one). Chrome extends that in-flow area by the container's
end padding, which cancels against the frame difference, so it is measured as
is. Separately, the box **where it is drawn** — an `absolute` box's border box,
a `relative` box at its offset, and either one's `transform`ed bounding box
(`compose_node_transform`, then `transform_rect_bbox`) — counts with no
margins and **without** the end padding, so it is measured less the
container's `padding-right`/`padding-bottom`. An `absolute` box has only that
second contribution: translated back inside the padding box it overflows
nothing, whatever its untransformed size. Before this every box counted its
untransformed border box plus the end padding — no margins, no transforms, an
`absolute; inset: 0` overlay (`LoadingOverlay`) in a padded scroller painted
two bars, and a `relative` box offset toward the start lost range Chrome gives
it. Two kinds of in-flow box count their border box alone: a non-replaced
`display: inline` box (its block-axis margins do nothing, CSS 2.1 §10.6.1, and
its inline-axis ones are in the line the container's inline layout measures;
a replaced element at `display: inline` is `DisplayMode::InlineBlock` and keeps
its margins), and a **grid item** (`ContainerFrame::grid`): Chrome counts the
grid's *tracks*, not an item's margin box, and layout does not keep them, so a
margin box overshoots (`margin-right: 300px` past a 100px column: Chrome 0) and
the border box, as before #776, under-counts only where a margin grows an
`auto` track (Chrome 86x46 and 0x120 in the pinned rows, rinch 0). Rows:
`tests/scroll_range_margin_transform_776_tests.rs`. Not counted yet:
a margin that collapses through a child's end (a `height: auto` child whose
last child has `margin-bottom: 80px`: Chrome 30px of travel, rinch 0).

**Styling the bar (#416).** Two inherited custom properties:
`--rinch-scrollbar-color: <thumb> [<track>]` and `--rinch-scrollbar-width: auto
| thin | none`. One declaration on `:root` restyles every scroll region.
`thin` is a 4px thumb; **`none` removes the bar from paint *and* from hit
testing**, so an app drawing its own can switch rinch's off rather than cover
it up. A track is painted only when a second colour is given.

They are `--rinch-` custom properties rather than the real `scrollbar-color` /
`scrollbar-width` because both real properties are **gecko-only in Stylo** — a
codegen-time filter, not a `#[cfg]`, so the servo build rinch uses emits no
parser entry and drops the declaration (grep this repo's generated
`properties.rs` for `scrollbar_color`: nothing). Custom properties cascade and
inherit normally in that build, which is what makes the root declaration work.

The `auto` default is **not a fixed colour**: the thumb is 40% black or 40%
white, chosen by the luminance of the container's computed `color`. A light
theme's text is dark, so the thumb is the 40% black it has always been; a dark
theme's text is light, so it flips to white and becomes visible — which is the
whole of #416, with no opt-in. Only the polarity follows the palette, so a
`color: red` container does not get a red thumb; `color` is read rather than
`background-color` because backgrounds are transparent by default.

**Viewport holes: a `GameViewport` is the base layer (#1866).** Nothing painted beneath a
punching `data-viewport` node in paint order shows inside its box (its border box, rounded by
its own `border-radius`); what paints after it (its border and children, later siblings, a
higher `z-index`, popups) draws over it as usual. Where paint reaches the node (in the element
arm of `crates/rinch-dom/src/paint/mod.rs`, after its own background and inset shadow, before
its border), one of two things happens:

- **The GPU window and the embed overlay split it** (`offer_base_split`): the node is offered to
  the painter as a **base** surface (`SurfaceSplit::base`, `BaseSplit`), which #1865's slicing
  takes out of the scene like a `RenderSurface` (region, `own` = the fitted image, `ancestors` =
  the clips open around it, `above_in`/`above_out`) plus a `clear` coverage, the box under its
  radius (`BaseSlice`). **Nothing is erased in the scene**: below keeps the viewport's own
  background and everything beneath it, and the split compositor replaces it inside the clear
  (`fs_surface`'s base term: `below · (1 − clear)`, black under a letterboxed CPU frame, #354, then
  the game `· own` over that, then `above_in`, the ancestors' `mix`, `above_out`; *How
  `GameViewport` fits on the split path* in `docs/design/split-layer-gpu-surfaces.md`). The
  runtime hands paint the viewports it may split (`set_base_viewports`, name → id, frame size,
  letterbox) and the GPU textures (`SplitFrame::set_base_views`) or uploaded CPU frames
  (`upload_split_frames`) the compositor samples; the UI is then rendered over the window's own
  base, not transparent. A new game frame is a recomposite from the kept slices, no repaint; a
  frame of another size marks the node damaged. The embed overlay
  (`RinchOverlayRenderer::render(device, queue, &mut ctx)`) paints with
  `set_bases_without_game`: every punching viewport is a base with no texture, keyed
  `BASE_WITHOUT_GAME_IDS + node id`, and `composite_straight` writes the overlay texture in
  straight alpha. A renderer that does not hold the context's last sliced frame (a new one after a device loss, or after another renderer's or context's turn) has the frame painted again (frame stamps, #1976: `docs/internals/game-engine.md`). A tainted slice (grayscale over a viewport: D5a) repaints that frame with the
  viewports cleared in the scene, as `scene()` does; with no game to keep out of the scene there
  is nothing to suspend, and the next painted frame tries the split again.
- **Otherwise it is erased in the scene** (`clear_viewport_box` → `Painter::erase`, an opaque fill
  composited `destination-out`): the software painter always, `RinchContext::scene()` (a host
  rendering the scene with its own Vello), and a GPU frame whose base split was **refused** (a
  group effect around it, a rotation or skew, an overlap with an earlier split, the memory cap,
  a taint). A refusal is reported (`take_base_refused`) and the runtime paints that frame again
  with nothing split and the games composited as layers under the UI (`LayerCompositor`, the
  pre-#1865 path), for `BASE_FALLBACK_FRAMES` (120) frames before it tries the split again (R1,
  Joe 2026-10-27; `shell/base_fallback.rs`, which `paint_gpu` builds every frame through). The
  second build is forced **full**: the first consumed the frame's damage, and left to it the
  second was `Unchanged`, so a change on the retry frame was never rendered (#1883 review, D1;
  `a_refused_base_split_rebuilds_the_frame_in_full_and_retries_after_the_fallback`). The first frame that names base viewports after one that did not (the fallback's end, the
  window's first) is built in full as well: on a transparent window the UI texture's base is
  transparent with layers and without, so the renderer asks no full frame there, and a partial
  frame that missed the viewport neither split nor refused it while the layers were dropped, a
  hole with no game (#1883 verification, D2;
  `the_frame_that_ends_a_fallback_splits_the_viewport_on_a_transparent_window`,
  `gpu_game_viewport_base_layer_survives_the_end_of_a_fallback_on_a_transparent_window`). A CPU
  frame is uploaded only when it is new (`collect_surface_frames_shared`'s flag,
  `gpu_game_viewport_base_layer_uploads_a_cpu_frame_only_when_new`). The retry is
  on a fixed count, not on the refusal's cause: when a transient cause ends (an opacity animation
  settles), the window stays layered, with the band below, until the count runs out, and the next
  frame splits again (the band then goes at once). A refusal that persists (two overlapping
  `GameViewport`s, D7a) keeps the window layered for good, video and `RenderSurface` splitting
  included. A GPU `GameViewport` punches only once its texture source exists (before, it was
  cleared with nothing behind it). On Vello the erase is a plain `BlendMode::DestinationOut` layer, and it is
  **approximate inside a clip**: Vello renders a clip, in every 16px tile its outline crosses, as
  a layer (backdrop saved, content drawn from transparent, composited back, `fine.wgsl`
  `CMD_BEGIN_CLIP`/`END_CLIP`), so the erase clears only the clip's own content there and the
  clipping box's background stays in a band along the clip's edges near the viewport (12,960 px
  for a flush `overflow: hidden` panel; pinned on `scene()` by
  `gpu_game_viewport_base_layer_under_clipping_ancestors_on_the_embed_overlay`'s control).
  Closing the clips, erasing and reopening them (461d5e9e) removed that band but composited each
  clip's content in two halves, a halo of up to 64/255 along every closed clip's whole ring and a
  partial frame that differed from a full one (it culls the viewport, so it closes nothing); the
  split replaced it. Pins: `gpu_game_viewport_base_layer_{no_halo_along_closed_clip_edges,
  partial_frame_equals_full}` (both paths), `…_replaces_what_is_beneath`,
  `…_under_clipping_ancestors_on_the_{desktop_window,embed_overlay}`
  (square, rounded, part-scrolled scroller), and `game_viewport_base_split_1866_tests` (rinch-dom,
  the painter's split and refusal, no GPU).

The **software** painter blends the erase in place;
a square box under no rotation, skew or clip mask where it lands is written `Source`
transparent instead (the same pixels; 0.16 ms against 3.6–5.3 ms for a 1080p game frame),
counted as `rect_erases`. On the **software window** a `GameViewport` with a frame no longer
takes the inline arm (which drew the frame and nothing else, not its border or children —
review D2): the element arm erases, draws the frame `contain`-fitted in the border box
(`paint_viewport_frame`), then the border and children over it, as the GPU window composites
its layer under them. **The game fills the border box** on every backend: a border covers its
edge, the padding shows it (a `RenderSurface` frame sits in its content box instead, #1861).
The letterbox is the erase: on an opaque software window it is
presented black (softbuffer drops alpha; it was the window's white base before), the colour
the GPU compositor's backdrop gives a `GameViewport` frame's bars (#354). A `visibility: hidden` viewport erases nothing (#854's paint half). Where it used to be
a hole cut out of the backgrounds of **clipping** ancestors only (`find_viewport_rects`, #974,
with `BorderRadii::path_with_holes` keeping the cut inside a rounded box, #1030): a
non-clipping ancestor's background covered the game, the cut ignored the viewport's own
radius, and it was cut at the untransformed rect outside every clip (#1386). An arc pixel of a
rounded clipping ancestor keeps `c (1 - c)` of its background at coverage `c` (the two edges
conflate; at most a quarter). **Where the rule cannot hold** a debug build warns once per
viewport (`tracing::warn!`, and `paint::take_viewport_warnings` for tests), naming the viewport
and the element: a **group effect** on the viewport or an ancestor (`opacity < 1`,
`mask-image`, and the `filter` functions paint approximates; rinch parses no
`mix-blend-mode`), where the erase clears only the group's own pixels and the group is then
composited over what is beneath it; and, for a compositor layer only, a **transform other
than a translation**, where the erase turns or scales while the layer is blitted axis-aligned.
Pins: `crates/rinch-dom/tests/game_viewport_base_layer_1866_tests.rs` (software painter),
`crates/rinch/src/app/game_viewport_base_layer_1866_tests.rs` (desktop GPU window, `#[ignore]`d
`gpu_…`), `crates/rinch/tests/embed_game_viewport_base_layer_1866.rs` (`RinchOverlayRenderer`,
`#[ignore]`d `gpu_…`), `game_viewport_inline_tests::a_game_letterbox_clears_a_non_clipping_ancestors_background`.
On a **transparent** window a hole nothing fills is
see-through to the desktop, so a viewport whose content can be absent opts out by
stamping **`data-viewport-ready="false"`** — paint then erases nothing and
the node paints its own `background` (a placeholder, poster, or error affordance). The
attribute is an **opt-out and absence means ready**: `GameViewport` stamps nothing and
punches unconditionally, unchanged. `VideoViewport` opts in — it stays `"false"` until
mpv hands a real frame to the compositor (`VideoPlayer::has_frame`, reset by
`set_source`) and returns to `"false"` on a `PlaybackState::Error`, which is issue #186.
A node that carries the attribute must say exactly `"true"` to punch, so a mis-stamped
value fails safe. **A not-ready viewport shows no frame either** (#348), on both backends:
the software inline arm reads the same rule (`rinch_dom::paint::viewport_ready`) and lets
the node paint as a plain element even while its surface still holds a buffer — a video
that errored mid-stream keeps its last one (video is drawn inline on the GPU window too
since #1768, so this arm is video's on both) — and the GPU shell hands the compositor no
layer for a not-ready `GameViewport` (`RinchApp::viewport_ready(name)`, in `paint_gpu`). It used to be a frozen
frame on software and the placeholder on GPU. The flip is the attribute write's own
damage, the node's box. Pins: `crates/rinch/src/app/viewport_ready_348_tests.rs`
(software pixels and `viewport_ready`); the two `paint_gpu` call sites have no fixture.

**Rings are filled non-zero, not even-odd (#1865 D9).** Every shape rinch cuts a hole
out of where the hole lies inside the outline — a rounded solid border, an elliptical
solid outline, an outer shadow's layers when the border box fits inside them and the
clip round the border box when it does not, an unblurred inset shadow — is one path
filled `Fill::NonZero`, the hole's contours appended wound against the outline
(`punched` in `paint/borders.rs`). They used to be `Fill::EvenOdd` with both contours
wound the same way. Vello 0.7's MSAA even-odd fill (`fill_path_ms_evenodd`,
`vello_shaders/shader/fine.wgsl`, unchanged on upstream `main` as of 2026-10-23) clears
its workgroup winding words before its first barrier, and nothing orders that after the
previous fill's last reads of them, so a subgroup narrower than the 64-invocation
workgroup can zero a row another has not read. Lavapipe (8-wide subgroups, run one after
another) does it every time: a rounded border's corner tile came out as a 1/8-coverage
wash (`(223, 239, 223)` for green on white). A `workgroupBarrier()` at the end of that
function, in a patched `vello_shaders`, cured it with the even-odd fills left in place.
On RADV (one 64-wide wave per workgroup, so no race) the switch moves a few pixels by
one MSAA sample (at most 16/255): the #1891 review's 379-style sweep found 25 such pixels
in 9 cases, not confined to tile edges, and against analytic coverage they lean neither
way, so they are two MSAA routines disagreeing, not an even-odd error. tiny-skia is
byte-identical. Under Area anti-aliasing (Android) the switch also removes a solid band
main painted for a zero-size ring such as `box-shadow: 0 0 0 0`. Non-zero with the hole
reversed is also what Vello's analytic area coverage needs (see
`BorderRadii::path_with_holes`). Pinned backend-neutrally by
`crates/rinch-dom/tests/ring_fill_rule_1865_tests.rs` (no compound path is even-odd; the
hole's centre winds to 0) and on a real adapter by
`gpu_render_callback_frames_sit_in_the_content_box`, which CI's `test-gpu (lavapipe)` job
runs. **Still even-odd:** the backgrounds and inset-shadow clips with viewport holes
(`path_with_holes`, whose holes need not be disjoint) and an SVG `fill-rule="evenodd"`,
both drawn by the same Vello code on the GPU painter.

**Which viewports have a frame, and which punch, is a paint input, so a change in it is
damage** (#349). No DOM write carries it: a surface that unregisters while its
`data-viewport` node stays in the document, or one that delivers its first frame, changes
what that node draws with the node untouched. The shell hands every paint's sets to the app
— `RinchApp::install_viewport_frames` (software: the inline frame map and the hole set) or
`RinchApp::install_viewport_holes` (GPU: the hole set; `None` is "every viewport punches")
— and `note_viewport_paint_inputs` compares them with the last paint's
(`ViewportPaintInputs`) and marks paint-dirty the viewport nodes whose membership in either
set changed (every viewport node on a change between `None` and a set), plus
`request_repaint`: the viewport's painted, clipped box on software, a re-encoded scene on
GPU, and nothing at all on a frame whose sets are the last one's. **That box is the frame
and the hole**: since #1866 the hole is the node's own erase, through its clip chain and
transform (it used to be cut out of ancestor backgrounds at the untransformed rect, and hole
pixels outside the damage outlived the surface, #1386). Before, the cached pixmap
or scene kept the last frame and its hole until something unrelated repainted that area. A
shell that calls `rinch_dom::paint::set_active_viewports` itself, around the app, gets none
of this. `unregister_render_surface` also asks the window for a redraw when it removed a
registered surface, since a surface can go with nothing else changing. The memory is by
**name** and the `RenderSurface` map is not covered: two surfaces under one name whose
shown one goes (#1388), and a `data-render-surface` id that leaves its map (#1387), still
keep a stale frame. Pins: `crates/rinch/src/app/viewport_inputs_349_tests.rs` and
`viewport_inputs_349_more_tests.rs` (membership swaps at a constant count, among others);
`perf_regression_tests::idle_viewports_repaint_nothing` is the exact-frame pin that an
unchanged set costs nothing. The GPU shell's own call site
(`paint` in `shell/rinch_runtime.rs`) has no fixture: it needs a window and a device.

**SVG paint.** `paint/svg.rs` draws an `<svg>`'s shapes (`path`, `rect`,
`circle`, `ellipse`, `line`, `polyline`, `polygon`) itself, walking `<g>`
groups to any depth (#1539) iteratively, in document order. Each element
derives an `SvgInherited` from its parent's: `fill`/`stroke`
(`InheritedPaint`; `currentcolor` stays symbolic, as CSS inherits the
keyword, and resolves against each shape's own `color`), `stroke-width`,
`stroke-linecap`, `stroke-linejoin` (a usable declaration replaces the
inherited value; the `<svg>` starts from black / none / 2 / butt / miter), and
the transform: the parent's times the element's own `transform` attribute
(`parse_svg_transform`, SVG's grammar — `translate(10-5)` is two numbers —
and an unparsable list is ignored whole, as Chrome does; the `<svg>`'s own
`transform` attribute and a CSS `transform` on a shape are not read). The
painters stroke in the transform's space, so a `scale` scales the stroke, as
in a browser. `<rect rx/ry>` and `<ellipse rx/ry>` follow SVG 2's `auto`
rule (an absent, `auto` or negative radius takes the other; one that is no
number is `0`, as in Chrome 156, which also reads the attribute keyword
`auto` as `0` where rinch keeps SVG 2's `auto`; #1538); a rect's
radii are clamped to half its width and height separately, equal radii are a
`RoundedRect` and unequal ones a path of quarter ellipses
(`elliptical_rounded_rect`); a zero-size rect, circle or ellipse draws
nothing, stroke included. The `<svg>` box is what culls and damages its
content: a transform that moves a shape outside it paints unclipped where a
browser clips (`overflow: hidden` on a non-root `<svg>`) and is not damaged
there, as for coordinates outside the `viewBox` before (#1990). Pins:
`crates/rinch-dom/tests/svg_groups_transforms_shapes_1538_1539_tests.rs`
(Chrome 156 `elementFromPoint` answers), the parser's unit tests in
`paint/svg.rs`. rinch's Stylo build
has no `fill`/`stroke` property (Gecko-only in Stylo), so `declared_paint`
reads each shape's own: its inline `style` declaration, which beats the
presentation attribute, else the attribute, with every `var()` substituted
from the element's computed custom properties (#1800,
`substitute_vars`; a fallback may hold a `var()`). A missing variable with no
fallback is invalid at computed-value time, so the shape takes the `<svg>`'s
paint; a literal `style` value that is no paint is dropped and the attribute
applies, as Chrome 155 does. Not read: a `fill`/`stroke` in a stylesheet
rule, and `stroke-width`/`stroke-linecap`/`stroke-linejoin` in `style` (still
attributes only). Pins: `crates/rinch-dom/tests/svg_paint_var_1800_tests.rs`.

### Split GPU surfaces in the Vello painter (#1865)

Design record: `docs/design/split-layer-gpu-surfaces.md`, *Painter split*.
`paint_surface_frame` offers each GPU frame to `Painter::split_surface`
before drawing it; the default refuses (`Refusal::Unsupported`) and the frame
is drawn in the scene as before. `VelloPainter` takes it only on a frame
opened with `begin_slicing(window)` (after `reset`, which closes it), and the
surfaces it took come back from `take_slices()` as `paint::slicing::SurfaceSlice`:
region R; `above_in` and `above_out` (scenes in window coordinates, rendered where the window draws them and R kept, or `None`);
`own` and `ancestors` coverage (each a whole-pixel rect or a mask scene); the
in-scene image transform; `tainted`.

- **Refusals**, the frame then drawn in-scene: a group effect open around it
  (a layer with opacity below 1 or a non-normal blend, or any isolated
  layer: masks, text-shadow groups), a transform that rotates or skews, or a
  region overlapping an earlier split surface's, or a surface the runtime
  named with `refuse_split(ids)` after `begin_slicing` (`Refusal::MemoryCap`,
  its memory cap, D6). R is the content box snapped out to whole pixels, a
  pixel further left only when that left edge (or the fitted frame's, in R's
  first column) is fractional; routing bounds grow the same way. Two surfaces
  touching at whole pixels both split.
- **Per-region repainting (S4)**: `compute_damage_attributed` names the
  node whose paint draws each change (`NodeDamage`); paint reports watched
  nodes' starts (`set_paint_order_watch`, `Painter::note_node_start`,
  forwarded by `ClipTrackingPainter`) as counts of surfaces split so far
  (`VelloPainter::node_starts`). A frame whose every change in a kept region
  started after its surface skips below over that region
  (`split_below_kept`). A change to a box that is no stacking context with a
  negatively z-indexed descendant hoisted out of it is never placed: that
  descendant paints before the box starts (`hoists_negative_z`, bounded at
  256 nodes, `split_hoist_walk_nodes`; #1929 is the older damage bug next to
  it). Design record: *Damage per slice → As built (S4)*.
- **No surface costs one bool per draw**: `route!` (a macro, not a generic)
  tests `VelloPainter::routing`, set when a surface splits, and otherwise is
  the draw itself; no closure or bounds are built.
- **Routing** (`paint/slicing.rs`): every draw after the split goes by its
  device bounds. Disjoint from every R: the window's scene, unclipped. Inside
  one R: that surface's above scene only. Across an edge, or bounds unknown
  (`append`): every above scene it meets, and the window's scene inside the
  inverse clip (the window with every R cut out, holes wound backwards,
  **non-zero**; even-odd nested in a clip drops coverage on lavapipe). The
  inverse clip is opened only around such draws and closed before any push or
  pop, so it never wraps a layer boundary.
- **Two above scenes.** `above_in` takes what is painted while every entry
  open at the split is still open, **without** those entries: the compositor
  applies the ancestors' coverage once, to the surface and `above_in`
  together, as the scene does (applying it to each separately conflated on a
  rounded card's ring, #1887 review). `above_out` starts when the first of
  them closes, with those still open replayed; a draw over R while one of
  those is not a whole-pixel rect clip taints the slice. Pushes and pops
  after the split go to whichever is current. Without `begin_slicing` nothing is mirrored
  and no shape is copied; a sliced frame with nothing split encodes the
  window's scene byte for byte as an unsliced one
  (`tests/split_surface_painter_1865_tests.rs`).
- **Taint** (D5a): a non-normal blend layer (grayscale's saturation overlay)
  over an R, not inside an isolated layer opened after the split, marks the
  slice: the runtime must paint the frame again with it in-scene.
- Not routed yet: content the runtime appends to `scene_mut()` after
  `paint_document` (drag ghost, inspect highlight, tooltip) — PR 3.
