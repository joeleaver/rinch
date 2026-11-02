# Images, debug/MCP, dialogs, clipboard, tray, notifications, hotkeys, theme, audio DSP

Read this page when your task touches: image loading, DevTools, rinch-debug, clipboard, system tray, notifications, global hotkeys, theme system, `rinch-av`'s DSP output (`open_dsp_output`, `rinch-dsp`).

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

### Image Support

Images render on **both** desktop backends — GPU (Vello, an image-brush `scene.fill`) and software (tiny-skia, `draw_pixmap`) — via `<img>` elements and `background-image: url(...)` CSS. Remote/file images load asynchronously on background threads; `data:` URIs (e.g. base64 PNG) are decoded synchronously and inserted straight into the cache (`request_image_load_for_node`).

**Architecture:**
```
rinch-core:  ImageLoader trait + ImageLoadResult enum + app scheme registry (no deps)
rinch-dom:   ImageCache + FileImageLoader + decode pipeline (image crate)
rinch:       NetworkImageLoader (rinch-http, gated behind image-network feature)
```

**Key files:**
- `crates/rinch-core/src/image.rs` — `ImageLoader` trait, `ImageLoadResult` enum
- `crates/rinch-dom/src/image_cache.rs` — `ImageCache`, `DecodedImage`, `FileImageLoader`, async load
- `crates/rinch-dom/src/paint/image.rs` — `paint_image()` with object-fit support: all five of `fill`/`contain`/`cover`/`none`/`scale-down` (`ObjectFitValue`, `computed_style/values.rs`)
- `crates/rinch/src/image_loader.rs` — `NetworkImageLoader` (feature-gated)

**How it works:**
1. `set_attribute("src", ...)` on `<img>` or `BackgroundValue::Image` during style resolution triggers a load
2. `ImageCache` checks if already cached; if not, spawns a background thread via `request_image_load()`
3. The background thread calls `ImageLoader::load()` then decodes with the `image` crate to RGBA8
4. Results go to a static `Mutex<Vec<PendingImage>>` queue
5. `drain_pending_images()` at the start of layout picks up decoded images, updates Taffy intrinsic dims
6. `paint_image()` renders via `Painter::draw_image` with proper affine transforms. Vello samples an image at each pixel's centre, as tiny-skia does (#1027), so `VelloPainter::draw_image` and `fill_repeating` draw it with its own transform. (Vello 0.7 sampled from the pixel's top-left **corner** — an image at a whole-pixel origin came out averaged with its neighbours — and rinch moved the image half a device pixel up-left, `vello_painter::centre_sampling`, to compensate; Vello 0.9 moved the sample to the centre, so the shift was dropped for images with the Vello 0.11 upgrade, #1955. Gradients still take it: Vello evaluates them at the corner.) Pins: `vello_painter::sampling_tests` (a CPU model of the shader arm) and the `#[ignore]`d `compositor::gpu_alpha_tests::gpu_alpha_vello_draw_image_*` (real GPU)

