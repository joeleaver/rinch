# Game engine integration (RenderSurface, embed)

Read this page when your task touches: RenderSurface, GPU device sharing, the embed API, RinchContext.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Game Engine Integration

Two complementary patterns for integrating with game engines and custom renderers:

### RenderSurface (Recommended)

Rinch owns the window. Your renderer submits frames into a `RenderSurface` component. Rinch handles layout, compositing, and event routing.

**Key types** (all re-exported in prelude):

| Type | Purpose |
|------|---------|
| `RenderSurfaceHandle` | Main handle — `writer()`, `gpu_registrar()`, `set_event_handler()`, `set_key_handler()`, `set_pointer_events()`, `set_render_callback()`, `set_continuous()`, `request_frame()`, `layout_rect()` |
| `RenderSurface` | Component — `RenderSurface { surface: Some(handle) }` |
| `SurfaceWriter` | Thread-safe CPU pixel submission (`Send + Sync + Clone`) |
| `GpuTextureRegistrar` | Thread-safe GPU texture registration (`Send + Sync + Clone`) |
| `SurfaceEvent` | Input events dispatched to surface handler (with `set_pointer_events(true)`: `PointerDown/Move/Up/Cancel` carrying a `SurfacePointer { id, kind, pressure, primary }`, and `Pinch`, instead of `MouseDown/Move/Up`) |
| `create_render_surface()` | Factory function |

**Usage:**
```rust
let surface = create_render_surface();
surface.set_event_handler(|event| { /* handle mouse/keyboard */ });

let writer = surface.writer();
std::thread::spawn(move || {
    writer.submit_frame(&pixels, w, h); // CPU pixels
});

// Or for GPU textures:
let registrar = surface.gpu_registrar();
registrar.set_texture_source(wgpu_texture, wgpu_view, w, h);
registrar.notify_frame_ready();

rsx! { RenderSurface { surface: Some(surface), style: "flex: 1;" } }
```

