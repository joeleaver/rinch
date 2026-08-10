//! Web collaborative rich-text editor demo for `rinch-web`.
//!
//! Two independent [`EditorHandle`]s share one document through the **same**
//! `rinch-editor-collab` Step↔CRDT adapter the desktop uses — now running in the
//! browser (yrs compiled to wasm, with no randomness shim to wire up: it carries
//! `fastrand/js` itself). The default web build links zero CRDT code;
//! it is pulled in only by `rinch-web`'s `collaboration` feature.
//!
//! There is no network here — the two editors are wired into a synchronous, in-process
//! **loopback**: each side's outbound delta is delivered straight to the other's
//! [`EditorHandle::collab_receive`]. Because that delivery happens synchronously inside
//! the originating keystroke, one pass updates *both* panes. A real app keeps this exact
//! seam and only swaps the loopback for a transport — the outbound closure sends bytes to
//! a `WebSocket`; its `onmessage` (on the main thread) calls `handle.collab_receive`.
//!
//! Click into either pane and type — the edit converges in the other. The shared content
//! is deliberately **flat** (headings, paragraphs, marks): the staged collab scope
//! (design A22) is flat text-blocks + marks.

use wasm_bindgen::prelude::*;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{Editor, create_editor};

const PANE: &str = "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px;";
const LABEL: &str = "font-family: sans-serif; font-size: 13px; font-weight: 600; color: #57606a; text-transform: uppercase; letter-spacing: 0.04em;";
const HOST_BOX: &str = "border: 1px solid #d0d7de; border-radius: 8px; overflow: hidden;";

#[component]
fn app() -> NodeHandle {
    // Two editors, each over its own fresh document. The host pane loads the shared
    // content via its `Editor { content }`; the guest starts empty and adopts the
    // host's document when it joins below.
    let host = create_editor();
    let guest = create_editor();

    let tree = rsx! {
        div {
            style: "min-height: 100vh; padding: 28px; font-family: -apple-system, system-ui, sans-serif; display: flex; flex-direction: column; gap: 16px;",
            div {
                h2 { style: "margin: 0 0 4px; color: #1f2328;", "Collaborative editing on the web" }
                p {
                    id: "subtitle",
                    style: "margin: 0; color: #57606a; font-size: 14px;",
                    "Two editors sharing one yrs CRDT in the browser — the same adapter desktop uses. Click into either pane and type; the edit converges in the other."
                }
            }
            div {
                style: "display: flex; gap: 24px; align-items: stretch;",
                // Editor A — the host.
                div {
                    style: PANE,
                    div { style: LABEL, "Editor A · host" }
                    div { id: "pane-a", style: HOST_BOX,
                        Editor {
                            editor: host.clone(),
                            content: "<h1>Shared notes</h1>\
                                <p>Type in <strong>either</strong> pane — every keystroke syncs through the CRDT.</p>\
                                <p>Marks travel too: <strong>bold</strong>, <em>italic</em>.</p>",
                        }
                    }
                }
                // Editor B — the guest (adopts the host's content on join).
                div {
                    style: PANE,
                    div { style: LABEL, "Editor B · guest" }
                    div { id: "pane-b", style: HOST_BOX,
                        Editor { editor: guest.clone(), content: "" }
                    }
                }
            }
        }
    };

    // The `Editor` components rendered synchronously while the tree above was built, so
    // the host has already loaded its content. Wire the loopback now: the host projects
    // its (loaded) document onto a fresh CRDT and the guest joins from the snapshot —
    // adopting the host's content — then each side relays its local deltas to the other.
    // `collab_receive` does not re-broadcast, so there is no feedback loop.
    let guest_in = guest.clone();
    let snapshot = host
        .start_collaboration_host(move |delta| {
            guest_in.collab_receive(&delta);
        })
        .expect("host content is flat (A22 scope)");
    let host_in = host.clone();
    guest
        .start_collaboration_guest(&snapshot, move |delta| {
            host_in.collab_receive(&delta);
        })
        .expect("snapshot is valid flat content");

    // Park a cursor in the host so the first click+type is obvious.
    host.set_selection(Selection::cursor(Pos(1)));
    tree
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    rinch_web::mount(ThemeProviderProps::default(), app);
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
