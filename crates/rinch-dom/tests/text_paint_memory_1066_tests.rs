//! #1066 (review of #1700, N1) — what one paint of a long paragraph holds on
//! the heap. `PaintText` used to copy every line's items and glyphs on every
//! paint: 12.3 MB for an 8000-line `<pre>`, where the walk it replaced held
//! 6.5 KB. A line is kept now only when a masked pass of a paragraph with
//! passes per span list or per fragment draws it.
//!
//! One test in its own binary: the counting allocator counts every thread,
//! so nothing else may run beside it.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let n = LIVE.fetch_add(l.size() as isize, Relaxed) + l.size() as isize;
        PEAK.fetch_max(n, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, s: usize) -> *mut u8 {
        let d = s as isize - l.size() as isize;
        let n = LIVE.fetch_add(d, Relaxed) + d;
        PEAK.fetch_max(n, Relaxed);
        unsafe { System.realloc(p, l, s) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// A `<pre>` of `lines` lines in a 200px scroller, `pre_css` on the `<pre>`,
/// and `span` around the first line when given.
fn page(lines: usize, pre_css: &str, span: Option<&str>) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css(&format!(
        "body {{ margin: 0; }} .sc {{ height: 200px; width: 400px; overflow: auto; }} \
         pre {{ margin: 0; font: 12px/16px ProbeFace; {pre_css} }}"
    ));
    let mut text = String::new();
    for i in 0..lines {
        let line = format!("line {i} with some ordinary words of text in it to fill");
        match (i, span) {
            (0, Some(style)) => {
                let _ = writeln!(text, r#"<span style="{style}">{line}</span>"#);
            }
            _ => {
                let _ = writeln!(text, "{line}");
            }
        }
    }
    let body = doc.body();
    doc.set_inner_html(body, &format!("<div class=\"sc\"><pre>{text}</pre></div>"));
    doc.resolve_layout(420.0, 300.0);
    doc
}

/// The heap one paint grows by at its peak, after a warm-up paint.
fn paint_peak(doc: &mut RinchDocument) -> isize {
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    let mut painter = TinySkiaPainter::new(420, 300);
    let mut paint = |doc: &mut RinchDocument| {
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (420.0, 300.0),
            &mut doc.font_cx,
            &mut cx,
        )
    };
    paint(doc);
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    paint(doc);
    PEAK.load(Relaxed) - base
}

#[test]
fn a_long_paragraphs_paint_holds_no_copy_of_its_lines() {
    for (name, pre_css, span) in [
        ("no shadow", "", None),
        ("root's list", "text-shadow: 0 1px 0 rgb(200, 0, 0);", None),
        (
            "one span's own list",
            "",
            Some("text-shadow: 0 1px 0 rgb(200, 0, 0)"),
        ),
    ] {
        let small = paint_peak(&mut page(500, pre_css, span));
        let large = paint_peak(&mut page(4000, pre_css, span));
        println!("{name}: 500 lines {small} B, 4000 lines {large} B");
        // Measured on base (b92c90ec), before `PaintText`: no shadow 6560 B
        // at both sizes; the shadowed paragraphs grow by about 186 B a line
        // in what the shadow pass holds outside `PaintText` (745 KB at 4000
        // lines). PR #1700's first head: about 1540 B a line in every case.
        let per_line = (large - small) as f64 / 3500.0;
        assert!(
            per_line < 200.0,
            "{name}: {per_line:.0} B a line ({small} B at 500 lines, {large} B at 4000)"
        );
        if pre_css.is_empty() && span.is_none() {
            assert!(
                large <= small + 1024 && large < 32 * 1024,
                "{name}: {small} B at 500 lines, {large} B at 4000"
            );
        }
    }
}