**Background layers** (#468, `crates/rinch-dom/src/paint/background.rs`):
**every** layer of `background-image` (#1142) is sized, positioned and tiled by
its entry of `background-size`/`-position`/`-repeat`/`-origin`, a shorter list
repeating to the image count (css-backgrounds-3 §2.2), over a painting area that
is always the border box (`background-clip` is not read), painted from the last
listed to the first over `background-color`. The first layer is
`ComputedStyle::background` plus `background_size`/`_position_x`/`_y`/
`_repeat_x`/`_y`/`_origin` (with the colour under it in `background_underlay`);
the layers below it are `ComputedStyle::background_layers` (top to bottom, empty
for one image, a `none` or conic layer left out). A transition or animation of
`background-position` moves the **first** layer only; the others snap to their
new values. Before #1142 every list was read at its first entry, so
`ColorSwatch`'s four-layer checkerboard drew one layer. **Gradients**
(`paint/gradient.rs`, #1798): a stop's position is a `LengthPercentageValue`
resolved against the gradient line at paint (a length was read as a hundredth
of the line until #1798, so `31px` was `31%`), fixed up by css-images-3 §3.5.3
(missing ends are 0%/100%, a position before an earlier one moves up to it,
unpositioned stops are spaced between their neighbours); the peniko gradient
spans the first stop to the last, `Extend::Repeat` for `repeating-*`. A radial
gradient keeps its ending shape (`circle <length>`, two radii, the four extent
keywords) and centre; an ellipse is a circle of radius `rx` drawn under a
vertical scale of `ry / rx` (the shape to fill is mapped through the inverse),
and its stops are laid out from radius 0 because tiny-skia has no start
radius (a repeating gradient's period rotated to begin there). tiny-skia
interpolates unpremultiplied, so `skia_painter::premultiplied_ramp` rewrites
the stops into the premultiplied ramp CSS (and Vello) draws: a transparent stop
takes its neighbour's colour, other alpha changes get 8 premultiplied samples.
Not read: interpolation hints, `in <colorspace>`, and a corner direction's
dependence on the box's aspect ratio (`to top right` is 45°). **`mask-image`**
(#1798): `ComputedStyle::mask_image` holds its gradient layers (a `url()` layer
is not read; the other `mask-*` longhands are gecko-only in Stylo and keep
their initial values). `paint::paint_node` wraps a masked box: a clip to its
border box, an isolated layer holding `paint_node_unmasked`, then a
`BlendMode::DestinationIn` layer (tiny-skia `DestinationIn`, Vello
`Compose::DestIn`) into which each mask layer is filled over the border box. A
masked box is a stacking context, so its hoisted descendants paint inside the
layer. The software painter composites a `DestinationIn` layer over everything
its parent layer holds, not only what the mask drew, so an empty or
all-transparent mask hides the content as Vello does. Hit testing clips a
masked box's subtree to its border box (`hit_test_node`), as Chrome 155 does;
the mask's alpha is not consulted inside it (nor by Chrome). The background
tile cache is keyed by the tile's own size as well as its raster size: stop
positions are lengths in the tile's space. Pins:
`rinch-dom/tests/gradient_mask_1798_tests.rs` (Chrome 155 pixels),
`hit_testing::tests::a_masked_boxs_overflow_is_not_hit_outside_its_border_box`,
`rinch-visual-test`'s `css_export` gradient test. Not Chrome's: a mask on a
non-atomic `display: inline` element (a `<span>`): its text is drawn by its
inline formatting context's root and is not masked, and its own `0x0` box
masks away everything it paints itself, positioned descendants included —
Chrome masks the span's fragments and shows a positioned descendant that lies
inside them.
Not Chrome's: a
gradient pixel whose centre lies exactly on a hard stop can take the colour
after the stop where Chrome takes the one before (#1740). `space`/`round` paint as `repeat`. **No layer is ever dropped**
(#1143's second review): a layer whose one tile covers the box is one fill (a
gradient) or one clipped image draw (`cover`), never rasterised; any other
layer that repeats on either axis is **one**
`Painter::fill_repeating` — its tile rasterised by a `TinySkiaPainter` at the
device size it covers (`ceil`; the tile is laid out in CSS space under `scale(w/tw, h/th)` so an angled
gradient keeps its slope under `scaleX`/skew; then a scaling pattern transform, bilinear when
that is not exact; a `scale()`d box rasterises bigger), cached **per document**
(`NodeTree::background_tiles`, freed with it) by
image (gradient angle + resolved stops, or url + `DecodedImage::id()` — never a
data pointer, which the allocator reuses) + raster size (`background_tile_rasters` / `background_tile_cache_hits`).
The cache holds at most 32 MB: a tile that would pass it evicts the least recently
used tiles, never one the frame being painted has used, and is otherwise kept for that frame only
(a per-frame overflow map, also ≤ 32 MB, so the frame's other boxes hit it) (#1156:
it was cleared whole, so a working set just over 32 MB rasterised every tile every
frame), and each `paint_document` forgets the tiles no frame has used for
`TILE_CACHE_IDLE_FRAMES` (60) painted frames (#1157: it was a thread-local never
released; an idle window paints no frame and keeps what it holds until it paints;
`NodeTree::background_tile_cache_bytes`). Pins: `background_tile_cache_eviction_1156_tests.rs`.
The tile is
filled as a tiny-skia `Pattern` or a Vello `Extend::Repeat` image brush from the
same pixels (the brush's parameters are pinned; no test renders it on a GPU), and clipped to the one-tile strip on an axis that does not repeat.
A build without the software painter (`embed`) has no rasteriser, and a `url()`
layer needs none (#1158): `background::pattern_source` hands the decoded image
itself to `fill_repeating` (`RepeatTile::premultiplied: false`, straight alpha
as `draw_image` hands Vello an `<img>`; its transform scales the image to the
tile), one `Extend::Repeat` brush however many tiles (80,100 image draws for a
3x2 image over 800x600 before). It is not cached (no raster to cache); the
pixels are copied into a `Blob` per paint, as `draw_image`'s are. On a GPU it
draws what the tile-by-tile path drew, pixel for pixel, natural size, scaled and
fractionally placed (the `#[ignore]`d `gpu::*` probes of
`background_tiling_embed_tests.rs`, `-- --ignored`, run with
`--no-default-features`). Only a repeating gradient in such a build, a
`no-repeat` tile, or a tile over 4M device px goes tile by tile, over the **visible** tiles only
(`visible_paint_rect`), uncapped. Tiles are laid out from the border box
**rounded to a whole pixel of its own (pre-transform, scaled) space**, as Chrome
snaps the box, so a box at `left: 10.5px` draws crisp tiles from pixel 11; only
`background-position` and a transform keep a fraction. **In software a
repeating axis keeps that sub-pixel position** (#1151): the pattern's origin
keeps its fraction and is sampled bilinearly, so the edge pixel blends as Chrome
155 blends it (a red/blue tile at `0.5px` reads `(128,0,128)` there) and a
`background-position` animation moves smoothly. tiny-skia samples a pattern
whose map is a pure translation nearest, fractional or not, so
`TinySkiaPainter::fill_repeating` adds one ulp of scale to keep it bilinear.
That costs about 2.7x per pixel of a fractionally placed repeating fill (a
full 1200x800 window, DPR 1: 3.8 → 10–12 ms per paint). **On the GPU it still
steps by whole device pixels**: `VelloPainter::fill_repeating` places a tile
whose texels are device pixels on the nearest whole pixel and samples nearest,
because Vello clamps its bilinear taps at the image's edge after
`Extend::Repeat`, and a fractional phase made the stripe width pulse (4.5–5.5px
for a 10px tile, ±9% brightness) during an animation. An axis that does **not**
repeat rounds a fractional `background-position` to a device pixel, as Chrome
does, in device space under a plain translation (`DeviceSnap`; snapping in shape
space seamed under a fractional translate); rinch also rounds a fractional
translate there, where Chrome blends (a known gap). So does the tile-by-tile
path on every axis, to keep its tiles from seaming (`embed`, a tile over 4M px).
A scaled image tile is rasterised nearest and sampled again, where Chrome samples
once (#1753). The common case — one
tile covering the box — is one fill and no clip, as before. `background-position-x`/`-y` are
`TransitionProperty`s (paint-only: `affects_layout` is `false`), and
`transition: background-position` expands to both. An image's `auto` size is
its intrinsic size now; it used to be stretched over the border box.

**A block `<img>` is its natural width, and keeps its ratio** (#788, #1150).
It is `item_is_replaced` in Taffy, so a block container does not stretch it
(CSS 2.1 §10.3.4: `img { display: block }` is 40 wide for a 40x30 image, and
`margin: 0 auto` centres it), and its measure is `replaced::measure`, the
canvas's (#1173): a lone style `width` or `height`, or a `min-*`/`max-*` clamp,
gives the other dimension through the image's ratio, inline, block and as a
flex item. So the `Image` component (`width: 100%; height: auto`) under a block
parent is now as tall as its ratio says, where it kept the natural height. A
flex container still stretches an `<img>` (Chrome does too); a grid container
still stretches it, where Chrome does not (#1280). **The
`width`/`height` attributes of `<img>`, `<video>` and `<iframe>` are
presentational hints** (#684): each maps to the property of its name, read by
HTML's rules for parsing dimension values
(`rinch_core::dom::parse_html_dimension`: `"100"`, `"100px"` and `"100abc"` are
100px, `"50%"` a percentage, `".5"`, `"+1"` and `"50*"` errors — Chrome 153's
answers), as a declaration block on `Node::presentational_hints` that
`synthesize_presentational_hints_for_legacy_attributes` hands Stylo at
`CascadeLevel::PresHints`: below every author rule (a `*` rule, a layered
one, `width: auto`) and above the UA sheet's normal rules; `revert` drops the
hint and `revert-layer` keeps it. A write or removal of
either attribute on one of the three rebuilds the block
(`RinchDocument::sync_presentational_hints`) and restyles that element alone
(`note_attribute_change`); any other attribute on them, and `width`/`height`
on any other element, still cascades nothing. On an `<img>` and a `<video>`
the pair also maps to `aspect-ratio: auto w / h` when both are lengths above
zero (`replaced::attribute_ratio`), written into the same hint block, so an
author `aspect-ratio` replaces it (#1286: `aspect-ratio: auto; width: 200px;
height: auto` on an unloaded hinted image is 200x0, as in Chrome). It is the
ratio of an image with no natural size (not loaded, or failed) —
`<img width=100 height=50 style="width: 200px; height: auto">` is 200x100
before the image arrives — and a loaded image's own ratio replaces it. A
`<canvas>`'s two attributes stay its bitmap size, read as integers (#1173).
**An `<svg>` is a replaced element too** (#1797, `replaced::svg_context`): its
`width`/`height` are presentation attributes, mapped into the same hint block
but read as a CSS `<length-percentage>` with a bare number in pixels
(`replaced::svg_length`: `"14"`, `"14PX"`, `"2em"`, `"1e1"`, `"50%"`; not
`"14 px"`); and it has a `NodeContext::Replaced` measure whose natural size
is the attributes that are absolute lengths (`em` against its own font
size; a negative one is 0) and whose natural ratio is theirs when both are,
else the `viewBox`'s. A missing dimension comes through the ratio, else from
300x150; with neither and a ratio it fills its parent's width
(`replaced::SVG_FILL_WIDTH`). It is hollow (`ignores_inner_display`): its
children are its drawing, detached from Taffy, never CSS content. A
`viewBox` write restyles it. Before, an `<svg width="14" height="14">` with
no CSS size was 0x0. Not Chrome's (pinned in
`tests/svg_size_attributes_1797_tests.rs`): an **inline** `<svg>` with a
ratio and no natural size is 300 wide where Chrome fills its containing
block (a block-level one fills it); an invalid attribute (`"abc"`) counts as
absent, where Chrome lays an `<svg>` with an invalid `width` out at its
containing block's width.
**CSS `aspect-ratio`** (#1286) is `ComputedStyle::aspect_ratio`
(`AspectRatioValue { auto, ratio }`, a degenerate ratio is `auto`): a
non-replaced box hands it to Taffy's `Style::aspect_ratio` (block, flex,
grid, an atomic inline's root compute, absolute boxes); an `<img>`,
`<canvas>`, `<video>` and `<iframe>` do not (`replaced::measures_aspect_ratio`,
cleared in `taffy_style_from_computed`) — their measure context carries the
computed value (`NodeContext::Image::aspect`, `NodeContext::Replaced::aspect`,
kept by `replaced::sync_replaced_measure` on each cascade, which owes a layout
when it moves) and `replaced::measure` takes `AspectRatioValue::used_ratio`:
a `<ratio>` without `auto` replaces the natural ratio, `auto <ratio>` is used
only when there is none, and a `<ratio>` without `auto` relates the
`box-sizing` box's sizes (always the border box today, since `box-sizing`
is not read — #1278), any other the content box's. An internal table box (a
cell, a row, a row group — `TablePart::is_internal`; not the table or a
caption) takes no ratio, as in Chrome 155. `aspect-ratio` does not
transition (Chrome interpolates it) and an `@keyframes` stop of it is
ignored. Not Chrome's (pinned in
`tests/aspect_ratio_1286_tests.rs`, `known_differences_from_chrome`): Taffy
transfers the other axis's `min-*`/`max-*` onto a definite size
(`width: 100px; aspect-ratio: 2; max-height: 30px` is 60x30, Chrome 100x30 —
#1561), and a box with a ratio has no content-based automatic minimum size,
so text taller than the ratio overflows a visible box and a stretched flex
item shrinks (#1562); and Taffy applies a non-replaced box's `auto <ratio>`
to its border box too, where Chrome uses the content box (visible only with
padding or a border, every rinch box being `border-box` — #1278). Three gaps
the ratio does not reach: a `position: fixed` box with one vertical inset
and an auto height is given its content height afterwards, so it is 0 tall
(`top: 0; width: 100px; aspect-ratio: 2` is 100x0, Chrome 100x50 — a fixed
`<img>`/`<video>` with `top: 0` was 0 tall before `aspect-ratio` too; #1569);
a shrink-to-fit box (an `auto`-width `inline-block` or absolute box) whose
width comes from its content does not take its height from it (30x10 around
a 30x10 child with `aspect-ratio: 1`, Chrome 30x30 — #1570); and a flex item
grown from a 0 basis (`flex: 1`) under `align-items: center` takes no cross
size from the ratio (400x0, Chrome 400x100 — #1571). Also not
Chrome's: an unloaded or failed image with a non-empty `alt` reserves its
attributes' 100x50 where Chrome lays out the alt text (48x18, both attributes
ignored), and a failed one with one attribute and no `alt` is 100x0 where
Chrome draws its 16x16 broken-image box; an out-of-flow image with an `auto`
dimension fills its containing block (#1432, older than the hints); an
iframe's hinted size is its border box, since every rinch
box is (#1278), so it is 4px smaller than Chrome's; a percentage `height`
attribute does nothing on an inline-level image, as a CSS one does not
(#1420); a `<video>` with a mapped ratio and both dimensions `auto` is 300
wide where Chrome stretches it to its container. The other presentational
attributes (`hspace`, `vspace`, `border`, `align`, `<hr width>`, a table's)
map to nothing (#1419). Pins:
`crates/rinch-dom/tests/presentational_size_hints_tests.rs`.

**App-installed schemes and retry:** `App::image_scheme("myapp-blob", loader)` (or
`rinch::image::register_image_scheme`, the same registry without a builder) makes
`loader` the answer for every source with that URL scheme, process-wide; everything
else still goes to the document's own loader (`request_image_load` asks
`rinch_core::image::image_scheme_loader(src)` first, on the load thread). A loader
returns **encoded** bytes (`ImageLoadResult::Loaded`) and runs on a thread spawned for
that load, so it may block; a closure `Fn(&str) -> ImageLoadResult + Send + Sync` is
one. **A failed load is cached by source** (`ImageState::Failed`) and never retried on
its own: `rinch::image::reload_image(src)` (any thread; `RinchDocument::reload_image`
for a host holding the document) queues the source for every live document whose
cache holds it, counted by `image_cache::has_pending` so idle hosts wake, and
`drain_pending_images` restarts it. Each `ImageCache` records the sources it comes to
hold under its document's key in `LIVE_DOCUMENTS` (`ImageCache::put`, every entry is
made there), so a document that never named the source is not woken (#1493: it was laid
out and asked for a redraw per reload, for nothing — the `paused_animation_frames_tests`
flake, another test's reload landing between two idle frames). The queue is per
document, each with the set of what it holds, so a call costs one lookup per live
document holding the source however many sources a document has not drained (#1473: it
scanned the whole queue; `tests/reload_queue_cost_1473_tests.rs` pins the count,
`image_cache::reload_queue_steps`). A document that never lays out still keeps one
entry per distinct source reloaded, until it drains or drops.
A failed source goes back to loading, a decoded one keeps its pixels until the new
answer lands (and if that fails), and one whose load is in flight has that answer
dropped and is asked for once more (`ImageCache::begin_reload` / `take_retries`), so a
"not yet" already on its way cannot beat the reload. "In flight" covers a **reload**
too: a failed source being reloaded is `Loading`, and a decoded one is listed in
`ImageCache::reloading` (its entry stays `Decoded`, so the picture stays up), so a
second reload during the first starts no load of its own; the first's answer is
dropped and one load follows it. Two parallel loads of one source would land in
either order and the older answer could win. **Every load pushes exactly one answer**:
`request_image_load` catches a panic in the loader or the decode (native, unwinding
builds) and pushes it as a failure (`image loader panicked: …`); a load that pushed nothing would leave its
source in flight for the life of the document, which a reload only waits on. The key
is the source as spelled (`reload_image("x:a")` does not reach `X:a`). `data` cannot
be registered (`register_image_scheme` panics), so no app loader is offered a `data:`
URL; the document's own loader still is, for a background `data:` URL and an
undecodable `<img>` one (#1464). `rinch_core::image::scheme_of(src)` is the parser
(named apart from `App::image_scheme`, which registers).
`crates/rinch-dom/tests/image_scheme_reload_tests.rs` and `review_1429_tests.rs` are
the pins, and `crates/rinch/src/app/review_1429_pimble_tests.rs` drives the whole
late-blob flow through a `RinchApp` with software frames.

**The same two calls on the web** (`crates/rinch-web/src/images.rs`). The browser loads
`<img>` itself, so `WebDocument::set_attribute` routes an `<img>`'s `src` through
`images::set_img_src`: a source whose scheme the app answers for is kept in
`data-rinch-src` (`LOGICAL_SRC_ATTR`; `get_attribute("src")` answers it) and the
browser's `src` gets what it resolved to, or is **removed** while the answer is "not
yet". **The app is never asked inside a DOM write** (`set_attribute` runs under the
`NodeHandle`'s `RefCell` borrow, and a resolver that wrote a signal panicked
`RefCell already borrowed`): an element given a source with no remembered answer shows
nothing until a microtask (`images::resolve_pending`, once per source) asks, and
`mount_tree` runs it before `mount_into` returns. Two ways to answer:
`rinch::image::register_image_scheme` unchanged (the loader is called on the one thread; its bytes become an object URL rinch owns and
revokes when a reload replaces it), or `rinch_web::register_image_url_scheme(scheme,
Fn(&str) -> Option<String>)` (browser only, may capture `Rc`s, answers a URL the app
owns; wins over a loader for the same scheme). Answers are remembered by source,
"not yet" included (`images::NOT_YET`): only a reload asks again, as desktop caches a
miss. Without it a resolver whose signal write re-created its `<img>` was asked again
from a fresh microtask for ever and the page froze (round-2 review, W7). A panicking
loader or resolver aborts the wasm app (no unwinding); only native unwinding builds
catch it.
`rinch::image::reload_image` is `rinch_core::image::reload_image`, which calls each
backend's reloader (`add_image_reloader`: rinch-dom's on its first document, rinch-web's
on its first `WebDocument`); on the web it asks the app again and re-points every
`<img>` **rinch was given** the source for, attached or not (`images::ELEMENTS`, filled by
`set_img_src`, pruned when the `src` changes or goes and by `forget_recursive` /
`WebDocument`'s drop, so a captured element in a hidden branch comes back with the new
picture and a page `<img>` outside every root is never touched; one rendered before the
scheme was registered is recorded too), leaving a picture on screen alone when the
answer is still "not yet". A reload of a source no element was given asks nothing and
forgets the answer (a rinch-made object URL is revoked); a "not yet" reload forgets an
app-owned URL and remembers "not yet", which is how an app that revoked one stops
rinch handing it out. `<img>`
markup set with `set_inner_html` goes through `set_img_src` too. The
editor's model is never read back from the DOM, so an `image` node's `src` stays the
app's URL. `background-image` is not resolved on the web.
`crates/rinch-web/tests/image_sources.rs`, `review_1435.rs` and `review_1435_reentry.rs`
(Chrome 153) are the pins.

**Network loading:** Enable `features = ["image-network"]` for HTTP(S) URL support. It goes through `rinch_http::fetch_blocking`, **not** a private `ureq` call, so image loads share the app's one HTTP agent — its cookie jar, proxy and TLS config (`image-network = ["dep:rinch-http"]`).

**Circular avatars:** a clipping ancestor with `border-radius` clips to a
`RoundedRect` rather than a plain rect, which is what crops an `<img>` to a
circle. The shape comes from `paint::clip_shape` — see **Overflow clipping**
under Rendering Backends for the predicate and its deviations.

### DevTools Panel

Press F12 to toggle the DevTools panel which shows:
- **Performance**: FPS, frame time, and render time
- **Elements**: DOM tree inspection
- **Styles**: Computed styles for selected elements (enable inspect mode with Alt+I)

### Debug & MCP Server (optional)

Enable with `features = ["debug"]` to expose your app's DOM, screenshots, and input injection to external tools via TCP IPC.

**Architecture:**

```
Claude (stdio) → rinch-mcp-server → TCP:port → rinch-debug (in app) → rinch runtime
```

Two crates work together:

| Crate | Type | Purpose |
|-------|------|---------|
| `rinch-debug` | Library | TCP IPC server embedded in the app, auto-starts on a random localhost port |
| `rinch-mcp-server` | Binary | Standalone MCP server that Claude talks to, discovers and connects to running apps |

**Enabling in your app:**

```toml
# Cargo.toml
[dependencies]
rinch = { workspace = true, features = ["debug"] }
```

The debug server starts automatically when the feature is enabled. Disable at runtime with `RINCH_DEBUG=0`. Force a specific port with `RINCH_DEBUG_PORT=9100`.

**MCP configuration** (`.mcp.json`):

For fastest startup, point to the pre-built binary:

```json
{
  "mcpServers": {
    "rinch": {
      "command": "/path/to/rinch/target/debug/rinch-mcp-server",
      "args": [],
      "cwd": "/path/to/rinch"
    }
  }
}
```

Build first with `cargo build -p rinch-mcp-server`. Using `cargo run` instead works but is slower to start.

**MCP tools available:**

| Tool | Description |
|------|-------------|
| `list_apps` | List all running rinch apps with debug enabled |
| `connect` | Connect to a specific app by name or PID |
| `screenshot` | Capture a PNG screenshot (returns as inline MCP image, directly viewable). On the GPU renderer it reads the texture the window is presented from (`desktop::presented_texture`): the composited one while compositor layers are up, else the UI texture. It used to read the composited texture whenever one had ever been made, so after a `GameViewport` or named surface unmounted every screenshot showed its last frame while the window itself was right (#1799) |
| `dom_tree` | Get the DOM tree as JSON with layout bounds (depth 3 by default — `max_depth` goes deeper, `root_id` scopes to a subtree, `verbose: true` adds each node's computed styles) |
| `query_selector` | Query `<body>` and the nodes under it by any CSS selector (`rinch_dom::testing::query_selector`, Stylo-matched since #1796); a string that is not valid CSS (`[data-rid=5]`) is read as one tag, `.class`, `[attr]` or `[attr=value]` |
| `get_node` | Get detailed info for a specific node by ID (includes computed styles, display mode) |
| `get_computed_styles` | Get computed CSS styles for a specific DOM node |
| `get_text_content` | Get text content within a node subtree |
| `click` | Simulate a mouse click at (x, y) coordinates. Optional `modifiers` (the `key_press` names, e.g. `["ctrl"]`, `["shift"]`, `["meta"]`) are held for the click: a `ModifiersChanged` before the press, another after the release restoring what was held — the only way to drive Ctrl/Cmd+click (following an editor link), Shift+click or Alt+click. Absent: whatever the app already holds |
| `type_text` | Simulate keyboard text input |
| `wait_frame` | Wait for the next render frame |
| `close_app` | Close the connected app gracefully |
| `right_click` | Simulate a right-click at (x, y) |
| `mouse_down` / `mouse_move` / `mouse_up` | The pointer primitives. **This trio is the only way to drive a drag** — `click` cannot, so any test of the DnD suite or a scrollbar thumb needs these. `mouse_down`'s `modifiers` stay held (through moves: a Shift- or Alt-drag) until the next `mouse_up`, which restores the state from before that press after its release; `mouse_up` takes its own `modifiers` for the release |
| `scroll` | Scroll a container at (x, y) |
| `drop_files` | Drop files on the window at (x, y), logical px (#1806): what the shell makes of an OS drop, through `handle_event` — one `FileHoverEnter` per path at the point (what the shell sends once a drag's URI list is read, #1962; the moment of path-less hover before it is not reproduced), then one `FileDropped`. So `data-onfiledragenter`/`-leave`, an editor's drop caret and picture claim, and `data-onfiledrop` (paths) / `data-ondropfiles` (`LocalFile`s) under the point run as for a real drop, with the point in `get_click_context()`. Paths go over verbatim, unchecked: a missing one reaches `data-onfiledrop` and is left out of `data-ondropfiles`, as a file deleted between a drop and its read is. An empty list is an error. Desktop only: rinch-web has no debug server (turning paths into `File`s there is not done) |
| `key_press` | Press and release a single key (with modifiers) — one physical keystroke, as distinct from `type_text`'s literal text. Issue #485: now sends a matching `KeyUp` after the `KeyDown`, so a release-aware app (a document-level interceptor, a registered node's `on_key`) sees both halves, not just the press |
| `key_down` / `key_up` | The keyboard primitives, mirroring `mouse_down`/`mouse_up` (issue #485) — **the only way to hold a key** across other commands ("is W still held", a chord that ends on release, a drag armed by a key), which `key_press`'s single press+release cannot drive. `key_up`'s `key` (and modifiers) should match the `key_down` it releases so the press and release pair up by the same string a real keystroke's would |
| `get_caret_position` | A caret point (`x`, `y`) for a `byte_offset`, in logical px in the same frame as `absolute` (#421). For an `<input>`/`<textarea>` it is the top of the caret paint draws (`form_control::painted_caret_rect`: password bullets, centring, a single-line field's horizontal scroll, #1721, a textarea's vertical one, #827); for other text it is the top of the caret the editor overlays draw (`RinchDocument::text_caret_window_rect`, in the content box). Both tools ask those painter-matched helpers and keep no geometry of their own (#1136) |
| `get_glyph_bounds` | The box of the **one** glyph cluster at a given `byte_offset` in a text node — not every glyph; logical px in the same frame as `absolute` (#421), over the layout the text is painted with (`RinchApp::input_glyph_rect` for a field, `RinchDocument::text_glyph_window_rect` otherwise; #1136) |
| `disconnect` | Disconnect from the app without closing it |
| `launch_app` | Launch a rinch app via `cargo run -p <package>`, wait for debug registration, auto-connect |
| `perf_stats` | The app's per-frame performance counters (cascades, Parley shapes, IFC setup passes, Taffy computes, full repaints by reason, repainted px, hit tests, effect runs, phase times) as `last_frame` / `current_frame` / `total` JSON; `reset: true` zeroes them after reading. Use it, not DevTools, to measure an interaction |

**Performance counters** (`rinch_dom::perf`, re-exported as `rinch::perf`). Every
`NodeTree` carries `perf: PerfCounters`: always-compiled `u64` counters bumped by
style, layout, text, paint and hit testing, rolled over per frame by
`RinchApp::end_perf_frame` (desktop: after each redraw; Android: after each
present attempt; embed: at each `update`).
`RINCH_PERF=1` prints one line per frame (the env var is read once), `perf_stats`
serves them over MCP, and `RinchApp::last_frame_perf` / `doc.tree.perf.frame()`
serve tests. **Prove a perf fix with a counter, not a timing**:
`crates/rinch-dom/tests/perf_counter_baselines.rs` asserts every counter of the
frame **exactly** per scenario (hover with/without a `display: contents`
wrapper, append, remove, resize, text edit, transform tick), so an increment
that stops counting fails too; a fix updates the numbers it moves, and that
edit is its proof.
The cost side (instruction counts in CI) is under **Performance regressions**
below.
Guide: `docs/src/guide/performance.md`.

**Discovery mechanism:** Each debug-enabled app writes `~/.rinch/debug/{pid}.json` containing its port, app name, and PID. The MCP server scans this directory to find running apps and auto-connects when only one is running.

**IPC protocol:** Length-prefixed JSON over TCP on localhost. 4-byte big-endian length prefix followed by JSON payload. Handshake exchanges protocol version on connect. **The app's debug server takes one client at a time**: while the MCP server (or any client) is connected, a second connection's handshake times out. A script driving the wire protocol directly must run with the MCP disconnected (`disconnect`), and the MCP reconnects with `connect` afterwards.

**Key source files:**
- `crates/rinch-debug/src/server.rs` - TCP listener (blocking I/O, no tokio dependency)
- `crates/rinch-debug/src/protocol.rs` - Wire protocol types and framing
- `crates/rinch-mcp-server/src/mcp_server.rs` - MCP tool implementations
- `crates/rinch/src/app/debug_commands.rs` - `execute_debug_command()`, the debug-command dispatch; `shell/rinch_runtime.rs` only calls it

### File Dialogs (optional)

Enable with `features = ["file-dialogs"]`. The cross-platform picker that
hands back `LocalFile`s on the web too is `rinch::file::FilePicker` (#1771);
its mechanism is under **File Drop** in `pointer-drag-tooltips.md`.

```rust
use rinch::dialogs::{open_file, save_file, pick_folder, message};

// Open file
if let Some(path) = open_file().add_filter("Text", &["txt"]).pick_file() { }

// Save file
if let Some(path) = save_file().set_file_name("doc.txt").save() { }

// Pick folder
if let Some(path) = pick_folder().pick() { }

// Message dialog
message("Success!").set_title("Info").show();
```

### Clipboard (optional)

Enable with `features = ["clipboard"]`:

```rust
use rinch::clipboard::{copy_text, paste_text, has_text};

copy_text("Hello").unwrap();
if has_text() {
    let text = paste_text().unwrap();
}
```

**Reading is slow and may block (#149).** A read is a request to whichever app owns
the clipboard; on X11 arboard waits up to **4s** for a hung owner, and the browser
can't be read synchronously at all. Every read comes in three shapes on every
platform: `paste_text()` (blocks, unbounded), `paste_text_timeout(Duration)` (blocks,
`Err(ClipboardError::TimedOut)` after that), `paste_text_async(cb)` (never blocks).
Same three for `paste_html`/`paste_image`. `paste_rich`/`paste_rich_async` resolve
`text/html` → bitmap → `text/plain` in **one** read (→ `RichPaste`), so a rich paste
never stacks three worst-case waits. `paste_rich_with_text*` answer the same plus the
`text/plain` offered beside an html answer, still one read (the editor's Ctrl+V uses it
so its paste hook sees both flavours). `copy_text_async`/`copy_html_async` queue a write
without waiting and drop its result; `copy_text_then`/`copy_html_then` (#1780) queue it
and call back with it — on the web, the settled `navigator.clipboard.write(Text)`
promise (rejection → `AccessFailed("<name>: <message>")`, no `navigator.clipboard` →
`NotAvailable` synchronously). The web's synchronous `copy_text`/`copy_html` still
answer `Ok` for the in-app buffer whatever the browser does. Web tests reach a
deterministic refusal through the doc-hidden `__set_clipboard_source` (the clipboard
of an `<iframe allow="clipboard-write 'none'">` in Chrome; in Firefox, which ignores
that policy, the same frame detached, whose write rejects with `InvalidStateError`); the
html copy falls back to `writeText(alt)` when its `ClipboardItem` write cannot be made or
is refused; every write promise is awaited, so a refused fire-and-forget copy is not an
uncaught rejection; the accepted write needs a user
activation in headless Chrome, so it is a trusted click from `real_keys.mjs`
(`rinch-web/tests/clipboard_copy_then.rs`, run by `run_real_keys.sh`).

On native all of them are served by **one clipboard worker thread** owning the
`arboard::Clipboard` (`crates/rinch-clipboard/src/native.rs`). That's what makes the
timeout useful: giving up doesn't cancel the request, so an abandoned read finishes on
the worker rather than wedging later callers behind a held lock. The worker talks to a
private `Backend` trait, so the queue/timeout/probe logic is unit-tested against a fake
clipboard with no display.

**Async callbacks are `Send` and do NOT run on the UI thread** (native: the worker;
web: the browser thread). Marshal before touching UI state — `Signal::send` for a
`Send` payload, or `park_main_callback` + `run_on_main_thread` + `resume_main_callback`
for a `!Send` continuation (an `EditorHandle`, an `Rc`). A *blocking* call made from
inside an async callback is reported as an error rather than deadlocking.

**Web (#150):** `paste_text()` answers from a buffer that rinch-web fills from the
document's `paste` ClipboardEvent — the only synchronous channel to content copied in
another app or tab — so a web app can paste from outside itself.
`paste_text_async` additionally tries `navigator.clipboard.readText()`. Because the
browser's `paste` event arrives *after* the keydown, web paste logic should hang off
`rinch_core::set_paste_interceptor(|data: &PasteEventData| -> bool)` (dispatched
**after** the buffers are filled, so `paste_text()` works inside it) rather than off an
intercepted Ctrl+V, which is still `prevent_default()`ed. One slot per document plus a
thread-global fallback, like `set_keyboard_interceptor` (#340/#478); desktop never
dispatches it (no OS paste event).

**Android: a long press on a text field shows the platform's floating toolbar**
(issue #813) — `ActionMode.TYPE_FLOATING`, started by `RinchActivity` on the
window's decor view (the 1x1 `RinchInputView` cannot host one: the framework
hides the toolbar unless the content rect intersects the originating view).
The shell sets `TextContextMenuPresentation::Shell`, so the runtime prepares
the caret and selection and hands back `AppAction::ShowTextContextMenu`; the
shell selects the word under the finger (`RinchApp::select_word_at_caret`, a
press inside a selection keeps it), shows the items `text_edit_state()` allows,
and performs a tapped item through `perform_text_edit` — the chord's own code.
Item taps, `RinchInputConnection.performContextMenuAction` and every dismissal
arrive through one queue in `rinch_android::text_action`, and
`ToolbarMirror` there is the loop's belief about the toolbar, with a count of
the finishes it asked for so a late report for a finished mode cannot take
down the one that replaced it. Two things that were silently wrong before and
are worth knowing: **a still finger never fired the long press** on an idle
loop, because K37's looper sleep has no deadline and nothing woke it
(`TouchGesture::long_press_due` now feeds `poll_timeout`); and **the `android`
feature now implies `clipboard`** — without it `handle_paste` has nothing to
read, so a hardware Ctrl+V inserted nothing and an IME's
`performContextMenuAction(paste)` returned `true` having pasted nothing (the
toolbar hides its Paste). Paste is also hidden while `hasPrimaryClip()` is
false, which reads no clip and raises no clipboard-access notice (measured),
and is asked **once per long press**, not per refresh: it is a binder round
trip (0.56 ms median, 10.7 ms p99 on the emulator) and the refresh runs every
turn the toolbar is up. **An item that finishes the toolbar is finished on
the Java side before it is reported** (PR #819 review, F1): both calls queue
an event and wake the loop, and reported first, a lone `Perform` turn
re-prepared a toolbar the loop believed was up and the UI thread started a
fresh one — an orphan whose items acted on nothing. **And a refresh never
starts a toolbar** (final review, N1): a refresh posted *before* the tap
reaches the UI thread after the item finished the mode, so
`ToolbarMirror::request` answers `Start | Update | Nothing`, the per-turn
refresh (run only while the mirror is shown) is always an `Update`, and
`showTextActionMode`'s `start` flag returns rather than starting a mode for
one — measured 11 of 11 orphans under an anchor moving every frame before
it. **A `Perform` carries its
source**, because Cut / Copy / Paste end the toolbar from either one (as in an
`EditText`, measured) but only a toolbar item has already finished it:
`ToolbarMirror::performing` takes the mirror down uncounted for a toolbar item
and asks for a counted finish for an IME request — treating the IME's like the
item's left an orphan no tap could take down, measured. Measured on an API 34
emulator with one Gboard build: its clipboard chip and clipboard panel both
paste as `commitText`, and its Text Editing panel's Paste and Select all
arrive as `performContextMenuAction` (its Cut and Copy stay disabled over a
rinch selection, which the input connection does not report to the IME).
The toolbar floats over the selection itself: `TextEditState.anchor` is the
selection's rect (a 1px caret rect when collapsed), scaled to physical px. The
rich-text `Editor` is
`desktop`-only and is not part of an Android build (#818).

**The built-in editor's Ctrl+V is asynchronous** — see the `anchor_selection` row in the
`EditorHandle` table. **So is the plain `<input>`/`<textarea>` paste** (#328), and the
chord, the context menu's Paste and Android's toolbar (`perform_text_edit`) all share it:
`RinchApp::handle_paste` starts `paste_text_async` and returns, answering whether a read
started (a read-only field starts none, as a read-only editor does). Its completion needs
`&mut RinchApp` (`handle_input_edit_command`), which no main-thread callback has, so it
rides the app's **deferred-work inbox** (`app/deferred_work.rs`): an `AppWorkSender`
(cheap, `Send`, holds the inbox weakly — work for a dropped app is dropped with its
captures) pushes a `Send` closure taking `&mut RinchApp` from any thread and wakes the host
with an empty `run_on_main_thread`; `RinchApp::run_deferred_work` runs the inbox in send
order and answers how many ran. **Every host calls it right after
`drain_main_callbacks`** — desktop's `proxy_wake_up` (main app and DevTools), the Android
loop, `RinchContext::update` — and repaints when it answers non-zero; a host that drives a
`RinchApp` itself and never calls it gets no plain-control paste. The completion
(`complete_input_paste`) lands in the field's **current** selection if the same field still
holds the keyboard in the same focus gesture — `RinchApp::focus_epoch`, bumped by every
`set_focus_target` transition (not by the two unmounted-editor self-heals, which clear only an
`Editor` target), so away-and-back also drops it — and is dropped otherwise, the
browser's rule. The rest is the ordinary edit path's: a field gone read-only refuses it, a
disabled one loses the keyboard (#315), a value written meanwhile is adopted first (#238),
paste plus its `oninput` rewrite is one undo step (#288), and a composition started during
the read keeps going after the pasted text. On Android the read answers inline, so the
paste lands later in the same loop iteration. Pins: `app/input_paste_async_tests.rs`,
`rinch/tests/embed_input_paste.rs` (the embed host step). Desktop's and Android's host
calls have no fixture.

**A single-line field folds the line breaks of what is inserted into it** (#824): every
`InsertText`/`Paste` that reaches `handle_input_edit_command` (a paste, an IME commit, a
key's text) for a field that is not a `<textarea>` goes through
`RinchApp::insertable_text` → `rinch_editable::single_line_insert`, Chrome 155's rule for
text inserted into an `<input>` (measured with `execCommand('insertText')`, which runs the
same insertion filter a paste does): line breaks at the end are dropped, every other
`\r\n`, `\r` or `\n` is one space (`"a\nb"` → `"a b"`, `"a\n"` → `"a"`). What folds to
nothing still replaces a selection and is no edit at a caret. Android's soft keyboard folds a
whole committed text before `apply_ime_action` splits it into one key per character. A
`<textarea>` keeps its line breaks. Pins: `app/single_line_insert_824_tests.rs`, the
`#824` section of `app/input_paste_async_tests.rs`.

**A programmatic `<input>` value is sanitized, not folded** (#1554): an `<input>`'s value
is its `value` attribute run through HTML's value sanitization for its type
(`rinch_dom::form_control::input_value`, reached through `form_control::control_value`,
which paint, focus, the adoption of a programmatic write, the `onchange` payload,
`live_value` and `:placeholder-shown` all read). Chrome 155's rule: the Text state (no
`type`, `text`, an unknown type), `search`, `tel` and `password` **strip** every `\r` and
`\n` (`"p\nq\r\nr"` → `"pqr"`); `url` and `email` strip them and trim ASCII whitespace,
a `multiple` email per address. `number`, the date/time family, `color` and `range` have
rules of their own that are not applied (their value is answered as written); `hidden`,
`checkbox`, `radio`, `file` and the buttons have none. The **attribute keeps what was
written**, as in a browser: `sync_input_cursor_to_dom` writes the engine text into
`value` only when the attribute does not already read as it, so focusing a field holding
`"a\nb"` leaves the attribute alone and the first edit that changes the text writes it
(`"abX"`). A `value_fn` binding writing `"a\nb"` sees `live_value` `"ab"` and writes the
same attribute again, which is an identical write and changes nothing. Pins:
`rinch-dom/tests/input_value_sanitization_1554_tests.rs` (Chrome 155's table),
`app/input_value_sanitization_1554_tests.rs` (nothing painted below the box).

### System Tray (optional)

Enable with `features = ["system-tray"]`. Uses the same unified `Menu`/`MenuItem` types as native menus:

```rust
use rinch::prelude::*;
use rinch::tray::TrayIconBuilder;
use rinch::menu::{Menu, MenuItem};

let menu = Menu::new()
    .item(MenuItem::new("Show").on_click(show_current_window))
    .separator()
    .item(MenuItem::new("Quit").on_click(close_current_window));

let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_png(include_bytes!("../assets/icon.png"))?
    .with_menu(menu)
    .build()?;
```

**Key features:**
- `with_icon_png(data)` — Load tray icon from PNG bytes (use `include_bytes!`)
- `with_icon_rgba(rgba, w, h)` — Load from raw RGBA pixel data
- `with_icon_path(path)` — Load from a PNG file path
- **Push-based events** — Menu callbacks fire via `MenuEvent::set_event_handler` on the main thread. No polling thread, no wasted CPU.
- **Left-click** — runs `TrayIconBuilder::on_activate(cb)` if given, else
  `show_current_window` (the old hard-wired behaviour, kept as the default;
  #1520). The callback is registered in the menu registry under a
  `tray-activate-{n}` id, so it obeys #183's owner rule; ksni's `activate` and
  the global `tray-icon` click handler (which maps the clicked tray's id to its
  activate id, `ACTIVATE_IDS`) both dispatch it on the main thread.
- **Linux `Title` is stable** (#1536): `with_title(..)`, else the tray id; `set_tooltip` changes only `ToolTip` (Plasma sorts by Title). `tray-icon` has no title concept, so Windows/macOS ignore `with_title`.
- **Runtime changes** (#1520): `TrayIcon::set_icon(&TrayIconImage)` (decode
  once, swap cheaply; `set_icon_png`/`set_icon_rgba` too), `set_tooltip`,
  `set_menu(Option<Menu>)`, all `&self` and **never blocking the caller**. On
  Linux they go through `KsniUpdater`, a `rinch-tray-update` thread that applies
  a **coalesced** `KsniPending` (newest icon/tooltip/menu, latest state per
  check key) with `Handle::update`, which blocks — that thread, never the
  caller's — until the service emits; a hung watcher holds one pending set, not
  a queue. Elsewhere `tray-icon`'s setters run on the main thread. `set_menu`
  releases the old menu's `MenuRegistration` (callbacks and watchers) at once,
  as a rebuilt bar does.
- **Tray-only apps**: `WindowProps::visible: false` (#1520; the field was
  ignored before). `create_window` makes the window with `with_visible(false)`
  and no renderer, mounts the root, then `hide_window`s it — no frame is
  presented (X11: created unmapped, destroyed). `can_create_surfaces` creates a
  window only while no document is mounted, so the root is never mounted twice;
  `show_current_window` re-creates the OS window over the same document.
  `examples/tray-only`. **On macOS `visible: false` still leaves a Dock icon**:
  pair it with `App::activation_policy(ActivationPolicy::Accessory)` (#1528,
  `shell/activation.rs`, handed to winit's
  `EventLoopBuilderExtMacOS::with_activation_policy` when the event loop is
  built). Unset (the default) leaves it to winit: `Regular` unbundled, the
  `Info.plist`'s `LSUIElement` in a bundle. A no-op on Windows and Linux.
  `visible: false` does not imply `Accessory`, deliberately.
- **The item id is the app's own** (#1529): ksni's `Tray::id` was the constant
  `"rinch-app"` for every app, so a panel that keys per-item settings on the
  `Id` treated all rinch apps as one. It is now `tray_item_id`:
  `TrayIconBuilder::with_id(..)`, else the `WindowProps::app_id` the running
  app declared (`rinch_runtime::declared_app_id`, recorded when the event loop
  starts — so only for a tray built after `App::run` began), else the
  executable's file name via `resolved_app_id`, the Wayland `app_id`'s
  derivation. An app declaring an `app_id` and building its tray in `main`
  passes the same string to `with_id`. `with_id` is unused off Linux: Windows
  keys a notification icon by executable + `uID` (tray-icon sets no GUID) and
  macOS a status item by the app, and tray-icon's `TrayIconId` is an
  in-process counter, so neither had the shared-constant problem. Pin:
  `tray::tests::the_tray_item_id_is_the_apps_own`.
- **Dropping the `TrayIcon` removes the icon, then releases its callbacks** (#377).
  On Linux, ksni's own `Handle` does not stop the service when dropped (its loop
  ignores a closed request channel), so `TrayIcon`'s `Drop` sends ksni's
  `Shutdown` and waits up to `SHUTDOWN_WAIT` (1 s; ~10 ms on a healthy
  session) for a helper thread to report that the D-Bus connection closed. The
  wait runs off the caller's thread because ksni's `wait` is a `block_on`, which
  panics inside a tokio runtime, and it is **bounded** because ksni reads the
  request only between D-Bus calls: a watcher that stops answering leaves it in
  a call with no timeout, and an unbounded wait froze the main thread for good
  (review of #1054). The callbacks go only after the close: before #377 the icon
  stayed on the panel for the life of the process, and once #183 released the
  callbacks on drop, every item on it was dead. **On a timeout the callbacks
  are kept, not released** — parked in the thread-local `PARKED` list and
  released by the next tray build or drop on the thread once the helper reports
  or disconnects — because an icon that may still be shown should keep working
  items; the cost is their memory meanwhile. Pinned by
  `tray::tests::dropping_the_tray_shuts_the_service_down_before_releasing_the_callbacks`,
  `a_shutdown_that_outlasts_the_bound_keeps_the_callbacks_until_it_closes`, and
  two `#[ignore]`d live ones: `live_a_dropped_tray_leaves_the_status_notifier_watcher`
  (session bus) and `live_dropping_a_tray_under_a_hung_watcher_is_bounded`
  (private bus running `crates/rinch/tests/fixtures/hung_sni_watcher.py`).
  **`build()` is bounded too** (#1057): ksni's `spawn()` `block_on`s a setup
  ending in `RegisterStatusNotifierItem`, also untimed, so a hung watcher held
  `build()` — the app's startup — forever. It now runs through `run_bounded`
  and gives up after `BUILD_WAIT` (5 s) with `TrayError::CreateFailed` (no new
  variant: `TrayError` is exhaustive, so one would break downstream matches).
  A setup that finishes after the give-up goes to `on_late`, which shuts the
  late service down, because the callbacks were released with the error
  (`spawn_bounded`). Each timed-out build leaves its setup thread and D-Bus
  connection waiting on the hung watcher (the setup cannot be cancelled), so an
  app must not retry `build()` in a tight loop.
  Pinned by `a_bounded_run_*`,
  `a_tray_service_that_arrives_after_the_build_gave_up_is_shut_down`, and the `#[ignore]`d
  `live_building_a_tray_under_a_hung_watcher_is_bounded` (fixture run with
  `--hang-first`).
- **`TrayIcon` is `!Send`/`!Sync`** on every platform, deliberately: its
  `MenuRegistration` holds `Rc`s (and `tray-icon`'s own handle off Linux holds
  one too) because release must run on the thread whose
  thread-local `MENU_CALLBACKS` holds the entries. On Linux it was `Send` until
  #183, so a `static OnceLock<TrayIcon>` stopped compiling there. Keep it in a
  local of `main` or a `thread_local!`.
- **A disabled item registers no callback** on either path: `build_muda_item`
  returns early, and the ksni conversion mints no `ksni-{N}` id for one.

**Minimize-to-tray pattern:** Combine system tray with `on_close_requested` + `hide_current_window()`:

```rust
let window_props = WindowProps {
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false // Don't exit, just hide
    })),
    ..Default::default()
};
```

### Notifications (optional)

Enable with `features = ["notifications"]` (off by default, no windowing needed; issue #1522). `rinch::notification::Notification::new(title).body(..).icon_png(..).on_click(..).on_dismiss(|DismissReason| ..).on_error(|&NotificationError| ..).show() -> Result<NotificationHandle, NotificationError>`. Backends, thin and on crates already in Cargo.lock (no package added): Linux `org.freedesktop.Notifications` over zbus in a current-thread tokio runtime on a worker thread (`notification/backend/freedesktop.rs`; the #1521 reasoning for tokio), Windows WinRT toasts on a worker thread (`windows` 0.62), macOS `UNUserNotificationCenter` through raw `objc2` messages + `block2` (needs an `.app` bundle: unbundled, `show` is `Err(Unsupported)`). The feature does not build for wasm32 (the workspace tokio has `rt-multi-thread`). `show()` does no IPC on the calling thread; what only the platform can refuse (no server, permission, a failed toast) arrives through `on_error` on the main thread, else a `tracing::warn!`. Callbacks follow the #183 owner rule (owner captured when each `on_*` is called), are delivered through `dispatch_main_callback` into a thread-local registry keyed by rinch's id, and a click ends the notification (no `on_dismiss` after it). Identity: `set_app_id` > the window's `app_id` (Linux: `shell::rinch_runtime::current_app_id`) > the executable's name (no `.exe`); Linux sends it as the `desktop-entry` hint and writes no file, Windows uses it as the AUMID and registers `HKCU\Software\Classes\AppUserModelId\<id>` (`DisplayName` only, never over an existing key; `set_register_app(false)` opts out; no Start-menu shortcut; no COM activator, so a toast clicked after exit does nothing). Linux mappings are keyed by (server unique name, id): a server restart ends the old ones (`on_dismiss(Unknown)`) and a reused id never reaches them. The icon file goes in a private 0700 directory (`$XDG_RUNTIME_DIR`, else the cache dir), opened `create_new`. Pins: `notification/tests.rs` (the Linux backend against a private `dbus-daemon` with a fake server). Guide: `docs/src/guide/platform.md#notifications`.

### Global Hotkeys (optional)

Enable with `features = ["global-hotkeys"]` (implies `desktop`; off by default, adds no package to `Cargo.lock` — `hotkey::tests::the_backends_add_no_package_to_the_lock`). `rinch::hotkey::register(spec, |ev: &HotkeyEvent| …) -> Result<Hotkey, HotkeyError>` gives **press and release** of a system-wide shortcut on the main thread (one `batch` each, through `rinch_core::dispatch_main_callback`); `Hotkey::rebind`, drop unregisters, and a hotkey registered during a render dies with its component (the #183 menu rule: owner captured at registration, `on_cleanup` unregisters, a dead owner's report is dropped). Shortcut strings read their key through the menu's table (`menu::key_spelling_code`, shared — `every_menu_key_spelling_names_the_same_key_here` pins it) and add sided modifiers (`RightCtrl`), modifier-only bindings (last token a modifier), `Super` distinct from Ctrl, and any W3C `code`. `HotkeyCapture` turns the app's own key events into a shortcut for a settings field. Backends (`crates/rinch/src/hotkey/backend/`): Windows = a `WH_KEYBOARD_LL` hook whose proc only `PostThreadMessageW`s to its own pump thread (`LowLevelHooksTimeout`), plus `RegisterHotKey` to reserve (and swallow) plain chords and report `ERROR_HOTKEY_ALREADY_REGISTERED` as `Conflict`; each key-down first drops keys `GetAsyncKeyState` says are up (`Matcher::key_reconciled`) and a 100 ms poll runs while any key is believed down, so a key-up the hook missed (UIPI) neither strands a binding nor refuses the next chord. macOS = Carbon `RegisterEventHotKey` for plain chords (no permission), a listen-only `CGEventTap` started only for a modifier-only or sided binding (Input Monitoring; `permission()` / `request_permission()`); the tap reconciles modifiers against each event's flag bits and does not feed Caps Lock. Lock keys never count as a held key. X11 = `XGrabKey` to reserve chords (`BadAccess` → `Conflict`) + XInput2 raw key events for press/release; while our own grab is active the server sends core `KeyPress`/`KeyRelease` instead, which feed the same `Matcher`, with XKB detectable auto-repeat on (else a held chord was ~25 Released/Pressed pairs a second). Wayland = the GlobalShortcuts portal over raw zbus on a private thread inside a current-thread tokio runtime (not `ashpd`, which refuses to build with both its runtime features, and the workspace has one crate on each; zbus builds with both and then needs a tokio runtime, which the runtime gives either way). The app first names itself with `org.freedesktop.host.portal.Registry.Register` (KDE refuses an app with no id; the id needs an installed `.desktop` entry; `hotkey::set_app_id`, default the window's app id). When Register answers "App info not found", rinch writes a `NoDisplay=true` stub to `$XDG_DATA_HOME/applications/<id>.desktop` (`shell::rinch_runtime::ensure_desktop_entry`: skipped if a system or user entry exists, `create_new` so nothing is overwritten) and retries for up to 2 s; `hotkey::set_write_desktop_entry(false)` opts out. Combining `system-tray` and `file-dialogs` already puts zbus in tokio mode under rfd (#1526). The user binds in a system dialog, so `register` answers `Binding::Pending` and the outcome arrives as `Binding::SystemChosen`/`Declined` through `on_binding_change`. One session for the backend's life; commands queued in one turn are one BindShortcuts; a portal that will not re-bind gets a new session after everything held is released; a rebind opens ConfigureShortcuts on v2. A rebind bumps the entry's generation, which rides in the backend key (`backend_key`, bits 40+), so a report queued for the old binding is dropped. `HotkeyEvent::chorded` marks a `Released` after another key went down during the hold (toggle apps ignore it). Guide: `docs/src/guide/platform.md#global-hotkeys`. Example: `examples/global-hotkey-demo`.

### Theme System (optional)

Enable with `features = ["theme"]` for the theme system inspired by Mantine.

Theme is configured at the runtime level using `ThemeProviderProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { style: "color: var(--rinch-primary-color);",
            "Uses theme CSS variables"
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    App::new(app)
        .title("Themed App")
        .size(800, 600)
        .theme(theme)
        .run();
}
```

ThemeProvider generates CSS variables:
- Colors: `--rinch-color-{name}-{0-9}`, `--rinch-primary-color`
- Spacing: `--rinch-spacing-{xs,sm,md,lg,xl}`
- Radius: `--rinch-radius-{xs,sm,md,lg,xl}`, `--rinch-radius-default`
- Shadows: `--rinch-shadow-{xs,sm,md,lg,xl}`
- Typography: `--rinch-font-size-{xs,sm,md,lg,xl}`, `--rinch-font-family`
- Semantic: `--rinch-color-body`, `--rinch-color-text`, `--rinch-color-dimmed`

**Generic font families are repaired on Android** (`crates/rinch-dom/src/fonts.rs`,
issue #322). fontique maps the `monospace` slot to the literal name `"monospace"`,
which `/system/fonts` is not indexed under, so the slot is empty and `<code>`,
`<pre>` and the whole default `font_family_monospace` stack render proportional.
Build font contexts with `rinch_dom::fonts::new_font_context()` — never
`parley::FontContext::new()` — so every context resolves a stack identically;
the document's, the hit-test one, and the input hit-test one must agree glyph for
glyph or a tap lands on the wrong character. The repair fills `monospace`,
`ui-monospace`, `ui-sans-serif` and `ui-serif` only where the platform left them
empty; `ui-rounded` and `fangsong` are deliberately left alone (Android has no
face for either).

**A font stack that resolves to nothing ends in the primary `sans-serif` face**
(`rinch_dom::fonts::parley_font_family`, #1198). parley queries a cluster with
the Unicode `Emoji` property — the ASCII digits, `#` and `*` have it, as keycap
bases — against the stack and then the `emoji` generic, *ahead of* the script
fallback. So in a stack whose every family is missing (`font-family:
Helvetica` on Linux, which fontique cannot alias to Nimbus Sans as Chrome's
fontconfig does, or any typo), digits were drawn from Noto Color Emoji at
1.245em while the letters took the script fallback. **Every** parley layout
rinch builds takes its family from `parley_font_family` (twelve sites across
`rinch-dom` and `rinch`; `missing_family_fallback_tests::no_source_builds_a_parley_font_family_by_hand`
fails on a new site that builds a `FontFamily::Source` itself): an empty stack is
`sans-serif`, and a stack that resolves to no family *in that font context* gets
the slot's first family appended **by name** — the face, not the generic,
because the generic expands to the platform's whole list, whose text faces
(DejaVu Sans, FreeSans) cover emoji and would win them ahead of the `emoji`
generic. Where no app font claims `sans-serif` that face is where the letters
already fell back to; with a claim (`AppFont::sans_serif`) the letters of such a
stack move to the claimed face too. A stack that resolves to anything is untouched, emoji included, and a
context with no `sans-serif` face (wasm/embed before any font is registered)
appends nothing. **Accepted consequence:** on a host whose primary sans is
DejaVu Sans, an emoji in a missing-only stack is DejaVu's monochrome glyph,
because the appended face is *named* and a named family that covers an emoji
draws it (below). Cached per thread by (primary `sans-serif` family id, stack).

**An emoji-presentation cluster counts each generic as its primary face**
(`rinch_dom::fonts::TextFamily`, #1204). parley shapes an `Emoji`-property
cluster against the stack and then the `emoji` generic, and a generic expands to
the platform's whole list (about 180 families on a Linux desktop), so DejaVu Sans
or FreeSans won U+1F600 / U+2B1C under `sans-serif` and under the theme's
`DEFAULT_FONT_FAMILY`. Now a cluster whose base has the `Emoji` property and
that holds U+FE0F, or one holding an `Emoji_Presentation` character and no
U+FE0E (icu grapheme clusters, `fonts::emoji_presentation_ranges`), gets a span
whose stack is the computed one with every generic but `emoji` replaced by its
slot's first family, then the `emoji` generic, then the original generics as
coverage (without that tail, 𝄞️ and Thai ก️ drew `.notdef` on this host, and so
did U+2B1C in an embed context whose emoji-capable app face was second in
`sans-serif` — review of #1270). Chrome 153
measured on this host: a generic is one face, a named family keeps its place
(`'DejaVu Sans'` draws U+1F600 in DejaVu), and a named emoji face after the
generic wins (the theme's Segoe UI Emoji). Text-default `Emoji` characters
(digits, `#`, ©, U+2764 alone) keep the stack as written, so #1198 is
untouched. Every ranged builder pushes its families through
`parley_text_family(..).push_to(&mut builder)`
(`emoji_presentation_family_tests::no_source_pushes_a_font_family_by_hand`); the
IFC's tree builder wraps each emoji range of a text op in a span of
`parley_emoji_font_family` (`IfcText::finish`). Every IFC text is shaped in the
root's family. A cluster split across two text nodes (an emoji and its U+FE0F)
is not found. The fixtures use bundled stand-ins, not the host's emoji face.

**Window chrome inset (not ThemeProvider-generated).** `--rinch-window-top-inset`
is published at runtime by whatever chrome rinch draws above your content — the
DOM menu bar (28px, on Linux and in the browser alike: `render_with_menu_bar`
publishes it on its own wrapper *and* on `scope.body_handle()`) and the
`BorderlessWindow` titlebar (36px, +28 when
the menu bar sits below it). That chrome reserves space with in-document padding,
so normal flow content clears it automatically; a `position: fixed` element does
**not**, because fixed resolves against the real viewport (matching browsers and
rinch-web). Full-height overlays must therefore opt in:

```rust
div { style: "position: fixed; top: var(--rinch-window-top-inset, 0px); bottom: 0; left: 0; right: 0;" }
```

The `left: 0; right: 0` is not decoration: a fixed box with both insets of an
axis `auto` keeps its static position on that axis (#633), so `top`/`bottom`
alone leaves its x wherever its parent put it.

`Drawer`, `Modal`, and the top-anchored `Notification` positions already do this.
`DropdownMenu`'s and `Select`'s click-catching backdrops, and the DOM menu
bar's dismiss overlay (`.rinch-app-menu-bar__overlay`), are fixed at
`top: 0` and deliberately do **not** take the inset: a dismiss region has to
cover the chrome, or clicking the title bar leaves the menu open. Do **not**
"fix" an overlay of your own by insetting the fixed containing block — that
would break CSS semantics and make desktop diverge from rinch-web.

### Cross-target platform calls (#1773)

- **Message dialogs on the web.** `crates/rinch/src/dialogs.rs` is two modules:
  `native` (rfd, every dialog) and `web` (wasm32: `message` /
  `MessageDialogBuilder` / `MessageLevel` over `window.alert` /
  `window.confirm`). `rfd` is a `cfg(not(target_arch = "wasm32"))` dependency,
  so `file-dialogs` on wasm links no crate, and the module is compiled on
  wasm32 with or without that feature (`lib.rs`:
  `cfg(any(feature = "file-dialogs", target_arch = "wasm32"))`, #1821). Pin
  (built without `file-dialogs`, rinch-web's dev-dependency):
  `rinch-web/tests/confirm_dialog_1773.rs` (stubs `window.confirm`/`alert`; a
  real one would block headless Chrome).
- **`rinch::init_logging()`** (`crates/rinch/src/logging.rs`): desktop (and
  the `android` feature on a non-Android host) = tracing-subscriber `fmt` to
  stderr, `RUST_LOG` else the given filter, `log` bridged by tracing-log;
  Android = `android_logger::init_once`, no tracing-subscriber (its `LogTracer`
  would take the `log` slot logcat needs); wasm32 and a native build without
  the desktop shell = `logging::simple`, a hand-written `tracing::Subscriber`
  plus `log::Log` (filter: a level and `target=level` directives, longest
  target wins) writing to the console method of the event's level on wasm32
  and to stderr natively, and on wasm32 a panic hook (message + JS stack to
  `console.error`, then the previous hook, so a test harness's still runs).
  Hand-written so that a web app's lock gains nothing: tracing-subscriber on
  wasm brought eight packages into every web app's dependency tree (measured
  on `examples/menu-bar-web/Cargo.lock`). `log` is a plain rinch dependency
  (already in every lock through `tracing`); `log::set_logger` with a leaked
  box, since `set_boxed_logger` needs `log`'s `std` feature. The web panic hook
  has no fixture: a wasm32 panic aborts, so a test cannot survive one.
  With a `log` logger already set, tracing-subscriber's `try_init` sets the
  subscriber first and then fails at `LogTracer` (`util.rs` in 0.3.22), so
  `tracing` events reach rinch's output and `log` records stay with the app's
  logger; `simple::install` sets the two slots independently, same result.
  Pinned natively by child processes in `rinch/tests/init_logging_1773.rs`
  (`log_records_and_tracing_events_both_reach_stderr`,
  `an_app_logger_set_first_keeps_the_log_records`).
- **`rinch_storage::open_app_store`** (`crates/rinch-storage/src/app_store.rs`):
  native `dirs::data_dir()/<app>/<store>` (Android: always `Unavailable`,
  since `dirs` reads it as Linux and answers a path the app cannot write;
  names are checked against Windows' rule on every target, since `C:` joined
  onto the data dir replaces it and `app:x` names an NTFS stream), web the
  IndexedDB database
  `"<app>/<store>"`. One database per store because `IdbStore::open` creates
  its object store only in `onupgradeneeded`, which an existing database never
  fires: a second store name in one database is never created
  (`rinch-storage/tests/idb.rs::a_second_store_in_an_existing_database_is_not_created`).
- **Owner-only `FsStore` files** (#1819, `crates/rinch-storage/src/native.rs`):
  on Unix each `put`'s temp file is opened `create_new` with mode `0600`
  (`OpenOptionsExt::mode`, so the mode is there from creation and the umask
  can only narrow it), then renamed into place, so a file written with another
  mode is replaced by a `0600` one at its next `put`; `FsStore::open` creates
  missing directories with `DirBuilder` mode `0700` and leaves an existing
  base alone; `open_app_store` `chmod`s its own store directory to `0700`
  (`native::restrict_dir`), which covers files not written since. Windows:
  nothing set, the parent's inherited ACL applies. Pin, its own binary because
  it sets the process umask to 0: `rinch-storage/tests/owner_only_1819.rs`.

### Audio DSP output (#1963)

`rinch_av::audio::open_dsp_output` (`crates/rinch-av/src/audio/dsp.rs`, web half `dsp_web.rs`) plays a
`rinch_dsp::DspProcessor` on the audio thread. User guide: `docs/src/guide/audio-dsp.md`.

- **`rinch-dsp` has zero dependencies** on purpose. On the web the processor is a raw `cdylib`
  instantiated in an AudioWorklet with an empty import object, and anything that imports from the
  page (wasm-bindgen glue) breaks it. `export_dsp!` gives the module its C ABI. The exports are
  `unsafe extern "C"` over one `static Slot`. `Slot::host` is a private `unsafe fn`, so nothing safe
  reaches the cell: the `unsafe impl Sync` holds only under the exports' one-thread contract (the
  PR as opened had safe exports and a safe `host`, a data race natively).
- **ABI version**: `rinch_dsp::ABI_VERSION`, exported as `rinch_dsp_abi`. The worklet script has
  it substituted in (`{ABI}` in `dsp_web::PROCESSOR`) and refuses any other value. Bump it on any
  change to the exports' signatures or meaning, and rebuild `crates/rinch-av/tests/fixtures/dsp_echo.wasm`
  (the recipe is in the fixture crate's `src/lib.rs`).
- **Native callback** (`native::callback`, device-free and unit-tested): byte rings from `rtrb`
  carry length-framed messages and replies (`push_frame` checks room for header and bytes before
  writing either, so the one consumer never sees half a frame). The audio side reads messages into
  one buffer sized to the ring, and reserves the reply buffer to the reply ring's size. The planar
  buffer is pre-sized from the requested `buffer_size` (4096 frames when the backend chooses). It
  grows only if a device hands a longer buffer. A full reply ring drops and counts
  (`Status::dropped_replies`). The processor's panic is caught (`catch_unwind`, so it does not
  unwind into cpal) and recorded in `Status::error` (a `OnceLock`); the callback is silent after
  that.
- **Web port protocol**, worklet → page: a `Uint8Array` is a reply, `{applied: n}` returns `n`
  bytes of message budget (`Shared::in_flight`), and `{error}` is a failure, sent once. The page
  enforces both budgets: the worklet itself never drops a message. The page checks
  `WebAssembly.Module.imports/exports` before creating the node. The worklet checks the ABI,
  `try`/`catch`es `process`, and the node's `onprocessorerror` is a second route. Its views of
  module memory are cached and re-made when `memory.buffer` changes identity (growth detaches the
  old buffer) or the render pointer moves.
- **Tests**: `dsp_web::tests` renders through an `OfflineAudioContext` (`open_dsp_output_in`, test
  only) and reads samples per channel. `tests/dsp_web.rs` covers order, rate, quanta, every failure
  route (with a hand-assembled module for the ABI-version check) and the queue bounds. These run
  in CI's existing `cargo test -p rinch-av --features audio,camera --target wasm32-unknown-unknown`,
  under the autoplay-allowing `webdriver.json`.
