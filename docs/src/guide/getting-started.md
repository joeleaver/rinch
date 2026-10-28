# Getting Started

From zero to window in about ninety seconds.

## Prerequisites

- **Stable Rust**, edition 2024. Rinch itself is built and tested with the one
  version its `rust-toolchain.toml` names (1.99.0 today); no nightly feature is
  needed.
- A C/C++ compiler (Stylo has native dependencies)
- On Linux: `libfontconfig-dev` and `pkg-config` (for font discovery)

Building the repository's own examples needs more. `ui-zoo-desktop` and
`video-call` use real-time audio and video (rinch-rtc's `media` feature), so
on Debian or Ubuntu they also need `libvpx-dev` (VP8), `libclang-dev`
(bindgen, for the libvpx and camera bindings) and `libasound2-dev` (audio);
`nasm` is optional (openh264's assembly). rinch-rtc's tests and
`cargo clippy --workspace --all-targets` also need `libssl-dev`, for the
test-only interop peer, which links the system OpenSSL.

## Create a Project

```bash
cargo new my-app
cd my-app
```

## Add Rinch

```toml
# Cargo.toml
[dependencies]
rinch = { git = "https://github.com/joeleaver/rinch.git", features = ["desktop", "components", "theme"] }
```

**What the features do:**
- `"desktop"` — windowing, event loop, software renderer. Required for desktop apps.
- `"components"` — the 60+ component library (Button, TextInput, Modal, etc.).
- `"theme"` — CSS variable generation, color palettes, dark mode. Auto-enabled by `"components"`.
- `"gpu"` — adds GPU rendering via Vello/wgpu beside the software renderer; the window uses the GPU and falls back to software when it will not start (see [Rendering Backends](windows.md#rendering-backends)). Optional.
- `"clipboard"` — clipboard read/write. Optional.
- `"file-dialogs"` — native open/save dialogs. Optional.

**No `[patch]` lines at all.** rinch depends on crates.io wgpu 30, on Vello
0.11 from `joeleaver/vello-fork` and on winit 0.31.0-beta.4 from
`joeleaver/winit-fork`, each fork at a pinned rev, as ordinary dependencies, so
an app copies no patch for any of them. Delete any old `[patch.crates-io]`
lines for winit (`winit`, `winit-core`, `winit-common`, `winit-wayland`,
`winit-x11`, `dpi`) and for wgpu (`wgpu`, `wgpu-core`, `wgpu-hal`,
`wgpu-types`, `naga`): the winit ones now point Cargo at a branch rinch no
longer uses. Use `rinch::wgpu` and `rinch::vello` rather than a `wgpu`/`vello`
dependency of your own, so the versions never drift; add `wgpu = "30"`
yourself only to turn on an extra feature (e.g. `webgpu`), and Cargo unifies
it with rinch's.

## Write Your App

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        Stack { gap: "md", p: "xl",
            Title { order: 1, "Hello, Rinch!" }
            Text { color: "dimmed", "Your first app. It only goes up from here." }

            Group { gap: "sm",
                Button { onclick: move || count.update(|n| *n += 1), "+" }
                Button { variant: "outline", onclick: move || count.set(0), "Reset" }
            }

            Text { size: "xl", {|| format!("Count: {}", count.get())} }
        }
    }
}

fn main() {
    App::new(app)
        .title("My App")
        .size(800, 600)
        .theme(ThemeProviderProps {
            primary_color: Some("cyan".into()),
            ..Default::default()
        })
        .run();
}
```

## Run It

```bash
cargo run --release
```

> **Always use `--release`** for running apps. Debug mode is slow because Stylo and Parley do serious work. Release builds are fast.

You should see a window with a title, description, two buttons, and a count that updates when you click "+".

## What Just Happened

1. `#[component]` injected a `__scope: &mut RenderScope` parameter. You never see it, but `rsx!` needs it.
2. `rsx!` built a DOM tree: created elements, set attributes, wired event handlers.
3. `{|| format!("Count: {}", count.get())}` created an Effect that reads `count` and updates a text node. When `count` changes, that closure re-runs and updates *only that text node*.
4. `App::new(app)…run()` opened a window, loaded theme CSS, mounted the component, and started the event loop. `App` is the one entry point: every startup option — title, size, theme, menu bar, window props, GPU device — is a method on it, and they all compose.

Your `app` function ran exactly once. It will never run again. The closures handle everything from here.

## What's Next

- [RSX Syntax](./rsx-syntax.md) — The full macro syntax: elements, attributes, events, control flow.
- [State Management](./hooks.md) — Signals, Memos, Stores, Context.
- [Components](./components.md) — The full component library.
- [Theming](./theming.md) — Colors, dark mode, CSS variables.
- [Writing Components](./writing-components.md) — Build your own.
- [WASM](./wasm.md) — Run in the browser: the same `App` chain runs there through `rinch_web::AppExt`.