**A press on a surface keeps the pointer** (#1502), on desktop and the web: its moves
and releases go to that surface wherever they are, until the release of the button that
started it (a chord's release ends nothing, #1087). Desktop keeps one capture per pointer
(`RinchApp::surface_captures`, so two fingers each hold one) and routes only the
surface's events: DOM hover and the cursor still follow the pointer. A capture whose
release went missing is ended through the missed-release heal (`release_press_gestures` →
`release_surface_captures`): a window blur and a platform `PointerCancel` end every
capture, a left press only the pressing pointer's (another finger's press proves nothing,
and two fingers holding a surface is a pinch); the surface hears `PointerCancel` or, without
pointer events, a `MouseUp` at the cursor. A surface that is unregistered or leaves the
document drops its capture silently (`prune_surface_captures`). Ctrl+wheel on a surface with
pointer events is a `Pinch`; desktop negates winit's `delta_y` so the direction matches the
browser's `deltaY` (`wheel_zoom_scale` takes the DOM sign). Pointer ids: desktop mouse `1`,
tablet tool `2`, fingers above `1 << 32`; the web passes `pointerId` through; Android and
embed report every pointer as the mouse. Pins: `app/surface_pointer_tests.rs`,
`shell/rinch_runtime/winit_input.rs` `pointer_source_tests`, `rinch-web/tests/render_surface_pointer_events.rs`.

**Pointer lock on a surface** (#2092). `RenderSurfaceHandle::request_pointer_lock`
is the window's lock held by the surface's element (`element`, set by
`RenderSurface::render` — the desktop `<div>`, the web `<canvas>` — and cleared
at its cleanup), with the surface id recorded so `dispatch_pointer_motion` sends
it `SurfaceEvent::PointerMotion` while the lock is granted;
`unregister_render_surface` releases it. An embed host installs a
`PointerLockHost` and forwards `PlatformEvent::PointerMotion`; mechanism and
pins: `app-and-windows.md`, "Pointer lock and raw motion (#2092)".

**Bound elements** (#2219). A surface whose element the app builds (a named
surface's `data-viewport` element, an app canvas) gets one with
`RenderSurfaceHandle::bind_element`. The handle's `element` field is an
`Rc<SurfaceElement>` shared by the clones: `mounted` is the component's element,
and its `Drop` (the last clone gone) forgets the binding and calls
`pointer_lock::surface_dropped` (a `try_with` release, safe in TLS teardown).
Note the registry holds a clone of every registered surface, so a named or
mounted surface's handle "drops" only after `unregister_render_surface`, which
releases anyway. Bindings live in the thread-local `BINDINGS` (surface id,
element, a serial, a `Weak` of the event handler), not in the handle, so a
scoped binding's `on_cleanup` captures only `(id, serial)` and undoes only its
own binding, never a later one. `element()` is the binding, else `mounted`. A scoped re-bind of the same element whose binding is already `owned` returns early, so a handler or effect that binds on every run does not pile cleanups on its owner (a handler or effect runs with its component as owner, so its binding is scoped too).
A binding change ends the surface's lock when `pointer_lock::held_element(id)`
is some other element than the new one (and the other surface's, when the
element is taken from it); `LockOnRebind::Move` calls `pointer_lock::request`
on the new element instead, and releases only if that is refused. A lock's
motion goes through `dispatch_locked_surface_event`: the registry like any
surface event, else the binding's handler, so a handle that was never mounted
still hears it. The element leaving the document is the existing element-lock
path (`pointer_lock::prune` on desktop; the browser on the web). Pins:
`app/surface_bind_element_2219_tests.rs`, `rinch-web/tests/surface_bind_element_2219.rs`.

**A focused surface only swallows the keys it claims (issue #482).** `set_event_handler`
still receives every `KeyDown`/`KeyUp` while the surface is focused — that delivery is
unaffected. `set_key_handler(|key: &SurfaceKeyData| -> bool)` answers a separate, additive
question asked *after*: should this key stop at the surface, or continue past it? Leave it
unset (the default) and the surface claims nothing, so a host's own `window`-level
keybindings (and, on the web, the browser's own shortcuts — reload, find, …) keep working
the moment a canvas takes focus, and on desktop so do DevTools (F12), inspect mode (Alt+I)
and Tab. Return `true` only for a key the surface genuinely uses (WASD steering a
character, Space to jump):

```rust
surface.set_key_handler(|key| matches!(key.code.as_str(), "KeyW" | "KeyA" | "KeyS" | "KeyD" | "Space"));
```

Before this existed, a focused surface swallowed **every** key unconditionally — on the web
via `preventDefault()`/`stopPropagation()` on every `keydown`, on desktop by never falling
through to the same DevTools/inspect/Tab handling an unfocused canvas always had. The
document-level keyboard interceptor (`rinch_core::events::set_keyboard_interceptor`) is
unaffected either way: it already saw every key before a focused surface did, on both
backends, and still does — a host that wants an app-global chord to win regardless of
surface focus should register it there rather than through `set_key_handler`.

**On `rinch-web`, an unclaimed key also keeps the browser's own default action** — no
longer `preventDefault()`ed just because the surface is focused, matching how an unfocused
`<canvas>` already behaves. Space, PageUp/Down and the arrow keys can scroll the page, and
Tab leaves the canvas for the next tab stop. A web game that steers with any of those must
claim them through `set_key_handler`, or the page scrolls under it.

**Render-callback pacing and position (#1768).** `RenderSurfaceHandle::continuous` is an
`Option<bool>`: `None` (never set) keeps each backend's original pacing — desktop runs the
callback in every paint, the web's rAF loop every frame — and `Some(true)` / `Some(false)` are
"every frame" / "on demand" on both. The rule is two pure functions, `draws_this_frame` and
`wants_next_frame`, used by both drivers: desktop's `invoke_render_callbacks_where` (and through
it an embedded context's `scene()`) and the web's `start_raf_loop`. `frame_requested` is taken
(`replace(false)`) just before the callback, so a `request_frame()` made inside it is seen
afterwards: desktop then calls `request_repaint()` once the `IN_RENDER_CALLBACK` guard is down
(inside it the call is suppressed), the web keeps its loop going. A request is not taken while
the surface has no size. `set_and_notify_size` requests a frame for an on-demand surface on every
size change, and `schedule_canvas_init` on every web mount (a new canvas is blank). An embedded
context reports a wanted frame through `render_surface::surfaces_want_frame` in `needs_update`
and as `update`'s `RequestRedraw` — only for a surface with a non-zero size, or a continuous
surface under `display: none` made the host spin at 100% with no callback run (review of
#1790). No size, no frame, everywhere: the web loop also stops at size 0 and
`set_and_notify_size` restarts it at the next size. The desktop paint runs **every** registered
surface's callback, and a surface whose node left the document while its component lived on (a
captured handle hidden by an `if`) kept its last size, so `RinchApp::update_render_surface_layouts`
(`forget_absent: true` on the desktop window only — the registry is per thread, and an embedded
context sharing it would zero another context's surfaces; it runs only its own ids) zeroes the
size of every registered inline surface not in the document, with no resize callback;
`all_surface_layout_rects` also skips a surface node inside a detached subtree. On the web `wake_for_frame` uses `try_borrow` on the callback
cell: inside the callback it is borrowed, and the running loop picks the request up. Position:
an inline `RenderSurface` got only its size before (`update_layout_size_by_id`), so
`layout_rect()` said `(0, 0)` on desktop, in embed and on the web alike; the desktop paints and
`RinchContext::prepare_render_surfaces` now call `update_layout_rect_by_id` with the painted
box, and on wasm `layout_rect()` reads the canvas's `getBoundingClientRect` when called. Pins:
`render_surface::pacing_tests_1768`, `app/render_surface_pacing_1768_tests.rs`,
`tests/embed_render_surface_pacing_1768.rs`,
`rinch-web/tests/render_surface_pacing_1768.rs` (Chrome 155; also passes in Firefox via
Playwright).

**A GPU texture layer composites in an opaque window too (#1774).** With composite layers
present, `WgpuRenderer` renders the Vello UI over a transparent base (`desktop::ui_base_color`)
and the compositor clears the window base under the layers, blits them, then blends the UI on
top. That base used to go transparent only for CPU-pixel layers (`composite_layers`), so a
window whose only layer was a zero-copy GPU texture (`gpu_layers`, a `GpuTextureRegistrar`
surface named by `data-viewport`) rendered its UI over opaque white, which covered the layer;
a transparent window's base is transparent anyway, so it worked there. Pin:
`shell::compositor::gpu_alpha_tests::gpu_alpha_a_gpu_layer_shows_in_an_opaque_window_1774`
(`#[ignore]`d, needs a GPU: `cargo test -p rinch --features gpu --lib -- --ignored gpu_alpha`).

**The GPU render callback (#1768 part 3, desktop): frames drawn in the scene.**
`set_gpu_render_callback` stores a `FnMut(&GpuSurfaceFrame)`. How a surface is drawn is decided in
**one place**, `render_surface::surface_route` → `SurfaceRoute` (`Pixels` | `GpuImage(device)`):
the GPU route needs a GPU callback, a device (`gpu_frame_device`: `renderer::gpu_presenting` and
`gpu_handle()`), and a host that can draw a registered image (`invoke_render_callbacks` from the
desktop window passes `true`; an embedded context's `invoke_render_callbacks_for` passes
`false`, since its host renders the scene with a renderer of its own, and an image registered
with another renderer panics in Vello). A future split-layer fast path (the UI rendered below
and above the surface, the compositor applying clip, radius, opacity and transform) is meant to
be another variant there, behind the same callbacks. `draw_gpu_frame` draws into the surface's
own `gpu_target` (`Rgba8Unorm`, `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC | COPY_DST`,
`Rgba8UnormSrgb` view), never the app's `texture_source` (review of #1861, D3), reallocating
it when the size moves, and sets `gpu_frame_fresh`. `paint_gpu` then takes
`collect_gpu_surface_frames()`, has `WgpuRenderer::sync_surface_images` register each target
with the window's Vello renderer (`register_texture`, kept while the texture is the same one,
dropped otherwise; declared `AlphaPremultiplied`), marks the fresh surfaces' nodes paint-dirty
(their boxes are the frame's damage, so a GPU partial frame #1718 re-renders them), and hands
the images to paint (`rinch_dom::paint::set_surface_gpu_images`). Paint draws them where a CPU
frame is drawn — the `data-render-surface` node's content, `object-fit: contain`, through
`Painter::draw_gpu_image` (a no-op by default; `VelloPainter` fills with the image brush and the
half-pixel brush transform of #1027; `ClipTrackingPainter` forwards it — a wrapper that does not
draws nothing) — so every clip, layer, opacity group and transform around the node applies.
Nothing is a hole or a `data-viewport` any more; the surface is sized like a CPU surface (its
layout box times the transform's axis scales, see review 2 below), which also ended the two-sizes-a-paint under a transform of the layer version
(review D1). **Atlas:** Vello packs every image of a scene into one atlas of at most 8192²
(`VELLO_ATLAS_SIDE`), re-created each frame, and silently drops an image it cannot place.
`gpu_target_size` clamps a target to `min(max_texture_dimension_2d, 8192)` a side and
`gpu_atlas_scale` shrinks all GPU frames of a paint alike when together they pass half the
atlas (`GPU_FRAME_ATLAS_BUDGET`); each warns once. **Video** left the GPU compositor in the same
change: `surface_takes_compositor_path(is_inline, is_video, gpu)` is false for video, so on the
GPU window `collect_viewport_frames_by_name` hands its CPU frames to paint
(`RinchApp::install_gpu_viewport_frames`, with the `GameViewport` hole set) and they are drawn
inline, as on software — black bars and all. The retention of the last layers while a video
was loaded went with it. `GameViewport` and `GpuTextureRegistrar` textures stay compositor
layers. Cost (spike, integrated Radeon 890M, wall time): a 640×360 surface 3–7 ms a frame in
the scene against 1–3 as a layer, full-window 1080p 12 against 2, 4K 15 against 3; 4K video
25 against 6. Pins (`#[ignore]`d, real GPU: `--ignored gpu_`): `gpu_render_callback_frames_are_drawn_as_elements`
(non-clipping ancestor background beneath, rounded clip, ancestor opacity, a later translucent
element above, z-index order, ancestor transform, overflow clip),
`…_frames_scroll_with_their_scroller`, `…_frames_go_to_the_scene`, `…_under_a_transform_has_one_size`,
`…_target_is_clamped_to_the_device_limit`, `…_frames_share_the_atlas`,
`…_never_draws_into_an_app_texture`, `…_frames_are_rounded_by_their_own_radius`,
`…_atlas_share_holds_for_idle_surfaces`, `…_unset_pacing_is_on_demand`,
`a_later_window_presents_with_the_shared_device`, `gpu_video_frames_are_drawn_as_elements`; and
`without_a_gpu_compositor_the_gpu_callback_falls_back_to_the_render_callback` everywhere.
**Review 2 of #1861.** *One device:* `WgpuRenderer::try_new_transparent` used to request a
new instance, adapter and device for every window, and a window re-created by hide/show (a tray
app, a layer-shell output loss) got a second device while `gpu_handle()` kept the first — GPU
frames were drawn on one and registered with a renderer on the other. Now the first window's
instance is kept beside `GPU_HANDLE` (`RINCH_INSTANCE`) and `shared_gpu_stack` hands every later
window that stack (as `App::external_gpu` always did); a shared adapter that cannot present to
the new window is an `Err`, i.e. the software fallback under `Renderer::Auto`. Checked live: a
hide/show in a nested KWin makes two renderers and one instance, and the surface draws after.
*Atlas:* a target whose size differs from the one it has (a new layout size, or the share
moving) is drawn again, idle or not, so the share holds for every surface and a shrunk one
recovers. *Size:* `all_surface_layout_rects` reports the layout size times the transform's axis
scales (not the painted bounding box, which letterboxed a rotated frame). *Radius:* the surface
arm in paint clips its frame to its own `border-radius`, GPU and CPU. *Pacing:* an unset GPU
surface is on demand (`invoke_render_callbacks_where`). The registration is
`desktop::SurfaceImages`, which the GPU tests drive too.

**Review 3 of #1861: the frame is the element's content.** The `data-render-surface` arm that
painted a frame over the whole border box (background square, border never painted) is gone:
the generic element arm paints the surface's background, shadow, border and outline as for any
element, and then `paint_surface_frame` draws the frame — GPU image or CPU pixels — into the
**content box** (`paint::content_box_insets`: border plus padding; a percentage padding counts
as zero there), `contain`-fitted, clipped to the inner radius (`BorderRadii::inner` by those
insets). The target size follows (`all_surface_layout_rects`: content box × the transform's axis
scales), so `layout_size()`, `layout_rect()`'s size and the resize callback report the content
box now, where they reported the border box. Pins: `gpu_render_callback_frames_sit_in_the_content_box`
(rounded background, square and rounded border, padding; GPU and CPU),
`gpu_render_callback_target_is_the_content_box`.

**No device-lost handling.** Nothing in rinch's desktop shell handles a lost GPU device, and with
one shared device for every window a lost device is never replaced: surfaces, Vello and every
window stop presenting until the app restarts (#1788).

**Part 4 notes:** the API is under `cfg(feature = "gpu")`, and `gpu` implies `desktop`, so the web
half needs a feature of its own: widen to `cfg(any(feature = "gpu", all(target_arch = "wasm32",
feature = "web-gpu")))` — an off-by-default `rinch` feature adding `dep:wgpu` (WebGPU only) —
with the canvas's current texture as the target. `SurfaceRoute::GpuImage` holds the desktop
`GpuHandle`; the web arm needs its own variant (or a device abstraction) rather than that type.

**Split-layer fast path (#1865, in progress).** Design record:
`docs/design/split-layer-gpu-surfaces.md`. PR 1 adds the compositor,
`shell/split_compositor.rs` (`SplitCompositor`, not yet wired into the present
path): below around the regions, then per surface `above_out ∘ mix(below,
above_in ∘ (surface · own ∘ below), ancestors)` in one pass with no blend
state, every blend in the shader in f32, the surface sampled with Vello's own
image arithmetic, so a split matches in-scene bit for bit except in the D10
classes. The ancestors' clip coverage is applied once, to the surface and
what is painted over it inside those clips (`above_in`, rendered without
them), as the scene does; applying it to each separately conflated on a
rounded card's ring (#1887 review).
The oracle is `shell/split_compositor_1865_tests.rs` (per-pixel allowances
computed from each case's own renders; `0` everywhere no class names), the
Vello facts it stands on are pinned in `shell/split_slices_1865_tests.rs`
(a registered texture no scene draws is never read; a whole-pixel clip has
binary coverage; the below slice is in-scene but one level near R; an
inverse clip must be non-zero with the hole wound backwards, since even-odd
nested in a clip drops coverage on lavapipe). Both are `#[ignore]`d GPU tests
named `gpu_split_*`; `split_composite_cost_probe` is the cost measurement.

**The split path in the desktop window (#1865 PR 3).** `paint_gpu` enables
splitting (`RinchApp::set_split_enabled`) unless a `GameViewport` layer is
under the UI; `build_gpu_frame_split` paints with `VelloPainter::begin_slicing`
and hands back a `shell::split_frame::SplitChange` (the surfaces whose kept
slices it replaces, and the new slices); `WgpuRenderer::apply_split` renders
the new slices into region textures (`SplitFrame`, each slice rendered where
the window draws it into a scratch target, region kept) and `present`
composites (`SplitFrame::composite`) when anything is split. A fresh frame of
a split surface is not damage (`RinchApp::mark_fresh_surfaces`): the window
composites without repainting. Damage meeting a kept region has the region
added, so the region's slices are replaced whole. A tainted slice repaints the
frame in-scene and suspends splitting for 120 frames. Debug builds log each
surface's placement change (`tracing::debug!`). **Video (PR 4):**
`split_frame::prepare_split_viewports` names the video viewports paint may
split (`set_viewport_split_ids`) and drops a split video's new frame from the
viewport damage, unless its size is not the slice's (a size change is caught
there, where the frame arrives; `stale_ids` covers `RenderSurface` textures).
Paint's video arm offers the frame to `split_surface` after its black
backdrop. After the frame's change is applied,
`SplitFrame::upload_split_frames` uploads the frames of the videos the window
keeps split, and only those (one texture per surface id, straight alpha;
counter `split_frame_uploads`). **Memory cap (PR 5, D6):** a window's split
memory (the scratch target at the size its kept slices need, each slice's
masks and above slices; not the window's own textures, a `RenderSurface`'s,
or the uploaded video frames, which in the scene sit in Vello's atlas:
`split_frame::COUNT_UPLOADS`) may be `SPLIT_MEMORY_CAP_WINDOWS` (4) times
the window's texture bytes (`split_frame::split_bytes`, `over_cap`). After
paint, `RinchApp::gpu_frame_kind` adds this frame's slices in paint order to
what stays split from earlier frames; the first that passes the cap and every
one after it are refused (`VelloPainter::refuse_split`,
`Refusal::MemoryCap`) and the frame is painted again with them in the scene.
They stay refused until the cap grows (a bigger window: a resize that shrinks
it keeps them), a split surface the cap did not refuse goes, or 120 painted
frames pass. A painter that splits what it was told to refuse sends every
surface to the scene, logs an error once and suspends splitting as taint
does. The scratch target is dropped
when it is larger than the kept slices need. Oracle:
`crates/rinch/src/split_runtime_1865_tests.rs` (`gpu_split_runtime_*`, a child of `render_surface`),
the runtime's own steps against an in-scene window.

**Sharing a high-capability GPU device (issue #57):** zero-copy compositing needs your texture on the *same* device rinch composites with (`gpu_handle()` → `device`/`queue`/**`adapter`**). `gpu_handle()` is `None` whenever the window presents with the software renderer (`RINCH_RENDERER=cpu`, or the `Renderer::Auto` fallback), so handle `None` unless you use one of the two entry points below — they always present on the GPU. By default that device is created with `Features::default()` / `Limits::default()`. To raise it:

| Entry point | Ownership | Use when |
|---|---|---|
| `App::new(component).gpu_config(RinchGpuConfig { required_features, required_limits, optional_features })` | rinch creates the device (surface-compatible adapter) with your extra features/limits, and the optional features the adapter has | You just need more capability — **recommended**, always presents correctly |
| `App::new(component).external_gpu(ExternalGpu { instance, adapter, device, queue })` | You create the whole stack; rinch makes only the surface, validates present-support, composites onto your device | You must keep your exact `DeviceDescriptor` |

Construct `wgpu` types from **`rinch::wgpu`** (wgpu 30; a separately pinned `wgpu` of another major won't type-match). Both are `#[cfg(feature = "gpu")]`, re-exported in the prelude. Example: `examples/gpu-device-config` (`RINCH_GPU_MODE=external` toggles the two modes).

**`GameViewport` is the base layer (#1866).** A `GameViewport`'s (and a
`GpuTextureRegistrar` texture's) frame is a layer *under* the UI — the desktop GPU compositor's,
or the embed host's under `RinchOverlayRenderer`'s texture — so the UI must be transparent over
it. Paint erases the viewport's box (its own radius, its clip chain, its transform) when it
reaches the node in paint order, so nothing painted before it shows there — ancestor
backgrounds whether or not they clip, earlier siblings, the window base — and what paints after
it (its border, later siblings, popups, a HUD) covers the game as usual. It replaced the hole
cut into *clipping* ancestors' backgrounds (#974), under which a non-clipping ancestor with a
background covered the game and the documented fix was `overflow: hidden` on it. A debug
build warns once per viewport where the rule cannot hold: an ancestor (or the viewport) with
`opacity < 1`, `mask-image` or a `filter`, whose group is composited over what is beneath it
after the erase, and a rotate/scale/skew transform, whose erase is not the axis-aligned rect the
layer is blitted at. The software window draws the frame in the scene right after the erase,
under the same transform, so only the group warning applies there. The game fills the viewport's **border box** (its border and children paint over it, its
padding shows it; `viewport_rect` reports that box), where a `RenderSurface` frame sits in its
content box. On the GPU window and the embed overlay the viewport is not erased in the scene: it
is split out as a **base** slice and the split compositor replaces what is beneath it inside its
box (#1865's compositor with one term changed); a refused split falls back to the layered path
with an in-scene erase, which is approximate inside clips. `RinchContext::scene()` erases in the
scene too. Mechanism and pins:
`paint-and-damage.md`, "Viewport holes". `RenderSurface` frames are drawn in the scene and erase
nothing.

**Web (canvas viewport, issue #91):** the **same** `RenderSurface` + `create_render_surface()` API works on `rinch-web`, but the model is inverted — the **browser** composites, so rinch only creates and manages a `<canvas>` "viewport hole"; the **app owns the GPU context**. rinch links **no wgpu** on web.

- `handle.canvas_element() -> Option<web_sys::HtmlCanvasElement>` (wasm only) — returns the `<canvas>` after mount (populated in a post-mount microtask; `None` before). Call `wgpu::Instance::create_surface(SurfaceTarget::Canvas(canvas))` on it to render via WebGPU/WebGL. (The lazy 2D-blit CPU path — `SurfaceWriter::submit_frame` — still works and is portable with desktop; it self-disables once you claim a GPU context.)
- **HiDPI out of the box:** `layout_size()` reports **physical** px (`CSS px × devicePixelRatio`, matching desktop), rinch sizes the canvas backing store to match, and `set_resize_callback(|w, h| …)` pushes the new physical size on every resize (ResizeObserver-driven) so you can reconfigure your wgpu surface.
- **Input** (pointer/wheel/keyboard incl. **`KeyUp`**/focus) over the canvas is delivered to `set_event_handler` — rinch does not swallow it. `set_render_callback` drives a `requestAnimationFrame` loop (every frame unless `set_continuous(false)`, which stops it between requested frames).
- **Teardown** is clean: the ResizeObserver and canvas listeners are removed when the component unmounts (no leaks).
- Example: `examples/webgpu-surface-web` (rinch DOM chrome + a WebGPU triangle; `trunk serve` over localhost so `navigator.gpu` is available). Mirrors `examples/game-embed` on desktop.

**Source files:**
- `crates/rinch/src/render_surface.rs` — All RenderSurface types and registry (incl. the wasm32 canvas path: `canvas_element`, `set_resize_callback`, `setup_canvas_events`, `setup_resize_observer`, `WebSurfaceCleanup`)
- `crates/rinch-web/src/event_delegation.rs` — document-level keyboard/focus routing into the surface (`KeyDown`/`KeyUp`/`TextInput`, focus-clear on outside click)
- `crates/rinch/src/shell/desktop.rs` — `GpuHandle`, `RinchGpuConfig`, `ExternalGpu`, `WgpuRenderer::new` device injection

**Optional device features (#2085).** `request_rinch_device` (`shell/desktop.rs`) asks for
`device_features(required, optional, adapter.features(), bgra_surface)`: the required features,
`optional & adapter`, and `BGRA8UNORM_STORAGE` for a `Bgra8Unorm` surface where the adapter has it.
A required feature the adapter lacks is an `Err` naming it, before `request_device` (which used to
report it as a generic failure); a `gpu_config` app panics on it at startup as before.
`GpuHandle::granted_optional_features()` is `device.features() & optional` (empty for
`external_gpu`). The shared paths are untouched: a later window, the DevTools window and a
`RenderSurface` reuse the one device, and `external_gpu` and the embed API never request one. The
web has no rinch-made device (the app requests its own for a canvas), and Android's shell requests
every adapter feature but the experimental ones. Pins: `pacing_and_features_tests` (pure), and
`gpu_optional_features_follow_the_adapter_2085` (`#[ignore]`d, lavapipe in CI: a feature the
adapter has is granted, one it lacks is left out, and the same one required fails, named).
- `crates/rinch/src/app_builder.rs` — the `App` builder (incl. `gpu_config` / `external_gpu`); `crates/rinch/src/shell/mod.rs` holds the deprecated shims

### Embed API

Your game owns the window and wgpu device. Rinch runs headless — you feed it events, it produces a Vello scene.

**Enable it — the module is feature-gated and the gate is not `desktop`.** `rinch::embed` is
`#[cfg(any(feature = "gpu", feature = "embed"))]` (`crates/rinch/src/lib.rs`), so on default
features every type below is a compile error rather than the documented behaviour. Use
`features = ["embed"]` for the headless case this section describes — it pulls in `rinch-dom`,
`parley`, `peniko`, `vello` and `wgpu` without the desktop shell — or nothing extra if you already
build with `"gpu"`.

**Key types** (all in `rinch::embed`, re-exported in prelude):

| Type | Purpose |
|------|---------|
| `RinchContext` | Main handle — `new()`, `update()`, `scene()`. Multiple contexts can coexist on one thread (#134): each holds its own `subscribe_signal_change` guard, and bounds signals / editor registrations / focus requests are scoped per document via `DomDocument::doc_key()`. Stores/contexts are namespaced per context with a thread-global fallback (#136): `create_store` inside a context lands in that context's namespace (cleared on drop), its effects/handlers resolve it first, and lookups fall back to stores created outside any context. |
| `RinchContextConfig` | Width, height, scale factor, optional theme, and `fonts: Vec<FontData>` registered before the mount (owned or `'static`, #2101; font data, so it fills only empty generic slots, #2102). After the mount: `register_font(impl Into<FontData>)`, and `register_app_font(AppFont)` for a face that claims a generic |
| `RinchOverlayRenderer` | Renders a context's UI to a texture: `render(device, queue, &mut ctx)` (since #1866; it took `ctx.scene()` before). Its `GameViewport`s are cut out by the split compositor, exactly; a host rendering `ctx.scene()` with its own Vello gets an in-scene erase, approximate inside clips. A renderer that does not hold the context's last sliced frame (new after a device loss, or another renderer or context took a turn) has the frame painted again: the stamp check below (#1976) |
| `OverlayCheck` | From `RinchOverlayRenderer::check_frames(device)`, never waiting (#2071). `render` calls it, so a host rendering every frame can ignore it; an event-driven host must call it, because a lost frame is the renderer's and `needs_update` belongs to the context, of which several renderers may take turns. `needs_render` stays true until a `render`; `check_again_at` until Vello's counters are in. On a lost frame `fed` goes to 0, which repaints the slices. Pin: `gpu_an_event_driven_host_gets_a_lost_overlay_frame_back_2071` |
| `GameViewport` | Component marking a transparent hole for game rendering. **Hittable by default** (#207): `wants_mouse` routes input by hit-testing the hole and walking up to `data-viewport`, so an unhittable hole makes the UI claim the mouse *everywhere*. `pointer-events: auto; background: transparent;` come from the UA stylesheet rule for `[data-viewport]` — not an inline style — so restyling the hole can't strip them, and a HUD root's inherited `pointer-events: none` can't reach it. Your own HUD controls under such a root still need `pointer-events: auto`. It also stamps **no** `data-viewport-ready`, and absence means ready, so its hole is unconditional (#186) — see the **Viewport holes** note under Rendering Backends. |
| `LayoutRect` | `{x, y, width, height}` in logical pixels |

**Host textures** (#2082, `crates/rinch/src/host_texture.rs`). `RinchContext::register_texture(_with)` stores the host's `wgpu::Texture` (a clone of the handle), its `HostTextureOptions` and its route (`Direct`: `Rgba8Unorm` + `COPY_SRC`; `Convert(view format)`: the other three 8-bit formats, or `Rgba8Unorm` without `COPY_SRC`, which need `TEXTURE_BINDING`) in `HostTextures`, by name, with a version bumped on every registration and `texture_changed`. Each Vello renderer keeps a `HostTextureImages` (the overlay renderer's own, or one the host keeps beside its Vello for `scene_with_textures`), which keeps **each context's registrations apart** (`per_context`, keyed by the context's `HostTextures::token`, an `Arc<()>` held as a `Weak`; a dropped context's images are unregistered at the next sync), so one renderer drawing several contexts in turn (#1976) registers nothing again. The context records which `HostTextureImages` its scene was painted with (`painted_with`, a per-value id) and repaints for another one, since the cached scene names that renderer's images. Before the paint, `RinchContext::install_host_textures` syncs it — a name whose texture has the same size, alpha and route keeps its `ImageData` (a swapped texture is pointed at with `vello::Renderer::override_image`, the same blob id, so the cached scene stays valid; a converted one is converted again into the same target), anything else is re-registered, a gone name unregistered, and any of those `mark_scene_dirty`s the context (one repaint); an `EveryRender` texture, or one whose version moved, is marked dirty for Vello's atlas (`mark_override_image_dirty`) or converted (a texel-for-texel `textureLoad` pass into an `Rgba8Unorm` texture that allows an `Rgba8UnormSrgb` view, written through that view for an sRGB source so the bytes survive the decode). The map goes to rinch-dom's `HOST_TEXTURE_IMAGES` (`paint::set_host_texture_images`) for the paint and is cleared after; `paint_host_texture` draws a `data-host-texture` element's image in its content box by its `object-fit` (initial `fill`), clipped by the inner radius, never split (#1865's `split_surface` is not offered it). `scene()` installs nothing, so it draws none. Cost with no host texture: two `is_empty` checks a render. Pins: `crates/rinch/tests/embed_host_texture_2082.rs` (5 `#[ignore]`d GPU tests: draw-into / swap / resize / `OnChange` / unregister with `paint_frames` counts; the converted formats and both alphas on four-quadrant textures sampled at eight points; refusals and the `scene_with_textures` route; `HostTexture`'s children over the texture; one overlay renderer taking turns on two contexts with a constant `host_texture_registrations`, and two Vello renderers taking turns on one context).

**Typical game loop:**
```rust
let mut ctx = RinchContext::new(config, my_ui);
let mut overlay = RinchOverlayRenderer::new(&device, w, h, format);

loop {
    let actions = ctx.update(&events);
    game.render();
    let ui = overlay.render(&device, &queue, &mut ctx);
    composite(game_texture, ui);
}
```

**Overlay renderer ↔ context frame stamps (#1976).** The overlay's slices live in the renderer
(`SplitFrame`), the scene in the context, and `RinchContext::sliced_frame` paints only when the
scene is dirty, so a renderer that missed the context's last sliced frame would draw the cached
scene with nothing it split out: everything painted after a `GameViewport` missing. Each sliced
frame a context paints takes a stamp from one process-wide counter (`NEXT_SLICED_STAMP`, so a
stamp names one frame of one context; `0` is "none"); the renderer keeps the stamp of the slices
it holds (`fed`) and passes it in, and a mismatch calls `request_repaint` first. That covers a new
renderer (a host's device loss: the host used to call `mark_scene_dirty` itself), two renderers taking
turns on one context, and one renderer on several contexts. A failed `SplitFrame::apply` clears
the split and resets `fed`, so the next frame asks again. Pins:
`gpu_a_new_overlay_renderer_on_an_existing_context_draws_the_hud_1976` and
`gpu_overlay_renderers_taking_turns_draw_their_contexts_current_frame_1976` in
`crates/rinch/tests/embed_game_viewport_base_layer_1866.rs` (the second kills "repaint only a
never-fed renderer" and "a per-context counter").

**`update()` runs the desktop loop's per-frame housekeeping** (#172, #141, #331):
the cross-thread queue, `drain_polls`, the deferred-work inbox, the events, the
**caret blink** (the one timed wake — `RinchContext::next_wake()` hands an
event-driven host the instant desktop would arm `WaitUntil` with; the clock is
one per **document**, keyed by `doc_key` — `rinch-editor-view/src/blink.rs`,
#1149 — so two contexts, or a desktop window beside a context, each blink their
own focused editor; a clock is held only for a registered editor), layout, then `AboutToWait`
(transitions, animations, images, the editor overlay pass, focus requests).
`scene()` drives the inline `RenderSurface`s **in its own document** as
desktop's paint does — physical layout size, render callbacks, frames painted
inline — and skips the pass (a document walk) while the thread has no inline
surface registered, so an unchanged `scene()` stays a cache hit. The compositor path
(named `data-viewport` surfaces, video, a `GpuTextureRegistrar` texture) is the
host's to composite.

**The host drives its own input method** (#1147): `RinchContext::ime_state()`
is the focus arbiter's `ImeState { enabled, cursor_area }` — on for a focused
`<input>`/`<textarea>`, a non-read-only editor or an `on_ime` target, off
otherwise and after a host-fed `WindowFocus(false)`; the area is the painted
caret as an `ElementBounds` in the context's **logical** px — and
`take_ime_change()` answers the `ImeChange::{Enable, Update, Disable}` that
takes the host from what it last reported to that state, through the
crate-private `ImeState::change_to`, the decision the desktop shell's
`sync_ime` applies to winit. `ImeState`, `ImeChange` and its two struct
variants are `#[non_exhaustive]` (a purpose hint or surrounding text can be
added later). Composition comes back in through
`update` as `PlatformEvent::Ime`. A `wants_keyboard()` node that takes no text
wants no IME. Pins: `rinch/tests/embed_ime_state_1147.rs`.

**`wants_mouse`/`wants_keyboard` decide HUD input routing, and `wants_keyboard`
does not mean "any node is focused"** (#548). A `<button>`/`<a href>` is
focusable with no `tabindex` (issue #252's tag-implied set), so a plain mouse
click on one claims `FocusTarget::Node` exactly like Tabbing onto a custom
widget — but `wants_keyboard()` used to OR in `has_focused_node()`
unconditionally, so clicking *any* button made it answer `true` until the next
click landed off a button, and a host following the documented pattern above
("route keyboard to rinch while `wants_keyboard()`") silently stopped seeing
its own Esc/hotkeys. `wants_keyboard()` now asks
`RinchApp::has_focused_key_consumer()` instead, which follows **how the node
got focus**, not whether anything is registered (the review of #1311 caught a
first cut that gated purely on registration, which made Tab-focus on a plain
button answer `false` exactly like a mouse click — breaking Tab navigation
and Enter/Space activation of ordinary controls under the documented
pattern):

- A **mouse-clicked** plain button/link: `false` — it only ever consumes
  Enter/Space through the runtime's own activation path and does not need
  the host to give up its keyboard just because a click focused it. This is
  #548's actual repro.
- A **Tab-focused** (or programmatic `NodeHandle::focus()`-ed) plain
  button/link: `true` — matching a browser's `:focus-visible` split, read
  from [`Node::is_focus_visible`](rinch_dom::node::Node::is_focus_visible):
  Tab/`request_focus` set it, a mouse press claiming `FocusTarget::Node`
  never does (`claim_press_focus` in `event_dispatch.rs` only clears the
  *previous* node's ring on a press).
- A **registered** custom widget
  ([`FocusEntry::on_key`](crate::focus_registry::FocusEntry::on_key) via
  `register_focus_target` — arrow-key navigation, a shortcut of its own):
  `true` either way, click or Tab — the runtime can't know which keys it
  wants without seeing them.
- An open native **`<select>`** popup (`FocusTarget::Select`): `true`
  unconditionally — a separate, pre-existing gap (neither
  `has_focused_input`/`has_focused_contenteditable` nor, before or after
  #548's first cut, the generic-node check ever covered it) folded into the
  same fix, since its own arrow/Enter/Escape handling needs every key.

`RinchApp::has_focused_node()` keeps its original, broader meaning ("is *any*
generic node focused") for callers that genuinely want that question; it is
the wrong one for input routing.

**Source files:**
- `crates/rinch/src/embed.rs` — `RinchContext`, `RinchOverlayRenderer`, `GameViewport`, `wants_keyboard()`
- `crates/rinch/src/host_texture.rs` — host textures (#2082): `HostTexture`, `HostTextureImages`, the conversion pass
- `crates/rinch/src/app/mod.rs` — `viewport_rect()`; `app/focus.rs` — `has_focused_input()`, `has_focused_contenteditable()`, `has_focused_node()`, `has_focused_key_consumer()`, `node_is_focus_visible()` (#548)
- `crates/rinch/src/focus_registry.rs` — `wants_key_routing()`, the `FocusEntry::on_key` check behind `has_focused_key_consumer()`

**Documentation:** `docs/src/guide/game-engine.md`
