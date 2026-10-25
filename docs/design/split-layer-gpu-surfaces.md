# Split-layer GPU surfaces

**Status:** Decided (issue #1865). Joe accepted D1–D9 as recommended and chose
D10 (the oracle's tolerance), 2026-10-23.
Implementation follows the PR sequence at the end.
**Scope:** the desktop GPU window (`gpu` feature, Vello on wgpu). A
`RenderSurface` GPU frame and a GPU-path video. Software, web and the embed
API do not change, and `GameViewport` stays the base layer (#1866).

## Goal

#1768 part 3 (#1861) draws a GPU surface **in-scene**. Its texture is
registered with the window's Vello renderer and drawn as the element's
content, so it is clipped, rounded, faded, transformed, scrolled and
overlapped like any other block element. The cost is that every new surface
frame means a full Vello render: a 3–6 ms floor on the reference GPU, more
at 4K, and a partial frame barely helps.

This record designs an internal fast path that produces the same pixels at
roughly the cost of the old compositor layer. The app never chooses, and
there is no new API. The one decision point, `surface_route`, picks
**split** or **in-scene** per surface per frame. A pixel oracle pins
split == in-scene for every element behaviour.

## Summary of the design

The document paints into one window-sized **below** slice plus, for each
split surface, a small **above** slice the size of that surface's **region
R**. R is the content box's bounding box, snapped outward to whole device
pixels.

- **Below** is today's UI texture. It holds everything painted before the
  surface, and everything painted after it *outside R*.
- **Above** holds the content painted after the surface *inside R*, and
  nothing else. Usually nothing is painted there, so the slice does not
  exist.
- **The compositor** builds the frame in this order: below, then the
  surface's texture source-over within its clip chain, then above.

The issue sketched two window-sized slices that switch at the surface. The
region-sized above slice (D1) is what makes the empty case the common one:
a sidebar painted after a video in DOM order goes into below, not above.

```
paint order:   [ before the surface ] [ surface ] [ after the surface ]
                         |                 |          |            |
                         v                 |      outside R     inside R
                       below <-------------+--------- |            |
                                           |                       v
compositor:  below ──► surface (clip chain, source-over) ──► above (R only) ──► present
```

## Measured cost

These numbers come from the spike, extended for this record. The spike is
the local branch `spike-in-scene-gpu`, file
`crates/rinch/src/shell/spike_in_scene.rs`. Run it with
`cargo test -p rinch --features gpu --lib -- --ignored spike_ --nocapture --test-threads=1`,
through `heavy.sh`.

- **Hardware:** AMD Radeon 890M (RADV, Vulkan). The device has no timestamp
  queries, so each figure is wall time per frame including submit and wait,
  median of 120 frames.
- **Scene:** a static UI of 240 rounded cards. The surface changes every
  frame.
- **Rows:** A, B, C and V come from `spike_in_scene_cost`. S and M come from
  the new `spike_split_cost`. Both ran in the same session (2026-10-23).
- **Noise:** about ±0.3 ms between runs. Column A moved from 1.07 to 1.35 ms
  between two runs of the same case.

| ms, median | 1080p, 640×360 | 1080p, full | 4K, 640×360 | 4K, full |
|---|--:|--:|--:|--:|
| A: layer under the UI (old path) | 1.2–1.4 | 1.1 | 2.4–2.7 | 3.0 |
| B: in-scene, partial frame (#1861) | 3.3–3.6 | 8.4 | 3.5–6.1 | 14.5 |
| C: in-scene, full frame | 3.6–4.1 | 8.4 | 5.0–7.3 | 13.9 |
| **S1: split, above slice empty** | **1.19** | **1.05** | **2.68** | **3.11** |
| **S2: split, above slice cached** | **1.51** | **1.24** | **2.67** | **3.82** |
| S4: split, above slice re-rendered | 2.99 | 4.23 | 6.15 | 6.44 |
| S3: split, below (partial) and above re-rendered | 7.50 | 5.52 | 7.05 | 8.95 |
| M: Vello render of a clip mask, region-sized | 1.71 | 3.28 | 1.76 | 3.39 |

Video, 4K, full window, CPU frames: the layer path with upload (V1) took
5.5 ms; in-scene (V2) took 18.3 ms. A split video is V1's upload plus S1's
composite, so expect about 5.5–6 ms. That case was not measured on its own.

What the numbers say:

- **Surface-only frames** cost what the old layer did, with or without a
  cached above slice (S1, S2 against A). This is the case the issue is
  about: a game or video running under a static UI. Vello does not run at
  all.
- **A UI change inside R as well**, such as a ticking time label in a
  video's controls, costs S4, provided only the above slice re-renders.
  That needs per-slice damage (see *Damage per slice*). S4 is about equal to
  in-scene for a small panel and half of it or less for a full window.
- **S3** is the case where below also re-renders over R, and it is worse
  than in-scene for a small panel (7.5 against 3.5 ms): two Vello floors
  instead of one. Per-slice damage exists to keep a surface frame plus an
  above-only change out of S3. A frame that genuinely changes both (a
  scroll) pays S3. It pays it only on that frame, not on every surface frame.
- **The Vello floor scales with the target.** A region-sized render (M at
  640×360) costs about half of a full-window one. This is why the above
  slice and the mask are region-sized.
- **Caveat:** one iGPU, wall time, no GPU timestamps. Absolute values will
  differ on discrete GPUs. The ordering is what this record relies on.

## Memory

| | 640×360 | 1080p, full | 4K, full |
|---|--:|--:|--:|
| above slice (R × 4 bytes, Rgba8) | 0.9 MB | 8.3 MB | 33.2 MB |
| clip mask, when needed (R × 4 bytes; Vello renders to Rgba8 only) | 0.9 MB | 8.3 MB | 33.2 MB |
| composited target (window × 4 bytes, one per window) | — | 8.3 MB | 33.2 MB |

- **The composited target** already exists whenever a window has layers.
  The split path needs it whenever a surface is split.
- **The above slice's partial-frame target** shares the window's existing
  `partial_target` (#1718). That target only ever grows, up to the window.
- **Against in-scene,** a split surface leaves the Vello atlas. In-scene,
  the atlas holds a copy of each surface every frame. It is 8192² and capped
  at half that by `GPU_FRAME_ATLAS_BUDGET`, which is the "about six 4K
  surfaces" ceiling. A split surface costs its above slice and mask (zero,
  zero in the common case) instead of its atlas share.

## Painter split (rinch-dom)

The split happens where `paint_surface_frame` (`paint/mod.rs`) draws the
frame today. That is after the element's background, border and outline,
and before its children. Everything already emitted is "before" and lands
in below unchanged, so the surface's own border and background stay in
below by construction.

### Trait

`Painter` gets one method with a default:

```rust
/// The surface `split` is drawn by the compositor, not in this scene. Every
/// op from here on is routed by region. Default: refused, the caller draws
/// the frame in-scene (software painter, wrappers that do not forward).
fn split_surface(&mut self, split: &SurfaceSplit) -> SplitVerdict { SplitVerdict::InScene(Refusal::Unsupported) }
```

- **`SurfaceSplit`** carries the surface id, the content rect, the fitted
  image rect, the element's inner radii and `node_transform`.
- **`SplitVerdict::Split`** means `paint_surface_frame` emits nothing for
  the frame. The clip for its own radius goes into the mask; see
  *Compositor*.
- **`SplitVerdict::InScene(reason)`** means it draws exactly what it draws
  today. The image is registered for every GPU surface, see *Risks* 1.
- **`ClipTrackingPainter`** forwards the call.
- **tiny-skia** keeps the default. Software paints in order already.

### The open stack

The Vello painter keeps a mirror of every open `push_clip`, `push_layer` and
`push_isolated_layer`: kind, fill, transform, shape, blend and opacity. It
keeps it only on frames that have a GPU surface, and a flag set before paint
says which frames those are. This is the same bookkeeping
`ClipTrackingPainter` already does for bounds (`CLIP_CULL`), widened to whole
ops so they can be replayed.

When the split happens:

1. **Check the stack.** A split is refused if any open entry is a group
   effect: a layer with opacity < 1, a blend other than `Normal`, or an
   isolated layer (masks, text-shadow groups). The verdict is
   `InScene(GroupEffect)`. Open clips, and `Normal` layers at opacity 1, are
   fine.
2. **Compute R.** R is `node_transform` applied to the content rect, then
   its bounding box snapped outward to device pixels. A transform that is
   not axis-aligned (a rotation or skew) is refused, `InScene(Transform)`;
   see D4.
3. **Build the clip chain.** It is the open clips plus the element's own
   inner-radius clip. If every one of them is a rectangle on whole device
   pixels, the chain is a scissor rect. Otherwise a small **mask scene** is
   recorded: the clips replayed, the image rect filled white, all translated
   by −R.origin. See D2.
4. **Start routing.**
   - **Below:** push the inverse of R: the window rect with R cut out, filled
     **non-zero** with R wound the other way. Not even-odd: on lavapipe an
     even-odd clip nested inside another clip drops coverage it has (pixels
     left of R lost their after-content, about 24 levels off), where the
     backwards-wound non-zero path is right on lavapipe and RADV alike
     (*Risks* 2).
   - **Above:** a new scene for this surface, translated by −R.origin. The
     open clips are replayed into it, then R is pushed as a clip.

### Routing after the split

Every op after the split is routed by its device-space bounds, which
`ClipTrackingPainter` already computes for the cull:

- **Disjoint from R:** the op goes to below only.
- **Inside R:** the op goes to above only.
- **Straddling R:** the op goes to both. The inverse-R clip and the R clip
  keep each copy to its side.

### Pops past the split

A pop that closes an entry opened **before** the split would, in below, pop
the inverse-R clip instead, because that clip sits on top. So the painter
does three things in below:

1. pop inverse-R,
2. pop the entry,
3. push inverse-R again.

In the above scene, the same pop closes the replayed copy.

Because R lies on whole pixels, the coverage of inverse-R is exactly 0 or 1
(measured, *Risks* 2), and the ancestors' own clip layers are never split in
below. Below is the in-scene render outside R and the before-content inside
it, **bit for bit except one 8-bit level** on translucent or antialiased
after-content within one Vello tile (16 px) of R's edge: where a clip edge
runs through a tile, Vello sends that tile's content through its 8-bit blend
stack, one rounding more than in-scene. Snapping R to the tile grid does not
avoid it (an edge on a tile boundary still puts segments in the next tile).
D10 accepts that level.

### Anything after `paint_document`

The drag ghost, the inspect highlight and the `title` tooltip are appended
after `paint_document` (`build_gpu_frame`). They go through the same routing,
so a tooltip over a video lands in its above slice.

### Several surfaces

Each surface gets its own R and above scene. Below's inverse clip is the
window rect with every R cut out, each wound backwards, filled non-zero. A later surface whose R overlaps an
earlier split surface's R is refused, `InScene(Overlap)`: it is then simply
an image inside the earlier surface's above slice. See D7.

### Backdrop-reading content

There is one op in rinch that reads the backdrop it lands on:
`filter: grayscale()`'s `BlendMode::Saturation` overlay (the `has_filter` branch of
`paint_node_unmasked` in `paint/mod.rs`). Text-shadow's `Plus` layers sit inside an isolated layer, and
the brightness overlay is source-over, so neither reads the backdrop.

In an above slice, a Saturation overlay would desaturate a backdrop with no
surface in it. So an above slice that receives one is **tainted**, and that
frame is painted again with the surface in-scene. See D5.

## Compositor

The new compositor work, in `shell/compositor.rs`, is three passes plus
present. GameViewport base layers (#1866) still come first, unchanged.

1. **Below:** `overlay_ui` as today, which premultiplies Vello's straight
   alpha and blends over. It uses `Clear` when there is no base layer.
2. **Surface:** a new `blit_surface` pass for each split surface, in paint
   order.
   - It blends source-over onto below. `blit_layer_view` *replaces* today,
     because the layer was the bottom of the stack. A split surface is
     drawn over the element's own background, so a transparent texel must
     show that background, exactly as in-scene.
   - The image rect is the fitted rect (`object-fit: contain`, from
     `fit_image`), sampled bilinearly at pixel centres. This is the same
     convention `draw_gpu_image` uses with `centre_sampling` (#1861).
   - The texture's alpha is premultiplied in the shader when its
     `alpha_type` says straight.
   - **Clip chain:** a scissor rect when the chain is pixel-aligned
     rectangles. Otherwise a multiply by the mask texture, which Vello
     renders from the mask scene. The mask is re-rendered only when the
     mask scene's inputs change (a hash of the replayed clips), which
     happens on scroll, resize or a radius transition. Those are frames
     that re-render below anyway.
   - **Opacity:** none. A surface inside an opacity group is refused (D3),
     so the blit never applies one. Opacity is in the issue's compositor
     list. With D3a it moves to the fallback list, and only D3b or D3c
     bring it back.
   - **Transform:** an axis-aligned scale and translate is applied when
     mapping the image rect into R. Anything else is refused (D4).
3. **Above:** premultiplied over, limited to R (a viewport-limited
   `overlay_ui`). It is skipped when the slice is empty.
4. **Present:** `present_step` as today. `presented_texture` (#1799) must
   treat a split frame as layered, so a screenshot reads the composited
   texture.

The composited target is in the surface's format, which may be Bgra8, while
Vello's textures are Rgba8Unorm. That is why steps 1 and 3 are passes, not
copies, as `overlay_ui` is already.

### Border, padding and background

All three are before-content, so they are in below. R covers only the
content box, so after-content that overlaps the border (a negative
`outline-offset` on a later element, for instance) is outside R and stays in
below, in order.

The surface's own outline is painted before the frame. That holds in-scene
and in split alike, so it needs no special case.

## Damage per slice

- **A new surface frame alone** marks nothing paint-dirty. That is a change
  from #1861, where it marks the surface node. The frame is a recomposite
  from the cached slices (S1 or S2). `build_gpu_frame` returns
  `GpuFrame::Unchanged` for the scene, and the runtime still composites and
  presents.
- **UI damage** comes from the paint-dirty nodes. Each damaged node is
  placed by paint order:
  - A node before the surface, or after it with ink outside R, damages
    below.
  - A node after the surface with ink inside R damages the above slice.
  - A rect that cannot be attributed (a removed node's old rect, for
    instance) damages every slice it intersects. That is conservative.
- **Re-rendering a slice** works like `render_partial` (#1718). Below uses
  the existing path. Above renders its scene into the shared partial target
  and copies the damaged rects, translated by −R.origin, into its texture.
- **An empty above slice** is one into which the last full paint routed no
  draw op, with no partial frame adding one since. Its texture is dropped.
  A partial frame cannot prove that something which left the slice made it
  empty, so the texture stays until the next full paint. That is correct,
  only a little memory late.
- **R moving or resizing** (scroll, layout) damages old R ∪ new R in below.
  It re-renders the above slice in full, and the mask if there is one. Slice
  textures grow in `PARTIAL_TARGET_STEP` steps, so a 1-pixel resize does not
  reallocate.

## The decision point

`surface_route` (`render_surface.rs`) stays the only place that routes a
surface. Its GPU arm grows a placement:

```rust
pub(crate) enum SurfaceRoute {
    Pixels,
    GpuImage { device: &'static GpuHandle, placement: Placement },
}
pub(crate) enum Placement { Split, InScene(Refusal) }
pub(crate) enum Refusal { Unsupported, GroupEffect, Transform, Overlap, Tainted, MemoryCap, ... }
```

Before paint, `surface_route` decides from what it knows:

- whether the host can split (the desktop GPU window can; an embed context
  takes `Pixels` as today),
- the memory cap (D6),
- the painter's verdict for this surface on the last frame.

That last input is what sizes the target. A surface refused last frame is
sized for the atlas (`gpu_atlas_scale`). One planned for a split is sized at
full resolution, since it uses no atlas.

The painter's verdict at the split itself is final for the current frame.
For every refusal except `Tainted`, the frame is drawn in-scene on the spot,
and the verdict is recorded for the next `surface_route`.

Debug builds log one line each time a surface's placement changes, for
example `rinch: render surface 3 drawn in-scene: inside an opacity group
(div.card)`. Release builds do not log.

A route change cannot flicker, because in-scene and split produce the same
pixels (the oracle). A video fading in through `opacity` is in-scene for the
fade and split afterwards.

## Fallback to in-scene

| Reason | Detected | Why |
|---|---|---|
| Group effect on the surface or an ancestor: opacity < 1, non-Normal blend, mask, isolated layer | at the split, from the open stack | a group composites as one; it cannot be cut into slices (D3) |
| Transform not axis-aligned | at the split | the compositor maps rects only (D4) |
| R overlaps an earlier split surface's R | at the split | one above slice per surface (D7) |
| Backdrop-reading op in an above slice (grayscale's Saturation overlay) | while routing | the backdrop of the above slice has no surface in it (D5) |
| Slice memory over the cap | `surface_route`, before paint | D6 |
| Host cannot split (embed context, software) | `surface_route` | unchanged from #1861 |

Not on this list, because they split exactly:

- a z-index sibling above or below,
- a scroll clip on whole pixels,
- the surface's own `border-radius`, border and padding,
- `filter: brightness()` on an ancestor (its overlay is source-over
  after-content),
- text-shadow above the surface,
- scrollbars over it.

## Video, and later GameViewport

- **Video** takes the same route. Its frames are CPU pixels, so the split
  uploads them with `FrameUploader` into a texture and blits it with
  `blit_surface`. That is the old V1 path plus the clip chain, about
  5.5–6 ms for full-window 4K against 18.3 in-scene. In-scene fallback stays
  as #1861 has it: a new `ImageData` per frame.
- **`GameViewport`** is not on this path. It stays the base layer (#1866):
  drawn under the whole UI, with rinch adding one blend per game frame.
  #1866 changes the viewport paint code and the base-layer ordering in
  parallel. This design assumes base layers come first in the composite,
  below's overlay second, and split surfaces after that. That ordering must
  be re-checked against #1866's PR when it lands. There was no PR when this
  record was written.
- **`GpuTextureRegistrar`** is unchanged.

## Oracle

**The rule:** for each case, render the same document twice through the real
runtime path, with the placement forced to `InScene` and then to `Split`.
Read back the presented texture and compare.

D10 sets the rule. Each tolerance applies only to the pixels it names, which
the test computes from the case's geometry; **every other pixel must be
bit-equal**, so a regression cannot hide inside a blanket tolerance.

| Pixel class | Allowed difference |
|---|---|
| Every pixel not in a row below | **bit-equal** |
| Translucent or antialiased after-content over the surface (inside R) | ≤ 1/255 per channel (the above slice is stored in 8 bits, then blended) |
| Translucent or antialiased after-content outside R, within one 16-px tile of R's edge | ≤ 1/255 (Vello's blend stack in tiles the inverse clip's edge crosses) |
| Fractional coverage of the surface's own clip chain (mask) | ≤ 1/255 (one extra 8-bit rounding) |
| Transparent window, pixel with alpha < 255 | ≤ 1/255 (below is stored straight, then premultiplied) |
| AA ring of an **ancestor** clip that crosses R | bounded by D8 |

### Cases

Each case is an element behaviour from the issue's list and from the #1861
docs:

- own `border-radius`, border, padding and `object-fit: contain`
  letterboxing;
- an ancestor clip on whole pixels (scrolled half out);
- an ancestor rounded clip, with and without a background on the clipping
  ancestor;
- opacity on the element and on an ancestor (a fallback, so equal by
  construction; the test pins that the fallback happened);
- a z-index sibling above and below;
- a positioned overlay straddling R (controls bar), and one disjoint from R
  but later in DOM order (the sidebar case: the above slice must be empty);
- a translate-and-scale transform, and a rotation (fallback);
- two disjoint surfaces, and two overlapping ones (fallback);
- an alpha surface with straight and with premultiplied alpha;
- a partial frame with damage inside R only, with damage outside R only, and
  with a scroll;
- a tooltip over the surface;
- grayscale over the surface (taint, then a repaint);
- a transparent window.

### Positive controls

Positive controls are built with the oracle, not afterwards. An oracle that
never fails proves nothing:

- a split composite with the above pass skipped must fail the overlay case;
- a split that blits with `Replace` instead of over must fail the alpha
  case;
- a below slice without the inverse-R clip must fail the straddling case.

### Where the oracle runs

The GPU half is `#[ignore]`d like `gpu_partial_frame_1718_tests`, since CI
has no adapter. The CPU half runs in CI: the painter's routing (op counts per
slice, balanced pushes per scene, each refusal reason) needs no GPU. D9 asks
whether to give CI a software Vulkan for the GPU half.

## Decisions

All nine were accepted as recommended (Joe, 2026-10-23).

### D1. Slice shape

- **a.** Two window-sized slices per surface, switching at the surface, as
  in the issue sketch. Above holds everything painted after the surface,
  anywhere. A sidebar or footer after the surface in DOM order makes it
  non-empty, so the common case is not empty. It also costs 8.3 or 33 MB
  per surface.
- **b.** Below is window-sized and holds the after-content outside R. Above
  is R-sized and holds only what is painted over the surface.
  *(recommended)* Above is empty unless something really overlaps the
  surface, and it costs R × 4 bytes. The price is the inverse-R clip and
  the re-push on pops.

### D2. Clip chain for the surface blit

- **a.** Analytic rounded rects in the shader, up to N, with a fallback
  beyond. No extra render, but the antialiasing is not Vello's, so curved
  edges differ by up to a coverage step from in-scene and the oracle cannot
  be tight there.
- **b.** A Vello-rendered mask for any chain. Exact (±1), but a region-sized
  render (M: 1.7–3.4 ms) each time the chain moves.
- **c.** A scissor rect when every clip is a whole-pixel rectangle, which is
  free and exact. Otherwise a Vello mask, re-rendered only when the chain
  changes. *(recommended)* The common cases are a plain rect or a scroller.
  The mask's cost lands on frames that re-render below anyway.

### D3. A group effect wrapping the surface (opacity, mask, blend)

- **a.** Fall back to in-scene for any group on the surface or an ancestor.
  *(recommended for the first ship)* A fade costs in-scene for its
  duration, with no visible switch.
- **b.** As a, but compositor opacity when the group holds only the
  element's own box and the surface is opaque. In that case the result is
  exactly α·frame + (1−α)·backdrop inside the content box. This needs an
  "opaque" flag on surfaces, which GPU textures do not have today.
- **c.** Nested group slices: the group gets its own below, surface and
  above, is composited into a group texture, then blended with its opacity
  or mask. This is general, and it is a compositing tree. A follow-up issue
  if fades over video matter.

### D4. Transforms

- **a.** Translate and axis-aligned scale in the compositor. Rotation and
  skew fall back. *(recommended)* `all_surface_layout_rects` already works
  in axis scales.
- **b.** A full affine in the compositor: a transformed quad, the mask in
  local space, and R as the transformed bounding box. More shader and more
  oracle cases, for rotated videos.

### D5. Backdrop-reading content found over the surface while painting

- **a.** Paint the frame again with that surface in-scene, and keep it
  in-scene until the document's style or structure generation changes.
  *(recommended)* It costs one extra CPU paint on the frame where it appears,
  and it is rare (only grayscale over a surface).
- **b.** A pre-paint scan of every node painted after the surface whose box
  meets R, looking for `filter: grayscale`. No repaint, but a walk on every
  frame that has a surface.

### D6. Memory cap for split slices (above slices and masks; the composited target not counted)

- **a.** A fixed 64 MB per window.
- **b.** Twice the window's own texture bytes: 16.6 MB at 1080p, 66 MB at
  4K. This scales with the display, the way the slices do. *(recommended)*
- **c.** No cap.

  Over the cap, surfaces are split in paint order until the next one does
  not fit. The rest fall back with `MemoryCap`, then use the atlas with its
  own never-silent budget (#1861).

### D7. Two split surfaces whose regions overlap

- **a.** The later one in paint order falls back. It is then an image inside
  the earlier one's above slice. *(recommended)*
- **b.** Both fall back.

### D8. Ancestor clips with antialiased edges crossing R

An example is a rounded `overflow: hidden` card with a video filling it. On
that 1-pixel ring the clip's coverage c is applied to below, the surface and
above separately, rather than once to their composite (conflation). The
error is c(1−c) × (the clipping ancestor's background − what is behind the
ancestor), at most a quarter of that difference, on the ring only. It is
**zero when the clipping ancestor has no background**, or when its
background matches what is behind it. Browsers that composite a video in a
rounded clipping parent have the same artifact.

- **a.** Accept it. The oracle pins the bound on ring pixels, and bit
  equality where the ancestor has no background. *(recommended)*
- **b.** Fall back whenever such a ring crosses R. Exact, but it sends
  "video in a rounded card" to in-scene, one of the most common layouts.

### D9. Running the GPU oracle in CI

- **a.** Leave it `#[ignore]`d. Run it locally on a GPU before a batch, as
  #1718's are.
- **b.** Add a lavapipe (Mesa software Vulkan) job for the `gpu` oracle
  tests on `batch/` PRs. It costs CI minutes and a new job to keep green.
  The upside is that a split regression cannot land silently.
  *(recommended)*

### D10. The oracle's tolerance (decided after PR 1's measurements)

PR 1's measurements (*Risks* 2) showed that "pixel for pixel" cannot hold
for translucent or antialiased content painted after the surface near or over
it: slicing adds one 8-bit rounding there, on RADV in the below slice and on
every adapter in the above slice.

- **A.** Bit-equal everywhere except up to 1/255 per channel on translucent
  or antialiased after-content over the surface or within one Vello tile of
  R's edge; the mask edges keep their one level, and D8 its bound.
  **(chosen, Joe, 2026-10-23)**
- **B.** Split only when nothing painted after the surface reaches R or its
  tile neighbourhood, and stay bit-exact. A video with visible controls, a
  tooltip or a caption over it would lose the speed-up.

## Risks

1. **Registered but undrawn images. Resolved: free.** Vello 0.7 builds its
   atlas only from the images a scene draws (`Patch::Image` entries in
   `vello_encoding`'s resolver); the override table is consulted only for
   those. Pinned in `shell/split_slices_1865_tests.rs`: a registered texture
   that is then destroyed raises no error through renders whose scene does
   not draw it, and the control, a scene that does, raises one. RADV and
   lavapipe agree.
2. **Binary coverage at whole-pixel edges. Resolved, with D10.** Measured on
   the Radeon 890M (RADV) and on lavapipe, Msaa16 (the desktop's AA):
   - A clip on whole device pixels (direct, inverse, under an integer
     translation) leaves every pixel exactly in or out, on both. The control,
     half a pixel off the grid, leaves partial pixels.
   - Below against in-scene, over six placements of R (on and off the tile
     grid, across a rounded fractional ancestor clip): RADV rounds 46 pixels
     by exactly one level, all on translucent or antialiased after-content
     within one tile of R's edge, outside R; lavapipe rounds none. The
     control, R half a pixel off the grid, is off by more than 8 levels.
   - On lavapipe an **even-odd** clip nested in another clip renders wrongly
     (pixels left of R lost their after-content, about 24 levels off), while
     the same clip as a backwards-wound **non-zero** path is right on both.
     The inverse-R clip is therefore non-zero.
     The cause is Vello's, not lavapipe's (found for D9): its MSAA even-odd
     fill clears workgroup memory with no barrier after the previous fill's
     reads, which lavapipe's 8-wide subgroups lose every time; a barrier in
     a patched `vello_shaders` cures it. rinch's own rings are non-zero since
     (`docs/internals/paint-and-damage.md`, "Rings are filled non-zero").
3. **Sampling equality for scaled frames.** Video letterboxed into a
   different size is resampled by Vello in-scene and by the compositor's
   sampler in a split. These must agree to ±1, which means the same filter
   and the same pixel-centre convention. Unscaled 1:1 frames, the common GPU
   surface case, are exact either way.
4. **Encoding cost.** Ops straddling R are encoded twice. The open-stack
   mirror stores shapes on every frame that has a surface. Both are CPU
   costs, measured in PR 2 with the perf counters.
5. **Frame scheduling.** A surface-only frame must composite and present
   with `GpuFrame::Unchanged`. Today the surface node's paint-dirt is what
   forces a present. Frame pacing for continuous surfaces (#1861) must not
   regress.
6. **#1866 lands in parallel** on the viewport paint code and the base-layer
   order. Re-read its PR before PR 3.
7. **Device lost and resize** recreate the slice textures. Resize is already
   a full paint.
8. **Perf baselines.** New counters (slices rendered, placements, fallbacks
   by reason) are unlisted, hence `0`, in documents without surfaces. Shell
   scenarios with a surface move, and the PR says why for each, per the
   baseline contract.

## PR sequence

1. **Compositor primitives and the oracle harness.**
   - `blit_surface` (source-over, premultiply flag, fitted rect, axis
     transform, scissor or mask) and the region-limited over pass.
   - The forced-placement test harness.
   - The binary-coverage test (*Risks* 2) and the registered-image cost
     measurement (*Risks* 1).
   - No runtime change.
2. **Painter split in rinch-dom.**
   - `Painter::split_surface`, the Vello painter's open-stack mirror,
     routing, the inverse-R re-push and the mask scene.
   - The refusal reasons, and taint.
   - CPU tests in CI: op counts per slice, balanced scenes, each refusal.
   - Not wired to the runtime.
3. **Runtime wiring.**
   - `SurfaceRoute` placement, `build_gpu_frame` returning slices, the
     desktop composite, per-slice damage, surface-only frames without Vello.
   - `presented_texture`, the debug note, counters and baselines.
   - The GPU oracle matrix and its positive controls.
   - `RenderSurface` GPU frames only.
4. **Video on the split path,** through upload and `blit_surface`, and its
   oracle cases (letterbox scaling, *Risks* 3).
5. **The memory cap (D6)** and the lavapipe job (if D9b). Docs:
   `docs/internals/game-engine.md` and `paint-and-damage.md`, and the user
   guide's note that GPU surfaces cost about the same as a layer.

Each PR stands alone and keeps the in-scene output. Until PR 3, nothing
routes to split.
