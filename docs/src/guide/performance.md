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
| | `style_invalidations` | Elements whose attribute or state change Stylo's invalidator examined — one per changed element per resolve. What it restyled shows in `elements_cascaded` |
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
| | `ifc_setup_passes` | IFC structural setup passes, scoped or whole-document |
| | `ifc_scoped_passes`, `ifc_scope_containers`, `ifc_scope_nodes` | Passes scoped to the formatting containers a structural change reached, and how many containers and nodes they set up again. Everything else keeps its splices, boxes, marks and Taffy cache |
| | `ifc_full_passes`, `ifc_full_{initial,theme,unattributed}` | Passes over the whole document, and why: the first layout, a whole-document restyle, or `NodeTree::ifc_dirty` written directly with no reason |
| | `taffy_root_computes` | Root Taffy computes (the same count as `NodeTree::taffy_computes`) |
| | `taffy_measure_calls` | Calls to the measure function during root computes |
| | `inline_block_computes` | Standalone Taffy computes that size an atomic inline |
| | `calc_fixpoint_passes` | Extra computes run by the `calc(%, px)` fixpoint |
| Paint | `paint_frames` / `paint_cached_frames` | Frames actually painted, and redraws that reused the cached frame |
| | `repaint_partial`, `repaint_full` | Software frames limited to their damage, and full repaints |
| | `repaint_none` | Software frames whose scene was marked dirty but whose damage named nothing on screen, so the pixels on screen were kept and nothing was painted (an alt-tab, a focus move onto a box no rule styles) |
| | `damage_rects` | Rects the partial repaints cleared and painted. The damage is a short list (at most 8) of disjoint rects, so two small changes far apart cost their own areas and not the span between them |
| | `repaint_full_{first_frame,resize,unattributed,region_too_large,restyle,theme,invalidated,gpu}` | The reason for each full repaint. `region_too_large`: the damage's rects add up to half the surface or more. `restyle`: the whole document was restyled (a stylesheet, the viewport, the device pixel ratio or the root font-size changed). `unattributed`: something called `RinchApp::mark_scene_dirty`, which asks for a frame without saying what changed, and nothing else named any damage. Everything the framework itself changes names its damage — a software `GameViewport` frame included, since #361 paints it inline — so on a running app this counts only an embedder's own calls, and a window re-created by the shell |
| | `repainted_px` / `surface_px` | Pixels repainted, and pixels in the surface (their ratio is the fraction of the surface repainted) |
| | `paint_nodes_visited` | Nodes `paint_node` visited. A box that would draw nothing — outside the window, outside the damage, or outside every clip the painter has open (a scroller's rows past its viewport) — is dismissed by its parent's loop without a visit, so a scroller costs its visible rows, not its list (#910) |
| | `removal_damage_steps` | Work a removal (or a move to another parent) did to record the old pixels of what it took out: one per node, plus one per ancestor its clip-chain walk stepped through (#909). Linear in what was removed, never in what else is pending |
| | `stacking_order_builds` | Stacking sequences built, by paint and by hit testing |
| Software painter | `glyph_cache_hits` / `glyph_cache_misses` | Glyphs drawn from the rasterised-glyph cache, and glyphs rasterised (then cached). A steady frame has no misses |
| | `clip_masks`, `clip_mask_px` | Clip masks pushed, and the mask pixels they were filled and intersected over (each clip's own bounds, not the surface; none for a clip that fully covers the clip enclosing it, which a scroller around a partial repaint does, and no intersection for one lying wholly inside the enclosing clip's fully covered area) |
| | `paint_layers`, `layer_px` | Opacity layers opened, and the layer pixels composited back (the part of each layer anything was drawn into) |
| | `paint_surface_allocs` | Surface-sized masks and layer pixmaps allocated rather than reused from the painter's pool. Zero in a steady state |
| | `paint_surface_trims` | Pooled masks and layer pixmaps released at the end of a frame because none of the last 8 frames needed that many at once |
| | `image_premultiplies` | Images premultiplied at draw time. A cached `<img>` or `background-image` is premultiplied once, on its first software paint; a live frame source (`RenderSurface`, video, `GameViewport`) on every draw — unless every pixel of the submitted frame was opaque, which `submit_frame` checks on the submitting thread |
| | `opaque_image_copies` | Opaque frames copied straight into the surface instead of sampled: no rotation or skew, a positive scale, destination edges on whole pixels, and no clip partial where the frame lands. Byte-identical to the sampled draw (`opaque_image_copy_tests`) |
| Input | `hit_tests`, `hit_test_nodes_visited` | Hit tests run, and the nodes they visited |
| | `hit_extents_computed` | Subtree extents the hit tester computed so it can skip subtrees nowhere near the pointer. They are kept until the document or its layout changes, so a run of moves over a still document computes each one once. A scroll keeps them (a scroll container's extent is its own box), so a wheel notch after the first computes none, not one per row |
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
  paint shows up in the counters like any other frame. The capture after it
  adds nothing: on the software backend it reads the frame just presented back,
  as the GPU backend always has, rather than painting a second time (#364).

## The committed baselines

`crates/rinch-dom/tests/perf_counter_baselines.rs` runs the paths that matter
for responsiveness on a 40-row list and asserts today's counter values:

- idle
- a colour-only hover, with and without a `display: contents` wrapper
- a colour-only class toggle
- an attribute no selector reads (nothing is cascaded)
- a container class a descendant rule depends on
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
same scenarios, and `structural_timings_at_scale` times an append, a removal and
a toggle on 500 and 2000 rows, with and without a `display: contents` wrapper
per row. Run them in release:

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
  layout — a constructed state since #904, when an `inline-flex` label stopped
  reaching it), `ellipsis_builds` two (the text-leaf one reached by a flex item behind a `display: contents` wrapper, which rinch does not blockify, #998; a flex or grid container's own text never ellipsizes, as in Chrome), `shape_atomic_inline` two and
  `pseudo_element_passes` two. A per-counter check cannot see a deleted
  increment while a sibling site still fires; one document per site can.
- `crates/rinch/src/app/perf_regression_tests.rs` drives a real `RinchApp` the
  way the desktop loop does (`AboutToWait`, then the paint the loop asked for):
  - idle: a plain page, a focused `<input>`, a paused animation, a running
    `Loader`, and a `Loader` in a closed `Drawer` (which idles: the drawer's
    closed rule pauses it, #912);
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

## CI regression job

Counters say *what* a frame did; they do not say how much it cost. The `Perf`
workflow (`.github/workflows/perf.yml`) covers cost. On every pull request it
counts the **instructions** each hot path executes. It counts them on the merge
commit GitHub builds for the PR, and on that commit's first parent, the tip of
`main` it was merged onto. It fails when one count grew by more than a
threshold. Every run re-bases on `main` as it is at that moment, so the Δ says
what merging the PR would do now. Changes other PRs landed on `main` are not
counted against it.

It counts instructions and not time because a hosted runner's timings move with
whatever else shares the machine, while an instruction count moves only when
the code does. The counts come from Valgrind's Callgrind, driven by
[Gungraun](https://github.com/gungraun/gungraun) (the renamed `iai-callgrind`).
The benchmarks live in `crates/rinch-bench`:

| Benchmark | What one run measures |
|---|---|
| `dom::hover.list_500` | `:hover` on one row of a 500-row list (`display: contents` wrappers, an `inline-block` chip per row), then layout |
| `dom::class_toggle.list_500` | A colour-only class change on one row, then layout |
| `dom::append_row.list_500` / `remove_row` | Append or remove one row, then layout |
| `dom::resize_1px.list_500` | Layout at a viewport 1px wider |
| `dom::set_text_content.list_500` | `set_text_content` on one row's wrapped text, then layout |
| `dom::flex_label_hover.list_500` | A colour-only `:hover` on one of 500 `display: flex` rows whose text is a direct child (a flex item's text leaf), then layout and paint. Paint colours a leaf itself, so no compute runs (#904) |
| `dom::drawer_toggle.list_500_closed` | Open, then close, a closed `Drawer` holding 500 rows, with the theme's and the component library's stylesheets loaded; each re-cascades the whole subtree. The one benchmark that sees what a component-library selector costs (#935: a pseudo-element rule is matched without a bloom filter) |
| `dom::inset_move.list_500` | One drag step of an absolute panel beside the list: `set_styles` of `left`/`top` (the inset fast path), then layout |
| `dom::full_paint.text_page_warm` | A full `TinySkiaPainter` paint of 40 wrapped paragraphs, with the glyph cache already warm |
| `shell::pointer_move_warm.warm_x50` | 50 pointer moves inside one row of a 500-row scroller, each followed by `AboutToWait` |
| `shell::pointer_move_cold.cold` | The first move after a layout, which builds the hit-test cache |
| `shell::hover_frame.partial_repaint` | A move onto another row, then the frame: layout and a partial software repaint |
| `shell::keyed_for.reverse_200` | Reverse a keyed `for` of 200 rows, then the frame |
| `shell::memo_flush.selection_40` | Move the selection among 40 rows, each with a `Memo<bool>` and an effect: the signal write and its effect flush |

Each benchmark builds its fixture in a setup that Callgrind does not count. The
setup also runs the operation once where that warms a cache, so what is counted
is the second hover and not the first. All text is set in the bundled
`Inter-Regular.ttf`, so no scenario measures or draws through the host's
fonts. That does not make a count portable: `resize_1px` is 111.2M
instructions on the CI runner and 147.6M on a developer workstation, because
the toolchain, glibc's CPU-specific `memcpy` and the CPU all move it. Compare
two counts only when one machine produced both. The CI job runs base and head
on the same runner for this reason, and a local before/after is only
comparable with another local run.

### Reading the report

The job writes a table to its summary page and to one comment on the PR, which
it edits in place on each push. A row looks like this (the numbers are from a
deliberately slowed `update_hover`):

| Benchmark | base | head | Δ | |
|---|--:|--:|--:|---|
| `dom::hover.list_500` | 177,782 | 185,785 | +4.50% | ❌ regression |

The columns are Callgrind's `Ir`, instructions executed inside the measured
operation. Two runs of the same binary agree to within about **0.03%**. The
remaining noise comes from hash-table probing, whose seeds vary per process. So
any Δ larger than a fraction of a percent is the code.

The base run uses the head's copy of `crates/rinch-bench`, so both sides run
the same scenarios. When the base cannot run them, the outcome depends on
whether the base has the crate at all:

- **The base has no `crates/rinch-bench`.** The report shows the head alone and
  the job passes. No merge commit's first parent can lack the crate once it is
  on `main`, so this applies only to PRs based before then.
- **The base has the crate and the run still failed.** The job fails. Usually a
  PR changed an API the benchmarks call without updating them, or a benchmark
  panics on the base. Otherwise, breaking the benchmarks would be a way past the
  check. The `perf-regression-accepted` label downgrades this failure to a
  warning, the same as a regression.

If the head's own benchmarks do not build or run, the job fails. The PR comment
then says so, rather than keeping the previous run's table.

### The threshold and the escape hatch

The job fails when any benchmark's `Ir` grows by more than **3%**. Change the
threshold with the repository variable `PERF_REGRESSION_THRESHOLD`, a number of
percent.

A PR that makes a path more expensive on purpose can carry the
**`perf-regression-accepted`** label. With the label, the regression is still
reported, as a warning, and the job passes. Adding or removing the label runs
the job again. Say in the PR which benchmark moved and why, in the pull request
template's *Performance* section, the same way a counter-baseline update does
(see [The baselines are the contract](#the-baselines-are-the-contract)). The
two checks complement each other. The baselines are exact and say *which* work
a frame did, while the instruction counts say what that work *cost*. A change
can move one without the other.

The comment is posted by a separate `comment` job, which is the only job with
`pull-requests: write` and runs none of the PR's code. It updates the comment
that carries the `<!-- rinch-perf-bench -->` marker **and** is authored by
`github-actions[bot]`, so a human comment that quotes the marker is left alone.
A pull request from a fork gets a read-only token and cannot comment. For those
PRs the report is on the job's summary page only. A PR that changes only
`docs/**` or Markdown files does not run the job.

The cargo cache holds only dependency artifacts. A fresh checkout makes every
workspace crate look stale to cargo, so those rebuild on every run anyway. Only
a push to `main` writes the cache, and only when its key misses. The key hashes
every `Cargo.lock` and `Cargo.toml`, the toolchain and the compiler environment.
Pull requests only read it.

### Allocation is not what it costs in a real frame

glibc's `malloc` and `free` cost a different number of instructions depending
on what the heap already holds. Which blocks can merge and which bin a free
lands in depend on everything the process did before, and the order of a
setup's allocations follows `HashMap` iteration, which std seeds randomly.
Measured on `set_text_content`, identical runs gave 5.34M, 5.71M and 5.83M
instructions, and every instruction of the difference was inside `malloc.c`.

So the benchmark binary installs `rinch_bench::alloc::BenchAlloc`. Inside the
measured operation, every allocation bumps a pointer into a region reserved in
setup, and every free does nothing. Both cost the same few instructions each
time. Outside the operation, the system allocator handles everything. The cost:
an allocation counts as a few instructions instead of glibc's 50 to 150, so a
change that only adds allocations moves the count less than it moves a real
frame. The code that builds what it allocates is still counted in full.

### Running it locally

You need Valgrind and the Gungraun runner, at exactly the version of the
`gungraun` library in `Cargo.lock`:

```text
sudo apt-get install valgrind
cargo install gungraun-runner --version 0.19.4 --locked
cargo bench -p rinch-bench                      # all benchmarks, human-readable
cargo bench -p rinch-bench -- '*::hover::*'     # one benchmark (a glob over the module path)
```

Each run is compared with the previous run of the same benchmark on disk, so
running before and after a change prints the Δ directly. To reproduce the CI
table, save each side as JSON and compare the two files:

```text
cargo bench -p rinch-bench -- --output-format=json --callgrind-args=--cache-sim=no > before.jsonl
# ...make the change...
cargo bench -p rinch-bench -- --output-format=json --callgrind-args=--cache-sim=no > after.jsonl
python3 .github/scripts/perf_compare.py --base before.jsonl --head after.jsonl
```

Each benchmark leaves a Callgrind profile at
`target/gungraun/rinch-bench/hot_paths/<group>/<bench>/callgrind.*.out`.
`callgrind_annotate --inclusive=yes <file>` or KCachegrind shows where the
instructions went.

A full run takes about 40 seconds on a 24-core workstation with
`--parallel=4`. Valgrind runs code about 50 times slower than native, and the
benchmarks are sized for that: 500 rows where the operation touches one row,
200 where it touches every row.
