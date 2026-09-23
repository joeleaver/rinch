# Measuring Performance

Rinch counts the work each frame does. Every document carries a fixed set of
per-frame **counters**: elements cascaded, Parley layouts shaped, whole-document
IFC setup passes, Taffy computes, full repaints and why, pixels repainted, hit
tests, effect runs, and a few phase timings. The counters are always compiled
in. An increment is one store into memory the document already owns, so
reading them costs nothing you would notice.

Prefer the counters to timings when you want to prove that a change made
something cheaper. Timings move with the machine's load. A counter moves only
when the code does different work: "a colour-only hover now runs 0 Taffy
computes instead of 1" means the same thing on every host.

There are three ways to read them.

## `RINCH_PERF`

Set `RINCH_PERF` (any value) and every completed frame prints one line to
stderr: the phase times, then every counter that is not zero.

```text
$ RINCH_PERF=1 cargo run --release -p ui-zoo-desktop
[PERF] frame 212: style 0.08ms layout 0.02ms paint 1.41ms present 0.35ms | style_resolves=1 elements_cascaded=3 ... repaint_partial=1 repainted_px=41236 surface_px=1382400 ...
```

The variable is read once, the first time a frame ends.

On the desktop, one redraw is one frame. On Android, one pass of the loop that
tries to present a frame is one frame. On an embedded `RinchContext`, one
`update()` plus the `scene()` call that follows it is one frame.

## The `perf_stats` MCP tool

With the `debug` feature enabled, the `perf_stats` MCP tool (debug-protocol
command `perf_stats`) returns this JSON:

```json
{
  "frames": 212,
  "last_frame":    { "elements_cascaded": 3, "taffy_root_computes": 0, ... },
  "current_frame": { ... },
  "total":         { ... }
}
```

Each object holds every counter, keyed by the counter's name. Pass
`reset: true` to zero the counters after they are read. That gives you a way to
measure a single interaction:

```text
perf_stats(reset: true)       # start from zero
click(x: 120, y: 340)         # the interaction
wait_frame()
perf_stats()                  # `total` now covers only the click and its frames
```

This is the tool to use for measurements. The DevTools panel is a poor
substitute, because while it is open it rebuilds its own tree view on every
dirty frame of the app, and that work shows up in what you measure.

## From code

- **`RinchDocument`** (`rinch_dom`): `doc.tree.perf` is a
  `rinch_dom::perf::PerfCounters`.
  - `frame()` returns the frame in progress.
  - `last_frame()` returns the last frame that ended.
  - `total()` returns everything counted since the last reset.
  - `end_frame()` closes the current frame.
  - `reset()` zeroes everything.

  A document that nothing drives never ends a frame, so `frame()` then covers
  everything since the last `reset()`.
- **`RinchApp`**: `last_frame_perf()`, `total_perf()`, `reset_perf()` and
  `end_perf_frame()`.
- **`RinchContext`** (embed): `perf_last_frame()`, `perf_total()` and
  `reset_perf()`.

`rinch::perf` re-exports the module. Each counter is a `Counter` variant with a
stable `name()`. `FrameStats::to_json()` and `FrameStats::summary_line()`
produce the two output formats shown above.

```rust
use rinch_dom::perf::Counter;

doc.tree.perf.reset();
doc.set_attribute(row, "class", "row selected");
doc.resolve_layout(800.0, 600.0);
let frame = doc.tree.perf.end_frame();
assert_eq!(frame.get(Counter::TaffyRootComputes), 0, "a colour change must not lay out");
```

## The counters

