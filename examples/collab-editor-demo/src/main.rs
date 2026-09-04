//! Live collaborative editing demo (M9). Two **independent** `EditorHandle`s share
//! one document through the `rinch-editor-collab` Step↔CRDT (yrs) adapter.
//!
//! There is no network here — the two editors are wired into a synchronous,
//! in-process **loopback**: each side's outbound delta is delivered straight to the
//! other's [`EditorHandle::collab_receive`]. Because that delivery happens
//! synchronously inside the originating keystroke, a single layout/repaint pass
//! updates *both* panes. A real network app keeps this exact seam and only swaps the
//! loopback for a transport (the outbound closure sends bytes to a socket; the
//! receiver calls `rinch::post_remote_delta` from its thread).
//!
//! Click into either pane and type — the edit converges in the other. The shared
//! content is deliberately **flat** (headings, paragraphs, marks): the staged collab
//! scope (design A22) is flat text-blocks + marks, so lists/blockquotes/tables are
//! out of scope for now and would fail loud.

use rinch::prelude::*;
use rinch_editor_core::{Pos, Selection};

const PANE_LABEL: &str = "font-family: sans-serif; font-size: 13px; font-weight: 600; color: #57606a; text-transform: uppercase; letter-spacing: 0.04em;";
const PANE_COL: &str = "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px;";

#[component]
fn app() -> NodeHandle {
    // Two editors, each over its own fresh document. `create_editor` makes them
    // empty; the host pane loads the shared content via its `Editor { content }`.
    let host = create_editor();
    let guest = create_editor();

    let tree = rsx! {
        div {
            style: "min-height: 100vh; padding: 28px; background: #f6f8fa; display: flex; flex-direction: column; gap: 16px;",
            div {
                style: "font-family: sans-serif;",
                h2 { style: "margin: 0 0 4px; color: #1f2328;", "Collaborative editing" }
                p {
                    style: "margin: 0; color: #57606a; font-size: 14px;",
                    "Two independent editors sharing one yrs CRDT. Click into either pane and type — the edit converges in the other."
                }
            }
            div {
                style: "display: flex; gap: 24px; align-items: stretch;",
                // Editor A — the host.
                div {
                    style: PANE_COL,
                    div { style: PANE_LABEL, "Editor A · host" }
                    Editor {
                        editor: host.clone(),
                        content: "<h1>Shared notes</h1>\
                            <p>Type in <strong>either</strong> pane — every keystroke syncs through the CRDT.</p>\
                            <p>Marks travel too: <strong>bold</strong>, <em>italic</em>.</p>",
                    }
                }
                // Editor B — the guest (adopts the host's content on join).
                div {
                    style: PANE_COL,
                    div { style: PANE_LABEL, "Editor B · guest" }
                    Editor { editor: guest.clone(), content: "" }
                }
            }
        }
    };

    // The `Editor` components rendered synchronously while the tree above was built,
    // so the host has already loaded its content. Wire the loopback now: the host
    // projects its (loaded) document onto a fresh CRDT and the guest joins from the
    // snapshot — adopting the host's content — then each side relays its local
    // deltas to the other.
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

fn main() {
    App::new(app)
        .title("Collaborative Editor Demo")
        .size(1100, 720)
        .run();
}
