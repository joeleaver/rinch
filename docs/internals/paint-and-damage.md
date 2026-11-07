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

**Present mode and vsync (#2095).** `WgpuRenderer` resolves its present mode
itself (`choose_present_mode`, `shell/desktop.rs`) rather than passing wgpu's
`AutoVsync`, so the shell can say what the surface does: vsync on is
`FifoRelaxed` where offered, else `Fifo` (what `AutoVsync` picked before);
off is `Immediate`, else `Mailbox`, else `Fifo` (`AutoNoVsync`'s order). The request lives in `rinch::window` (`VSYNC_OFF`, any thread;
`App::vsync` stores it before the loop); a renderer reads it when it is made,
so a hide/show re-creation and the DevTools window take it. `set_vsync` while
the loop runs sends `RinchNativeEvent::ApplyVsync`; `RinchRuntime::apply_vsync`
calls `WgpuRenderer::set_vsync` on the main and DevTools renderers (an
uncapped main window beside a FIFO DevTools window that repaints with it would
be paced by the DevTools present), publishes, and requests a redraw of both.
`set_vsync` only sets `surface_config.present_mode` and a `reconfigure` flag
(`retarget_present_mode`: no flag when the mode is unchanged); `present()`
configures the surface before `get_current_texture`, so no surface texture is
held when it changes, and `resize` (which configures anyway) clears the flag.
The UI texture, `UiTextureState`, the split slices and Vello's in-flight frames
(`check_frames`, #2077) are untouched: the next present copies the UI texture
as it stands. What the app reads: `set_vsync` answers from `PACING`, the main
window's `PacingOutcomes` (both answers, computed from the surface's present
modes when its renderer is made: `renderer_started` / `note_renderer_pacing`
at create and show; `UNSUPPORTED` for the software renderer; `None`, read as
`Pending`, while hidden and after shutdown, `clear_pacing`), and
`rinch::window::presentation()` is a thread-local signal the runtime
republishes on each of those — so it is UI-thread only; off it, a thread
reads its own `Pending`. `rinch::window::vsync()` reads a thread-local signal
too (for subscribing) and answers the `VSYNC_OFF` atomic: `set_vsync` updates
the calling thread's copy, and `publish_pacing` the UI thread's, so an
off-thread request reaches UI readers when the shell applies it. The shell
reads the request with `vsync_requested()`, which subscribes nothing.
Experimental features are never requested as optional ones
(`device_features` subtracts `Features::all_experimental_mask()`; wgpu
refuses them without an `ExperimentalFeatures` opt-in, as Android's shell
already handles). The frame loop itself is unchanged: it
paints on request (`AboutToWait`, render-surface pacing, input), so with an
uncapped mode a continuous surface draws back to back and an idle window
still waits (`ControlFlow::Wait`); nothing calls `pre_present_notify`, so
winit does not hold redraws for Wayland frame callbacks. Software (softbuffer)
has no present mode; Android presents `Fifo` and does not read the setting.
Pins: `pacing_and_features_tests` (`shell/desktop.rs`, including the
`apply_vsync_to` fan-out over fake surfaces),
`window::tests::set_vsync_answers_what_the_surface_will_do`,
`rinch_runtime::window_restore_tests::the_runtime_publishes_its_windows_pacing_2095`
(renderer start, apply, hide and shutdown without a surface). Not pinned by
a test: the reconfigure call in `present()`, and the call sites in
`create_window`/`show_window` (they need a real surface); the uncapped
frame rate itself was measured by hand (`gpu-device-config`,
`RINCH_GPU_MODE=frames RINCH_MEASURE_FRAMES=N`), as no CI job has a window.

Both use the same `Painter` trait (`crates/rinch-dom/src/paint/painter.rs`):
- `VelloPainter` — records commands into `vello::Scene`
- `TinySkiaPainter` — rasterizes directly to RGBA pixmap

**GPU antialiasing is area coverage (#2286 stage 1a).** Every Vello render of a
window or overlay scene reads `crate::gpu_antialiasing::scene_aa()` and every
renderer is made with `scene_aa_support()` (`crates/rinch/src/gpu_antialiasing.rs`):
`desktop.rs`'s window and offscreen renderers, `render_ui` and `render_partial`,
`split_frame::render_slice`, `embed.rs`'s `RinchOverlayRenderer`, and the test
oracles that stand in for them. The default is `AaConfig::Area`; it was `Msaa16`
on the desktop and the embed overlay until #2286 (Android was already `Area`, and
`android_runtime.rs` keeps its own constant and ignores the option). The mode is
`App::gpu_antialiasing`, `rinch::set_gpu_antialiasing` or `RINCH_GPU_AA`
(`area`/`msaa8`/`msaa16`, over the app's), and is latched by the first renderer
made, since a Vello renderer has the pipelines of the modes in its `AaSupport`
only (a renderer made with `area_only()` cannot render `Msaa16`; a later set is
logged and ignored). Why: Msaa16 gave text edges at most 15 partial levels
(#2286's probe: mean coverage error to FreeType 0.049 against Area's 0.041), and
its fine pass (`fill_path_ms`, workgroup atomics and barriers per fill) cost
37.7 ms for a 1280×800 page of 13px text on lavapipe against 24.8 ms for Area's
pure-ALU `fill_path`; on an Adreno 619 the whole frame was 58 ms against 17. MSAA stays
selectable for a path that overlaps itself: area coverage sums a path's signed
area per pixel, so a self-crossing stroke or overlapping contours (variable
fonts) shade an edge pixel by the sum, where MSAA counts the samples the union
covers. It is not a fix for seams between separate abutting paths: Vello
blends each path with its own coverage in both modes (`fine.wgsl`, the blend
after the fill), and the #2286 review measured those seams about equal under
tiny-skia, Msaa16 and Area. (#2286's probe: ≤ 81 pixels of abutting cells,
borders and rounded grids differed from Msaa16 by more than 8/255, none by more
than 32, at scales 1, 1.25 and 1.5.) The MSAA even-odd race below matters wherever
MSAA renders: an app that selects it, and the exact-comparison fixtures. Pins: `gpu_the_window_draws_text_with_area_coverage_2286`
(`app/gpu_text_aa_2286_tests.rs`, CI's `test-gpu`: more than 64 distinct partial
levels in a text window, where Msaa16 has at most 15), and
`gpu_antialiasing::tests` (the env and request order, one mode's pipelines).
`text_aa_cost_probe_2286` (same file, skipped by name in CI) times both modes.

**An area-coverage render is not repeatable on a parallel GPU (±1/255).**
`path_count.wgsl` hands each tile's segments their slots with `atomicAdd`, so
their order in the tile changes from run to run, and Area's `fill_path`
(`fine.wgsl`) sums their float area contributions in that order: two renders of
one scene differ by 1/255 here and there (the #2286 review: a dense page
differed in 16 of 16 runs, up to 429 channels; Msaa16, whose coverage is an
integer count of samples, differed in 0 of 16). This holds on any GPU that runs
the segments in parallel, lavapipe included. So **exact pixel comparisons
between two renders are pinned to Msaa16**: in `rinch`'s own test build
`scene_aa()` is a per-thread mode that defaults to `Msaa16` (and every renderer
`scene_aa_support()` makes has every mode), which keeps the damage, cull,
clip-group, split and base-layer fixtures exact and keeps MSAA covered; the
area fixtures pin `GpuAntialiasing::Area` with `TestAa::pin` and compare within
1/255 (2/255 where an inversion's rounding adds another). The embed integration
tests (`tests/embed_*`, built against the non-test library) call
`rinch::set_gpu_antialiasing(Msaa16)` before their first renderer. A new
fixture that compares two GPU renders exactly must not pin Area.

**Text rendering policy (#2286 stages 1b and 1c).** `rinch_dom::paint::text_rasterization`
holds the app's `TextRasterization` (`antialias`, `hinting`, `stem_darkening`, each
`Auto` by default; `#[non_exhaustive]` with setters, so stages 2 and 3 add
fields — the subpixel order and LCD filter, a contrast curve), the OS's
`SystemTextSettings`, and their resolution, `ResolvedTextRasterization`. The resolved
policy is a thread-local (`thread_policy()`), filled on a thread's first glyph
run from the source the facade installs (`rinch::text_rasterization::set_text_rasterization`,
called by `App::run`/`run_android`, resolves the app's choice against
`system_text_settings()`); without a source, as in every test, it is the default
resolved against nothing (grayscale, hinted, darkening 0.10 px). Tests override
it per thread with `set_thread_policy`/`reset_thread_policy`, so parallel
tests do not see each other's. Both leaf painters read it in their glyph draw —
`vello_painter::encode_glyphs` and `TinySkiaPainter::draw_glyphs` — so every
caller of `draw_text` (text, shadows, tooltips, the editor) gets it without a
parameter:
- **hinting**: the run's `hint` is ANDed with `policy.hint()` (off only for
  `TextHinting::None`; both painters have one hinted mode).
- **stem darkening**: `darkening_for(colour, device font size)` — only a solid
  brush, judged by the text colour alone (Skia keys its contrast LUT the same
  way; paint does not know the backdrop): sRGB luma × alpha ramps 0.5 → 0.85,
  full up to 24 device px per em, none from 64, × the policy's amount (0.10 px
  per side by default), quantised to 1/64 px so the glyph caches see few
  values. Vello gets it as `DrawGlyphs::font_embolden` (`kurbo::expand_path`,
  the amount on every side; Vello's glyph cache keys on it). tiny-skia gets
  swash's `Render::embolden` with `offset(0, -a)`: swash's embolden
  (FreeType's `EmboldenXY` without the halving) grows a glyph about evenly
  left and right but by `2a` at the top and none at the bottom (measured: the
  ink's centre moved 0.022 px right and 0.100 px up at `a` = 6/64), so the
  offset moves it back down to the same per-side growth as Vello's within
  0.03 px; the amount is in `GlyphRunKey` (`darkening`, the image-space
  amount's bits). A face with colour
  outlines asks for the COLR glyph without the offset first (COLR glyphs are not
  emboldened by swash). Why: sRGB-encoded blending gives light-on-dark text
  0.73–0.79 of its linear-light weight against 1.21–1.27 dark-on-light; 0.10 px
  **The amount is in device pixels under any transform** (#2286 review):
  `run_darkening_px(brush, font_size, transform)` judges the run at
  `font_size × device_scale(transform)` (√|det| of the linear part), and each
  painter converts it to its outline space. Vello hints a uniform-scale run at
  device size (it folds the scale into the font size in
  `vello_encoding::resolve`), so it takes the device amount there and the
  device amount over the scale otherwise; tiny-skia rasterises at `font_size`
  and scales the image, so it always takes the amount over the scale. Before,
  both judged the unscaled size, and under `scale(2)` Vello grew 0.094 device px
  and tiny-skia 0.19. **Blurred `text-shadow` masks** (`text_shadow::draw_masked`)
  rasterise their copy in the shadow's colour at full alpha, so a black shadow
  is not darkened and a light one is, as unblurred shadows and the kernel of
  copies already were (the copy used to be white, so every blurred shadow was
  thickened); `CallHasher::draw_glyphs` keys the mask on the run's darkening and
  hinting, not its colour. **Light-grey text on a light background is
  darkened too** (the light theme's placeholder, `#adb5bd` on white): the
  policy reads the text colour only, as Skia's does.
  lifts it to 0.93 (#2286 §5). Pins: `crates/rinch-dom/tests/stem_darkening_2286_tests.rs`
  (software: with `Off` the white row is the black row's exact inverse; on, it
  carries 8–35% more ink, the black row is byte-identical, the ink's centroid does
  not move; Vello: the encoded `font_embolden` and `hint` per run, and under
  `scale(2)` and `scale(2) rotate(90deg)`; tiny-skia under `scale(2)` against
  the same device size unscaled; a black blurred shadow unchanged, a light one
  darkened and cached apart),
  `gpu_light_text_is_darkened_and_off_turns_it_off_2286` (the same on lavapipe),
  and the module's unit tests.
- **antialias**: `ResolvedAntialias::Subpixel { order, filter }` turns on
  subpixel text (stage 3, "Subpixel (LCD) text" below; `policy.subpixel()` /
  `lcd_mode()`); `None` draws grayscale (no aliased path). The app's
  `subpixel_order`/`lcd_filter` override the OS's; `Some(SubpixelOrder::None)`
  turns it off; `SystemTextSettings::subpixel_capable == Some(false)` (macOS)
  makes `Auto` and `Subpixel` grayscale.

The OS read (`rinch::text_rasterization`, on a `rinch-text-settings` thread started
by `set_text_rasterization`, cached in a `OnceLock`; the first paint waits for it if
it is still running): Linux dlopens `libfontconfig.so.1` (`libloading`; nothing
links it here, though fontique does) and runs `FcInitLoadConfigAndFonts`,
`FcNameParse("sans-serif")`, `FcConfigSubstitute`, `FcDefaultSubstitute`,
`FcFontMatch`, then reads `antialias`, `hinting`, `hintstyle`, `rgba` and
`lcdfilter` from the match (the route Chromium's `FontRenderParams` and #2286's
`ftref2286.c` take; KDE writes its settings as `fonts.conf` edits, which only a
match applies). GNOME's GSettings/XSETTINGS values are not read. Windows reads
`SPI_GETFONTSMOOTHING`, `…TYPE` (ClearType) and `…ORIENTATION`; macOS is fixed
(grayscale, no hinting, no subpixel, `subpixel_capable: false`). `print_the_system_text_settings` (ignored)
prints the host's.

The software renderer includes **dirty region caching**: when only a few nodes change, only the affected rectangular area is cleared and repainted. Subtrees outside the dirty region are skipped during paint traversal.

**The damage is named, and it is a short list of rects** (`paint::compute_damage` → `paint::DamageRegion`). A software frame repaints what its damage names: the paint-dirty nodes, removed nodes' rects, and the drag ghost's and inspect highlight's old *and* new rects (`last_ghost_rect`, `last_inspect_rect` — both translucent, so their rect is repainted every frame they are up). **A frame in which nothing was marked paints nothing** (`repaint_none`): an empty damage used to mean "repaint everything", which made every alt-tab, focus move, selection clear, IME preedit and scroll into view a full repaint and hid sites that forgot to name their node. So a change that reaches pixels without a `DomDocument` write — a shell-written attribute such as `data-text-sel`, `data-focused`, `data-preedit`, a `scroll_offset` — must call `NodeTree::mark_paint_dirty` on its node. **A marked node that names no rect of its own still damages something**: `compute_damage` falls back to the ancestor that paints it (`boxed_owner`: the `<select>` for an `<option>` or its text, else the nearest ancestor with a box), and names nothing only for a node that is detached or has a `display: none` *ancestor* (its own `display` is not asked: a span that has just become `display: none` still has last frame's glyphs on screen). An IFC root whose text layout is rebuilt, or that stops being a root, is pushed paint-dirty too: new glyphs are a paint change whether or not its box moved. "Stops being a root" is read from the root's IFC signature, not from its `text_layout`, which a move verb has usually dropped already — a block whose only span was moved to another parent kept its glyphs on screen until #1064. `<style>`, `<script>`, `<head>`, `<meta>`, `<link>` and `<title>` compute `display: none` (the UA sheet's value; `apply_stylo_styles_to_taffy` skips them and used to leave the default `Flex`, which made a `<style>`'s CSS text an IFC line that flipped its box every structural pass and dragged the fallback to the page root). A box-less element with children (a `display: contents` wrapper, a 0x0 positioned anchor) names its subtree's painted bounds now *and* where they were last painted, so a moved tooltip anchor takes its child's old pixels along. `RinchApp::request_repaint` schedules a frame whose damage is named; `RinchApp::mark_scene_dirty` says "something changed, I don't know what" and repaints in full when nothing else is damaged (`repaint_full_unattributed` — the framework's own caller is window re-creation; the software `GameViewport` frames were one until #361 painted them inline with the viewport's box as their damage). A whole-document restyle (`NodeTree::note_full_restyle`: stylesheet, viewport, DPR, root font-size, theme) sets `whole_document_damaged` and repaints in full (`repaint_full_restyle`). The rects are up to `MAX_DAMAGE_RECTS` (8), pairwise disjoint, merged only where they overlap or one rect wastes little; each is cleared on its own, paint prunes subtrees touching none of them, and draws through one clip — their union as a path, opened with `TinySkiaPainter::push_damage_clip`, not a plain `push_clip`. That clip is not a mask to what it contains (#1007): tiny-skia draws an opaque fill through a different pipeline with a mask than without one (`Source` + lerp against scale + `SourceOver`), and the two round an anti-aliased edge pixel one LSB apart, so while it is the only clip open, a fill or stroke lying inside one damage rect is drawn unmasked, as the full repaint draws it. Any clip pushed above it — one `push_clip` skips as covering the damage included — ends that until its pop; an opacity layer does not. A box that straddles a damage rect's edge is still masked, so its opaque edge pixels inside the damage can still be one LSB from a full repaint's. Pins: `crates/rinch-dom/tests/damage_clip_pipeline_tests.rs`, `damage_clip_escape_tests.rs` (nothing drawn unmasked leaves the damage, miter spikes included), and `r2d-round` in `game_viewport_inline_tests::fresh_frames_match_the_reference_draw`. "Too big" is the **sum** of their areas against `FULL_REPAINT_FRACTION`. Pins: `crates/rinch/src/app/named_damage_tests.rs` (a local pixel oracle per converted site, each killed by removing that site's damage) and `rinch_dom::paint::damage`'s unit tests.

**The desktop GPU window repaints its damage too** (#1718). `RinchApp::build_gpu_frame` makes the same decision `build_pixels` does (`frame_damage`, shared) against the renderer's own state (`WgpuRenderer::ui_invalid_reason`: the UI texture has never been rendered, was resized, or was rendered over another base colour — composite layers came or went), and returns `GpuFrame::Full`, `Unchanged` (nothing rendered, the UI texture presented again) or `Partial`: a scene encoded with the damage's prune (`set_dirty_rects`, no clip) that `desktop::render_partial` renders **untranslated** into a target reaching from the surface origin to the damage's far corner and copies each damage rect from into the UI texture. Untranslated because Vello's output depends on where the target puts a pixel: a scene translated by 31px drew a glyph edge 7 levels off. Vello renders before the surface is acquired, so a skipped present loses no damage. Inline `RenderSurface` frames mark their nodes paint-dirty on GPU as on software. `build_scene` (embed, Android, DevTools) still encodes everything; after a partial frame it re-encodes (`scene_is_full`). Measured (1200x800, RADV 890M, a 170-row `Tree` beside a long document, hover sweep): full encode 0.3–0.7 ms + Vello 1.2–2.3 ms per frame, partial 0.17–0.32 ms + 0.54–0.92 ms; about 0.4–1 ms of Vello's cost is fixed per render whatever the target. Pins: `app/gpu_partial_frame_1718_tests.rs` (GPU-free frame decisions; `#[ignore]`d pixel identity with a full render over a hover and scroll sweep, and the probe).

**The composited frame handed to the app** (#2215, `rinch::window::on_frame_composited`, `crates/rinch/src/composited_frame.rs`). `WgpuRenderer::present` has one tail for its three paths (layers, split surfaces, the plain UI texture): submit the frame's encoder, then, on the app's window only (`hands_over_frames`, set in `RinchRuntime::renderer_started`; the DevTools renderer never), `hand_over_frame` with `desktop::presented_frame` — the texture the screenshot reads (`presented_texture` is its first half): the composited texture (`FrameAlpha::Premultiplied`, the surface's format) while there are layers or split surfaces, else the UI texture (`Straight`, `Rgba8Unorm`). Both are made `TEXTURE_BINDING | COPY_SRC` already, so **no intermediate texture or copy exists for the hook**: the swapchain texture (`RENDER_ATTACHMENT | COPY_DST` only, and not `TEXTURE_BINDING` on every backend) is never handed over. The UI texture holds the whole frame after a partial frame because `render_partial` copies its rects into it in place. With no handler registered `hand_over_frame` is one thread-local read; with one it creates one encoder, calls the handlers (batched, `untracked_handler`) and submits it before `queue.present`. A frame whose surface was not acquired (occluded, timeout) returns before the tail and hands nothing over. Handlers live in a thread-local list with the pointer-motion lifetime (component-owned through `on_cleanup`, else the `FrameSubscription`'s `Drop`). `present` is the surface acquire and present around `compose(target)` (composite, submit, hand over), which a test drives through `WgpuRenderer::offscreen` (cfg(test); `surface` is `None` only there) with a texture standing in for the swapchain's. The handed-over texture also carries rinch's own usages (`STORAGE_BINDING | COPY_DST`, `RENDER_ATTACHMENT`): the docs say read it only, since a write corrupts the next partial frame's base. A buffer that a copy in `frame.encoder` writes cannot be mapped inside the call (the copy is unsubmitted; `map_async` fails), so the documented readback records into the app's own encoder, submits it on `frame.queue` and maps; the callback runs at a `device.poll`, which rinch does once per presented frame. Pins: `app/gpu_composited_frame_2215_tests.rs` (`#[ignore]`d: the layered path through `compose` with the documented readback, equal to the stand-in's pixels, and the trap failing; a full then a partial frame, the handed-over texture read back by the handler's own copy through `frame.encoder` — left box from the full frame, right box in its new colour — and a cancelled handler not called), `composited_frame::tests` (lifetimes), `presented_texture_tests` (which texture and alpha).

**A dirty region clears what the last paint drew, not what the tree says now.** Each node keeps what it was last *painted* with — `prev_layout` (its box) and `Node::painted` (its own `box-shadow`/`outline`/`text-shadow` reach — the last is one and a half blur radii (three standard deviations, where the blurred mask of `paint/text_shadow.rs` stops) around its offset, measured from the border box, so text that overflows its box casts past it; an IFC root's reach also holds the `text-shadow` lists its inline elements declare, since it draws their text (#1048; on a line `text-overflow: ellipsis` cuts too, #1065), and what it draws past its unshifted line boxes — `vertical-align`-shifted text, its inline elements' backgrounds and borders (which reach past the line by their padding and border), wavy underlines — from `paint::text::inline_ink_past_lines`, the function the clip proof reads (#1815, #1817; a root that clips its overflow counts none of it, and shifted text casts its shadows from where it is drawn) — its own transform, and whether it clipped and how it was positioned — #909) — all **parent-relative**, and only the paint that consumes the region writes them (`NodeTree::consume_paint_dirty`, O(1) per dirty node — O(text runs) for an IFC root, and a box-less inline's consumption re-reads its root's ink — always, not only while an inline element casts its own `text-shadow`, since a transition writes an inline's padding with no re-shape and the root would keep a pill's old reach (#1815) — once per root per consume, however many of its members were dirty, and not when the root was dirty itself; each span walks only the lines its byte range is on (`inline_span_line_visits`, pinned in `perf_regression_scenarios`); the software dirty-region frame, a full software repaint and `build_scene` all call it). `paint::previous_painted_rect` sums them up the box-tree chain, so a node's old rect is exact however many resolves ran since, when an *ancestor* moved rather than the node, and when its ink or transform changed in the same frame. `compute_dirty_region` takes, per dirty node, that painted rect and its current box grown by its current own ink (a box-less inline names its IFC root's rect, grown by that root's ink, where it is and where it was painted); a **positioned** box (`absolute`, `fixed`, `relative`) whose position changed is also grown by its subtree's reach (`opacity_layer_bounds`), which is what carries an untouched overflowing child along with a dragged panel or a reflow-shifted `relative` row (a subtree that walk cannot bound repaints in full). Static boxes are not walked — a reflow shifts every row — so a static box's in-flow child whose own ink reaches past a reflow-shifted parent is not covered (#880's probe J2, also on main); **a box whose place in the paint order changed is walked whatever its `position`** (#1384): `PaintedState::order` keeps a `stacking::PaintOrderKey` — whether the box was an entry of an ancestor's sequence (`paints_at_stacking_root`), the `z-index` that entry sorted by, and whether it was a stacking context — and `paint::reorders_subtree` compares it with the node's answer now, which is how a restyle and a transition or animation tick (which writes `computed_style` with no cascade) are caught alike. **Any** difference owes the subtree's reach, now and where it was painted. There is deliberately no shortcut for a positioned `z-index: auto` box that becomes a context at `z` 0 with its own entry staying put: besides trapping descendants that sort away from 0, becoming a context starts group opacity over everything under it and puts its entry's clip chain around descendants that otherwise escape it (#549) — a shortcut on "no `z-index` descendant" left all of that stale (review of #1399). The rects are clipped by the box's own clip chain, unless something under it is absolute or fixed against a box above it (`subtree_has_escaping_absolute`; skipped when the box is and was a containing block) — wherever that descendant sits, inside the box's own rect included: then they take the chain an absolute box at the node would have, so the clippers such a descendant escapes are dropped and the ones above its containing block still clip. A flip scrolled wholly out of a clipper repaints nothing unless it holds such a descendant. **Three things repaint the whole window on a flip frame.** A subtree the reach walk cannot bound: a `position: fixed` descendant, `sticky`, or **more than its 512-node budget** — so a flip (a fade crossing opacity 1, a transform starting) on a box holding more than 512 nodes repaints in full, where `main` repainted the box's rect (a 200x200 clipped list of 300 two-node rows: 240000 px against 41616; the first and last frame of a transition, not the ones between). An absolute with a clipper between it and its containing block, that clipper being under the flipping box (`subtree_has_partially_escaping_absolute`): the reach walk narrows it at that clipper (#550), so the reach does not hold it. And a flipping box that itself clips, or clipped when painted, is not a containing block both now and then, and has an absolute under it with no containing block in between (#1408: the box started or stopped clipping that descendant in). The two escape walks are linear in the subtree and **not** bounded by the 512-node budget: the reach walk answers for an `opacity: 0` or collapsed (`estimated_height`) child without entering it, and the escape walks skip only `display: none` (20000 rows under an `opacity: 0` child: 4.1 ms against 1.0 ms for a colour change, debug build). A node whose order held pays one compare and no walk. Before it, opacity crossing 1, a `z-index` or `position` change, or a transform starting left a `z-index` descendant that overflows the box in its old order over a sibling (10000 of 10000 px). Not covered: an `opacity`, `filter` or `transform` **value** change on a box that stays a stacking context still damages only the box's own rect, so in-flow content overflowing it keeps its old pixels (#1395, pre-existing); and the region stops being measured once it reaches `paint::FULL_REPAINT_FRACTION` of the surface. A removal records each removed node's painted rect, children before the parent's state is forgotten. This is what let the editor overlays (caret, selection, node outline) and the inset fast path (#280) stop forcing a full repaint per keystroke and per drag move: a second resolve used to overwrite the old rect, and the forced full repaint also hid a move and an ink change landing in one frame (#880's review). An IFC's direct write of an atomic inline's position pushes the box paint-dirty when it moves, so a chip moved by the text before it is repainted, overflowing children and all. `build_scene` also sets `has_previous_frame = false`, so a software frame after a GPU one is full. The paint prune (`paint_node`, `paints_nothing_without_visit`) tests each box grown by the same own ink, so damage over a shadow's or an outline's bleed and not its box repaints it there (#889 item 1) — except an atomic inline's (`inline-block`/`-flex`/`-grid`): it is drawn by its IFC root, which is culled by its own box, so damage over only an inline-block's shadow bleed below its line is still not repainted (pre-existing; #889). Not covered: the `text-shadow` of text that overflows its box, a `filter` reach, and the #550 partial escape of an absolute from an `overflow` box inside a subtree that **moved** without changing its paint order (a flip repaints in full for it, above). Pins: `crates/rinch/src/app/repaint_old_rect_tests.rs` (and `build_scene_consume_tests.rs`, `--features embed`), `stacking_flip_damage_1384_tests.rs`, `stacking_flip_damage_1384_review_tests.rs`.

**Damage is clipped by what clips the damaged box** (#909). Every rect `compute_damage` names for a node — where it paints now, where it was painted, its IFC root's or owner's box — is intersected with that node's clip chain (`paint::clip_chain_bounds`) before it joins the region, and a removed node's rect is intersected when it is recorded; so a row a scroller moved below its viewport names nothing, and a keyed `for` change in a 300x400 scroller repaints 304 x 404, not the column down to the window's edge. The walk mirrors what clips a box in paint and errs wide in three places: a `position: fixed` box takes no clip above it, an `absolute` box skips every clipper below its containing block (whose own clip it takes, as `Collector::span` does), and a `display: contents` or degenerate clipper is not taken. The **current** chain answers for where a box paints now — it is exactly what the next paint clips with. The **painted** chain answers for its old pixels from each node's `PaintedState` (`clips`, `position`, transform) and `prev_layout`, walked along the **current** parents. Two things make that true. Every style change pushes its node, so a painted state holds between two consumptions; and the paint consuming a whole-document restyle, which pushes nothing, re-reads **every** painted node's state (`NodeTree::consume_paint_dirty` — O(n) on a frame that has just restyled and repainted all n, and it also stops ink and transform going stale after a theme change). And a move to another parent (`append_child`, `insert_before`, `insert_child`, `replace_node`'s incoming node) records the subtree's old rects under the old chain and forgets its painted state first (`record_pixels_left_by_move`), exactly as a removal does, so a moved node is not placed along parents it was not painted under; a reorder within one parent and a never-painted or parentless node pay nothing. A node never painted gives no clip. Every detach does the same before the subtree leaves (#966): `remove_node`, `remove_child`, `replace_node`'s displaced node, `set_text_content`'s orphans, and what `set_inner_html` and the pseudo-element pruning free — they used to repaint only the parent's box, leaving an overflowing child or a shadow on screen, and an orphan appended elsewhere had its old rect placed (and clipped) along its new parents. `Frame::Painted` places a box by the `position` it was **painted** with (`PaintedState::position`, #961), so a box that goes static↔fixed in one frame clears where it was — except that the painted frame still reads the **current** IFC content-box offset (`ifc_content_box_offset`), so an atomic inline that goes out of flow in a padded IFC root keeps its old pixels (#1621); a box that becomes or stops being a containing block changes its paint order, which the #1384 rules above already answer (in full when it clips, #1408). A rect clipped away entirely still counts as *named*, so the `boxed_owner` fallback does not repaint the clipper's box in its place. The contents-wrapper subtree rects are not clipped (one chain cannot speak for a subtree whose boxes may escape it). Pins: `crates/rinch/src/app/damage_clip_chain_tests.rs`, `damage_position_change_961_tests.rs`, `detach_damage_966_tests.rs`.

**The software painter's caches.** `TinySkiaPainter` keeps three things across draws and frames (`paint/skia_painter.rs`). **Rasterised glyphs** now come from the glyph-mask atlas both painters share (#2286, below): keyed by font blob id, face index, exact device size, hinting and stem darkening as the thread's text-rasterization policy gives them (#2286 stage 1; on the grid the device amount, off it the amount over the transform's scale), variation coords and a quarter-pixel x bucket. On the atlas's grid (no rotation, skew or uneven scale) a glyph is blitted at a whole device pixel, so tiny-skia's `draw_pixmap` never sees a fractional translation; left to it, a translation of exactly `k + 0.5` covers pixels from `k + 1` but samples the image's row (column) 1 there, so the glyph lost its first row and its first column and drew its last twice. Layout puts a line on a half pixel wherever centring splits odd space (a table cell's `vertical-align: middle`), and since parley 0.12 snaps baselines inside the layout, the sum is often exactly `k + 0.5`: text there read with flat-cut tops and a sliver off a line's first glyph. Pinned by `half_pixel_glyph_blit_tests` (vertically the same pixels one row down; horizontally, since #2286, the half-pixel mask with the same ink). Under any other transform the painter draws the bucket-0 mask at the run's size through it, rounded by `glyph_placement`, as it always did. swash gets one stable `CacheKey` per font file, where `FontRef::from_index` minted a fresh one per call and defeated swash's own scaler cache. A colour **bitmap** glyph (CBDT/CBLC — Noto Color Emoji, Android's emoji — or sbix) is kept the same way (#804; it painted nothing before): read at its strike's own ppem (the smallest at least the font size, else the largest), premultiplied, then area-averaged to the font size (`glyph_atlas::color_bitmap_glyph`, `scale_strike`) — premultiplied first because swash's own resize filters straight alpha and drags the colour under transparent pixels into the edge; a face with no colour strikes takes the one swash `Render` it always did. The GPU path (Vello) draws colour glyphs itself. **Clip masks and layer pixmaps** stay surface-sized (tiny-skia takes a mask only at the pixmap's size) but come from a pool and carry the rect that can be non-zero: a clip is filled, intersected and later zeroed over its own bounds — and not at all when its shape (a rect or rounded rect, kept axis-aligned by its transform) covers every pixel the enclosing clip can let through, since the intersection would be the enclosing mask byte for byte, and filled but not intersected when the enclosing clip is 255 wherever it can be non-zero: a partial repaint inside a scroller no longer fills the scroller's whole box, and one around a scroller no longer intersects it (#907; the painter tracks each mask's fully covered rect, and debug builds check every such claim against the mask itself) — and a layer composites back — and is cleared — only over the area its draws touched (the painter's own bookkeeping; it still ignores `push_layer`'s bounds, so tiny-skia stays blind to `layer_bounds` bugs). **Premultiplied images** live on `DecodedImage::premultiplied()`, made on the first software paint; a live frame source is still premultiplied per draw — unless `SurfaceWriter::submit_frame` found every pixel opaque (checked on the submitting thread, `SurfacePixelData::opaque` / `PaintImage::opaque`), in which case it is never premultiplied, and a draw with no rotation or skew, a positive scale (scaled draws included), whole-pixel destination edges and no clip partial where it lands is a row copy (`TinySkiaPainter::copy_opaque_image`, #361; byte-identical to `draw_pixmap`, pinned by `opaque_image_copy_tests` and the randomized `opaque_image_copy_differential_tests`; counter `opaque_image_copies`). The frame's pixels are an `Arc` shared between the surface and paint (`SurfacePixelData::data`), so collecting a frame for a paint copies nothing. **What stays resident:** the glyph atlas (above, shared with the GPU painter); each translucent cached image's premultiplied copy, beside its straight-alpha pixels for as long as the image is cached (an opaque image keeps none); and the pooled surface buffers — at most 8 masks (1 byte/px: 8.3 MB each at 4K, 2.6 MB at 1080x2460) and 8 layer pixmaps (4 bytes/px: 33 MB each at 4K, 10.6 MB at 1080x2460). The pool is trimmed at the end of every software frame (`TinySkiaPainter::end_frame`, called by `build_pixels`) to the most either kind had open at once in the last 8 frames, so a deep nesting is given back 8 frames later; an app that goes idle straight after one keeps what that frame needed until it paints again, is resized, or is dropped (`paint_surface_trims` counts releases). Measured at 1200x800, release, `skia_painter_perf_tests` (before → after): a 40-paragraph text page 116 → 5.4 ms, 40 rounded scrollers 112 → 6.1 ms, 30 opacity layers 205 → 7.9 ms, 60 images 5.6 → 2.4 ms, a 200x40 partial repaint 5.6 → 0.23 ms. What remains of a text frame is the per-glyph `draw_pixmap` (about 0.85 µs a glyph). `TinySkiaPainter::set_reference_mode` turns all of it off, and `skia_painter_oracle_tests` demands byte-identical pixels against it, cold and warm; counters `glyph_atlas_*`, `clip_mask*`, `paint_layers`/`layer_px`, `paint_surface_allocs`/`paint_surface_trims`, `image_premultiplies`.

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
the scroller and the line it is measured through `overflow_absolutes` (#770, below).
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

**Absolutes below the first box (#770).** The walk stops at the first
**box**, so an absolute laid out in any box but its scroll container — under a
`position: relative` wrapper that is no scroll container, a level below its
containing block, or escaping a scroller between it and a containing block
further up — was in no box's range (before #765, in the wrong box's: the
phantom bar). css-overflow-3 gives it to the **nearest scroll container at or
above its containing block** (Chrome 156: a scroller between the box and its
containing block gets nothing, one above the first gets nothing, a
non-scrolling box between passes it up transformed). The layout read-back
lists every laid-out absolute box with its kind (`NodeTree::overflow_absolutes`,
one push), and the container each belongs to is `out_of_flow::overflow_scroller`'s:
from the
containing block — the layout parent, an ancestor further up, or for a
positioned span the block holding its line (`chain_end`) — up the
**containing-block chain** (an in-flow box to its parent, an `absolute` box on
the way to its own containing block, a `fixed` one to the viewport: none) to the
first box that clips; a scroll container (`overflow` `hidden`/`scroll`/`auto`) takes it,
an `overflow: clip` box drops it (it clips what overflows it; not modelled:
`clip` on one axis with `visible` on the other). The initial containing block's
scroll is the page's, which in rinch is the `<body>`'s (UA `overflow-y: auto`;
its scroll carries an ICB box, `chain_to_containing_block`), and the `<html>`
box's when the body does not scroll or does not hold the box. A box its layout
parent already measures (`contributes_to_scrollable_overflow`) is left out.
`content_extents` ends with `extend_with_overflow_absolutes`, which reads a
**memo**, `paint::scrollbar::OverflowAbsolutes` (beside the list):
the first range asked for after the geometry changed walks each listed box up
once (`place_listed`: it finds the container as `overflow_scroller` does and
sums the offsets on the way — each box's layout offset less its scroll, since
`replace_after_scroll` writes an escaped scroller's scroll into the box's
`layout`), and keeps one `(container, right, bottom)` per container; every
later call is one binary search. It is keyed on `HitCache::shape_generation`,
which every invalidation moves — a layout, a mutation, a restyle, a tick that
changes a `HitStyleKey`, and `out_of_flow::replace_all` (it moves boxes after
the clamp below measured them) — and a scroll moves only when it reaches an
extent (a box that does not clip on both axes): a scroll of a clipping box
leaves every listed box's measure where it was. A box with a **transform** on
its way (its own or any box's to the container, identity included) is listed
`live` instead and measured by every call (`border_box_in`, transforms composed
as paint composes them), because a tick writes a transform's value without an
invalidation; so is a box whose way up the walk does not follow (an `absolute`
box between its containing block and the container). Either way, less the
container's end padding, as an `absolute` child counts. **Cost:** a document
with no absolute box pays an `is_empty` per call; with 500 badges a warm call
is a binary search (`abs_badge_extents`), and the read-back pays one push per
box. Layout's clamp first asks `content_extents_in_flow` — the range without
the listed boxes, which only add to it — and measures them only for an offset
past that, so a relayout of a long list scrolled anywhere but its end measures
none of them (`abs_badge_relayout`). An absolute box with no area counts nothing, here and as a child (Chrome
156: `0x500`, `10x0`, `0x0` boxes scroll nothing). Rows:
`tests/abs_scroll_range_770_tests.rs`. Still not counted: an **in-flow**
descendant's overflow below the first box (#2054), so a box that
overflows its `height: 50px` parent scrolls nothing where its absolute twin
now does.

**A range that moves damages the bars.** A bar is drawn from its container's
range, and the range moves with content the container's own box does not
change for — a box moved past its edge, a row grown, a child removed — so
nothing named the bar: a partial frame kept no thumb where one had appeared,
or the old thumb's length. It was reachable with a narrow in-flow row before
#770; #770 made it reachable from every absolute box on a page (the split
runtime's random sequence moved a 200px bar to `left: 210px` in a 400px page,
and the partial frame drew no page bar where the full frame did — batch
#2062). Now `NodeTree::painted_bars` holds each scroll container's
`Scrollbars` (scale 1) as the last paint consumed it, and
`paint::bar_watch_scrollers` names the containers to check: every scroll
container that is an ancestor of a paint-dirty node whose **geometry** may
have moved (never painted, `prev_layout != layout`, its unrounded Taffy rect changed (#2116), its `position` changed, or
a transform on it now or when painted) or of a parent a subtree left
(`NodeTree::bar_check_parents`, pushed by `mark_subtree_paint_dirty` for the
subtree's root), and every dirty container itself. Any container whose range
a box reaches is its ancestor (an absolute box's is at or above its
containing block). A dirty node whose geometry held — a colour, a caret blink
— measures no range. `compute_damage` names, for each whose bars differ from
the record, the strip of each axis whose track changed (10px along that edge:
the 6px bar 2px in, and its anti-aliased edge; both strips when anything but
the tracks differs), clipped by its clip chain and attributed to no node (a
bar is drawn by its container's paint or after its last positioned
descendant, #1717). The two strips share one rounded edge, so they stay two
rects instead of merging into the container's box. `consume_paint_dirty`
records the same set, and after a whole-document restyle (which pushes
nothing) re-reads every scroll container, so a container with no record
painted no bar. Pins: `crates/rinch/src/app/scrollbar_range_damage_tests.rs`.

Limits, all pre-existing. **#769 is fixed**:
the wheel's finders — `find_scroll_container` / `find_horizontal_scroll_container` (test-only since #2084's review; the wheel now walks with `wheel_scroll_target` over `scrollbar::can_scroll_toward`, itself on `overflows`),
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

**A scroller's range is Chrome's whole-pixel range (#2116).** Taffy rounds
a box's *location* on its own and its *size* from its absolute edges, so the
rounded rects of a container's children need not add up to its rounded size:
the in-window menu bar's two-item dropdown (border 1, padding 4, two 32.8px
rows and a separator) fits exactly and measured a pixel past its content box at
some vertical positions, so it painted a bar and took the wheel. An IFC root
did the same from the other side: its lines are unrounded (three 24.8px lines
are 74.4px) and its box rounded to 74. Chrome 156 rounds the scrollable
overflow and the padding box each to a whole CSS pixel (from 1/64px layout
units), independently of the box's position and of the device scale (measured
at 1, 1.25 and 1.5), and shows a bar when the first is larger: 0.4px of
overflow is none, 0.5px is a pixel's range. So for a box that scrolls on either
axis, `content_extents` (`scroller_extents`) measures from Taffy's
**unrounded** layout — the container's size, and the rect of every box Taffy
laid out directly in it (`child_rect`; a box whose `layout` is not a rounding
of its unrounded rect, such as an absolute one moved by a scroll, is read
rounded) — and an overflowing scrolling axis reports the content box plus that
whole-pixel range, so the bar, `overflows`, `can_scroll_toward`, the wheel's
walks, layout's clamp and `scrollHeight` all read one number; an axis that
fits keeps its measure, capped at the content box. The rounded rects also
mis-sized real overflow: a 100.4px box with 2.3px padding and ten pixels of
overflow measured a range of 8.3 at some positions (Chrome: 10). The range
then lands on the device grid as before (#1543). Pins:
`rinch-dom/tests/subpixel_overflow_2116_tests.rs` (the dropdown, an IFC root,
Chrome's threshold at three client heights, and whole-pixel overflow, each at
ten positions and three scales; the half-pixel thresholds on both sides; the
horizontal axis). On the web the browser decides, and these are its answers
for the same markup.

**Cost.** Only the in-flow boxes' margin boxes are read unrounded (the walk
keeps them apart from its other contributions, `Measured::flow`), and that
walk is memoized per container (`scrollbar::ExactFlow`) keyed on
`NodeTree::layout_epoch` — moved by a layout pass, a text-only IFC rebuild,
`out_of_flow::replace_all` and a subtree's removal, and deliberately not by a
paint-only restyle or a scroll (`HitCache::shape_generation` moves on every
paint-dirty push, so a hover frame walked every row unrounded: +3.8% on
`shell::hover_frame`). Every call makes the rounded walk it made before,
monomorphized so it has no per-box unrounded branch
(`extend_with_children::<false>`), and then a lookup. Layout's clamp first
asks the rounded in-flow measure (`content_extents_in_flow_rounded`) and
skips any offset more than `ROUNDED_RANGE_SLACK` (4px) inside it, so a
relayout of a long list scrolled anywhere but its end measures nothing
unrounded. Callgrind against main (`abs_badge_extents`, 500 rows): 143904 →
142082; `abs_badge_relayout` unchanged.

Two limits. Only an in-flow box Taffy laid out directly in the scroller is
read unrounded: an absolute, hoisted, `sticky` or `relative`-offset or
transformed box *where it is drawn* (whose `layout` is written after Taffy,
or which is measured with the rounded walk's other contributions) and an
inline one placed by its line are still read at their rounded rects, against
the unrounded client box, so their sub-pixel overflow can still decide a
pixel the wrong way. And only the *difference* is Chrome's:
`scrollHeight`/`clientHeight` (`content_extents`/`visible_extents`) are the
content-box measures, each of which can be a pixel off Chrome's rounded value
while the range between them is right.

**A sub-pixel change that moves the range damages the bar.** The range is
measured unrounded, so content growing from 100.4 to 100.55px in a 100px
scroller gives it a pixel's range while every rounded rect holds (the row stays
`{y: 10, h: 90}`). `PaintedState::unrounded` records each node's unrounded
Taffy rect at `consume_paint_dirty`, and `geometry_may_have_moved` compares it
last, after the rounded checks, so a dirty node whose box held at both
precisions still measures no range. Pin:
`scrollbar_range_damage_tests::a_sub_pixel_growth_that_crosses_the_whole_pixel_range_damages_the_bar`.

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
keyword, and resolves against each shape's own `color`), the inherited
stroke properties (see **SVG strokes** below; a usable declaration replaces
the inherited value), and
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
rule. Pins: `crates/rinch-dom/tests/svg_paint_var_1800_tests.rs`.

**SVG strokes.** Every stroke property of the SVG 2 painting chapter is read
by `SvgInherited::derive` through `declared` (the same rule as
`declared_paint`: the inline `style` declaration, then the presentation
attribute, `var()` substituted; a bad attribute is ignored and the parent's
value applies; a bad literal `style` value is dropped so the attribute
applies; `initial` reads as `inherit`). None of them is a property in
rinch's Stylo build (Gecko-only there), so a **stylesheet rule is not read**
for any of them; `opacity` is the exception, below. All inherit through `<g>`
except `opacity` and `vector-effect`. Initial values are SVG's: `stroke-width`
**1** (it was 2 before; every built-in icon sets its width), butt, miter,
miter limit 4, no dashes, opacities 1.
- `stroke-width`, `stroke-dasharray`, `stroke-dashoffset` are
  `<length-percentage>`s (`parse_svg_length`: a number, `px`, `em` of the
  element's font size, the absolute units; a percentage is of the viewport's
  normalized diagonal, `sqrt((vbw² + vbh²) / 2)` of the viewBox), resolved to
  user units where declared. A negative width is ignored; a zero width
  strokes nothing (tiny-skia would draw a hairline).
- `stroke-dasharray`: an odd list is repeated (tiny-skia's `StrokeDash`
  takes only even ones); all zero, or `none`, is solid; a negative or
  unparsable value makes the declaration invalid, so the parent's applies
  (Chrome 152: alone it is solid, under a dashed `<g>` it is the group's
  dashes). The offset is brought into the first period (`rem_euclid`) so
  kurbo (Vello) and tiny-skia agree on a negative one.
- `stroke-miterlimit` below 1 is ignored. `stroke-linejoin` `miter-clip` and
  `arcs`, and any unknown cap or join keyword, are ignored (Chrome 152 does
  not implement the two SVG 2 joins; an unknown keyword used to *replace*
  the inherited join with miter).
- `stroke-opacity`/`fill-opacity` (number or percentage, clamped) multiply
  the paint's alpha.
- `opacity` (not inherited) on a shape or `<g>` is a layer, so a stroke over
  its own fill, or overlapping children, are composited whole. It is read
  from `computed_style.opacity` (Stylo resolves it from a rule or `style`),
  else the attribute; a rule saying exactly `1` cannot be told from none, so
  there the attribute wins. A shape's layer is its ink box; a group's is the
  `<svg>` box (its content is not measured first), which a browser clips to
  anyway. The `<svg>`'s own CSS opacity is `paint_node`'s; its `opacity`
  attribute is a layer here.
- `paint-order` (inherited): only whether the stroke precedes the fill
  matters (no markers are drawn).
- `vector-effect: non-scaling-stroke` (not inherited): the outline is
  transformed into device space (`shape_path` + `apply_affine`) and stroked
  with the identity transform, its width and dashes times the device scale,
  so they are CSS px on screen whatever the SVG and CSS transforms. The fill
  is unaffected. The other values are `none`, as in Chrome.
- Not drawn: markers (`marker-*`). A dash pattern far shorter than a pixel is drawn as
  asked, however many dashes that makes.

Both painters take the `kurbo::Stroke` (`to_skia_stroke` already carried
dashes and the miter limit into tiny-skia). Ink bounds: tiny-skia's draw and
damage-mask bounds already grow by `width / 2 * max(miter_limit, 1.5)`;
Vello's routing bounds used `width` times the transform's stretch, which a
miter limit above 4 outgrew, and now use `stroke_outset` (half width times
the miter limit at a miter join, `√2` at a square cap). The `<svg>` box is
still what damages SVG content, so a thick stroke past it is unclipped and
undamaged as before. Pins:
`crates/rinch-dom/tests/svg_stroke_properties_tests.rs`, one table of
Chrome 152 pixel answers (canvas `getImageData`) run on tiny-skia and, in
the `#[ignore]`d `gpu_*` test, on Vello.

### Glyph-mask atlas (#2286)

Both painters draw text from one cache of swash coverage masks,
`paint/glyph_atlas.rs` (feature `glyph-atlas`: on with the software painter, and
in the `embed` build without it). Stage 2 of the text-antialiasing plan in
#2286's design study.

- **Key.** Font blob id and face index, the exact device font size
  (`f32::to_bits`), hinting, the stem darkening in device pixels (stage 1's
  `run_darkening_px`; swash embolden with the `(0, -amount)` re-centring offset), variation coords, glyph id, and the horizontal
  sub-pixel bucket: `place` rounds the device x to the nearest quarter pixel,
  and a fraction past the last bucket rounds up to the next pixel's bucket 0
  (glifo clamps it to bucket 3 instead, up to 0.25 px off; this is at most
  0.125). The baseline is rounded to a whole device row, half up, as hinting
  assumes and as Chrome draws text: so on the GPU too a line's glyphs now sit
  on whole rows (Vello's outlines were placed at the exact fractional
  baseline), the same rows as the software painter's.
- **Transforms.** `atlas_scale` admits a transform with no rotation or skew and
  the same positive scale on both axes: the glyph is rasterised at `font_size ×
  scale` (so a HiDPI factor or a CSS `scale()` stays crisp) and its origin
  mapped to the device. Anything else is not on the grid: Vello draws outlines,
  tiny-skia the bucket-0 mask through the transform.
- **Vello.** Each glyph is `Scene::draw_image_mask`, a fork addition: an image
  draw whose alpha is coverage, painted in a tint (`fine.wgsl`'s `image_paint`,
  mask mode 1). The image is the mask with the coverage in all four channels
  (premultiplied), made once (`GlyphMask::gpu_image`), one `Blob` for the
  mask's life, so Vello's image atlas (keyed by blob id) uploads it once and
  keeps it resident while it is drawn; drawn at a whole device pixel with
  `ImageQuality::Low`, it is copied texel for texel, under Area and MSAA alike.
  Not drawn from the atlas, and counted in `glyph_outline_fallbacks`: a run in
  a face with colour glyphs (a COLR, CBDT or sbix table, checked once per font,
  so nothing is rasterised to be thrown away; Vello's own COLR and bitmap
  paths), a glyph swash cannot render (cached as failed; a font swash cannot
  read at all goes to Vello only if skrifa can, as Vello's `draw_glyphs`
  panics on an unreadable font), a run whose size is animating (below), a device size past
  `ATLAS_MAX_FONT_PX` (64), a run with a glyph transform (synthetic italic), a
  non-solid brush, and a transform off the grid. A split frame routes a run by
  the union of its masks' rects.
- **Why a tint and not a colour baked into each image.** Baking works with no
  fork change (the design's prototype), but multiplies the masks by the colours
  text is drawn in, and stage 3's subpixel text needs a per-channel composite
  against the destination pixel, which only the fine shader can do: mask mode 2
  (below).
- **Ownership.** One atlas a thread (`thread_atlas`), shared by every painter on
  it: the window's Vello painter, its snapshots and split slices, a software
  window, the text-shadow scratch painter. A painter can be given its own
  (`set_glyph_atlas`), which the fixtures do. Counted per painter: the shell adds
  the window painter's `take_glyph_stats` to a GPU frame, and the software
  painter's `SkiaPainterStats::atlas` to a software frame.
- **Eviction.** Each entry remembers the generation it was last drawn in
  (`begin_frame`, called from each painter's `reset`). Past the budget (32 MiB
  accounted: mask bytes, the GPU copies, 128 a entry) the oldest go first, down
  to three quarters of it, but **never what any painter's last or current
  frame drew**. Each painter registers with the atlas (`register_painter`,
  given back on drop) and starts its frames with `begin_painter_frame`; the
  atlas keeps, per painter, the generations its last two frames started at,
  and protects everything drawn since the oldest of those (painters that have
  not started a frame in 64 generations, a hidden window or an idle scratch
  painter, protect nothing). So a screen bigger than the budget (6,000 distinct
  CJK glyphs at 2x are about 40 MB) is not rasterised again on every repaint
  (#2301 review: it was, 2,550 glyphs a static frame), and two windows sharing
  the thread's atlas, each a big screen, do not evict each other's (round 2:
  they did, about 4,000 glyphs a repaint). The bound is the budget, or what the
  painters' last frames drew, whichever is larger: one huge text node (all its
  lines are painted, off-screen ones too, #2314) can take a frame far past the
  budget. That overshoot is given back as each painter starts its next frames,
  whether or not they draw text, once nothing drew it in the protected frames;
  an app that paints no frame at all keeps it until it does. Past 512 runs, the
  quarter drawn longest ago go whole, none protected.
- **Animated sizes.** A `font-size` or `scale()` animation, or animated
  variation coordinates, would make a new set of masks every frame: about twice
  the CPU of outlines. `SizeChurn`, one per Vello painter, calls a run's size
  (device size, emboldening, hinting, coords) new when its font file did not
  draw it in the last 8 frames (`SIZE_MEMORY_FRAMES`); a new size in a font
  file that drew new sizes in each of the two frames before is animating and is
  drawn as outlines. When it stops changing it is no longer new and is back on
  the atlas. Remembering 8 frames matters for partial frames, which draw only
  their damage: a caret line and a clock in one face repainted on alternate
  frames, or a size that comes back (A, B, A), are not new (round 2 of the
  review: they were, and stayed on outlines). A still page draws no new size
  after its first frame, and a translation changes no size, so neither falls
  back; a static heading beside an animating run in the same face keeps the
  atlas. Per painter, because a frame may reset several painters; a painter
  made fresh each frame never sees an animation. The global switch is
  `TextRasterization::gpu_glyph_outlines`. **The switch is visible**: outlines
  (Vello's rasteriser) and masks (swash) differ, so the frame an animation
  starts drawing outlines, and the frame it settles back onto masks, moves
  glyph edges by about a quarter pixel and changes the ink by about 1%
  (measured in the review: +0.23 to 0.42 px horizontally, 0.9% ink). Accepted:
  a mask per animation frame costs about twice the CPU of outlines. Evicting is safe at any point
  between lookups: a Vello scene holds its own reference to every image it
  draws.
- **GPU lifetime.** rinch holds no GPU object here. The texture is Vello's image
  atlas, the renderer's: a renderer made again after a device loss starts empty
  and uploads each mask when a scene first draws it, from the blob. Its own
  eviction (age 0, "Vello Fork" in `app-and-windows.md`) drops what a render does
  not draw when it needs the space. `glyph_atlas_uploads` counts the GPU copies
  rinch makes, not Vello's re-uploads.
- **Text shadows.** An unblurred shadow is the same run in the shadow colour:
  the same masks, another tint. A blurred one is rasterised by the scratch
  software painter, from the atlas; its cache key (`CallHasher::glyph_x`,
  `glyph_y`) is the atlas's placement, the quarter pixel and the row.

Pins: `glyph_atlas::tests` (placement, keys, buckets, the GPU copy, colour
glyphs, eviction order, a frame over budget, size churn),
`crates/rinch-dom/tests/glyph_atlas_2286_tests.rs` (counted frames: a static
page over budget rasterises nothing again; `font-size` and `scale()`
transitions draw outlines with 0 uploads mid-animation and settle back; a
`translateX` one never falls back; a colour face is not rasterised; a glyph
swash cannot render is an outline; an unreadable font does not panic), the `#[ignore]`d
`crates/rinch/tests/gpu_glyph_atlas_2286.rs` (GPU pixels equal to the software
painter's at 1x and 1.5x, one mask in two colours, every fallback, a new
renderer, eviction under the GPU), and the fork's `image_mask_draw_data`.

### Subpixel (LCD) text (#2286 stage 3)

- **Masks.** A run whose `RunSpec::lcd` is `Some(LcdMode { order, filter })`
  is rasterised by `rasterise_lcd`: swash's `Render` with `transform(scale(3,
  1))` (`(1, 3)` for VRGB/VBGR) and the bucket offset scaled with it, so the
  outline is hinted at the real size, emboldened (stem darkening) before the
  stretch, and only then stretched, as FreeType's `FT_RENDER_MODE_LCD` does.
  `lcd_mask` filters the 3x coverage with FreeType's filters in FreeType's
  arithmetic (`lcddefault` `[8, 77, 86, 77, 8]/256` and `lcdlight`
  `[0, 85, 86, 85, 0]/256`, shifted down and clamped; `lcdlegacy`'s in-pixel
  matrix in 65538ths; none) and groups each three subpixels into one pixel,
  swapping red and blue for BGR/VBGR. The five-tap filters widen the mask by a
  pixel each side. `MaskKind::Subpixel`, three bytes a pixel, logical RGB; the
  GPU copy is `[r, g, b, mean]` (premultiplied in name only: the shader reads
  it raw). The `LcdMode` is part of the run key. Colour glyphs are colour in
  an LCD run.
- **Who asks for LCD masks.** Both painters, under the same rule: the policy
  is subpixel, the run is on the atlas's grid (`atlas_scale`), the painter
  allows it (`set_subpixel_text`, default on; off for the blurred-shadow
  scratch painter), and **no layer is open** — any `push_layer` or
  `push_isolated_layer`, counted the same way on both (`VelloPainter::layers_open`,
  `TinySkiaPainter::layer_marks`, so tiny-skia's no-op layers for opacity 1
  within epsilon count as Vello's real ones do). Clips do not count. Inside a
  layer the destination is the layer's own pixels, composited later, and a
  layer may hold a blend that would see the folded backdrop (below); Skia and
  Chrome draw grey there too. The decision is per run, so text fading in with
  an `opacity` animation is grey until the layer goes away.
- **Vello: mask mode 2** (`Scene::draw_image_lcd_mask`, the fork's
  `IMAGE_MASK_LCD`; `app-and-windows.md` "Vello Fork"). The fine shader's
  `lcd_composite`, in the nearest-sampling arm, finds the destination per pixel:
  the current layer's pixel over each open clip's backdrop (the blend stack, and
  its spill past four), innermost out, until one makes it opaque (alpha ≥
  0.998). Opaque: each channel takes `dst * (1 - c * tint.a) + tint * c`, and the
  opaque result is written into the current layer. Folding the backdrop into a
  layer is exact for source-over clip layers (the composite is linear in the
  backdrop: `mix(B, Y over B, k) = (kY) over B`, and it holds through any
  nesting), which is why the CPU side allows LCD only inside clips. Not opaque
  (transparent base, a translucent background, a split slice rendered over
  transparent): the grey coverage in the texel's alpha, as mode 1. So a
  transparent window draws subpixel text exactly where something opaque is
  under it. Partial frames are re-rendered by Vello from the scene, so they
  find the same destination.
- **tiny-skia** (`blit_subpixel_mask_into`): straight into the pixmap at whole
  pixels, through the clip mask; per channel where the pixel's alpha is 255,
  grey source-over elsewhere, in `f32` then rounded, as the shader does.
  Layers start transparent, so the per-pixel test alone would already be grey
  in most of them; the layer count makes it agree with Vello in the no-op ones.
- **Gamma and contrast.** No LUT: coverage is blended in sRGB-encoded space, as
  for grey text and as FreeType's callers (cairo, Qt: KDE's text) blend LCD
  output. Light text gets the same stem darkening (applied to the outline
  before the stretch, so the same device amount). The design's §5 Skia-style
  LUT is not done for either kind.
- **Not done / not known.** A rotated display's order is not read on any
  platform (winit does not expose `wl_output`'s subpixel or transform, and
  Windows' SPI orientation is system-wide); `TextRasterization::subpixel_order`
  is the app's fix. Fractional scale factors keep LCD: masks are in device
  pixels, so the grid lines up wherever the compositor shows the buffer 1:1.
  Nested clips that share a fractional edge differ between the painters (Vello
  multiplies the edge coverage once per clip layer, tiny-skia's intersected mask
  keeps it), grey text or not; the GPU fixture keeps its clip edges whole.
- **Against FreeType** (`lcd_quality_probe_2286` writes the atlas's masks and
  the glyph list; design-2286's `ftref2286` renders it with
  `FT_RENDER_MODE_LCD`, lcddefault, hintslight, on the same quarter-pixel
  origins). Per-channel mean absolute error, of full scale, over inked pixels:
  Inter 11px 0.0067, 16px 0.0077; Noto Sans 11/13/16px 0.0079/0.0085/0.0083
  (correlation 0.997–0.999, the fringe strength and the ink equal to 1%);
  DejaVu Sans 0.029–0.039. Inter 13px is 0.196, all of it vertical hinting:
  swash's autohinter puts that size's cap height a row above FreeType's light
  autohinter, and FreeType's own grey and LCD agree with each other, so the
  grey masks (stage 2) differ there the same way. DejaVu's gap is hinting too
  (its bytecode against FreeType's light autohinting). The filter and the
  grouping match.
- **Cost.** A 1280x800 page of 13px text (`lcd_cost_probe_2286`, Area):
  lavapipe on a loaded host, three runs of grey against LCD masks, 38.8/44.1,
  39.5/37.9, 25.5/21.1 ms: no difference above the noise (mode 2 is a few
  more ALU operations on text pixels only, and reads the blend stack only
  inside clips). The atlas holds 50% more bytes (70 KiB to 105 KiB for that
  page: three coverage bytes a pixel, masks a pixel wider each side). The
  instruction benches track the warm paint: on the Vello painter
  `vello_paint::text_page_subpixel` is 3,579,697 instructions against the grey
  `text_page_unsliced`'s 3,577,349 (+0.07%); on tiny-skia
  `full_paint::text_page_subpixel_warm` is 28.7M against the grey
  `text_page_warm`'s 39.8M, cheaper, because `blit_subpixel_mask_into` writes
  the pixmap directly where a grey glyph is coloured into scratch and drawn
  through `draw_pixmap` (the subpixel page also fills a white background).
- **Pins.** `glyph_atlas::tests` (`an_lcd_run_is_per_channel_coverage`,
  `subpixel_orders`, `the_filters_are_freetypes`,
  `lcd_masks_are_keyed_and_uploaded_as_rgb`, `lcd_masks_take_the_stem_darkening`),
  `text_rasterization::tests` (`never_subpixel_on_macos`,
  `the_app_overrides_order_and_filter`),
  `crates/rinch-dom/tests/subpixel_text_2286_tests.rs` (software pixels: opaque
  page, clip, BGR swap, opacity/no-op opacity/mask/filter layers, transparency
  and translucency per pixel, rotation and scale; Vello's mask choice), and
  `crates/rinch/tests/gpu_subpixel_text_2286.rs` (lavapipe: parity with the
  software painter under Area and Msaa16 at 1x and 1.5x, nested clips to depth
  6, the grey fallbacks, BGR).

### `vello_clip_tiles`: what a frame asks Vello for

`VelloPainter::clip_tiles` estimates the tiles a frame's clips and layers cost;
the shell adds it to `Counter::VelloClipTiles`. The guide has the meaning and the
limits (`docs/src/guide/performance.md`, *Vello's tiles*); this is the mechanism.

- **The arithmetic** is `vello_painter::tile_cost(bounds, target)`, a pure
  function: `tile_alloc.wgsl`'s rounding (floor the near edges, ceil the far
  ones, 16px, empty is 0) applied to the box cut by the target. Vello's
  `draw_bboxes` are also cut by the enclosing clips' bounds (`clip_leaf.wgsl`);
  the estimate is not, by definition, so a nested clip reads high.
- **The window's scene** is counted in `VelloPainter::opened`, the one place
  every clip and layer that reaches it passes through, split path included. An
  elided clip never gets there.
- **The split path** adds the rest in `Slicing::kept_tiles`, read by
  `take_slices`: the inverse clip per open, the mask scenes, and each above
  scene, whose mirrored and replayed pushes are tallied per scene and dropped
  when the scene has no draw (`finish` drops it, so it is never rendered). A
  slice's target is R's far corner (`Slice::tile_target`), not the scratch
  texture `render_slice` rounds it up to.
- **The target** comes from the shell, `set_tile_target`, after `reset` and
  `begin_slicing`. `paint_frame_scene` sets the damage's far corner on a partial
  frame, because that is the target `GpuFrame::Partial` renders to; S4's
  `below_kept` shrinks that target after the paint, so such a frame reads high.
  With none set, nothing is cut: a drag snapshot, whose count `append_scene` adds.
- **Reporting.** `reset` zeroes it, so of a frame's several paints (taint, the
  memory cap, a damage grown over a region) the last, the rendered one, is what
  `build_gpu_frame_split` and `build_scene_with` add. `build_gpu_frame_split` adds
  nothing when it painted nothing: the painter still holds the previous scene.

The fixtures in `crates/rinch-dom/tests/window_tiles_tests.rs` were each checked
by removing the fix they pin. Two traps found doing that: `opacity_layer_shape`
proves a hidden dimmed box empty, so the `display: none` guard moves no tile
unless the box holds something that defeats the proof (a fixed descendant); and
paint opens no clip around content that fits it, so a clip fixture needs an
entry crossing the clip's edge, or the elision has nothing to elide.

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