| Group | Counter | What it counts |
|---|---|---|
| Style | `style_resolves` | `resolve_styles` calls, including the synchronous one each DOM insertion makes |
| | `elements_cascaded` | Elements that went through a full selector match and cascade |
| | `style_nodes_visited` | Nodes the style walk visited, including cached ones |
| | `pseudo_element_passes` | `::before`/`::after` resolutions (two per cascaded element) |
| | `full_restyles`, `full_restyle_{viewport,theme,stylesheet,dpr,root_font_size}` | Requests for a whole-document restyle, in total and by reason. Two requests can share one walk (a theme change that also changes the root font-size counts twice) |
| | `full_style_walks` | `resolve_styles` walked from `<html>`, because it had no roots or no layout has completed yet |
| | `taffy_style_syncs` / `taffy_style_changes` | Nodes synced to Taffy, and how many of those actually changed their Taffy style |
| Text | `shape_measure_ifc` / `shape_measure_text` | Parley layouts built inside the Taffy measure function |
| | `shape_ifc_build` | Parley layouts built by `build_ifc_layouts` (the layouts paint uses) |
| | `shape_atomic_inline` | Parley layouts built while sizing an `inline-block`/`-flex`/`-grid` box |
| | `ellipsis_builds` | `text-overflow: ellipsis` truncations (each one shapes several candidates) |
| | `shape_paint` | Parley layouts built by paint itself: input values, `<select>` labels, the fallback for uncached text |
| | `ifc_measure_cache_hits` | IFC measures answered from the per-root measure cache without shaping |
| | `ifc_measure_invalidations` | Roots whose cached measures a restyle or content change dropped (O(1) each) |
| | `ifc_signature_changes` | Roots a structural pass found new or changed, and so re-measures; every other root keeps its cached measures and paint layout |
| Layout | `layout_resolves`, `layout_skipped_paint_only`, `layout_skipped_text_only` | `resolve_layout` calls, and how many of them took each early return |
| | `ifc_setup_passes` | Whole-document IFC setup passes (the `ifc_dirty` branch) |
| | `taffy_root_computes` | Root Taffy computes (the same count as `NodeTree::taffy_computes`) |
| | `taffy_measure_calls` | Calls to the measure function during root computes |
| | `inline_block_computes` | Standalone Taffy computes that size an atomic inline |
| | `calc_fixpoint_passes` | Extra computes run by the `calc(%, px)` fixpoint |
| Paint | `paint_frames` / `paint_cached_frames` | Frames actually painted, and redraws that reused the cached frame |
| | `repaint_partial`, `repaint_full` | Software frames limited to a dirty region, and full repaints |
| | `repaint_full_{first_frame,resize,no_dirty_nodes,region_too_large,empty_region,overlay,theme,invalidated,gpu}` | The reason for each full repaint. `no_dirty_nodes` means no dirty region came out: either nothing was paint-dirty, or what was produced no rect |
| | `repainted_px` / `surface_px` | Pixels repainted, and pixels in the surface (their ratio is the fraction of the surface repainted) |
| | `paint_nodes_visited` | Nodes `paint_node` visited |
| | `stacking_order_builds` | Stacking sequences built, by paint and by hit testing |
| Software painter | `glyph_cache_hits` / `glyph_cache_misses` | Glyphs drawn from the rasterised-glyph cache, and glyphs rasterised (then cached). A steady frame has no misses |
| | `clip_masks`, `clip_mask_px` | Clip masks pushed, and the mask pixels they were filled and intersected over (each clip's own bounds, not the surface) |
| | `paint_layers`, `layer_px` | Opacity layers opened, and the layer pixels composited back (the part of each layer anything was drawn into) |
| | `paint_surface_allocs` | Surface-sized masks and layer pixmaps allocated rather than reused from the painter's pool. Zero in a steady state |
| | `paint_surface_trims` | Pooled masks and layer pixmaps released at the end of a frame because none of the last 8 frames needed that many at once |
| | `image_premultiplies` | Images premultiplied at draw time. A cached `<img>` or `background-image` is premultiplied once, on its first software paint; a live frame source (`RenderSurface`, video) on every draw |
| Input | `hit_tests`, `hit_test_nodes_visited` | Hit tests run, and the nodes they visited |
| | `hit_extents_computed` | Subtree extents the hit tester computed so it can skip subtrees nowhere near the pointer. They are kept until the document or its layout changes, so a run of moves over a still document computes each one once |
| Reactive | `effect_runs`, `signal_notifies` | Effect bodies run, and signal writes. A memo's internal marker (which passes a wake on to the memo's dependents) is **not** counted, nor is a recompute, nor is a dependent the equality cut-off skipped: `effect_runs` is the effect bodies that actually ran |
| | `rerender_events_queued` | `ReRender` events queued to the desktop event loop. At most one is queued at a time — writes between two event-loop turns coalesce into it — so this counts turns that had something to resolve, not writes |
| Time (ns) | `time_style_ns`, `time_layout_ns` (`time_ifc_setup_ns`, `time_taffy_compute_ns`, `time_build_ifc_ns`), `time_paint_ns`, `time_present_ns` | Wall-clock time per phase, summed over the frame. Each phase *call* is timed, never each node; a phase can run several times a frame (every DOM insertion runs the two style phases), so this is a few clock reads per call |

