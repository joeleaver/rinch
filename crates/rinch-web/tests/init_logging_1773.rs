//! `rinch::init_logging` in the browser (issue #1773): `tracing` events reach
//! the console method of their level, and so do `log` records.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Replace `console[name]` with a recorder; returns the original.
fn stub_console(name: &str, key: &str) -> JsValue {
    let window = web_sys::window().unwrap();
    let console = js_sys::Reflect::get(&window, &"console".into()).unwrap();
    let original = js_sys::Reflect::get(&console, &name.into()).unwrap();
    js_sys::Reflect::set(&window, &key.into(), &js_sys::Array::new()).unwrap();
    let f = js_sys::Function::new_with_args("line", &format!("window.{key}.push(String(line));"));
    js_sys::Reflect::set(&console, &name.into(), &f).unwrap();
    original
}

fn restore_console(name: &str, original: JsValue) {
    let window = web_sys::window().unwrap();
    let console = js_sys::Reflect::get(&window, &"console".into()).unwrap();
    js_sys::Reflect::set(&console, &name.into(), &original).unwrap();
}

fn recorded(key: &str) -> Vec<String> {
    let window = web_sys::window().unwrap();
    let arr: js_sys::Array = js_sys::Reflect::get(&window, &key.into()).unwrap().into();
    arr.iter().map(|v| v.as_string().unwrap()).collect()
}

#[wasm_bindgen_test]
fn events_go_to_the_console_by_level() {
    // Positive control first: before the call, an event reaches no console.
    let warn = stub_console("warn", "__w1773");
    tracing::warn!("before init 1773");
    let before = recorded("__w1773");
    restore_console("warn", warn);
    assert!(before.is_empty(), "no subscriber yet: {before:?}");

    rinch::init_logging();

    let warn = stub_console("warn", "__w1773");
    let info = stub_console("info", "__i1773");
    let log = stub_console("log", "__l1773");
    tracing::warn!("disk nearly full 1773");
    tracing::info!("saved 1773");
    tracing::debug!("not at info 1773");
    log::warn!("a log record 1773");
    let (w, i, l) = (
        recorded("__w1773"),
        recorded("__i1773"),
        recorded("__l1773"),
    );
    restore_console("warn", warn);
    restore_console("info", info);
    restore_console("log", log);

    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        w[0].contains("WARN") && w[0].contains("disk nearly full 1773"),
        "{w:?}"
    );
    assert!(
        w[1].contains("a log record 1773"),
        "`log` records reach it too: {w:?}"
    );
    assert_eq!(i.len(), 1, "{i:?}");
    assert!(i[0].contains("saved 1773"), "{i:?}");
    assert!(l.is_empty(), "debug is below the default info: {l:?}");
}
