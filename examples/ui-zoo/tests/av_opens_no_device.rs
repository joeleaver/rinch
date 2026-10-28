//! Opening ui-zoo's Audio / Video page opens no device (#1916, design record
//! `docs/design/rtc-media.md` 5.2).
//!
//! The section used to open the camera (on a thread of its own) and the
//! microphone while it rendered, so merely looking at the page turned both on.
//! They now open from the "Start camera" and "Start microphone" buttons.
//!
//! Every device open in rinch-av passes `rinch_av::testing`'s gate, which this
//! test counts and *denies*: no run of it, against the fixed code or the old
//! one, reaches a real camera or microphone. Device lists are not opens, and
//! the test lists none.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};

fn walk(n: &NodeHandle, out: &mut Vec<NodeHandle>) {
    out.push(n.clone());
    for c in n.children() {
        walk(&c, out);
    }
}

fn text_of(n: &NodeHandle) -> String {
    let mut all = Vec::new();
    walk(n, &mut all);
    all.iter().filter_map(|c| c.text_content()).collect()
}

/// Click the innermost element with a handler whose text contains `label`.
fn click(root: &NodeHandle, label: &str) {
    let mut all = Vec::new();
    walk(root, &mut all);
    let target = all
        .iter()
        .rev()
        .find(|n| n.get_attribute("data-rid").is_some() && text_of(n).contains(label))
        .unwrap_or_else(|| panic!("no clickable {label:?} on the page"));
    let id: u64 = target.get_attribute("data-rid").unwrap().parse().unwrap();
    rinch_core::events::dispatch_event(rinch_core::events::EventHandlerId(id as _));
}

fn opens_reach(n: usize) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if rinch_av::testing::device_opens() >= n {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn the_av_page_opens_no_device_until_a_button_asks() {
    rinch_av::testing::deny_devices(true);
    ui_zoo::sections::init_av_state_with(Vec::new(), Vec::new(), Vec::new());

    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
    let page = ui_zoo::sections::av_section(&mut scope);

    // The old camera open ran on a thread spawned by the render: give one
    // the time it would need to get there.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        rinch_av::testing::device_opens(),
        0,
        "rendering the Audio / Video page opened a device"
    );

    // The positive control: the buttons do reach rinch-av (and are denied).
    click(&page, "Start camera");
    assert!(opens_reach(1), "Start camera opened no camera");
    click(&page, "Start microphone");
    assert!(opens_reach(2), "Start microphone opened no microphone");
}
