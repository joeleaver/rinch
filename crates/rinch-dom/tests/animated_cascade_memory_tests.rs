//! The heap stays bounded while a `color` animation runs through the cascade
//! (review of #2564).
//!
//! Each frame of a `color` / `font-size` animation cascades its node with the
//! frame's sample (#777, #787), which mints Stylo rule nodes holding the
//! sample's declaration block. Stylo frees unused rule nodes only through the
//! rule tree's GC, which rinch's cascade never ran: measured +14.8 MB over 30k
//! frames for one infinite animation. `resolve_layout` now calls
//! `RuleTree::maybe_gc`.
//!
//! Live heap bytes, counted by the allocator (not RSS). One test in its own
//! binary: the counting allocator counts every thread, so nothing else may
//! run beside it.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, s: usize) -> *mut u8 {
        LIVE.fetch_add(s as isize - l.size() as isize, Relaxed);
        unsafe { System.realloc(p, l, s) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const T0: f64 = 1_000_000.0;

#[test]
fn a_running_colour_animation_does_not_grow_the_heap() {
    let mut doc = RinchDocument::new();
    doc.set_animation_clock_ms(Some(T0));
    doc.load_css(
        "html, body { margin: 0; }
         .a { color: rgb(0, 0, 0); animation: c 1000ms linear infinite; }
         @keyframes c { from { color: rgb(0, 0, 0); } to { color: rgb(200, 100, 0); } }",
    );
    let body = doc.body();
    let a = doc.create_element("div");
    doc.set_attribute(a, "class", "a");
    doc.append_child(body, a);
    let span = doc.create_element("span");
    let t = doc.create_text("label");
    doc.append_child(span, t);
    doc.append_child(a, span);
    doc.resolve_layout(800.0, 600.0);

    let mut ms = T0;
    let mut frames = |doc: &mut RinchDocument, n: usize| {
        for _ in 0..n {
            // Off the 1000ms period, so every frame has a new sample.
            ms += 16.7;
            doc.set_animation_clock_ms(Some(ms));
            doc.tick_animations();
            doc.resolve_layout(800.0, 600.0);
        }
    };
    // Warm up: caches fill, the GC's free list cycles a few times.
    frames(&mut doc, 1000);
    let before = LIVE.load(Relaxed);
    frames(&mut doc, 3000);
    let grown = LIVE.load(Relaxed) - before;

    // Positive control: the frames really went through the cascade.
    assert!(
        doc.tree.get(a.0).unwrap().base_style.is_some(),
        "the animated node is cascaded with its sample"
    );
    // Without the GC this grew ~490 bytes a frame (~1.5 MB over 3000 frames).
    assert!(
        grown < 300_000,
        "the live heap grew {grown} bytes over 3000 animation frames"
    );
}
