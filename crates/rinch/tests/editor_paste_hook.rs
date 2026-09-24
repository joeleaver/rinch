//! The editor's paste hook (`Plugin::handle_paste`) on desktop, end to end: a
//! real Ctrl+V / Ctrl+Shift+V `PlatformEvent`, the clipboard read on its worker
//! thread (the in-memory test backend), the completion hopping back to the main
//! thread, and the paste landing through `EditorHandle::paste` at the anchor.
//!
//! An integration test, in its own process, because the completion crosses
//! threads: the worker hands it to `run_on_main_thread`, which only queues it
//! once a main thread is registered, and registering one is process-wide (a
//! `OnceLock`). Every body therefore runs on one long-lived "UI" thread that
//! registers itself and drains the queue, as `tests/embed_cross_thread.rs` does.
//!
//! Requires the `clipboard` feature (CI's `-p rinch` feature set has it):
//!     cargo test -p rinch --features clipboard --test editor_paste_hook

#![cfg(all(feature = "desktop", feature = "clipboard"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rinch::app::RinchApp;
use rinch::editor::{EditorHandle, create_editor};
use rinch::platform::{KeyCode, KeyRepeat, Modifiers, MouseButton, PlatformEvent};
use rinch::prelude::RenderScope;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Fragment, Mark, PasteContent, Plugin, PluginKey, Pos, Selection,
    Transaction,
};

// ── harness ──────────────────────────────────────────────────────────────────

type Job = Box<dyn FnOnce() + Send>;

/// Run `f` on the one shared UI thread, which registered itself as the main
/// thread with a queue-only dispatcher; propagate its panic.
fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                rinch::core::register_main_thread();
                rinch::core::set_cross_thread_dispatcher(|f| {
                    rinch::core::queue_main_callback(f);
                });
                rinch_clipboard::use_in_memory_clipboard();
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });
    let (result_tx, result_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = result_tx.send(result);
    });
    sender
        .lock()
        .expect("ui sender lock")
        .send(job)
        .expect("ui worker alive");
    match result_rx.recv().expect("ui worker responded") {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

const VP: (u32, u32) = (800, 600);

fn ev(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, VP, 1.0);
}

fn chord(app: &mut RinchApp, key: KeyCode, shift: bool) {
    let modifiers = Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        shift,
        ..Default::default()
    };
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
}

fn is_url(text: &str) -> bool {
    ["https://", "http://", "pimble:"]
        .iter()
        .any(|scheme| text.starts_with(scheme))
        && !text.contains(char::is_whitespace)
}

/// A pasted URL links the selected words, or at a caret inserts linked text
/// (a note's title for `pimble:`, the URL for the web). Counts the pastes it
/// was offered.
struct Linker(Rc<Cell<u32>>);

impl Plugin for Linker {
    fn key(&self) -> PluginKey {
        PluginKey("test.linker")
    }

    fn handle_paste(&self, state: &EditorState, paste: &PasteContent) -> Option<Transaction> {
        self.0.set(self.0.get() + 1);
        let url = paste.text.as_deref().map(str::trim).filter(|t| is_url(t))?;
        let link = Mark::new(
            state.schema().mark_type("link")?.clone(),
            Attrs::from_iter([("href", AttrValue::from(url.to_string()))]),
        );
        let (from, to) = (state.selection.from().0, state.selection.to().0);
        let mut tr = state.tr();
        if from < to {
            tr.add_mark(from, to, link).ok()?;
        } else {
            let label = if url.starts_with("pimble:") {
                "Linked note"
            } else {
                url
            };
            let text = state.schema().text_with_marks(label, vec![link]).ok()?;
            let len = text.text_len();
            tr.replace_with(from, from, Fragment::from_node(text))
                .ok()?;
            tr.set_selection(Selection::cursor(Pos(from + len)));
        }
        Some(tr)
    }
}

struct Page {
    app: RinchApp,
    handle: EditorHandle,
    offered: Rc<Cell<u32>>,
}

/// One focused editor over `<p>see the docs here</p>` ("the docs" is 5..13),
/// with the `Linker` plugin unless `plugin` is false.
fn page(plugin: bool) -> Page {
    let offered = Rc::new(Cell::new(0));
    let handle = create_editor();
    if plugin {
        assert!(handle.add_plugin(Rc::new(Linker(offered.clone()))));
    }
    assert!(handle.load_html("<p>see the docs here</p>"));
    let slot: Rc<RefCell<Option<EditorHandle>>> = Rc::new(RefCell::new(Some(handle.clone())));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let handle = slot.borrow_mut().take().expect("mounted once");
        let container = handle.mount(scope);
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        container
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    // A real press inside the first line focuses the editor.
    let (x, y, button) = (6.0, 12.0, MouseButton::Left);
    ev(&mut app, PlatformEvent::MouseDown { x, y, button });
    ev(&mut app, PlatformEvent::MouseUp { x, y, button });
    Page {
        app,
        handle,
        offered,
    }
}

