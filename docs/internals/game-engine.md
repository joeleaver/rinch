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
`shell/rinch_runtime.rs` `pointer_source_tests`, `rinch-web/tests/render_surface_pointer_events.rs`.

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

**Sharing a high-capability GPU device (issue #57):** zero-copy compositing needs your texture on the *same* device rinch composites with (`gpu_handle()` → `device`/`queue`/**`adapter`**). `gpu_handle()` is `None` whenever the window presents with the software renderer (`RINCH_RENDERER=cpu`, or the `Renderer::Auto` fallback), so handle `None` unless you use one of the two entry points below — they always present on the GPU. By default that device is created with `Features::default()` / `Limits::default()`. To raise it:

| Entry point | Ownership | Use when |
|---|---|---|
| `App::new(component).gpu_config(RinchGpuConfig { required_features, required_limits })` | rinch creates the device (surface-compatible adapter) with your extra features/limits | You just need more capability — **recommended**, always presents correctly |
| `App::new(component).external_gpu(ExternalGpu { instance, adapter, device, queue })` | You create the whole stack; rinch makes only the surface, validates present-support, composites onto your device | You must keep your exact `DeviceDescriptor` |

Construct `wgpu` types from **`rinch::wgpu`** (rinch pins a patched fork — a separate `wgpu` dep won't type-match). Both are `#[cfg(feature = "gpu")]`, re-exported in the prelude. Example: `examples/gpu-device-config` (`RINCH_GPU_MODE=external` toggles the two modes).

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
| `RinchContextConfig` | Width, height, scale factor, optional theme |
| `RinchOverlayRenderer` | Convenience Vello-to-texture renderer |
| `GameViewport` | Component marking a transparent hole for game rendering. **Hittable by default** (#207): `wants_mouse` routes input by hit-testing the hole and walking up to `data-viewport`, so an unhittable hole makes the UI claim the mouse *everywhere*. `pointer-events: auto; background: transparent;` come from the UA stylesheet rule for `[data-viewport]` — not an inline style — so restyling the hole can't strip them, and a HUD root's inherited `pointer-events: none` can't reach it. Your own HUD controls under such a root still need `pointer-events: auto`. It also stamps **no** `data-viewport-ready`, and absence means ready, so its hole is unconditional (#186) — see the **Viewport holes** note under Rendering Backends. |
| `LayoutRect` | `{x, y, width, height}` in logical pixels |

**Typical game loop:**
```rust
let mut ctx = RinchContext::new(config, my_ui);
let mut overlay = RinchOverlayRenderer::new(&device, w, h, format);

loop {
    let actions = ctx.update(&events);
    game.render();
    let ui = overlay.render(&device, &queue, ctx.scene());
    composite(game_texture, ui);
}
```

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
- `crates/rinch/src/app/mod.rs` — `viewport_rect()`; `app/focus.rs` — `has_focused_input()`, `has_focused_contenteditable()`, `has_focused_node()`, `has_focused_key_consumer()`, `node_is_focus_visible()` (#548)
- `crates/rinch/src/focus_registry.rs` — `wants_key_routing()`, the `FocusEntry::on_key` check behind `has_focused_key_consumer()`

**Documentation:** `docs/src/guide/game-engine.md`
