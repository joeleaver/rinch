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
| | `pseudo_element_passes` | `::before`/`::after` resolutions: one per cascaded element for each of the two that some stylesheet has a rule for, so none in a document with no such rule |
| | `full_restyles`, `full_restyle_{viewport,theme,stylesheet,dpr,root_font_size}` | Requests for a whole-document restyle, in total and by reason. Two requests can share one walk (a theme change that also changes the root font-size counts twice). A resize counts under `viewport` only when it flips a media query's answer |
| | `viewport_unit_restyles` | Elements a resize that flipped no media query restyled because their last cascade resolved a `vw`/`vh`/`vmin`/`vmax` (each with its subtree). A resize restyles nothing else |
| | `full_style_walks` | `resolve_styles` walked from `<html>`, because a whole-document restyle asked it to or no layout has completed yet |
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
| | `repaint_partial`, `repaint_full` | Software frames limited to their damage, and full repaints |
| | `repaint_none` | Software frames whose scene was marked dirty but whose damage named nothing on screen, so the pixels on screen were kept and nothing was painted (an alt-tab, a focus move onto a box no rule styles) |
| | `damage_rects` | Rects the partial repaints cleared and painted. The damage is a short list (at most 8) of disjoint rects, so two small changes far apart cost their own areas and not the span between them |
| | `repaint_full_{first_frame,resize,unattributed,region_too_large,restyle,theme,invalidated,gpu}` | The reason for each full repaint. `region_too_large`: the damage's rects add up to half the surface or more. `restyle`: the whole document was restyled (a stylesheet, the viewport, the device pixel ratio or the root font-size changed). `unattributed`: something called `RinchApp::mark_scene_dirty`, which asks for a frame without saying what changed, and nothing else named any damage. Everything the framework itself changes names its damage, so on a running app this counts only an embedder's own calls and the software `GameViewport` compositor frames |
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
- **A frame repaints what its damage names, and nothing else.** The damage is
  the paint-dirty nodes (where each is now and where it was last painted),
  the rects of removed nodes, and the drag ghost's and inspect highlight's
  old and new rects. A change that reaches pixels without a `DomDocument`
  write must name its node (`NodeTree::mark_paint_dirty`) or call
  `RinchApp::mark_scene_dirty`, which is counted as `repaint_full_unattributed`.
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
- a 1px resize, with and without `vw` rules
- a `transform` animation tick

`crates/rinch/src/app/perf_stats_tests.rs` covers the counters only the shell
fills: the full-repaint reasons, the dirty region, hit testing, and the
reactive deltas. It also pins the pointer-move path: a warm move over a
500-row scroller runs one hit test that visits the four boxes under the pointer
and builds no stacking sequence, a move with no `onmousemove` handler anywhere
runs one hit test, and five drag moves queued before a frame lay out once.

Each scenario in `perf_counter_baselines.rs` asserts the **whole frame**:
every counter's exact value, with any counter the baseline does not list
expected to be zero. `perf_stats_tests.rs` is exact on the counters each test
names and does not check the rest of the frame. That catches a path
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

## The regression scenarios

Three more files hold scenarios under the same exact-count contract. They are
new files rather than new rows in the two above so that parallel performance
PRs do not collide in one file.

- `crates/rinch-dom/tests/perf_regression_scenarios.rs` has one scenario per
  **increment site** of every counter that has more than one site. `shape_paint`
  has three (a `<select>` label, an `<input>` value, and text with no cached
  layout), `ellipsis_builds` two, `shape_atomic_inline` two and
  `pseudo_element_passes` two. A per-counter check cannot see a deleted
  increment while a sibling site still fires; one document per site can.
- `crates/rinch/src/app/perf_regression_tests.rs` drives a real `RinchApp` the
  way the desktop loop does (`AboutToWait`, then the paint the loop asked for):
  - idle: a plain page, a focused `<input>`, a paused animation, a running
    `Loader`, and a `Loader` in a closed `Drawer`, with and without the pause
    rule;
  - keyed `for` over 200 rows: move one row, insert one, remove one, and
    replace them all;
  - a wheel scroll over 500 rows;
  - ten queued `Drag::absolute` moves;
  - a theme toggle and a scale-factor change;
  - one frame per full-repaint reason: `first_frame`, `resize`,
    `unattributed`, `region_too_large`, `restyle`, `theme`, `invalidated` and
    `gpu`, plus `repaint_none`.
