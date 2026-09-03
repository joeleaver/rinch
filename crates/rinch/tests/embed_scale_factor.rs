//! The embed host's half of issue #211: a `RinchContext` pushes its config
//! scale factor into Stylo at mount, and `set_scale_factor` restyles a live
//! context on its next `update()`.
//!
//! Same harness rules as `multi_context.rs`: `RinchContext::new` registers the
//! creating thread as THE main thread process-wide, so every test body runs on
//! one shared worker thread.
//!
//! Every discriminating assertion here runs at a scale != 1 — at 1.0 a
//! working push and the hard-coded default are indistinguishable — and the
//! runtime test uses the fractional 1.25 so integer rounding can't hide.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;
use rinch_core::dom::NodeId;

type Job = Box<dyn FnOnce() + Send>;

/// Run `f` on the single shared "UI" worker thread and propagate its panic (if
/// any) back to the calling test thread.
fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
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

/// `.x` is 100x10; at >= 2dppx the width becomes 200, at >= 1.2dppx the
/// height becomes 20 (mirrors `app/device_pixel_ratio_tests.rs`).
const MEDIA_CSS: &str = "\
    .x { width: 100px; height: 10px } \
    @media (min-resolution: 2dppx) { .x { width: 200px } } \
    @media (min-resolution: 1.2dppx) { .x { height: 20px } }";

fn context_with_gated_target(
    physical: (u32, u32),
    scale_factor: f64,
) -> (RinchContext, Rc<Cell<Option<NodeId>>>) {
    let target: Rc<Cell<Option<NodeId>>> = Rc::new(Cell::new(None));
    let t = target.clone();
    let ctx = RinchContext::new(
        RinchContextConfig {
            width: physical.0,
            height: physical.1,
            scale_factor,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let style = scope.create_element("style");
            let css = scope.create_text(MEDIA_CSS);
            style.append_child(&css);
            root.append_child(&style);
            let div = scope.create_element("div");
            div.set_attribute("class", "x");
            t.set(Some(div.node_id()));
            root.append_child(&div);
            root
        },
    );
    (ctx, target)
}

fn target_size(ctx: &RinchContext, target: &Rc<Cell<Option<NodeId>>>) -> (f32, f32) {
    let id = target.get().expect("component ran");
    let doc = ctx.app().doc().expect("context has a document").clone();
    let d = doc.borrow();
    let node = d.tree.get(id.0).expect("target node exists");
    (node.layout.width, node.layout.height)
}

/// A context created with `scale_factor: 2.0` resolves `resolution` media
/// queries from its very first layout — `RinchContextConfig::scale_factor`
/// reaches Stylo, not just the logical-size division.
///
/// Kills: dropping the `set_device_pixel_ratio` push in `RinchContext::new`.
#[test]
fn a_context_mounts_with_its_config_scale_factor() {
    on_ui_thread(|| {
        // Physical 1600x1200 at 2x — the logical 800x600 every other test uses.
        let (ctx, target) = context_with_gated_target((1600, 1200), 2.0);
        assert_eq!(
            target_size(&ctx, &target),
            (200.0, 20.0),
            "both resolution-gated rules apply from the first layout"
        );
    });
}

/// `set_scale_factor` on a live context restyles on the next `update()`:
/// the push marks the document style-dirty, and `update`'s post-event dirty
/// check re-resolves at the new logical size. Fractional, so a value rounded
/// to 1 (height stays 10) or to 2 (width becomes 200) both fail.
///
/// Kills: `set_scale_factor` storing the field without forwarding to the app.
#[test]
fn set_scale_factor_restyles_on_the_next_update() {
    on_ui_thread(|| {
        let (mut ctx, target) = context_with_gated_target((1000, 750), 1.0);
        assert_eq!(
            target_size(&ctx, &target),
            (100.0, 10.0),
            "at 1.0 neither gated rule applies"
        );

        ctx.set_scale_factor(1.25);
        ctx.update(&[]);
        assert_eq!(
            target_size(&ctx, &target),
            (100.0, 20.0),
            "1.25 matches the 1.2dppx rule and not the 2dppx one"
        );
    });
}
