# rinch's own windowing layer (Wayland and Win32)

**Status:** Proposed. Discussed and decided in direction by Joe on 2026-11-13;
nothing is built. No issue exists yet.
**Scope:** the layer that turns an operating system's windows and input into
rinch's: windows and their roles, the event loop, input, IME, scale, outputs,
data transfer and cursors. Rendering (Vello, tiny-skia, softbuffer), menus
(muda), the tray, notifications, global hotkeys, the clipboard crate and
AccessKit are outside it and stay as they are.

Facts below were read at `main` `55b79a46` and `joeleaver/winit-fork`
`6f69d8c4` (the pin on that `main`), and on GitHub and crates.io on
2026-11-13.

## Decision

- rinch ships on **Linux (Wayland and X11) and Windows**. There is no Mac or
  iOS device to develop on. iOS is not supported today but is wanted later.
- **Popups placed against another window, and windows prepared hidden and
  shown on demand, are core features** of rinch, not extras for deskrinch.
- rinch gets **its own windowing backends for Wayland and for Windows
  (Win32)**.
- **winit stays the backend for X11, macOS and later iOS.** "Works through
  winit" is enough for X11.
- The layer starts **inside the rinch workspace**, but is written so it could
  become a crate of its own later: it depends on no rinch crate, and no rinch
  or winit type appears in its API.

## Why