- `crates/rinch/src/app/perf_regression_editor_tests.rs` covers a 30-paragraph
  editor: idle while focused, one caret blink, typing a character, ArrowRight,
  Enter, Backspace across a block boundary, and the toolbar's Bold.

Each scenario also asserts a **positive control**, a check that it did what its
name says (a repaint happened, a row moved, the document grew by one
character). Without it, an idle scenario whose app never mounted would pass as
a perfect idle.

Some of these scenarios do work they should not, and they pin it as it is. A
scenario like that says so in its doc comment and names the counter. A fix
then shows up as a baseline that falls.

## The baselines are the contract

Four files assert whole frames: `perf_counter_baselines.rs` and the three
regression-scenario files. Each checks every non-timing counter exactly, and
any counter the baseline does not list must be `0`. `perf_stats_tests.rs` is
exact only on the counters it names. Two counters are left out of the whole
frame: the `time_*` counters, which are wall-clock, and `rerender_events_queued`
in the shell scenarios, which is folded in from a process-wide atomic.
`queued_rerender_events_are_folded_into_the_frame` pins that counter on its
own, under a lock it shares with the one other test that writes the atomic.

**The numbers must not depend on the host's fonts.** A declared `line-height`
pins only the vertical axis. A width, a caret step or a repaint rect still
comes from the glyph advances of whatever font answered, so a scenario whose
numbers include horizontal text geometry sets its text in the bundled Inter:

- the shell scenarios build every app with `perf_expect::new_app`, which makes
  Inter the `sans-serif`, and set all text in `sans-serif`;
- the rinch-dom scenarios register Inter under their own family name.

Before this rule, the editor's ArrowRight repainted 608 px on one machine and
640 px on CI (Noto Sans against DejaVu Sans). Check a new scenario under a
second font set before you push: point `FONTCONFIG_FILE` at a config that maps
`sans-serif` to DejaVu Sans, and run it both ways. The two runs must print
identical frames.

**A pinned finding names its issue.** Some scenarios pin work that should not
happen (issues #904–#914). Each one's doc comment names its issue and says that
a fix must *lower* the number. A fall there is the fix proving itself, not an
increment that stopped counting.

This makes the baselines the performance contract for every change:

- **A PR that changes a baseline must explain it in its body.** For each
  scenario that moved, list the counters that moved, old and new, and why. The
  pull request template has a *Performance* section for this.
- **A counter that rose** is a path that got more expensive. Either fix it, or
  accept the cost and say why.
- **A counter that fell** is either a fix proving itself or an increment that
  stopped counting. The PR says which.
- **Never loosen an assertion to make it pass.** Do not turn an exact value
  into an upper bound, and do not remove a counter from a baseline. A value
  that honestly varies from run to run (the rotated `Loader`'s repainted area
  depends on the angle the clock lands on) gets a range with its derivation
  written next to it. Every other counter in that frame stays exact.

### Updating a baseline

When an assertion fails, it prints the frame it saw in the form the baseline is
written in, ready to paste:

```text
editor: type one character: the frame differs from its recorded baseline.
  taffy_root_computes: 1 (baseline 2)
  ...
Actual frame:
            (StyleResolves, 2),
            (ElementsCascaded, 2),
            ...
```

`PERF_BASELINE_PRINT=1` prints every scenario's frame, passing or not. Use
`--test-threads=1` so the frames do not interleave:

```text
PERF_BASELINE_PRINT=1 cargo test -p rinch-dom --test perf_regression_scenarios -- --nocapture --test-threads=1
PERF_BASELINE_PRINT=1 cargo test -p rinch --lib perf_regression -- --nocapture --test-threads=1
PERF_BASELINE_PRINT=1 cargo test -p rinch-dom --test perf_counter_baselines -- --nocapture
```

Run the shell scenarios with CI's feature set as well. The theme-toggle and
`gpu` scenarios compile only with `theme` and with `gpu` or `embed`:

```text
cargo test -p rinch --features desktop,embed,theme,clipboard,debug --lib perf_regression
```

### Reading a frame

The names are the ones `RINCH_PERF` prints and `perf_stats` returns. To find
out why a number moved, run the same interaction in a live app:

1. Call `perf_stats(reset: true)`.
2. Do the interaction.
3. Call `wait_frame()`, then `perf_stats()`.

Or set `RINCH_PERF=1` and read the one line each frame prints. Close DevTools
before you read the reactive counters (see the limits above).
