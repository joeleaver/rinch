//! Browser-driven tests for issue #3062 on the web backend: the key handler
//! order. The keyboard interceptor is a `window` capture listener, ahead of
//! every element handler; a key the page `preventDefault`ed first is left
//! alone by every rinch listener; the app-wide listeners (`on_app_key`) are
//! offered the rich-text editor's keys before its keymap; a focused render
//! surface keeps only the keyups of the presses it claimed (W1); and a
//! read-only editor's printable key is prevented but not stopped (E19).
//!
//! The order is `docs/internals/focus-and-keyboard.md` "The key handler
//! order"; `rinch/src/app/key_order_3062_tests.rs` pins the desktop's.
//!
//! The page's own listeners are installed once, before the first mount in this
//! binary, so that the page's `window` capture listener runs ahead of rinch's
//! (two listeners on one node run in the order they were added).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test key_order_3062
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch::render_surface::{
    RenderSurface, RenderSurfaceHandle, create_render_surface, set_focused_surface,
};
use rinch_core::Component;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

const HOST_MARKER: &str = "data-key-order-3062-host";

thread_local! {
    /// The keys the page's `window` capture listener `preventDefault`s.
    static PAGE_PREVENTS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    /// What reached the page's `window` bubble listeners: `down:KEY` /
    /// `up:KEY`.
    static PAGE_HEARD: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PAGE_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// The page's own listeners on `window`: a capture one that prevents the
/// keys in [`PAGE_PREVENTS`], and bubble ones that note every key reaching
/// them. Installed once, ahead of every mount.
fn page_listeners() {
    if PAGE_INSTALLED.with(|f| f.replace(true)) {
        return;
    }
    let window = web_sys::window().unwrap();
    let prevent = Closure::wrap(Box::new(|e: web_sys::KeyboardEvent| {
        if e.type_() == "keydown" && PAGE_PREVENTS.with(|p| p.borrow().contains(&e.key().as_str()))
        {
            e.prevent_default();
        }
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback_and_bool(
            "keydown",
            prevent.as_ref().unchecked_ref(),
            true,
        )
        .unwrap();
    prevent.forget();
    for kind in ["keydown", "keyup"] {
        let heard = Closure::wrap(Box::new(move |e: web_sys::KeyboardEvent| {
            let phase = if e.type_() == "keydown" { "down" } else { "up" };
            PAGE_HEARD.with(|h| h.borrow_mut().push(format!("{phase}:{}", e.key())));
        }) as Box<dyn FnMut(_)>);
        window
            .add_event_listener_with_callback(kind, heard.as_ref().unchecked_ref())
            .unwrap();
        heard.forget();
    }
}

fn page_heard() -> Vec<String> {
    PAGE_HEARD.with(|h| std::mem::take(&mut *h.borrow_mut()))
}

fn page_prevents(keys: &[&'static str]) {
    PAGE_PREVENTS.with(|p| *p.borrow_mut() = keys.to_vec());
}

fn new_host() -> web_sys::Element {
    page_listeners();
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(node) = stale.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

/// A key event of `kind` dispatched at `target`; answers whether it was
/// cancelled.
fn key_at(target: &web_sys::EventTarget, kind: &str, key: &str, code: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

/// A press and its release where the browser sends them: the focused element,
/// or `<body>`. Answers whether the press was cancelled.
fn press(key: &str, code: &str) -> bool {
    let target: web_sys::EventTarget = document()
        .active_element()
        .unwrap_or_else(|| document().body().unwrap().into())
        .into();
    let prevented = key_at(&target, "keydown", key, code);
    key_at(&target, "keyup", key, code);
    prevented
}

/// An interceptor that logs every press and takes the keys in `takes`.
fn intercept(log: &Log, takes: &'static [&'static str]) {
    let l = log.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        if k.is_down() {
            l.borrow_mut().push(format!("interceptor:{}", k.key));
        }
        k.is_down() && takes.contains(&k.key.as_str())
    });
}

/// An app-wide listener that logs every press and takes the keys in `takes`.
fn listen(log: &Log, takes: &'static [&'static str]) -> AppKeyListener {
    let l = log.clone();
    on_app_key(move |k: &KeyEventData| {
        if k.is_down() {
            l.borrow_mut().push(format!("app:{}", k.key));
        }
        k.is_down() && takes.contains(&k.key.as_str())
    })
}

struct Fields {
    root: RootHandle,
    host: web_sys::Element,
    surface: RenderSurfaceHandle,
    log: Log,
}

impl Fields {
    /// An `<input>` whose `onkeydown` logs every press and takes F2, and a
    /// render surface.
    fn mount() -> Self {
        let host = new_host();
        let log: Log = Default::default();
        let surface = create_render_surface();
        let (l, sf) = (log.clone(), surface.clone());
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |__scope: &mut RenderScope| {
                let l = l.clone();
                let node = rsx! {
                    div {
                        input {
                            id: "ko-in",
                            oninput: move |_v: String| {},
                            onkeydown: move |k| {
                                l.borrow_mut().push(format!("element:{}", k.key));
                                k.key == "F2"
                            },
                        }
                    }
                };
                let canvas = RenderSurface {
                    surface: Some(sf.clone()),
                }
                .render(__scope, &[]);
                node.append_child(&canvas);
                node
            },
        );
        let f = Self {
            root,
            host,
            surface,
            log,
        };
        f.input().focus().unwrap();
        f
    }

    fn input(&self) -> web_sys::HtmlElement {
        document()
            .get_element_by_id("ko-in")
            .unwrap()
            .dyn_into()
            .unwrap()
    }

    fn teardown(self) {
        page_prevents(&[]);
        set_focused_surface(None);
        rinch_core::clear_keyboard_interceptor();
        self.root.unmount();
        self.host.remove();
    }
}

/// The interceptor hears a press ahead of the element's `onkeydown`, and a
/// press it takes reaches no element handler. Before #3062 it ran from the
/// `document` bubble delegate, after the element's `onkeydown`, so it also
/// never heard a key the page's own element listener stopped; from `window`'s
/// capture phase it hears that key too.
#[wasm_bindgen_test]
fn the_interceptor_hears_a_key_before_the_elements_onkeydown() {
    let f = Fields::mount();
    intercept(&f.log, &["F4"]);

    assert!(!press("F3", "F3"));
    assert_eq!(take(&f.log), ["interceptor:F3", "element:F3"]);

    assert!(press("F4", "F4"), "the interceptor took it");
    assert_eq!(take(&f.log), ["interceptor:F4"], "and no element heard it");

    // Positive control: the element still takes the key the interceptor
    // leaves.
    assert!(press("F2", "F2"));
    assert_eq!(take(&f.log), ["interceptor:F2", "element:F2"]);

    // A page listener on the input stops the key from bubbling: rinch's
    // `document` delegate never sees it, the interceptor still does.
    let stop = Closure::wrap(
        Box::new(|e: web_sys::Event| e.stop_propagation()) as Box<dyn FnMut(web_sys::Event)>
    );
    f.input()
        .add_event_listener_with_callback("keydown", stop.as_ref().unchecked_ref())
        .unwrap();
    press("F6", "F6");
    f.input()
        .remove_event_listener_with_callback("keydown", stop.as_ref().unchecked_ref())
        .unwrap();
    assert_eq!(take(&f.log), ["interceptor:F6"]);
    f.teardown();
}

/// A key the page's own `window` capture listener `preventDefault`ed is the
/// page's: neither the interceptor, nor the element's `onkeydown`, nor the
/// app-wide listeners hear it, as Chrome runs no default for it. Before #3062
/// every rinch listener acted on it regardless.
#[wasm_bindgen_test]
fn a_key_the_page_prevented_first_is_left_to_the_page() {
    let f = Fields::mount();
    intercept(&f.log, &[]);
    let app = listen(&f.log, &[]);

    // Positive control: an unprevented key reaches all three.
    press("F3", "F3");
    assert_eq!(take(&f.log), ["interceptor:F3", "element:F3", "app:F3"]);

    page_prevents(&["F5"]);
    assert!(press("F5", "F5"), "the page prevented it");
    assert!(take(&f.log).is_empty(), "rinch left it alone");
    page_heard();

    app.remove();
    f.teardown();
}

/// A focused render surface keeps the release of a press it claimed; every
/// other keyup goes on to the page's `window` listeners, as a `<canvas>`'s do
/// (#3062, W1). Before, every keyup was stopped while a surface held the
/// keyboard.
#[wasm_bindgen_test]
fn a_focused_surfaces_unclaimed_keyups_reach_the_page() {
    let f = Fields::mount();
    f.input().blur().unwrap();
    set_focused_surface(Some(f.surface.id()));
    f.surface.set_key_handler(|k| k.code == "KeyW");
    page_heard();

    assert!(!press("k", "KeyK"));
    assert_eq!(
        page_heard(),
        ["down:k", "up:k"],
        "an unclaimed key, both legs"
    );

    assert!(press("w", "KeyW"), "the surface claimed it");
    assert!(
        page_heard().is_empty(),
        "its press and its release are the surface's"
    );
    f.teardown();
}

struct Editor {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Editor {
    /// One editor over `<p>hello</p>`, focused by a real press on its first
    /// character.
    fn focused() -> Self {
        let host = new_host();
        let handle = create_editor();
        assert!(handle.load_html("<p>hello</p>"));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self { root, host, handle };
        let para = f
            .editor_el()
            .query_selector("p")
            .unwrap()
            .expect("a paragraph");
        let text = para.first_child().expect("the paragraph has a text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, 0).unwrap();
        range.set_end(&text, 1).unwrap();
        let r = range.get_bounding_client_rect();
        let (x, y) = (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        );
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(f.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
        f
    }

    fn editor_el(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("an editor")
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// The paragraphs' text, as the page shows them.
    fn paragraphs(&self) -> Vec<String> {
        let ps = self.editor_el().query_selector_all("p").unwrap();
        (0..ps.length())
            .map(|i| ps.item(i).unwrap().text_content().unwrap_or_default())
            .collect()
    }

    /// A `keydown` on the capture textarea; answers whether it was cancelled.
    fn key(&self, key: &str, code: &str) -> bool {
        key_at(self.capture().as_ref(), "keydown", key, code)
    }

    fn teardown(self) {
        page_prevents(&[]);
        rinch_core::clear_keyboard_interceptor();
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(1);
    init.set_detail(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// The app-wide listeners are offered the editor's keys after its `on_key`
/// and before its keymap, and can take one from it: an app's Enter binding is
/// the app's. Before #3062 they were never offered a key the editor held.
#[wasm_bindgen_test]
fn on_app_key_takes_an_editors_key_before_its_keymap() {
    let f = Editor::focused();
    let log: Log = Default::default();
    intercept(&log, &[]);
    let l = log.clone();
    f.handle.on_key(move |k| {
        l.borrow_mut().push(format!("on_key:{}", k.key));
        false
    });
    let app = listen(&log, &["Enter"]);

    assert!(f.key("Enter", "Enter"), "the listener took Enter");
    assert_eq!(
        take(&log),
        ["interceptor:Enter", "on_key:Enter", "app:Enter"],
        "the interceptor, the editor's on_key, then the app's listeners"
    );
    assert_eq!(
        f.paragraphs(),
        ["hello"],
        "the keymap did not split the line"
    );

    // Positive control: without the listener the keymap splits it.
    app.remove();
    assert!(f.key("Enter", "Enter"));
    assert_eq!(f.paragraphs().len(), 2, "{:?}", f.paragraphs());
    f.teardown();
}

/// A key the page `preventDefault`ed first is the page's in an editor too:
/// the keymap does not act on it.
#[wasm_bindgen_test]
fn an_editor_leaves_a_key_the_page_prevented() {
    let f = Editor::focused();
    page_prevents(&["Enter"]);
    f.key("Enter", "Enter");
    assert_eq!(f.paragraphs(), ["hello"], "the keymap left it");

    // Positive control: unprevented, the same Enter splits the line.
    page_prevents(&[]);
    assert!(f.key("Enter", "Enter"));
    assert_eq!(f.paragraphs().len(), 2, "{:?}", f.paragraphs());
    f.teardown();
}

/// A read-only editor refuses a typed letter and prevents its default (no
/// scroll on Space), but the `keydown` goes on to the page's own listeners,
/// as a `readonly` field's does (#3062, E19). Before, it was stopped.
#[wasm_bindgen_test]
fn a_read_only_editors_printable_key_reaches_the_page() {
    let f = Editor::focused();
    f.handle.set_read_only(true);
    page_heard();
    assert!(f.key("a", "KeyA"), "the editor prevents the key's default");
    assert_eq!(page_heard(), ["down:a"], "and the page hears it");
    assert_eq!(f.paragraphs(), ["hello"], "nothing typed");

    // Control: an editable editor types the letter and owns the key.
    f.handle.set_read_only(false);
    assert!(f.key("a", "KeyA"));
    assert!(page_heard().is_empty(), "a typed letter is the editor's");
    assert_ne!(f.paragraphs(), ["hello"]);
    f.teardown();
}

/// A `window` `blur` forgets the presses the app-wide listeners were offered:
/// the release of a key held across it goes elsewhere, so the next release of
/// that key belongs to a later press. Before the fix a press the input's
/// `onkeydown` took after the window came back had its release offered to the
/// listeners, paired with the press made before the blur.
#[wasm_bindgen_test]
fn a_window_blur_forgets_the_presses_on_app_key_was_offered() {
    let f = Fields::mount();
    let l = f.log.clone();
    let app = on_app_key(move |k: &KeyEventData| {
        let phase = if k.is_up() { "up" } else { "down" };
        l.borrow_mut().push(format!("app:{phase}:{}", k.key));
        false
    });

    // F2 pressed on `<body>` reaches the listener; its release is lost with
    // the keyboard.
    f.input().blur().unwrap();
    let body: web_sys::EventTarget = document().body().unwrap().into();
    key_at(&body, "keydown", "F2", "F2");
    assert_eq!(take(&f.log), ["app:down:F2"]);
    let window: web_sys::EventTarget = web_sys::window().unwrap().into();
    window
        .dispatch_event(&web_sys::Event::new("blur").unwrap())
        .unwrap();

    // Back in the input, its `onkeydown` takes F2: neither the press nor its
    // release is the listener's.
    f.input().focus().unwrap();
    assert!(press("F2", "F2"));
    assert_eq!(take(&f.log), ["element:F2"]);
    app.remove();
    f.teardown();
}