**The winit fork is permanent.** It is upstream's `v0.31.0-beta.4` plus 25
commits (fork PRs #1–#5; 31 files, +2237 −492):

- the `wlr-layer-shell` window role (#1523, #1597, #2413);
- monitor changes and `xdg_output`'s logical rectangle (#2414);
- an `xdg_activation` token tied to the input that asked for it (#2501);
- popup fixes, including a backport of upstream #4819 (#2552);
- two X11 RandR/`Xft.dpi` fixes (#2414) and the X11 duplicate raw events fix
  (#2114);
- crates.io `dpi`, so the graph has one `dpi`.

Upstream does not take these. Its layer-shell PR (rust-windowing/winit#4044)
has been open since 2024-12 with no maintainer direction. 0.31 has been in
beta for a year (beta.1 2025-11, beta.4 2026-10; 0.30.13 is the latest
stable), and each beta has meant a rebase and API breaks: `#[non_exhaustive]`
enums in beta.3, the drag-and-drop rework in beta.3 and beta.4. In Joe's
experience upstream is also unwelcoming to LLM-authored PRs, which is how
rinch is developed, so upstreaming is not a way out.

**Most Wayland-shell work goes into the fork.** Every protocol deskrinch needs
becomes fork code written into winit's abstraction, which is shaped for
toplevel windows. In winit's Wayland window state the layer surface is a third
`WindowType` beside toplevel and popup.

**rinch already goes around winit on every platform it ships on:**
- **X11:** an `x11rb` connection of its own for `WM_HINTS`, skip-taskbar,
  sticky windows, XFixes click-through and the RandR CRTC rectangle.
- **Windows:** a replaced window procedure, because winit rewrites the whole
  extended style on every flag change (`overlay_window.rs`).
- **macOS:** `objc2` for the panel's flags and level.

**Some open issues are crashes in winit's code or in its error path:**
- #2531: the app panics when the compositor goes away. This is every logout,
  for a shell component.
- #2489: a panic in an `unwrap` inside winit-x11 (`GetGeometry`).
- #2434: a RustSec advisory reaches us through winit's Adwaita decorations.

### Alternatives considered (checked 2026-11-13)

| Option | State | Why not |
|---|---|---|
| Keep winit with the fork | 0.31 beta, fork +2.2k lines | The fork grows with every protocol, and every beta means a rebase. |
| tao 0.37.2 | active (2026-10) | Built on GTK 0.18 and gdkwayland on Linux. |
| SDL3 (sdl3-rs 0.22.1) | active | Has `xdg_popup`. Layer shell was declined (libsdl-org/SDL#7262, closed 2023). A C dependency. |
| baseview 0.4.2 | active | Made for audio-plugin windows. |
| glazier | archived 2024-07 | — |
| Zed's gpui (`gpui_linux`) | active | Has layer shell and popups on Wayland, but is tied to gpui. Worth reading, not depending on. |
| Own layer on every platform | — | Duplicates what winit does adequately on X11 and macOS, where we cannot or need not test. |

## Where we start

`crates/rinch-platform` exists, but the desktop shell does not go through it:

- **`RinchRuntime` implements winit's `ApplicationHandler` directly**
  (`shell/rinch_runtime.rs`). `PlatformEventLoop` and `PlatformWaker` have no
  desktop implementation. `PlatformWindow` is implemented by `WinitWindow`
  (`shell/window.rs`, `shell/desktop.rs`).
- **`PlatformEvent` has no window id.** Multi-window routing (#2416:
  `window_event_for`, roles, `open_windows.rs`) happens on winit's
  `WindowId` before translation.
- **winit's key types leak out of the shell.** They appear in `menu/mod.rs`,
  `hotkey/*` (all backends, matcher and shortcut), `app/focus.rs` and their
  tests. winit's `KeyCode` *is* `keyboard_types::Code`, so most of this is a
  change of import path. `winit::keyboard::Key` (Character / Named / Dead /
  Unidentified) needs a type of our own or `keyboard_types::Key`.
- **The rest of winit is in the shell.** Files mentioning it, with line counts:
  `rinch_runtime/winit_input.rs` (702), `overlay_window.rs` (1621),
  `layer_shell.rs` (1247), `rinch_runtime.rs` (1403), `outputs.rs` (505),
  `rinch_runtime/main_window.rs` (1803), `window_geometry.rs` (1417),
  `os_file_drag.rs` (797), and a handful of small ones.

Android (`shell/android_runtime.rs`, about 2.7k lines) and the web already run
without winit, so the shell has been written against a second platform model
before.

## Architecture

```
rinch (app, event_dispatch, focus, menus, hotkeys)
   │  rinch-platform: PlatformEvent + window id, PlatformWindow, waker
   ▼
shell/  ── one adapter per backend ───────────────────────────────┐
   │                         │                                    │
   ▼                         ▼                                    ▼
<windowing crate>         winit (+ slim fork)               android / web
 ├─ wayland  (sctk)        X11, macOS, iOS                  (unchanged)
 └─ win32    (windows-sys)
```

**One backend per process, chosen at start-up.** On Linux, `WAYLAND_DISPLAY`
picks the Wayland backend, and `DISPLAY` without it (or `App::prefer_x11`)
picks winit. On Windows it is the Win32 backend. During the move, an
environment variable and an `App` option force winit, so a regression has a
same-day way out.

**The crate's API.** The working name is `rinch-windowing`; the name is open.
Its API is its own and kept small:

- **Event loop:** callback-driven, like winit's `ApplicationHandler`, so the
  shell has one way of running under either backend. It provides a
  thread-safe `Waker`, timers (a wake at an instant), and an exit that returns
  an error instead of panicking.
- **Surfaces, each with a role:**
  - `Toplevel`: title, app id, decorations, min/max size, resizable,
    minimized and maximized, fullscreen.
  - `Popup { parent, positioner, grab: Option<InputCause> }`: the positioner
    gives the anchor rectangle, anchor, gravity, offset and constraint
    adjustment, as in `xdg_positioner`.
  - `Layer { layer, anchors, margins, exclusive_zone, keyboard, output,
    namespace, compositor_size }`.
  - Overlay hints on any role: non-activating, click-through, skip-taskbar,
    level, all workspaces.
- **Capabilities are asked, not assumed.** A role or hint the backend cannot
  give is a typed `Unsupported` at creation (the rule #2552 settled), never a
  silent downgrade. The one exception is the layer-to-toplevel fallback that
  exists today, which keeps its warning.
- **Events per surface:** configure (size and scale), close requested,
  focus, occlusion and visibility, redraw, popup dismissed, layer surface
  closed by the compositor.
- **Input events:** pointer (mouse, touch, pen and tablet as one stream with
  a source), wheel (lines or pixels), keys as `keyboard_types` `Code` and
  `Key` with text and repeat, modifiers, IME (preedit, commit, delete
  surrounding), raw pointer motion for pointer lock, and drag-and-drop data
  transfer.
- **Outputs:** list and change events, with name, logical rectangle, scale,
  refresh and transform.
- **Activation:** asking for a token (tied to the input that caused it) and
  activating with one.
- **Cursors:** named shapes, and custom RGBA images.
- **Surfaces for renderers:** `raw-window-handle` 0.6, which wgpu and
  softbuffer take. muda (`init_for_hwnd`), the tray and AccessKit get the
  native handle in the same way.

**Types:** the crate uses `dpi` (as now, one crate in the graph),
`keyboard-types` and `raw-window-handle`. No `rinch-*` type, no `winit` type.

### Rules that come from designing against both Wayland and Win32

The two are close to opposites, and an API written from one of them breaks on
the other.

1. **Placement is a request, and the result is an event.** On Wayland the
   compositor decides where a window is, a toplevel cannot place itself, and a
   size comes in a configure. On Win32 the app sets both directly. The API
   takes requests (a position, a size, a positioner) and reports what happened
   in events on both backends, even though Win32 could answer at once. The
   shell already works this way for layer surfaces, where `WindowEvent::Moved`
   confirms a move (#1597).
2. **A popup is described by its positioner on both.** The Win32 backend
   computes the placement itself (flip and slide against the monitor's work
   area) and creates an owned `WS_POPUP` window. The constraint logic is
   ours, and it is tested against the same cases as Wayland's.
3. **Focus and activation are policy on Wayland and calls on Win32.**
   Non-activating is a layer surface (or nothing) on Wayland, and
   `WS_EX_NOACTIVATE` plus `WM_MOUSEACTIVATE` on Win32. The API states intent
   (non-activating, activate with a token), never mechanism.
4. **The handler can be re-entered and must not block.** Win32 calls the
   window procedure from inside `DispatchMessage`, from `SendMessage` and from
   modal loops (moving and sizing, menus, OLE drag and drop). The backend
   queues the events it can defer, and delivers redraws from inside the
   move/size loop (`WM_ENTERSIZEMOVE`/`WM_TIMER`), so a resize repaints. On
   Wayland we drain the queue ourselves. The shell's handler must not assume
   it is called from the top of the loop.
5. **Scale belongs to each surface, and can change at any time.** On Wayland
   it comes from `wp_fractional_scale_v1` with `wp_viewporter`, falling back
   to integer `wl_surface.preferred_buffer_scale` / `wl_output.scale`. On
   Win32 it is per-monitor DPI v2, with the rectangle `WM_DPICHANGED`
   suggests.
6. **Losing the display is an error the app gets back,** not a panic: a
   `Broken pipe` on Wayland, `WM_ENDSESSION` on Windows (#2531).

## The Wayland backend

Built on **smithay-client-toolkit 0.21** (active; winit's Wayland backend uses
it too), `wayland-client` 0.31 and `wayland-protocols`.

| Need | Protocol | Fallback |
|---|---|---|
| Windows | `xdg_shell` (`xdg_toplevel`, `xdg_popup`, `xdg_positioner`) | — (required) |
| Shell surfaces | `zwlr_layer_shell_v1` (versions as now: `set_layer` v2, on-demand keyboard v4) | xdg-toplevel with a warning (GNOME) |
| Decorations | `zxdg_decoration_manager_v1` for server-side | client-side frame (below) |
| Scale | `wp_fractional_scale_v1` + `wp_viewporter` | integer buffer scale |
| IME | `zwp_text_input_v3` | none |
| Activation | `xdg_activation_v1` | none |
| Cursors | `wp_cursor_shape_v1` | themed cursors through sctk |
| Pointer lock | `zwp_pointer_constraints_v1` + `zwp_relative_pointer_v1` | — |
| Outputs | `wl_output` v4 names + `zxdg_output_manager_v1` | `wl_output` geometry |
| Drag and drop, file drop | `wl_data_device` | — |
| Frame pacing | `wl_surface.frame` callbacks; `wp_presentation` later | redraw on request |
| Idle inhibit (games, video) | `zwp_idle_inhibit_manager_v1` | — |

- **Keyboard:** sctk's seat keyboard with xkbcommon (keymap, compose and dead
  keys, repeat from `wl_keyboard.repeat_info`, with our own timer).
- **Client-side decorations:** start with **sctk-adwaita**, as winit does
  today, so GNOME looks the same on day one. Drawing the frame as rinch DOM
  (`BorderlessWindow` already exists) is a later option; see open questions.
- **Clipboard:** stays in `rinch-clipboard` (arboard with
  `wayland-data-control`) at first. With a connection of our own, a
  `wl_data_device` selection on it is possible later; it would also cover
  GNOME, which has no data-control.
- **What becomes simpler than today:**
  - Layer surfaces are a role, not a third `WindowType` patched into winit.
  - A layer surface closed by the compositor is an event. Today
    `layer_surface_closed` re-creates the window from `CloseRequested`.
  - Popups parented to layer surfaces need no backport.
  - `monitors_changed` is ours.

## The Win32 backend

Built on **windows-sys** (already a dependency), plus `windows` where COM is
needed (OLE drag and drop, later TSF).

- **The event loop:** a message pump with a message-only window as the
  waker's target (`PostMessage`), and waits by `MsgWaitForMultipleObjectsEx`
  with a high-resolution waitable timer. Modal move/size loops keep painting
  (rule 4).
- **Window classes and styles are ours.** Overlay hints become creation
  flags: `WS_EX_NOACTIVATE`, `WS_EX_TOOLWINDOW`, `WS_EX_TOPMOST`, and an
  empty hit-test region for click-through. The window-procedure replacement
  in `overlay_window.rs` goes away.
- **Borderless windows and resize handles:** `WM_NCCALCSIZE` and
  `WM_NCHITTEST`, so the system does the moving and resizing, with snap.
- **Transparency (#89):** `WS_EX_NOREDIRECTIONBITMAP` with a DirectComposition
  swap chain for wgpu, or `DwmExtendFrameIntoClientArea`. Settled in the
  Win32 stage.
- **DPI:** per-monitor-aware v2 through the manifest or
  `SetProcessDpiAwarenessContext`, and `WM_DPICHANGED`.
- **Keyboard:**
  - `WM_KEYDOWN`/`WM_SYSKEYDOWN` with scan codes mapped to `Code`, and text
    from `ToUnicodeEx`, with dead keys handled.
  - AltGr is told apart from Ctrl+Alt (the fake `VK_LCONTROL` that comes
    before `VK_RMENU`; the cause of #934 and #1291).
- **IME:** IMM32 first (`WM_IME_STARTCOMPOSITION`/`COMPOSITION`,
  `ImmSetCompositionWindow`/`ImmSetCandidateWindow` at the caret). TSF later,
  if an input method needs it.
- **Pointer:** `WM_POINTER*` for touch and pen, mouse messages for the mouse,
  `WM_INPUT` raw input for pointer lock.
- **Drag and drop:** an OLE `IDropTarget` (needs `OleInitialize` on the main
  thread).
- **Outputs:** `EnumDisplayMonitors`, `GetMonitorInfoW`, and
  `WM_DISPLAYCHANGE` / `WM_SETTINGCHANGE` for changes. This closes #2453 for
  Windows.
- **Popups:** owned `WS_POPUP` windows. The positioner is computed against
  `rcWork`, the grab is `SetCapture`, and the popup is dismissed by
  `WM_ACTIVATEAPP` and by a press outside the chain.
- **Dark title bars:** `DwmSetWindowAttribute` (`DWMWA_USE_IMMERSIVE_DARK_MODE`).
- **Menus, tray and AccessKit** are unchanged: muda takes the `HWND`, and
  `accesskit_windows` is added when Windows accessibility is wanted (rinch
  uses only `accesskit_unix` today).

## What stays on winit

- **X11, macOS, iOS.** Their behaviour is unchanged. The `x11rb` and `objc2`
  side channels stay where they are.
- **The fork shrinks.** Once the Wayland backend is the default, the
  layer-shell, activation, popup and `xdg_output` commits are dropped. What is
  left is the X11 fixes (#2114, the RandR/`Xft.dpi` part of #2414), the
  crates.io `dpi` commit, and a fix for #2489's `unwrap`. That is a small set
  to rebase on each upstream release.
- **When winit's Wayland path is removed:** after one release in which our
  backend is the default and the forced fallback has not been needed. Until
  then the fork's Wayland commits stay, frozen.

## Stages

Each stage lands in batches like any other work, and keeps winit working and
selectable.

0. **Make the seam real, inside rinch, with no behaviour change.**
   - `RinchRuntime` consumes `rinch-platform` events that carry a window id,
     and a winit adapter produces them.
   - `menu`, `hotkey` and `focus` take `keyboard_types` types (or our own),
     not winit's.
   - `rinch-platform` gains the role, positioner and output types the new
     crate will speak.
   - Done when no `winit::` path is left outside the winit adapter, and the
     perf baselines are unchanged.
1. **The new crate, with Wayland toplevels.** Event loop, waker, timers,
   `xdg_toplevel`, pointer, keyboard, scale, cursors, surfaces for both
   renderers, server-side decorations. Opt-in. Done when ui-zoo-desktop runs
   on it under KWin and sway with both renderers.
2. **Wayland to parity, then the default.** IME, drag and drop and file drop,
   activation, outputs, pointer lock, layer surfaces, popups,
   several windows (#2416), hidden and prewarmed windows (#2577, #2633),
   client-side decorations for GNOME, losing the display (#2531). Done when
   deskrinch, Pimble, playweft and postrinch run on it, and the existing
   Wayland live tests (`scripts/isolated-session.sh`, the nested-KWin output
   tour, the layer-drag tests) pass. Then it becomes the default on Wayland.
3. **Win32 toplevels.** Event loop, windows, input, DPI, both renderers, menus
   through muda. Opt-in. Done when ui-zoo-desktop runs on NICO-TOP.
4. **Win32 to parity, then the default.** IME, drag and drop, popups,
   overlays and levels, borderless windows with resize, transparency, outputs,
   pointer lock, several windows, hidden windows. Done when the same apps that
   ship on Windows run on it.
5. **Shrink the fork** (above).

Popups (#2552) are in flight on winit now. The rinch side of that work
(`open_popup`, the `Popup` element, dismissal rules) does not depend on the
backend, so it carries over. Only the fork's popup commits are temporary.

## Testing

- **Wayland:** the harnesses we have (nested `kwin_wayland --virtual`,
  headless sway, `scripts/isolated-session.sh`) run each stage's live tests.
  The backend's own unit tests use a fake compositor where sctk allows it,
  and pure functions for placement and configure decisions, as `layer_shell`
  and `overlay_window` are tested today.
- **Windows has no CI today:** every workflow runs on `ubuntu-24.04`. Stage 3
  adds at least a cross-compiled `cargo check` and `clippy` for
  `x86_64-pc-windows-msvc` (cargo-xwin, as beardictate's cross-build does).
  Live tests run on NICO-TOP, which is shared with Oxidegen and Playweft and
  short of disk. Windows behaviour will be tested much less than Wayland's.
  This is the largest risk in the plan; see below.
- **The perf baselines stay the contract.** A frame through the new backend
  does the same rinch-side work, so the counters do not move.

## Size and risk

| Part | Estimate |
|---|---|
| Stage 0 (seam, inside rinch) | 2–4k lines changed, mostly moved |
| Crate API and shared code | 2–3k lines |
| Wayland backend | 6–9k lines |
| Win32 backend | 7–10k lines |

For scale, winit's own backends at the pin are about 11k lines for Wayland
(`winit-wayland`), 13k for Win32 (`winit-win32`), 12k for X11, and 11k shared
(`winit-core`, `winit-common`). Ours serve one framework and freeze no public
API, so they can be smaller.

**Risks:**
- **Few users find the bugs.** winit has thousands of apps on hardware we do
  not own; this layer will have rinch's handful. IME, unusual keyboard layouts
  and Windows setups (several monitors with mixed DPI, remote desktop, ARM)
  are where bugs will hide. Mitigation: keep the winit fallback selectable
  until a stage has shipped, and write the platform rules down in
  `docs/internals/` as we learn them.
- **Windows is under-tested** (above).
- **Client-side decorations on GNOME:** sctk-adwaita keeps today's look, but
  carries the #2434 advisory until we draw our own frame.
- **Time taken from other work.** Stages 0–2 alone are a campaign the size of
  the split-layer GPU work (#1865).

## Open questions

1. The crate's name, and whether it lives in `crates/` from the start (it
   should, to keep the "separable" rule honest).
2. Client-side decorations: sctk-adwaita for good, or a rinch-drawn frame
   (which every app would then get on GNOME)?
3. The clipboard: keep arboard, or move the Wayland selection onto our own
   connection (and so cover GNOME)?
4. Do the popups in #2552 finish on the fork first (as planned), or wait for
   stage 2? The plan above assumes they finish on the fork.
5. Windows on ARM: a target or not?
6. Windows accessibility (`accesskit_windows`): in stage 4, or later?
7. #2489 (X11 `unwrap` panic): fix in the fork now; it stays on winit
   regardless.

## References

- rust-windowing/winit#4044 (layer shell, open since 2024-12)
- libsdl-org/SDL#7262 (layer shell, closed 2023)
- Zed: `crates/gpui_linux/src/linux/wayland/{layer_shell,popup}.rs`
- rinch: #1523, #1597, #1962, #2114, #2413, #2414, #2416, #2434, #2453,
  #2489, #2501, #2531, #2552, #2577, #2633, #89
- `docs/internals/app-and-windows.md`: "winit Fork", "Overlay windows",
  "Layer surfaces of the app's own", "Outputs", "Pointer lock and raw motion"