fn html(page: &Page) -> String {
    node_to_html(&page.handle.doc())
}

/// Drain the main-thread queue until the document changes (the clipboard
/// worker answered and the completion ran), or give up after two seconds.
fn wait_for_paste(page: &mut Page, before: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        rinch::core::drain_main_callbacks();
        let now = html(page);
        if now != before || Instant::now() > deadline {
            page.app.resolve_and_repaint(800.0, 600.0);
            return now;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Ctrl+V of `clip` (`(text, html)`) over `selection`; the document after.
fn ctrl_v(
    page: &mut Page,
    selection: Selection,
    clip: (&str, Option<&str>),
    shift: bool,
) -> String {
    match clip {
        (text, None) => rinch_clipboard::copy_text(text).unwrap(),
        (text, Some(html)) => rinch_clipboard::copy_html(html, Some(text)).unwrap(),
    }
    page.handle.set_selection(selection);
    let before = html(page);
    chord(&mut page.app, KeyCode::KeyV, shift);
    wait_for_paste(page, &before)
}

const LINKED: &str = "<p>see <a href=\"https://example.test/\">the docs</a> here</p>";
/// "the docs".
fn words() -> Selection {
    Selection::text(Pos(5), Pos(13))
}

// ── tests ────────────────────────────────────────────────────────────────────

#[test]
fn ctrl_v_of_a_url_over_selected_words_links_them() {
    on_ui_thread(|| {
        let mut p = page(true);
        let after = ctrl_v(&mut p, words(), ("https://example.test/", None), false);
        assert_eq!(after, LINKED);
        assert_eq!(p.offered.get(), 1);

        // Control: without the plugin the URL replaces the words.
        let mut p = page(false);
        let after = ctrl_v(&mut p, words(), ("https://example.test/", None), false);
        assert_eq!(after, "<p>see https://example.test/ here</p>");
    });
}

/// A browser's copy of a link offers html and text; desktop's read hands the
/// plugin the text beside the html, as the web's `paste` event does.
#[test]
fn ctrl_v_of_html_with_a_url_beside_it_shows_the_plugin_the_text() {
    on_ui_thread(|| {
        let clip = (
            "https://example.test/",
            Some("<a href=\"https://example.test/\">Example</a>"),
        );
        let mut p = page(true);
        assert_eq!(ctrl_v(&mut p, words(), clip, false), LINKED);

        // Control: unclaimed, the html is what goes in.
        let mut p = page(false);
        assert_eq!(
            ctrl_v(&mut p, words(), clip, false),
            "<p>see <a href=\"https://example.test/\">Example</a> here</p>"
        );
    });
}

#[test]
fn ctrl_shift_v_of_a_url_at_the_caret_inserts_the_linked_text() {
    on_ui_thread(|| {
        let mut p = page(true);
        let after = ctrl_v(
            &mut p,
            Selection::cursor(Pos(5)),
            ("pimble:1234/5678", None),
            true,
        );
        assert_eq!(
            after,
            "<p>see <a href=\"pimble:1234/5678\">Linked note</a>the docs here</p>"
        );
        assert_eq!(p.handle.selection(), Selection::cursor(Pos(16)));
    });
}

#[test]
fn ctrl_v_of_text_the_plugin_does_not_claim_gets_the_default() {
    on_ui_thread(|| {
        let mut p = page(true);
        let after = ctrl_v(&mut p, words(), ("plain words", None), false);
        assert_eq!(after, "<p>see plain words here</p>");
        assert_eq!(p.offered.get(), 1, "offered, declined");
    });
}

#[test]
fn ctrl_z_takes_a_claimed_paste_back_in_one_step() {
    on_ui_thread(|| {
        let mut p = page(true);
        assert_eq!(
            ctrl_v(&mut p, words(), ("https://example.test/", None), false),
            LINKED
        );
        chord(&mut p.app, KeyCode::KeyZ, false);
        assert_eq!(html(&p), "<p>see the docs here</p>");
    });
}

/// A read-only editor starts no clipboard read, so the plugin is never asked
/// and nothing lands; the writable control beside it pastes.
#[test]
fn a_read_only_editor_is_unaffected_by_the_hook() {
    on_ui_thread(|| {
        let mut p = page(true);
        p.handle.set_read_only(true);
        rinch_clipboard::copy_text("https://example.test/").unwrap();
        p.handle.set_selection(words());
        chord(&mut p.app, KeyCode::KeyV, false);
        let after = wait_for_paste(&mut p, "<p>see the docs here</p>");
        assert_eq!(after, "<p>see the docs here</p>");
        assert_eq!(p.offered.get(), 0, "no read, so no offer");

        p.handle.set_read_only(false);
        assert_eq!(
            ctrl_v(&mut p, words(), ("https://example.test/", None), false),
            LINKED
        );
    });
}