Some limits on what these numbers mean:

- **The reactive counters are per thread.** Two documents on one thread share
  them. A desktop window and its DevTools panel are two such documents, as are
  two embedded contexts. **While DevTools is visible, every frame's
  `effect_runs` and `signal_notifies` include its own work**: the shell writes
  its frame-time and FPS signals on every redraw, and its panels re-run. Close
  DevTools before reading the reactive counters.
- **The GPU backend always repaints in full.** Every painted frame on that
  backend counts as `repaint_full_gpu`.
- **The software-painter counters are the desktop and Android shells' only.**
  `TinySkiaPainter` counts into its own `SkiaPainterStats` (`stats()`,
  `take_stats()`), which the shell folds into the frame after each software
  paint. A test painting through `paint_document` directly reads the painter,
  not the document.
- **What the software painter keeps between frames.** Its glyph cache (cleared
  past about 16 MB of accounted bytes or 512 font/size runs); a premultiplied
  copy of each translucent cached image; and a pool of up to 8 clip masks
  (1 byte per surface pixel each) and 8 layer pixmaps (4 bytes per pixel each —
  33 MB at 4K). The pool is trimmed after every software frame to the most any
  of the last 8 frames had open at once, so an app that idles right after a
  deeply nested frame keeps that frame's buffers until it paints again.
- **A screenshot is a frame.** The debug `screenshot` command paints, and that
  paint shows up in the counters like any other frame.

## The committed baselines

`crates/rinch-dom/tests/perf_counter_baselines.rs` runs the paths that matter
for responsiveness on a 40-row list and asserts today's counter values:

- idle
- a colour-only hover, with and without a `display: contents` wrapper
- a colour-only class toggle
- appending a row
- removing a row
- setting one text node's content
- a 1px resize
- a `transform` animation tick

`crates/rinch/src/app/perf_stats_tests.rs` covers the counters only the shell
fills: the full-repaint reasons, the dirty region, hit testing, and the
reactive deltas. It also pins the pointer-move path: a warm move over a
500-row scroller runs one hit test that visits the four boxes under the pointer
and builds no stacking sequence, a move with no `onmousemove` handler anywhere
runs one hit test, and five drag moves queued before a frame lay out once.

Each scenario asserts the **whole frame**: every counter's exact value, with
any counter the baseline does not list expected to be zero. That catches a path
that got more expensive and, just as important, an increment that stopped
counting, which an upper bound alone would read as a saving. A test in the same
file checks that every counter rinch-dom owns fires on some path.

A performance fix is expected to change these numbers. Updating the baseline to
the new values is the proof that the fix worked; say in the PR which counters
moved and why. `PERF_BASELINE_PRINT=1` prints each scenario's frame in the
form the baseline is written in.

The file's `#[ignore]`d `perf_scenario_timings` prints wall-clock times for the
same scenarios. Run it in release:

```text
cargo test --release -p rinch-dom --test perf_counter_baselines -- --ignored --nocapture
```
