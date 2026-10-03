//! Review round 6 of #1233: a frozen editor (a collaboration freeze over a table too
//! large to read) owns its keys and its capture field the way a read-only one does —
//! Space and letters consumed, the capture `<textarea>` `readonly` — and the cure
//! gives them back.
#![cfg(all(target_arch = "wasm32", feature = "collaboration"))]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::create_editor;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn grown(snapshot: &[u8]) -> Vec<u8> {
    rinch_editor_collab::testing::grow_first_table_update(snapshot, 2000).expect("a table")
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document().element_from_point(x, y).unwrap();
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

fn capture() -> web_sys::HtmlTextAreaElement {
    document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap()
}

fn keydown(key: &str, code: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    capture().dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

const CONTENT: &str = "<table><tr><td><p>c</p></td></tr></table><p>Hello world one</p>";

#[wasm_bindgen_test]
fn rv6_a_frozen_editor_owns_its_keys_and_its_field_like_a_read_only_one() {
    let hostel = document().create_element("div").unwrap();
    hostel
        .set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
    document().body().unwrap().append_child(&hostel).unwrap();
    let guest = create_editor();
    let mounted = guest.clone();
    let root = rinch_web::mount_into(
        &hostel,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let w = scope.create_element("div");
            w.append_child(&mounted.mount(scope));
            w
        },
    );
    let host = create_editor();
    assert!(host.load_html(CONTENT));
    let snap = host.start_collaboration_host(|_| {}).unwrap();
    guest.start_collaboration_guest(&snap, |_| {}).unwrap();
    // focus on "Hello"
    let ps = hostel.query_selector_all("p").unwrap();
    let para = ps.item(ps.length() - 1).unwrap();
    let text = para.first_child().unwrap();
    let range = document().create_range().unwrap();
    range.set_start(&text, 2).unwrap();
    range.set_end(&text, 3).unwrap();
    let r = range.get_bounding_client_rect();
    let (x, y) = (
        (r.x() + r.width() / 2.0) as f32,
        (r.y() + r.height() / 2.0) as f32,
    );
    mouse("mousedown", x, y);
    mouse("mouseup", x, y);
    assert!(
        document().active_element().as_deref() == Some(capture().as_ref()),
        "positive control: focused"
    );
    // positive control: an editable editor consumes Space (it inserts)
    let before = guest.doc();
    assert!(keydown(" ", "Space"), "control: editable consumes Space");
    assert!(!guest.doc().same_ref(&before));

    assert!(guest.collab_receive(&grown(&guest.collab_snapshot().unwrap())));
    assert_eq!(guest.collab_oversized_tables().len(), 1, "frozen");
    let before = guest.doc();
    let space = keydown(" ", "Space");
    let letter = keydown("x", "KeyX");
    let ro = capture().read_only();

    assert!(guest.doc().same_ref(&before), "nothing edited");
    let frozen_ok = space && letter && ro;
    let id = guest.collab_oversized_tables()[0].id.clone();
    assert_eq!(guest.collab_delete_oversized_table(&id), Ok(true));
    let ro_after = capture().read_only();
    let before = guest.doc();
    let typed = keydown("y", "KeyY");
    let edited = !guest.doc().same_ref(&before);
    root.unmount();
    hostel.remove();
    assert!(
        frozen_ok,
        "frozen: space {space} letter {letter} readonly {ro}"
    );
    assert!(
        !ro_after && typed && edited,
        "after the cure: readonly {ro_after} typed {typed} edited {edited}"
    );
}
