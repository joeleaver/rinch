# Rendering Pipeline

Rinch uses a multi-stage rendering pipeline that transforms component code into pixels on the desktop backend. Two rendering backends are available: GPU (Vello/wgpu) and software (tiny-skia/softbuffer). The web backend uses browser-native DOM instead (see note at the end).

## Pipeline Stages (Desktop)

```
┌───────────────────────────────────────────────────────────────┐
│                   1. Component Input                            │
│  #[component] functions + rsx! macro generate DOM construction │
│  code via __scope.create_element(), create_text(),             │
│  create_effect()                                               │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   2. DOM Construction                           │
│  DomDocument creates nodes programmatically via RenderScope   │
│  (RinchDocument uses Taffy + Parley on desktop)               │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   3. Style Resolution                           │
│  Stylo (Firefox's CSS engine) computes styles for each node   │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   4. Layout                                     │
│  Taffy computes the position and size of each element         │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   5. Painting (via Painter trait)                │
│  paint_document() walks the DOM tree and emits drawing        │
│  commands through the abstract Painter interface               │
└───────────────────────────────────────────────────────────────┘
                              │
                    ┌─────────┴─────────┐
                    ▼                   ▼
┌──────────────────────────┐  ┌──────────────────────────┐
│  6a. GPU (Vello)         │  │  6b. Software (tiny-skia) │
│  Scene graph → wgpu →    │  │  Rasterize to RGBA pixmap │
│  GPU compositing         │  │  → softbuffer → display   │
└──────────────────────────┘  └──────────────────────────┘
                    │                   │
                    └─────────┬─────────┘
                              ▼
                          Display
```

## Input Stage

User code defines components using the `#[component]` macro and `rsx!` macro:

```rust
#[component]
fn counter() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}
```

The `#[component]` macro injects a `__scope: &mut RenderScope` parameter. The `rsx!` macro generates calls to `__scope.create_element()`, `__scope.create_text()`, and `__scope.create_effect()` to build the DOM tree programmatically. No HTML strings are generated or parsed at runtime.

## Layout Stage

`RinchDocument::resolve_layout` is the whole of it, and it has **three** paths
rather than one. Which it takes is decided by two flags on the tree, and the
flags are the part worth knowing, because a change that dirties neither is a
change the pass will not see.

| flag | set by | what it buys |
|---|---|---|
| `layout_dirty` | a structural mutation, a text-content change, a viewport change, a decoded image, a **Taffy** style that actually changed, and a restyle that changes how text **measures** | the Taffy compute |
| `ifc_dirty` | a structural mutation, a `display`/`position` change, a `DisplayMode` change, a decoded `<img>`, a full restyle | the inline-formatting-context setup passes, including the measure of every atomic inline |

- **Neither dirty** — styles are resolved, dirty Parley layouts are rebuilt, and
  the pass returns. A `:hover { color }` costs this and nothing more; it is the
  reason the early return exists.
- **`layout_dirty` only** — Taffy runs over the existing IFC structure. Text
  measure contexts are refreshed incrementally, and the atomic inlines something
  changed under are re-measured from a dirty set.
- **Both** — the IFC structure is rebuilt from scratch and every atomic inline in
  the document is measured.

A decoded `<img>` is on the `ifc_dirty` list for a reason worth knowing: it is
how a newly loaded image reaches a detached `inline-block` box, which no other
invalidation in the table would have reached.

### Why a typography change is a layout change

