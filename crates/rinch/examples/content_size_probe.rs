//! A live check of `WindowProps::size_to_content` (#2552): run inside a
//! nested compositor in an isolated session. It prints what each window was
//! given after each change of its content, and exits by itself.
//!
//! - `MODE=open` (default): a windowless app opens a notification (an
//!   `overlay` layer surface in the top-right corner, at most 360 wide) and a
//!   bar (a `top` layer surface across the top, its width the compositor's
//!   and its height its content's). Their content grows in ten steps
//!   (`NOTE <n> <size>`, `BAR <n> <size>`); then the notification's text goes
//!   past its maximum (`LONG <size>`, wrapped at 360); then it is hidden,
//!   changed and shown (`SHOWN <size>`); then, with two outputs, it is moved
//!   to the other one (`MOVED <size>`).
//! - `MODE=main`: the main window itself, created visible, is a content-sized
//!   layer surface at the bottom of its output (`MAIN <n> <size>`).
//! - `MODE=max`: a notification with no maximum but its output, given a line
//!   far wider than the output (`MAX <size>`, with the output's size).

use rinch::prelude::*;

const STEP_MS: u32 = 400;

fn words(n: u32) -> String {
    (0..n)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_size(label: &'static str, n: u32, handle: Option<WindowHandle>) {
    let size = handle.and_then(|h| h.size());
    eprintln!("{label} {n} {size:?}");
}

#[component]
fn open_mode() -> NodeHandle {
    let note_text = Signal::new(words(1));
    let bar_lines = Signal::new(1u32);
    let note: Signal<Option<WindowHandle>> = Signal::new(None);
    let bar: Signal<Option<WindowHandle>> = Signal::new(None);
    rinch_core::reactive::unowned(|| {
        for n in 1..=10u32 {
            set_timeout(n * STEP_MS + 500, move || {
                note_text.set(words(n + 1));
                bar_lines.set(n + 1);
            });
            set_timeout(n * STEP_MS + 500 + STEP_MS / 2, move || {
                print_size("NOTE", n, note.get());
                print_size("BAR", n, bar.get());
            });
        }
        let t = 11 * STEP_MS + 500;
        set_timeout(t, move || note_text.set(words(120)));
        set_timeout(t + 300, move || print_size("LONG", 0, note.get()));
        set_timeout(t + 600, move || {
            if let Some(h) = note.get() {
                h.hide();
            }
            note_text.set(words(3));
        });
        set_timeout(t + 900, move || {
            if let Some(h) = note.get() {
                h.show();
            }
        });
        set_timeout(t + 1300, move || print_size("SHOWN", 0, note.get()));
        set_timeout(t + 1600, move || {
            let outputs = outputs();
            if let (Some(h), Some(other)) = (note.get(), outputs.get(1)) {
                h.set_layer(note_surface(Some(other.id)));
            }
        });
        set_timeout(t + 2100, move || print_size("MOVED", 0, note.get()));
        set_timeout(t + 2500, exit)
    });
    rsx! {
        div {
            Window {
                title: "note",
                size_to_content: ContentSizing::new().with_max_width(360),
                wayland_layer: note_surface(outputs().first().map(|o| o.id)),
                on_open: move |h: WindowHandle| note.set(Some(h)),
                div { style: "font: 16px/20px sans-serif; padding: 8px", {move || note_text.get()} }
            }
            Window {
                title: "bar",
                size_to_content: ContentSizing::new(),
                wayland_layer: LayerSurface {
                    anchor: LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
                    compositor_size: LayerAxes::WIDTH,
                    output_id: outputs().first().map(|o| o.id),
                    ..Default::default()
                },
                on_open: move |h: WindowHandle| bar.set(Some(h)),
                div {
                    style: "font: 16px/20px sans-serif",
                    {move || (0..bar_lines.get()).map(|i| format!("line {i}")).collect::<Vec<_>>().join(" / ")}
                }
            }
        }
    }
}

fn note_surface(output_id: Option<OutputId>) -> LayerSurface {
    LayerSurface {
        layer: LayerShellLayer::Overlay,
        anchor: LayerAnchor::TOP | LayerAnchor::RIGHT,
        margins: LayerMargins {
            top: 8,
            right: 8,
            ..Default::default()
        },
        output_id,
        ..Default::default()
    }
}

#[component]
fn main_mode() -> NodeHandle {
    let lines = Signal::new(1u32);
    rinch_core::reactive::unowned(|| {
        for n in 1..=10u32 {
            set_timeout(n * STEP_MS + 500, move || lines.set(n + 1));
            set_timeout(n * STEP_MS + 500 + STEP_MS / 2, move || {
                print_size("MAIN", n, WindowHandle::main())
            });
        }
        set_timeout(11 * STEP_MS + 800, exit)
    });
    rsx! {
        div {
            style: "font: 16px/20px sans-serif; padding: 4px",
            for i in (0..10u32).collect::<Vec<_>>() {
                div { key: i, style: {move || if i < lines.get() { "display: block" } else { "display: none" }}, {format!("row {i}")} }
            }
        }
    }
}

#[component]
fn max_mode() -> NodeHandle {
    let handle: Signal<Option<WindowHandle>> = Signal::new(None);
    rinch_core::reactive::unowned(|| {
        set_timeout(1500, move || {
            let output = outputs().first().map(|o| o.size);
            eprintln!(
                "MAX {:?} output={output:?}",
                handle.get().and_then(|h| h.size())
            );
        });
        set_timeout(2000, exit)
    });
    rsx! {
        div {
            Window {
                title: "max",
                size_to_content: ContentSizing::new(),
                wayland_layer: note_surface(outputs().first().map(|o| o.id)),
                on_open: move |h: WindowHandle| handle.set(Some(h)),
                div { style: "font: 16px/20px sans-serif; white-space: nowrap", {words(400)} }
            }
        }
    }
}

fn main() {
    match std::env::var("MODE").as_deref() {
        Ok("main") => App::new(main_mode)
            .window_props(WindowProps {
                title: "main".into(),
                size_to_content: Some(ContentSizing::new().with_max_width(400)),
                wayland_layer: Some(LayerSurface {
                    anchor: LayerAnchor::BOTTOM,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .run(),
        Ok("max") => App::windowless(max_mode).run(),
        _ => App::windowless(open_mode).run(),
    }
}
