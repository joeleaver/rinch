//! #2757's measure: the live heap two documents (two windows) hold for one
//! 3840x2160 picture drawn at 1280x720, as deskrinch's wallpaper is, counted
//! by a global allocator the way DHAT's "live at the end" counts it.
//!
//! The picture is 31.64 MiB as RGBA8 and its half-size mipmap level 7.91 MiB.
//! Before #2757 each document held both (79.1 MiB for two); now the process
//! holds one of each (39.6 MiB) however many documents show it. The figures
//! are printed (`--nocapture`), and asserted against the per-document case.
//!
//! One test in its own binary: the allocator counts the whole process.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_core::image::{ImageLoadResult, register_image_scheme};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(
            new_size as isize - layout.size() as isize,
            Ordering::Relaxed,
        );
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const MIB: f64 = 1024.0 * 1024.0;

fn live() -> isize {
    LIVE.load(Ordering::SeqCst)
}

/// Two windows' documents, each showing `src` as a 1280x720 cover
/// background (or a plain colour for `None`), laid out and painted, with the
/// painter's pixmap freed.
fn two_windows(src: Option<&str>) -> Vec<RinchDocument> {
    let docs: Vec<RinchDocument> = (0..2)
        .map(|_| {
            let mut doc = RinchDocument::new();
            let bg = match src {
                Some(src) => format!("background: url({src}) center / cover"),
                None => "background: #345".into(),
            };
            doc.load_css(&format!(
                "html, body {{ margin: 0; }} div {{ width: 1280px; height: 720px; {bg}; }}"
            ));
            let body = doc.body();
            let div = doc.create_element("div");
            doc.append_child(body, div);
            doc
        })
        .collect();
    let mut docs = docs;
    for doc in &mut docs {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            doc.resolve_layout(1280.0, 720.0);
            if src.is_none_or(|s| doc.tree.image_cache.get(s).is_some()) {
                break;
            }
            assert!(Instant::now() < deadline, "the picture never loaded");
            std::thread::sleep(Duration::from_millis(5));
        }
        // Each frame's minify budget (16M source px) admits the 8.3M-pixel
        // picture; a few frames in case.
        for _ in 0..3 {
            rinch_dom::image_minify::begin_frame();
            let mut painter = TinySkiaPainter::new(1280, 720);
            let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
            rinch_dom::paint::paint_document(
                &doc.tree,
                &mut painter,
                1.0,
                (1280.0, 720.0),
                &mut doc.font_cx,
                &mut cx,
            );
        }
    }
    docs
}

#[test]
fn two_windows_hold_one_decode_and_one_mip_level() {
    let (w, h) = (3840u32, 2160u32);
    let encoded = {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([x as u8, y as u8, (x / 16 + y / 16) as u8, 255])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    register_image_scheme("wallpaper2757", move |_src: &str| {
        ImageLoadResult::Loaded(encoded.clone())
    });
    let src = "wallpaper2757:picture";

    // Warm up what every document builds anyway (fonts, the UA sheet), then
    // the plain-colour pair is the baseline.
    drop(two_windows(None));
    let before = live();
    let plain = two_windows(None);
    let plain_bytes = live() - before;
    drop(plain);

    let before = live();
    let pictured = two_windows(Some(src));
    let pictured_bytes = live() - before;

    let picture = (w * h * 4) as f64;
    let level = (w / 2 * h / 2 * 4) as f64;
    let extra = (pictured_bytes - plain_bytes) as f64;
    println!(
        "#2757 two windows, one {w}x{h} picture at 1280x720: live heap {:.2} MiB over a plain \
         colour's {:.2} MiB, so the picture costs {:.2} MiB (one decode {:.2} MiB + one level \
         {:.2} MiB = {:.2} MiB; per window it was {:.2} MiB)",
        pictured_bytes as f64 / MIB,
        plain_bytes as f64 / MIB,
        extra / MIB,
        picture / MIB,
        level / MIB,
        (picture + level) / MIB,
        2.0 * (picture + level) / MIB,
    );
    let (count, bytes) = rinch_dom::image_cache::shared_images_held();
    println!(
        "#2757 shared store: {count} picture(s), {:.2} MiB",
        bytes as f64 / MIB
    );
    // One decode and one level, with a little slack for the documents' own
    // bookkeeping; a second copy of either would be 7.91 MiB or more over.
    assert!(
        extra < picture + level + 4.0 * MIB,
        "the picture costs {:.2} MiB, over one decode and one level",
        extra / MIB
    );
    assert!(
        extra > picture,
        "control: the picture was decoded and held ({:.2} MiB)",
        extra / MIB
    );
    drop(pictured);
}
