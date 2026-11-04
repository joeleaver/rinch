//! The probe: a rinch window that shows one case's cleaned document in an
//! `HtmlView`, in the [`Setup`]'s frame and fonts, one case after another.
//! [`capture`](super::capture) drives it over rinch's debug server: Next
//! shows the next case, Down moves the document up by [`STEP`] (a window is
//! no taller than the screen).

use std::path::PathBuf;

use rinch::prelude::*;

use super::Setup;
use super::case::CleanedCase;

/// How far Down moves the document, in px: less than any window is tall.
pub const STEP: u32 = 800;
/// Room to the right of the document for the probe's buttons.
pub const BAR: u32 = 120;

/// Run the probe over `dirs`, before the first (Next shows the first).
/// Pictures load as the app's own registered image schemes say; register
/// any (an `https` loader) before calling.
pub fn run(dirs: Vec<PathBuf>, setup: Setup) {
    let window = WindowProps {
        title: "rinch html-compare probe".into(),
        width: setup.width + BAR,
        height: 1200,
        ..Default::default()
    };
    let fonts = setup.fonts;
    App::new(move |scope| probe(scope, dirs.clone(), setup.clone()))
        .window_props(window)
        .fonts(fonts)
        .run();
}

#[component]
fn probe(dirs: Vec<PathBuf>, setup: Setup) -> NodeHandle {
    let shown: Signal<Option<usize>> = Signal::new(None);
    let page = Signal::new(0u32);
    let count = dirs.len();
    let css = format!(
        "{}\n.probe {{ position: relative; display: flex; flex-direction: column; }}\n\
         .probe-bar {{ position: absolute; top: 0; left: {}px; width: 110px; z-index: 10; display: flex; flex-direction: column; gap: 6px; padding: 6px; background: #eeeeee; }}",
        super::frame_css(&setup),
        setup.width + 4
    );
    rsx! {
        div { class: "probe",
            style { {css} }
            div { class: "probe-bar",
                button {
                    class: "probe-next",
                    onclick: move || {
                        page.set(0);
                        shown.update(|s| *s = Some(s.map_or(0, |i| (i + 1).min(count))));
                    },
                    "Next"
                }
                button { class: "probe-down", onclick: move || page.update(|p| *p += 1), "Down" }
                span { class: "probe-at", {move || shown.get().map_or("-".to_owned(), |i| i.to_string())} }
            }
            div {
                class: "probe-sheet",
                style: {move || format!("margin-top: -{}px", page.get() * STEP)},
                // Keyed on the case, so each is drawn afresh.
                for i in shown.get().filter(|&i| i < count) {
                    div { key: i.to_string(), {document(__scope, dirs[i].clone())} }
                }
            }
        }
    }
}

#[component]
fn document(dir: PathBuf) -> NodeHandle {
    let cleaned: CleanedCase = std::fs::read(dir.join("cleaned.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(CleanedCase {
            html: "<p>cleaned.json is missing</p>".into(),
            body_style: String::new(),
        });
    let host = rsx! { div { class: "html-compare-host" } };
    let view = rinch::html::HtmlView {
        html: cleaned.html,
        body_style: cleaned.body_style,
        ..Default::default()
    }
    .render(__scope, &[]);
    host.append_child(&view);
    host
}
