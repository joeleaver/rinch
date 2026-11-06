//! #1356, review of PR #2271: an outside list marker is clipped by a scroller
//! that holds its item on the GPU painter too, as on the software one
//! (`rinch-dom/tests/list_marker_outside_1356_tests.rs::review_2271`). It used
//! to be `position: absolute`, escaped the scroller's clip and painted over the
//! header above it. Needs an adapter:
//! `cargo test -p rinch --features desktop,gpu --lib -- --ignored gpu --test-threads=1`.
use super::gpu_clip_groups_1645_tests::{Gpu, H, W, software};
use super::*;
use std::cell::Cell;
use std::rc::Rc;

/// A 50px header over a 60px `overflow: auto` scroller holding six red items,
/// scrolled by 40.
fn app() -> RinchApp {
    let scroller = Rc::new(Cell::new(None));
    let slot = scroller.clone();
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 320px; height: 200px; \
             background-color: white; font-size: 16px; line-height: 20px",
        );
        let header = scope.create_element("div");
        header.set_attribute("style", "height: 50px");
        root.append_child(&header);
        let sc = scope.create_element("div");
        sc.set_attribute("style", "height: 60px; overflow: auto; width: 300px");
        let ul = scope.create_element("ul");
        ul.set_attribute("style", "margin: 0");
        for label in ["a", "b", "c", "d", "e", "f"] {
            let li = scope.create_element("li");
            li.set_attribute("style", "color: rgb(255, 0, 0)");
            let span = scope.create_element("span");
            span.set_attribute("style", "color: black");
            span.append_child(&scope.create_text(label));
            li.append_child(&span);
            ul.append_child(&li);
        }
        sc.append_child(&ul);
        root.append_child(&sc);
        slot.set(Some(sc.node_id()));
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    let sc = scroller.get().expect("the scroller was built");
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_scroll_top(sc, 40.0);
    app.resolve_and_repaint(W as f32 + 1.0, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    app
}

/// The rows of red marker ink, `(first, last)`, in an RGBA picture.
fn red_rows(px: &[u8]) -> Option<(u32, u32)> {
    let mut rows: Option<(u32, u32)> = None;
    for y in 0..H {
        for x in 0..W {
            let p = &px[((y * W + x) * 4) as usize..][..3];
            if p[0] > 200 && p[1] < 90 && p[2] < 90 {
                let r = rows.get_or_insert((y, y));
                r.0 = r.0.min(y);
                r.1 = r.1.max(y);
            }
        }
    }
    rows
}

#[test]
#[ignore]
fn gpu_an_outside_marker_is_clipped_by_its_items_scroller() {
    let mut app = app();
    let cpu = software(&mut app);
    let gpu = Gpu::new().render(&mut app);
    let (cpu_rows, gpu_rows) = (red_rows(&cpu), red_rows(&gpu));
    let (first, last) = gpu_rows.expect("markers are drawn");
    assert!(
        first >= 50 && last < 110,
        "the GPU markers stay inside the scroller (50..110): {first}..={last}"
    );
    let (c_first, c_last) = cpu_rows.expect("markers are drawn");
    assert!(
        first.abs_diff(c_first) <= 1 && last.abs_diff(c_last) <= 1,
        "GPU {gpu_rows:?} and software {cpu_rows:?} agree"
    );
}