`font-family`, `font-weight`, `font-style`, `line-height`, `letter-spacing`,
`word-spacing`, `text-transform`, `white-space` and `overflow-wrap` are not Taffy
properties, and neither is `font-size` for a box whose own sizes are in `px`. A
declaration change in any of them re-wraps the text and therefore moves the box
around it, so it sets `layout_dirty` even though no Taffy field changed
(`ComputedStyle::same_measured_text_inputs`, issue #678). That predicate is
deliberately narrower than the one that decides whether the shaped glyphs have to
be rebuilt (`same_text_layout_inputs`, issue #654): `color` is baked into the
glyphs and moves nothing, and keeping it out of the first list is what keeps the
cheap path cheap.

A transition or an animation writes `computed_style` directly rather than through
the cascade, so a `transition: font-size` frame reaches none of that gating on
its own; both ticks invalidate the text measure of the nodes they are
interpolating.

### Why an atomic inline needs its own pass

An `inline-block`, `inline-flex` or `inline-grid` box is **detached from its
parent's Taffy child list** so the enclosing inline formatting context can
measure it as a Parley `InlineBox`. The root compute therefore never reaches it,
and three passes give one a size, all through `measure_inline_blocks`: the
`ifc_dirty` pass, which measures every atomic inline in the document; since
issue #661, a re-measure of the boxes a change actually reached
(`dirty_atomic_inlines`); and `resolve_percentage_inline_blocks`, which runs
after the root compute and is what lets a percentage inline size resolve against
a containing block that only has a width once the compute has run. A component
that declares `display: inline-flex` —
`Badge`, `Button` and the rest of the list in CLAUDE.md — is one of these
whenever it sits beside text rather than inside a `Stack`.

## Key Technologies

### rinch-dom

The custom DOM and layout engine built specifically for Rinch:

- **rinch-dom** - DOM implementation with Taffy layout, Parley text shaping, and a `Painter` trait for backend-agnostic rendering
- **VelloPainter** - GPU backend: records drawing commands into a Vello scene graph
- **TinySkiaPainter** - Software backend: rasterizes directly to an RGBA pixel buffer

### Stylo

Mozilla's CSS engine (from Firefox) provides:

- Full CSS specification support
- Efficient style computation
- Media query handling
- CSS custom properties

### Taffy

A flexbox/grid layout engine that computes:

- Element positions (x, y)
- Element sizes (width, height)
- Flexbox alignment and distribution
- CSS Grid support

### Painter Trait

All rendering goes through the `Painter` trait, which abstracts over both backends:

```rust
pub trait Painter {
    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape);
    fn stroke(&mut self, stroke: &Stroke, transform: Affine, brush: &Brush, shape: &PaintShape);
    fn draw_glyphs(&mut self, font: &FontData, font_size: f32, ...);
    fn draw_image(&mut self, image: &PaintImage, transform: Affine);
    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape);
    fn push_layer(&mut self, blend: BlendMode, opacity: f32, ...);
    fn pop_layer(&mut self);
    // ...
}
```

Application code never interacts with the Painter directly. Cargo features decide which backends a build carries, and a `gpu` build picks one of its two when the window opens (see below).

### Vello (GPU Backend)

A GPU-accelerated 2D graphics library (enabled with `features = ["gpu"]`):

- Scene graph-based rendering
- Efficient batching
- High-quality anti-aliasing
- Path rendering (beziers, fills, strokes)
- Text rendering with proper shaping
- Requires wgpu (Vulkan, Metal, DX12, or WebGPU)

### tiny-skia (Software Backend)

A CPU-based rasterizer, carried by every `desktop` build: the only backend without `gpu`, and the fallback with it:

- Direct pixel rendering to an RGBA buffer
- Presented via softbuffer (no GPU required)
- Dirty region caching — only changed areas are repainted
- Subtree pruning — nodes outside the dirty region, or outside the window, are skipped
- Works in headless environments, CI, containers, SSH sessions

## Rendering Backends

### Choosing a Backend

Your `Cargo.toml` decides which backends the build carries:

```toml
# GPU, with software as the fallback (recommended for most apps):
rinch = { workspace = true, features = ["desktop", "gpu"] }

# Software only (no GPU required, smaller build):
rinch = { workspace = true, features = ["desktop"] }
```

A `gpu` build then chooses when the window opens (`crates/rinch/src/shell/renderer.rs`): `App::renderer(Renderer::Auto)`, the default, tries the GPU and presents with software when it will not start; `Renderer::Gpu` panics instead; `Renderer::Software` skips the GPU. The `RINCH_RENDERER` environment variable (`auto`, `gpu`, `software`, `cpu`) overrides the app's choice. An app that configured the GPU device itself (`gpu_config` / `external_gpu`) always presents on the GPU. The details are in [Rendering Backends](../guide/windows.md#rendering-backends).

### GPU Rendering Flow

```rust
// Simplified GPU rendering flow
fn paint_gpu(&mut self) {
    let scene = self.app.build_scene(scale, size);
    self.renderer.render_to_surface(&scene, &params, &surface);
}
```

### Software Rendering Flow

```rust
// Simplified software rendering flow
fn paint_software(&mut self) {
    let (pixels, w, h) = self.app.build_pixels(scale, size, &layers);
    // pixels are blitted to the window via softbuffer
}
```

The software renderer includes **dirty region caching**: when only a small part of the UI changes (e.g., cursor blink, hover feedback), only the affected rectangular region is cleared and repainted. Nodes outside the dirty region are skipped entirely during the paint traversal.

The region is the union of the changed *nodes'* rects, so anything painted **outside** the node tree has to contribute its own. The drag ghost is the one such overlay: it is blitted into the framebuffer after the document paint, so `RinchApp` remembers the rect it covered and folds that into the next frame's dirty region — otherwise the frame that stops drawing the ghost would never clear where it had been, leaving it stuck on screen (issue #173). The GPU backend rebuilds the whole Vello scene every dirty frame and so has no equivalent case.

## Incremental Updates

When content changes, the pipeline can skip unchanged stages:

1. **Style cache** - Styles are cached per element selector
2. **Layout cache** - Layout is only recomputed for affected subtrees
3. **Scene diffing** - Only changed primitives are re-rendered

## Performance Characteristics

| Stage | Complexity | Caching |
|-------|------------|---------|
| DOM Build | O(n) | Incremental (surgical updates) |
| Style Resolve | O(n x rules) | Selector cache |
| Layout | O(n) | Subtree cache |
| Paint | O(visible) | Dirty region caching (software) |
| GPU Render | O(primitives) | GPU buffers (GPU mode) |

## Web Backend

The pipeline above is **desktop-only**. The web backend (`ui-zoo-web`) takes a completely different path:

```
#[component] + rsx! → DOM construction code → WebDocument (web_sys) → Browser-native DOM
```

On the web, `WebDocument` implements `DomDocument` using `web_sys` to create real browser DOM elements. The browser handles style resolution, layout, painting, and compositing natively. No Taffy, Parley, Stylo, Vello, or wgpu are needed for the web backend, resulting in a much smaller WASM binary.

## The frame clock

Every shell must send `PlatformEvent::AboutToWait` once per iteration of its
event loop. It is not an optimisation and it is not optional: it is the only
place `RinchApp` advances CSS transitions and CSS animations, marks the scene
dirty when either moved, resolves the dirty state the input handlers
deliberately leave for it to batch, and drains the focus requests effects
raise. The winit shell sends it from `ActiveEventLoop::about_to_wait`; the
Android shell sends it from `android_frame::pump_frame`, once per 16ms poll;
`embed::RinchContext` sends it from `update`.

A shell that omits it does not look broken. Every screen still paints and every
un-animated control still works — but a transitioned property is sampled once,
at the instant the transition starts, which is its *old* value, and stays there
for ever, while un-transitioned properties on the same element apply
immediately. A bottom sheet then answers a tap by becoming pointer-active
without moving: open, invisible, and covering the screen.

Turning the clock is only half of it. A transition on a paint-only property —
`opacity`, `transform`, a colour — marks its node `PAINT`-dirty and nothing
else, so `has_pending_layout()` stays false and no `RequestRedraw` is raised. A
shell decides whether to present from `RinchApp::scene_dirty`, which
`AboutToWait` sets whenever a transition or an animation moved; a shell that
does not consult it turns the clock in private, and the surface keeps the frame
from before the change until something unrelated forces a present.

And `scene_dirty` is set for every tick that had something to tick, not only
for ticks after which something is still running. A transition that *finishes*
on a tick applies its end value and then reports nothing active — and on a
shell whose first paint after the change is slower than the transition is long
(Android's is around 300ms against a typical 220ms), that is the only tick the
transition ever gets.

## Starting a transition

Turning the clock is what advances a transition; style resolution is what
starts, retargets, reverses or cancels one. `transition::start_transitions`
implements css-transitions-1 §3, "Starting of transitions", against the
properties the style differ found a change in:

| The running transition | What happens |
|---|---|
| none | start from the before-change value over the declared duration (§3 item 1) |
| its end value still equals the after-change value | **left exactly as it is** — same endpoints, same clock |
| it has already reached the after-change value | cancelled; nothing is started (§3 item 4.1) |
| the combined duration (`duration + delay`) is zero or less | cancelled; nothing is started (§3 item 4.2) |
| the after-change value is the value it would reverse back to | cancelled, and a **shortened** reversal started (§3 item 4.3) |
| anything else | cancelled and restarted from the current interpolated value, over the full declared duration (§3 item 4.4) |

The second row is the one that is easy to get wrong, because the caller diffs
the node's `computed_style` — which holds the **interpolated** value while a
transition runs — against the freshly resolved target. Every restyle of a
transitioning node therefore *looks* like a change, and rinch used to restart
the transition on each one with a brand new clock (#652). A declared 150ms
animation then ran for as long as restyles kept arriving, its duration a
function of how much else on the page happened to be animating; with `ease`,
which is slow near t=0, each restart advanced the value by a sliver and the box
crawled toward its target instead of arriving.

A property whose transition was left alone is still reported as *transitioning*
to the caller. That is what tells style resolution to write the interpolated
value back over the after-change style it has just assigned wholesale — without
it the box would snap to its end value for one frame, which is all #489 needed.

**A reversal is shortened.** A reversal taken when the transition is half way
*there* is half as much travel back, so it gets half the duration and lands on
its old start value at the moment the cancelled transition would have reached
its end value — the behaviour a hover in and straight back out depends on. The
factor folds the running transition's own factor back in
(`|f·progress + (1 − f)|`), so reversing a reversal is measured against the
declared duration rather than compounding toward zero.

Note that it keys on **progress**, not on elapsed time. Under `linear` the two
coincide, so reversing a 150ms transition after 75ms takes 75ms. Under `ease` —
which is what every transition in `rinch-components` declares, and `linear` what
none of them do — the output at input 0.5 is 0.8024, so the same reversal takes
**120.4ms**. It has to: 80% of the distance has been covered and 80% of it has
to be covered again.

**The delay is not shortened with it**, unless it is negative. A negative
`transition-delay` is an offset into the curve, so a shortened curve is entered
proportionally further along; a nonnegative one is a wait before the curve
begins, and the spec uses it as declared. `HoverCard` is what this protects: its
close direction carries `transition-delay: 150ms` as a grace period for moving
the pointer onto the card, and scaling that with the factor would halve the
grace period for anyone who leaves part way through the fade-in.

Two things rinch does **not** implement from §3. The **transitionability**
precondition, which appears in item 1 and again in item 4.2: a pair of values
that cannot be interpolated — a length against a percentage, which needs a
`calc()` that `ComputedStyle` cannot hold — still gets an `ActiveTransition`,
which then idles for its whole duration because `AnimatableValue::interpolate`
answers `None` for it. The property snaps either way; cancelling instead would
only save the idle ticks. And **item 3**, cancelling a running transition whose
property has stopped matching `transition-property`: rinch skips the property
and leaves the transition running, so the box snaps to the target on the restyle
and then jumps backwards on the next tick, which resumes writing the interpolated
value. Any restyle that changes `transition-property` mid-transition reaches it.
Both gaps are pre-existing; item 3 is tracked as issue #693.

## The before-change style, and who has one

Everything above assumes there *is* a before-change style to leave. §3 only runs
for a node that has one, and in rinch that is one bool on the node:
`has_been_styled`, set by `apply_stylo_styles_to_taffy` the first time the node
is cascaded. With it clear, the resolved style is assigned wholesale and no
transition is considered — which is what makes a freshly mounted element appear
at its final size instead of animating to it.

Two rules keep that flag honest, and they are the same rule read from the two
ends:

- **A node outside the document is never styled**, so it never acquires the flag
  (#651/#668). `resolve_styles` drops a `style_roots` entry whose node is not
  connected to `tree.root_id`.
- **A subtree that leaves the document loses the flag**, and any transition or
  animation running on it is cancelled (#699).
  `RinchDocument::detach_subtree_styles` does all three, for the whole removed
  subtree. Five places in `dom_impl/dom_document_impl.rs` write `parent = None`
  and four call it: `remove_node`, `remove_child`, `replace_node`'s displaced
  `old`, and `set_text_content` on an element with children, which orphans every
  one of them without freeing the slab. The fifth, `set_inner_html`, is safe by
  destruction — `NodeTree::remove_subtree` frees the entries and drops both
  animation maps with them.

Without the second, a node styled while it was connected, then detached, keeps
the flag *and* the `computed_style` it had in the document. If an ancestor's
class changes while it is out, its re-insertion resolves to a different value,
§3 sees old ≠ new on an already-styled node, and the box animates in from a
style the user never saw. A browser does not: a removed element is not rendered,
it has no before-change style, and re-insertion is a first style.

What the detach **does not** touch is the `computed_style` itself, or the shaped
`text_layout`. A detached node still reads back as it last did in the document —
`dom_tree(root_id: <a detached id>)` depends on that, and so do the staleness
gates that decide whether a re-inserted subtree needs re-shaping. The flag is
what the transition reads; the value is what everything else reads.

A **move** is not a detach. `append_child`, `insert_before` and `insert_child`
unlink a node from its old parent with the same lines `remove_child` uses, but
the node is back in the document before the call returns, so it never stopped
being rendered and a mid-flight transition goes on running — which is what a
keyed `for` reorder depends on, since it moves rows with `insert_after`.

**Unless the destination is itself detached** (#702). A mounted node moved into
a parent that is not connected to `tree.root_id` has left the document while
keeping a parent, so it fails the `parent = None` test the four detach routes
share — and #696 established that connectivity, not the parent field, is the
question that matters. Splice that parent in somewhere else later and the node
animates in from its pre-move style, which is #699's symptom by a fifth route.
`detach_subtree_styles_if_moved_out` is the answer, at **four** sites: the three
move verbs and `replace_node`, which splices its incoming `new` into `old`'s
parent and is a move out whenever that parent is detached.

Two guards decide, and the order matters. The destination must **differ from the
old parent** — a move within one container cannot change whether the child is
connected, because the child's reachability *is* its parent's — and the child
must already **have** a parent, since a node created moments ago cannot be a
move. Only a reparenting move reaches `depth_if_connected`, the same O(depth)
walk #696 filters `style_roots` with.

Counted, on 500 of each shape:

| workload | helper | walks | resets | nodes reset |
|---|---|---|---|---|
| rows built straight into their final parent | 0 | 0 | 0 | 0 |
| keyed reorder, each row to the front | 499 | **0** | 0 | 0 |
| rows reparented into a second connected list | 500 | 500 | 0 | 0 |
| **`rsx!` component sites**, 20 nodes each | 500 | 500 | 500 | **10,000** |

The keyed reorder is the hot path and it never walks. The last row is the one to
know about: `rsx!` does **not** build a component site by appending fresh nodes
into their final parent. `component_codegen` puts a site's children into a
`<template>` attached to nothing (#719) and `Component::render` adopts them into
a root that is *also* still detached, so every adoption is a move into a detached
parent — it walks, and it takes the whole subtree reset. The reset is
semantically a no-op there (a node created moments ago is already unstyled, with
empty transition and animation maps) and the cost does not show: best of 40,
release, three alternated rounds, the build *with* the helper was the faster of
the two every time. Say "a node appended straight into its final parent pays
nothing", not "building a tree pays nothing".

One behaviour change follows, and it is narrower than it looks. A node that is
mounted and **still connected** when it is moved into a detached parent, and
adopted straight back out in the same pass, loses its running transitions and
restarts its animations — a browser never sees that intermediate state, because
its style recalc is batched to the end of the task. The component *re-render*
path does not reach it (`reactive_component_dom` removes the old output first, so
#699 has already reset that subtree); handing a component a handle that is
mounted elsewhere and still connected does.

`display: none` is not a detach either, and it does not need to be. §3's
question is not *is this node in the document* but *is it being rendered*, and
a hidden node is not — nor is anything inside one, since `display` does not
inherit and a box under a hidden wrapper computes `display: block`. So the flag
is only half the answer, and the cascade asks the other half directly (#703):
before starting a transition, `is_rendered_for_transition` checks the node's
display **before** the change as well as after, then walks its ancestors. A
style change made while an element is hidden lands on it outright, the way a
browser applies one, so the element is already at its new value when it is
shown. In the other direction, an element that stops being rendered has its
transitions cancelled, and so does everything under it — a descendant's own
cascade need not run at all when an ancestor is hidden.

Three things about that are worth knowing.

- **It reads the old display, not just the new one.** A single class write can
  un-hide a box and retarget it at once. At that cascade the new display is
  already `block`; only the old one says there was nothing to transition from.
- **`was_hidden` exists because parents cascade first.** An ancestor restyled on
  the same pass is carrying its *new* display by the time a descendant is
  reached, so the pass records the nodes it found hidden as it goes.
- **`visibility: hidden` is rendered.** The box is generated, laid out and takes
  up space; it has a before-change style and its transitions run. Folding it in
  here would be wrong, and invisible in every `display` fixture.

The walk is O(depth) and sits at the last gate before a transition starts —
after `diff_animatable` has found an animatable change on a node that declares a
`transition` — so a node with neither never pays for it.

**This caught a shipped component, and the rule it puts on the library is worth
stating on its own: an overlay that animates must stay rendered.** Animate
`opacity`, `visibility` or `transform`; never toggle `display`. A `display` flip
cannot transition on **either** backend — a browser refuses it for the same
reason, which is why `@starting-style` and `transition-behavior:
allow-discrete` exist — and `rinch-components` ships one stylesheet to both.

The `Drawer` was the one component with it backwards (issue **#751**): its root
carried `display: none` while closed and its panel carried
`transition: transform`, and one reactive effect un-hid the root and retargeted
the panel in a single pass, exactly the shape above. So its 300ms slide-in ran
on desktop only until this change, and had never run on `rinch-web` at all. Its
closed state is `visibility: hidden` now, which is where `Popover` already was —
hidden, still **rendered**, and still out of paint, hit testing and the Tab
order on both backends.

The blast radius is wider than "a control restyled in an inactive tab":
**any component that un-hides an ancestor and retargets a transitioned property
in the same style pass** loses its animation. `Modal`, `Notification`,
`Tooltip`, `Select`, `DropdownMenu`, `Tabs` and `Stepper` are audited one
fixture each in `crates/rinch/src/app/overlay_animation_audit_tests.rs`, against
the rule "a transitioned property that changes on the reveal pass must have a
transition running" — which is the only form of the assertion that
discriminates, since "zero transitions ran" is what the *bug* looks like.

**The reactive helpers rest on this now, and no longer on a second mechanism of
their own.** `show_dom`, `match_dom`, `for_each_dom_typed` and the component
re-render effect used to call `NodeHandle::clear_animations()` before
`remove()`, which wrote an inline `transition: none; animation: none` over the
whole subtree — and nothing ever took it off, so a branch hidden once could
never animate again (issue #704). It also hid the detach reset: through one of
those helpers, deleting `detach_subtree_styles` changed nothing observable. The
method and all five call sites are gone. Every one of them was
`clear_animations(); remove();` and `NodeHandle::remove` is
`DomDocument::remove_node`, the first of the five routes listed above; on
`rinch-web` the browser already cancels a removed element's transitions and
treats re-insertion as a first style. **A removal path must not write styles**:
the node outlives the removal, so anything stamped there is permanent.

### The page-load guard arms transitions, and only transitions

`has_been_styled` is per node. There is a second, whole-document suppressor
beside it: `NodeTree::transitions_enabled`, false at construction and set at the
very end of the first `resolve_layout` ("Enable transitions after first layout
completes", `layout_engine.rs`). `has_been_styled` alone would not do the job,
because a tree is cascaded more than once before its first layout — appending a
`<style>` element re-resolves the whole document there and then
(`maybe_load_style_css`), so a component that appends its own stylesheet after
building its markup leaves every node already styled, and the next rule it loads
is a *change* on an already-styled node. Without the flag that is a transition
running on page load. `recompute_all_styles_full` forces the same flag off for
the duration of its re-cascade, for the same kind of reason: a theme change
applies instantly rather than every element transitioning from the old palette
to the new one.

**Neither rule is about animations, and since #762 neither reaches one.** A
`@keyframes` animation has no before-change style to be wrong about — it does
not interpolate from a previous style, it plays its own — and a browser runs one
on the very first frame the element exists. The animation half of
`apply_stylo_styles_to_taffy` sat inside the same `if` as the transition half,
and two faults followed:

- **An animation present in the first frame never started.** Nothing re-cascades
  those nodes afterwards, so it was simply gone. On desktop the first resize or
  scale-factor event happens to re-cascade the tree and the spinner starts,
  which is why this went unnoticed; an embedded `RinchContext` at a fixed size
  gets no such event and kept a dead spinner for the life of the context.
- **A theme change stopped every animation in the document, permanently.**
  `recompute_all_styles_full` cleared `tree.active_animations` and re-registered
  none, because the flag it had just forced off was the gate. Toggling dark mode
  killed every `Loader`, `Skeleton`, `Progress` stripe and spinner in the app.

So the animation block reads no flag, and `recompute_all_styles_full` no longer
clears the map. The re-cascade it runs is what reconciles it:
`animation::start_animations` matches a running animation **by name**, keeps its
clock — `start_time_ms`, `paused_elapsed_ms`, `play_state` — and drops one whose
declaration the new sheet no longer carries. On this pass, and only this one
(`NodeTree::refreshing_animations`, a flag of its own), a kept animation takes
everything else afresh: the `@keyframes` rule is looked up again and the
animation dropped if it is gone, the stops are re-extracted from the new base
style, and the duration and delay come from the new declaration.

Preserving rather than restarting is what a browser does, measured in Chrome
150.0.7871.100 by replacing a `<style>` element's `textContent` under a running
`animation: … 10s linear infinite` and reading `Element.getAnimations()`:

| The swap | `getAnimations()` | `currentTime` | effect |
|---|---|---|---|
| identical rules, one unrelated declaration changed | 1 | **unchanged** | unchanged |
| same `animation-name`, changed `@keyframes` body | 1 | **unchanged** | **new** keyframes, applied at once |
| `@keyframes` deleted, declaration kept | **0** | — | back to the base style |
| `animation` declaration deleted | **0** | — | back to the base style |

rinch matches all four on the theme path. Rows 2 and 3 took the refresh, and
row 3 is worth a sentence because an earlier revision of this page called it
unreachable: the theme sheet is replaceable and can carry `@keyframes` — rinch's
own `generate_theme_css_string` puts component keyframes in it — so a new theme
can drop one, and keeping the entry whole left that animation running where
Chrome 153 cancels it. A later measurement in Chrome 153.0.8010.36, under a
seeked `currentTime`, added three more rows the refresh also matches:

| The swap | Chrome 153 |
|---|---|
| `animation-name` `k` → `k2` | a **new** animation, `currentTime` 0 |
| `animation-duration` 10s → 20s | same animation, `currentTime` 3000, progress 0.15 |
| a panel shown by the theme, which also moves `font-size` 10px → 40px under a `1em → 11em` spinner | width 80px at 1000ms |

The refresh is scoped to `recompute_all_styles_full` on purpose, and that is
**not** the only pass that re-cascades the whole document. Two others drop every
node's cached style too and do not set the flag: appending a `<style>` element
(`maybe_load_style_css`), and a viewport change in `resolve_layout`. Measured
after the first layout, a `<style>` appended with a redefined `@keyframes kk`
leaves a running `kk` on its old body. Those two passes, and every targeted
restyle — a class change, a hover — still keep a kept animation's stops and
timing whole: an edited `@keyframes` body (issue **#766**), a changed duration or
delay (**#780**), and a stop derived from the base style, such as the implicit
`from` of a `to`-only rule carrying `color` (**#781**), stay stale. **#781**
tracks the two whole-document passes. The full restyle is the pass a theme
toggle takes, it runs rarely, and it can afford a keyframes lookup and a stop
extraction per animated node; a targeted restyle runs per hover and should not
pay that.
`crates/rinch-dom/tests/full_restyle_animation_refresh_tests.rs` pins every row.

**One divergence the theme path now reaches, not fixed here: a finished one-shot
animation replays on a theme toggle.** An animation with a finite iteration
count and no `forwards` fill is removed from `tree.active_animations` by
`tick_animations` once it completes. The full restyle re-cascades its node,
`start_animations` finds no entry of that name, and mints a new one from t=0.
Chrome does not replay it, and neither did `main` before #762 — its full restyle
cleared the map with the animation block gated off, so nothing was minted, which
was right by accident. It is the mechanism of issue **#783** (a finished
animation's entry is forgotten, so any later cascade of the node restarts it),
reached through the theme path. Component CSS declares only `infinite`
animations; the one finite iteration count rinch ships is the theme's
`prefers-reduced-motion` rule (`animation-duration: 0.01ms;
animation-iteration-count: 1`), whose replay would last 0.01ms.

One consequence comes with the first-frame start, and it is pre-existing rather
than new: an animation's clock begins at the cascade that styles the node —
`append_child`, for a freshly appended element — not at the frame that first
shows it. So the first frame is already a few milliseconds in, where a browser
defers an animation's start to the first frame it is rendered in. Every
post-mount insertion has always behaved that way; #762 only lets the first frame
join them. Issue **#768**.

`crates/rinch-dom/tests/animation_start_gating_tests.rs` is the pin, and its
module doc carries the measurement above plus a table of which fixture kills
which mutant — including the two results that are easy to get wrong by
reasoning: no animation fixture moves when the flag is removed from the
transition gate, and "a transition declared before the first layout does not
run" does **not** distinguish the flag from `has_been_styled`, because on a
first cascade both suppress.

## A hidden element's `@keyframes` animations

`@keyframes` is the same question with a stricter answer and a second consumer,
and it took its own change (issue #747).

The answer is stricter because css-animations-1 §3 does not merely refuse to
*start* something on an element that is not being rendered: such an element has
no animation effect at all, and showing it again starts a **new** animation from
the beginning rather than resuming the one it had. So rinch drops the
`ActiveAnimation` entries rather than parking them, and starts fresh ones on the
way back.

The second consumer is the frame clock. A transition self-limits — it has a
declared duration and dies after it — but `animation: … infinite` does not, and
`AboutToWait` schedules another frame whenever `tree.active_animations` holds an
animation that is not paused. An animation left running on something nobody
paints therefore keeps a desktop app rendering at full rate indefinitely: a `Loader` in a closed panel
or an inactive tab, with nothing on screen moving. That is why parking the
entries was not an option, and it is the same symptom #699 fixed for a *removed*
`Loader`.

Three sites do it, all in `apply_stylo_styles_to_taffy`:

- **Nothing starts on a node that is not rendered.** `animation_is_rendered` is
  the transition gate's ancestor walk asked of the state *after* the cascade —
  "is it rendered now" — so it takes no old display and no `was_hidden` list.
  Reusing the transition gate here would be wrong in a way nothing else notices:
  `was_hidden` names the nodes that were hidden *before* the change, so an
  ancestor un-hidden on this same pass is on it, and an inner wrapper shown
  beneath it would refuse its own restart. The gate also **takes away** what the
  node was already running, which the subtree walks cannot: a node *moved* into
  a hidden panel changes nobody's `display`, so the only thing that runs is its
  own re-cascade.
- **A subtree that stops being rendered loses its animations**, whole, whatever
  each box's own `display` computes to — `cancel_animations_in_subtree`, beside
  the transition cancel and under its own guard.
- **A subtree that starts being rendered again gets them back, from t=0** —
  `restart_animations_in_subtree`, at the same site that resets the scroll
  offset. This is the half that has no counterpart on the transition side, and
  it is not symmetry for its own sake: a node shown by an **ancestor** need not
  be re-cascaded at all. `set_style` drops the cached Stylo data of the node it
  was written to and of nothing else, so a panel un-hidden with
  `set_style("display", "block")` re-cascades the panel alone; without the walk
  its spinner would stop on the way in and never start again. A `class` write
  invalidates the subtree and so restarts descendants through the ordinary
  per-node path — the two routes differ, and the fixtures say which is which.
  The walk stops at any box whose own `display` is `none`, and skips the node it
  was called on, whose own cascade has already restarted it.

It costs O(subtree) on a change from `none` to rendered, which forces a full
layout and paint of that same subtree anyway; there is deliberately no "does
anything under here animate" guard, because there is no answer to that cheaper
than the walk. Measured on the worst case that can be built for it — hiding and
showing a 4000-node, animation-free panel, debug build — the walk costs **+4.2%**
of the cycle (21.06 → 21.95ms).

### `visibility: hidden` animates, and that is a frame clock nobody switches off

`visibility` is rendered here for exactly the reason it is rendered for a
transition: the box is generated, laid out and takes up space. It is also what a
browser does — a `visibility: hidden` element's animation runs in Chrome, it is
merely not painted.

The cost is worth stating plainly, because it is not the one-off a transition's
would be. An `animation: … infinite` has no duration to expire, so a rendered-
but-invisible spinner asks for a frame forever. That is now reachable through a
shipped component: #751 made the closed `Drawer`'s root `visibility: hidden`
precisely so its panel could transition, so a `Loader` placed inside a **closed**
drawer keeps the app rendering. Measured, software backend, 804x600, closed
drawer, 20 idle frames:

| closed-state spelling | `active_animations` | idle frames asking to redraw | ms per tick+paint |
|---|---|---|---|
| `display: none` | 0 | 0 / 20 | 0.001 |
| `visibility: hidden` (today) | 1 | 20 / 20 | 2.53 |
| open drawer, either spelling | 1 | 20 / 20 | 8.8 – 9.7 |

This is accepted rather than overlooked. Refusing an animation to a
`visibility: hidden` box would put desktop at odds with both the browser and the
transition rule next to it, for one component's benefit. The cure belongs to the
component — `animation-play-state: paused` on a closed overlay's subtree — and
since issue **#763** it works. A paused animation keeps its entry, because its
frozen sample still has to reach `computed_style` on each cascade, but it has
nothing to advance: `tick_animations` neither counts it nor marks its node dirty,
and `AboutToWait`'s "was there anything to tick" guard asks
`NodeTree::has_running_animations()` instead of whether `active_animations` is
empty. So a paused spinner schedules no frame, and resuming it continues from the
time it was paused at. A paused `font-size` (or any other text-measure) animation
is measured by **each cascade that writes its sample, and never per tick**.

That is the one cost this change adds, so it is stated with numbers: every
cascade of a node carrying a text-measure animation re-measures its text and
re-runs Taffy, whether or not the sample moved. Measured on 500 rows, release,
min of 20 (hover) / 12 (restyle):

| workload | no animation | with a paused `font-size` animation |
|---|---|---|
| one-row colour-only hover on that row | 0.143ms, 0 Taffy computes | 0.679ms, 1 compute |
| whole-document colour-only restyle | 16.1ms | 45.1ms (all 500 rows animated) |

It is bounded by how many nodes carry such an animation, which is rare, and the
behaviour it replaces paid a compute on *every frame* for the same node. The
narrowing is available and deliberately **not** done here: compare the node's
old `computed_style` against the post-animation style, rather than asking only
whether one of its animations has a `font-size` stop. That would make a hover
over an unchanged paused sample free.

The invalidation is also the **whole** of the guarantee, which is why issue
**#784** mattered: `compute_inline_block_layouts` measures an atomic inline out
of Taffy's cache, so text inside an `inline-block` was re-measured by nothing
when a `none` → rendered crossing changed it, and a paused sample stayed frozen
in the old font for good. `mark_atomic_inline_dirty` now marks those Taffy nodes
on an `ifc_dirty` pass instead of returning early.

A finished animation with `animation-fill-mode: forwards` or `both` is the same
shape (issue **#782**): its fill is as constant as a paused sample. The tick that
finishes it writes the fill and marks the node dirty — that frame shows the end —
and records it in `ActiveAnimation::fill_settled`; later ticks re-apply it
without dirtying anything, and neither `tick_animations` nor
`has_running_animations()` counts it.

`Drawer`'s closed rule does not declare the pause itself yet. An app that wants a
closed drawer holding a `Loader` to idle can add the rule below; the three
`Loader` variants animate `__oval`, `__bar` and `__dot` respectively, so it names
all three:

```css
.rinch-drawer__root--hidden .rinch-loader__oval,
.rinch-drawer__root--hidden .rinch-loader__bar,
.rinch-drawer__root--hidden .rinch-loader__dot { animation-play-state: paused; }
```

None of the three sites reads `transitions_enabled` (see "The page-load guard
arms transitions, and only transitions" above), and for the restart walk that is
a separate decision rather than a consequence. The walk shipped behind the flag,
and the flag is off for every cascade before the first layout completes — so a
panel shown by an inline `display` write on one of those passes dropped its
descendants' animations on the way out and did not start them again until
something unrelated re-cascaded the subtree. `animation_start_gating_tests.rs`
pins that shape twice, once for a spinner that never ran and once for one that
did, whose clock must restart inside the show pass.

The other flag-off pass, `recompute_all_styles_full`, reaches the walk too, and
an earlier revision of this page said it did not. A theme that un-hides a panel
runs the walk on the panel's cascade, **before** the descendants' own, so the
walk mints their entries from their pre-restyle `computed_style` — for an
`em`-sized spinner under a theme that also changes the font-size, stops on the
old basis (20px where Chrome gives 80px). Counting running animations cannot see
it, because every node is re-cascaded on that pass and the spinner runs either
way. What repairs it is the refresh above: each descendant's own cascade comes
after the walk and re-extracts the stops from its new style.

## Optimizations

Current and planned improvements to the rendering pipeline:

- **Dirty region caching** (software) - Only repaint the rectangular area covering changed nodes
- **Subtree pruning** (software) - Skip paint traversal for nodes outside the dirty region
- **Viewport culling** (both backends) - Skip nodes that fall outside the window. This is the
  dirty-region test against a second region that is always present, so it applies to a full
  repaint too. It matters most on the GPU: Vello discards invisible paths in its coarse stage,
  but only after flattening every one of them.
- **Clip elision** (both backends) - A clip layer that provably removes no drawn pixel is not
  pushed. Two cases qualify: a clip whose rect contains the whole render target (it can only
  remove pixels that are discarded anyway) and a clip nothing inside reaches past. Both require
  square corners — a rounded clip cuts the corners of its own box, so "nothing overflows" does
  not mean "nothing is cut".
- **Sensitivity flags** - Hover/active/focus only trigger repaints for nodes with matching CSS selectors
- **Batched redraws** - Multiple state changes are batched into a single repaint via the frame clock (`AboutToWait`, above)
- **Layer compositing** - GPU layers for transformed content (planned)
- **Text caching** - Glyph atlas for repeated text (planned)

Two things the first two do **not** do, because both would be visible bugs rather than
optimisations.

**Neither an ancestor's culling nor its elision can remove a `position: fixed` descendant.** A
fixed box is painted in viewport space, from the sequence of its nearest stacking-context
ancestor, so *that ancestor's* box says nothing about whether the fixed box is on screen — an
off-window stacking context holding one is therefore not pruned, and a clip whose subtree holds a
fixed box is never elided. Note the shape of that claim: it is about what an ancestor may
conclude, not about the fixed box itself. A fixed box is culled and elided on its own terms like
any other — one covering the window has its own clip elided, one 600px below the window is
culled — and both are correct, because in viewport space its own rect is the whole truth about
where it lands.

**How an ancestor knows: it measures the subtree, it does not read the node's category.** The
cull asks `layer_bounds::subtree_is_entirely_outside` — the same walk that sizes an opacity
layer — and prunes a stacking context only on a *definite* extent that misses the render target.
Every not-knowing means paint in full: a `position: fixed` descendant (which is what keeps the
paragraph above true, by construction), a `position: sticky` one (paint places it by an ancestor
walk the subtree does not contain), a `position: absolute` one that would escape a clip below its
containing block (#550), the walk's visit budget, and its depth cap.

The cull used to answer the syntactic question instead — decline for *every* stacking context
with children — and that was correct and expensive: an `opacity: 0.5` sheet parked below the fold
allocated, filled and composited a whole-surface pixmap every frame, 26.7ms against 0.8ms on
three of them at 1080x2460 (#562). Narrowing it to stacking contexts that *also* clip is the
obvious repair and is **wrong**: the skip-draw-and-recurse arm it hands the rest to runs before
any layer is pushed, so the sheet's fixed descendant came back unfaded, with the whole suite
green. Measuring the subtree is what makes the cheap answer and the correct answer the same one.

**And the cull tests a box's layout rect against the window grown by a margin**, not against the
window itself, because a `box-shadow`, an `outline` or a text run wider than its own box all put
ink outside the rect being tested.
