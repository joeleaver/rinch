//! Replaced elements as flex items, grid items and absolutely positioned
//! boxes (#2700, #1280). #2442 made an inline picture's `auto` width its
//! natural width clamped by its own `max-width`, with its padding outside;
//! these are the other layouts that size a picture:
//!
//! - **A flex row:** its automatic minimum size is its min-content size
//!   clamped by its max main size (css-flexbox-1 §4.5), so a `max-width:
//!   100%; padding: 7%` picture is 361.375 wide in 317px and does not shrink
//!   to 317. rinch compressed its min-content size to nothing, and its
//!   percentage padding counted 0 where it measured the picture.
//! - **A column and an absolutely positioned box:** the width clamped by
//!   `max-width` gives the height through the ratio. rinch kept the natural
//!   height (400 for a picture 317 wide).
//! - **A grid:** a replaced item's `normal` alignment is `start`, in both
//!   axes (css-align-3 §6.1); rinch stretched it (#1280).
//!
//! Every expected number is Chrome 157's for the same markup (an 800x400 PNG
//! in a 317px container, so nothing lands on a whole or half pixel), measured
//! with `getBoundingClientRect` relative to the container. rinch lays out in
//! whole pixels, so a size may be off by its rounding (0.6), and a size whose
//! two edges both round by more by up to one pixel (`close_edges`).
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// An 800x400 PNG.
const W800: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAyAAAAGQCAIAAADZR5NjAAAF6ElEQVR42u3WMQ0AAAgEMcC/sJeFBhLGVsJN10kKAIA/IwEAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAIMlAQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAXC4kQBXhhKf6rAAAAAElFTkSuQmCC";

const CSS: &str =
    "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(800.0, 600.0);
    d
}

/// `#i`'s border box, relative to its parent: (x, width, height).
fn rect(d: &RinchDocument) -> (f32, f32, f32) {
    let n: NodeId = d.query_selector("#i").expect("#i");
    let l = d.tree.get(n.0).unwrap().layout;
    (l.x, l.width, l.height)
}

#[track_caller]
fn close(got: f32, want: f32, what: &str) {
    assert!(
        (got - want).abs() <= 0.6,
        "{what}: rinch {got}, Chrome {want}"
    );
}

/// `<img id=i style={img}>` in a 317px `<div style={container}>`: at
/// `want` (x, width, height), Chrome 157's.
#[track_caller]
fn check(container: &str, img: &str, want: (f32, f32, f32)) {
    let d = doc(&format!(
        "<div style='width:317px; {container}'><img id=i src='{W800}' style='{img}'></div>"
    ));
    let what = format!("`{img}` in `{container}`");
    let (x, w, h) = rect(&d);
    close(x, want.0, &format!("x of {what}"));
    close(w, want.1, &format!("width of {what}"));
    close(h, want.2, &format!("height of {what}"));
}

// ---- A flex row -----------------------------------------------------------

#[test]
fn a_row_does_not_shrink_a_picture_below_its_max_width() {
    // The content box is 317 x 158.5 and the 7% padding (22.19) is outside
    // it. rinch was 317 x 181: shrunk to the row, the padding inside.
    check(
        "display:flex",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    // Length padding too: rinch was 317 x 168.
    check(
        "display:flex",
        "max-width:100%; padding:13px 17px",
        (0.0, 351.0, 184.5),
    );
    // Not stretched: rinch was 317 x 181.
    check(
        "display:flex; align-items:flex-start",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    // In a wrapping row: rinch was 317 x 181.
    check(
        "display:flex; flex-wrap:wrap",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
}

#[test]
fn object_fit_changes_nothing_in_a_row() {
    // rinch was 317 x 181.
    check(
        "display:flex; align-items:flex-start",
        "max-width:100%; padding:7%; object-fit:cover",
        (0.0, 361.375, 202.875),
    );
    // Right before.
    check(
        "display:flex; align-items:flex-start",
        "max-width:100%; height:97px; object-fit:contain",
        (0.0, 194.0, 97.0),
    );
}

#[test]
fn min_width_zero_or_border_box_still_lets_a_row_shrink_it() {
    // Right before: the automatic minimum is what kept it, and these change it.
    check(
        "display:flex",
        "max-width:100%; padding:7%; min-width:0",
        (0.0, 317.0, 180.688),
    );
    check(
        "display:flex",
        "max-width:100%; padding:7%; box-sizing:border-box",
        (0.0, 317.0, 180.688),
    );
    check("display:flex", "max-width:100%", (0.0, 317.0, 158.5));
}

#[test]
fn min_and_max_sizes_in_a_row() {
    // Right before.
    check(
        "display:flex",
        "max-width:61%; padding:3%",
        (0.0, 212.359, 115.672),
    );
    check(
        "display:flex",
        "max-width:100%; min-width:351px",
        (0.0, 351.0, 175.5),
    );
    check(
        "display:flex; align-items:flex-start",
        "width:103px; min-width:131px",
        (0.0, 131.0, 65.5),
    );
    // A max-height gives the width through the ratio, stretched or not.
    check(
        "display:flex; align-items:flex-start",
        "max-height:53px",
        (0.0, 106.0, 53.0),
    );
    check("display:flex", "max-height:53px", (0.0, 106.0, 53.0));
}

#[test]
fn a_stretched_picture_takes_its_width_from_its_content_height() {
    // Stretched to 153 with `padding: 7%`: the content box is 108.625 tall,
    // so 217.25 wide. rinch was 306 wide: the padding counted 0 where the
    // row measured it.
    check(
        "display:flex; height:153px",
        "max-width:100%; padding:7%",
        (0.0, 261.625, 153.0),
    );
    // Right before.
    check(
        "display:flex; height:153px",
        "max-width:100%",
        (0.0, 306.0, 153.0),
    );
    check(
        "display:flex; height:153px",
        "width:203px",
        (0.0, 203.0, 153.0),
    );
    check(
        "display:flex; height:153px; align-items:flex-start",
        "width:203px",
        (0.0, 203.0, 101.5),
    );
}

// ---- A flex column --------------------------------------------------------

#[test]
fn a_column_gives_a_clamped_picture_its_height_through_the_ratio() {
    // rinch was 317 x 400 and 361 x 444: the natural height.
    check(
        "display:flex; flex-direction:column; align-items:flex-start",
        "max-width:100%",
        (0.0, 317.0, 158.5),
    );
    check(
        "display:flex; flex-direction:column; align-items:flex-start",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    // A max-height gives the width through the ratio.
    check(
        "display:flex; flex-direction:column; align-items:flex-start",
        "max-height:53px",
        (0.0, 106.0, 53.0),
    );
}

#[test]
fn a_centred_picture_in_a_column() {
    // rinch was 213 x 419. Its x and width both round, so its edges are
    // compared: 52.328 and 264.672 in Chrome.
    let d = doc(&format!(
        "<div style='width:317px; display:flex; flex-direction:column; align-items:center'><img id=i src='{W800}' style='max-width:61%; padding:3%'></div>"
    ));
    let (x, w, h) = rect(&d);
    close(x, 52.328, "left edge");
    close(x + w, 264.672, "right edge");
    close(h, 115.672, "height");
}

#[test]
fn a_stretched_picture_in_a_column_is_held_by_its_max_width() {
    // Stretched: the 317 is clamped to 61% (193.37) and the height follows.
    check(
        "display:flex; flex-direction:column",
        "max-width:61%",
        (0.0, 193.359, 96.672),
    );
    // Right before.
    check(
        "display:flex; flex-direction:column",
        "max-width:100%; padding:7%",
        (0.0, 317.0, 180.688),
    );
    check(
        "display:flex; flex-direction:column",
        "max-width:100%",
        (0.0, 317.0, 158.5),
    );
    check(
        "display:flex; flex-direction:column",
        "width:203px",
        (0.0, 203.0, 101.5),
    );
}

// ---- A grid ---------------------------------------------------------------

#[test]
fn a_grid_does_not_stretch_a_picture() {
    // rinch was 317 x 181: stretched to the track, the padding inside.
    check(
        "display:grid",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    // rinch was 211 x 112.
    check(
        "display:grid; grid-template-columns:211px 1fr",
        "max-width:100%; padding:3%",
        (0.0, 223.656, 118.156),
    );
    // A taller area: rinch stretched it to 203 x 153.
    check(
        "display:grid; height:153px",
        "width:203px",
        (0.0, 203.0, 101.5),
    );
    // Right before.
    check("display:grid", "max-width:100%", (0.0, 317.0, 158.5));
}

#[test]
fn a_grid_still_follows_an_explicit_alignment() {
    check(
        "display:grid; justify-items:start",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    check(
        "display:grid; justify-items:start; align-items:start",
        "max-width:100%",
        (0.0, 317.0, 158.5),
    );
    // An explicit stretch is not `normal`: Chrome 157 gives 203 x 153.
    check(
        "display:grid; height:153px",
        "width:203px; align-self:stretch",
        (0.0, 203.0, 153.0),
    );
    check(
        "display:grid; height:153px; align-items:stretch",
        "width:203px",
        (0.0, 203.0, 153.0),
    );
}

#[test]
fn a_canvas_in_a_grid_keeps_its_size_and_a_text_field_still_stretches() {
    let d = doc(
        "<div style='display:grid; width:317px; height:191px'><canvas id=i width=100 height=50></canvas></div>",
    );
    let (_, w, h) = rect(&d);
    // rinch was 100 x 191.
    close(w, 100.0, "canvas width");
    close(h, 50.0, "canvas height");
    // An `<input>` is not a replaced element to the grid: 317 x 91 in Chrome
    // 157, as before.
    let d = doc("<div style='display:grid; width:317px; height:91px'><input id=i></div>");
    let (_, w, h) = rect(&d);
    close(w, 317.0, "input width");
    close(h, 91.0, "input height");
}

// ---- An absolutely positioned box ----------------------------------------

#[test]
fn an_absolutely_positioned_picture_takes_its_height_through_the_ratio() {
    let cb = "position:relative; height:50px";
    // rinch was 317 x 400 and 361 x 444.
    check(cb, "position:absolute; max-width:100%", (0.0, 317.0, 158.5));
    check(
        cb,
        "position:absolute; max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    // rinch was 212 x 419.
    check(
        cb,
        "position:absolute; left:0; max-width:61%; padding:3%",
        (0.0, 212.344, 115.672),
    );
    // rinch was 351 x 400.
    check(
        cb,
        "position:absolute; max-width:100%; min-width:351px",
        (0.0, 351.0, 175.5),
    );
    check(cb, "position:absolute; max-height:53px", (0.0, 106.0, 53.0));
    // Right before.
    check(
        cb,
        "position:absolute; width:203px; padding:3%",
        (0.0, 222.0, 120.5),
    );
}

// ---- Intrinsic sizes ------------------------------------------------------

#[test]
fn a_flex_or_grid_container_sized_from_its_content_still_compresses_it() {
    // In a shrink-wrapped table cell the picture's percentage has no basis
    // and its min-content contribution is 0, through a flex or a grid
    // container, as in Chrome 157.
    for display in ["flex", "grid"] {
        let d = doc(&format!(
            "<table style='width:1px'><tr><td><div style='display:{display}'><img id=i src='{W800}' style='max-width:100%'></div></td></tr></table>"
        ));
        let (_, w, h) = rect(&d);
        close(w, 0.0, &format!("{display} width"));
        close(h, 0.0, &format!("{display} height"));
    }
    // With 7px padding: 28 x 21 in Chrome (rinch was 14 x 14).
    let d = doc(&format!(
        "<table style='width:1px'><tr><td><div style='display:flex'><img id=i src='{W800}' style='max-width:100%; padding:7px'></div></td></tr></table>"
    ));
    let (_, w, h) = rect(&d);
    close(w, 28.0, "padded width");
    close(h, 21.0, "padded height");
    // A float around a flex container: the picture's natural size.
    let d = doc(&format!(
        "<div style='float:left'><div style='display:flex'><img id=i src='{W800}' style='max-width:100%'></div></div>"
    ));
    let (_, w, h) = rect(&d);
    close(w, 800.0, "float width");
    close(h, 400.0, "float height");
}

// ---- A picture that arrives after the first layout ------------------------

/// As [`check`], but the picture has no `src` at the first layout and
/// gets it after, so its natural size reaches a layout that has already
/// sized it as an empty image.
#[track_caller]
fn check_late(container: &str, img: &str, want: (f32, f32, f32)) {
    let mut d = doc(&format!(
        "<div style='width:317px; {container}'><img id=i style='{img}'></div>"
    ));
    let n: NodeId = d.query_selector("#i").unwrap();
    d.set_attribute(n, "src", W800);
    d.resolve_layout(800.0, 601.0);
    let what = format!("late `{img}` in `{container}`");
    let (x, w, h) = rect(&d);
    close(x, want.0, &format!("x of {what}"));
    close(w, want.1, &format!("width of {what}"));
    close(h, want.2, &format!("height of {what}"));
}

#[test]
fn a_picture_that_loads_late_is_sized_as_one_that_was_there() {
    check_late(
        "display:flex",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    check_late(
        "display:flex; flex-direction:column; align-items:flex-start",
        "max-width:100%",
        (0.0, 317.0, 158.5),
    );
    check_late(
        "display:grid",
        "max-width:100%; padding:7%",
        (0.0, 361.375, 202.875),
    );
    check_late(
        "position:relative; height:50px",
        "position:absolute; max-width:100%",
        (0.0, 317.0, 158.5),
    );
}

// ---- A picture with no picture, in a grid ---------------------------------
//
// Chrome 157 keeps an `<img>` at the start of its grid area only while it is
// a replaced box. One whose load failed, or with no `src` and `alt` text, is
// laid out as an ordinary box, and a 300x200 grid stretches it. One still
// loading, or with neither `src` nor `alt`, is a replaced box with no size
// yet: 0x0 at the start.

/// A `data:` URL that is not a picture: its load fails, synchronously.
const BROKEN: &str = "data:image/png;base64,AAAA";

/// `<img {attrs}>` in a 300x200 grid: its (width, height).
fn in_grid(attrs: &str) -> (f32, f32) {
    let d = doc(&format!(
        "<div style='display:grid; width:300px; height:200px'><img id=i {attrs}></div>"
    ));
    let (_, w, h) = rect(&d);
    (w, h)
}

#[track_caller]
fn check_grid(attrs: &str, want: (f32, f32)) {
    let (w, h) = in_grid(attrs);
    close(w, want.0, &format!("width of `<img {attrs}>`"));
    close(h, want.1, &format!("height of `<img {attrs}>`"));
}

#[test]
fn a_failed_picture_in_a_grid_stretches() {
    let broken = format!("src='{BROKEN}'");
    // Chrome 157: 300x200 with or without alt text.
    check_grid(&format!("{broken} alt='broken picture'"), (300.0, 200.0));
    check_grid(&broken, (300.0, 200.0));
    // Its own size holds: 100x50.
    check_grid(&format!("{broken} width=100 height=50"), (100.0, 50.0));
    check_grid(
        &format!("{broken} width=100 height=50 alt='broken picture'"),
        (100.0, 50.0),
    );
}

#[test]
fn a_picture_with_no_src_stretches_only_with_alt_text() {
    check_grid("alt='broken picture'", (300.0, 200.0));
    check_grid("", (0.0, 0.0));
    check_grid("width=100 height=50", (100.0, 50.0));
}

/// A load that fails on the loader's thread after 40ms.
struct FailsLater;
impl rinch_core::image::ImageLoader for FailsLater {
    fn load(&self, src: &str) -> rinch_core::image::ImageLoadResult {
        std::thread::sleep(std::time::Duration::from_millis(40));
        rinch_core::image::ImageLoadResult::Failed(format!("{src}: no"))
    }
}

#[test]
fn a_loading_picture_sits_at_the_start_until_its_load_fails() {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    d.tree.image_loader = Some(std::sync::Arc::new(FailsLater));
    let body = d.body();
    d.set_inner_html(
        body,
        "<div style='display:grid; width:300px; height:200px'><img id=i src='remote-2700.png' alt='broken picture'></div>",
    );
    d.resolve_layout(800.0, 600.0);
    // Loading: Chrome 157 gives 0x0 (with alt text too).
    let (_, w, h) = rect(&d);
    close(w, 0.0, "loading width");
    close(h, 0.0, "loading height");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !d.tree.image_cache.is_failed("remote-2700.png") {
        assert!(
            std::time::Instant::now() < deadline,
            "the load never failed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        d.resolve_layout(800.0, 600.0);
    }
    d.resolve_layout(800.0, 600.0);
    // Failed: an ordinary box, stretched.
    let (_, w, h) = rect(&d);
    close(w, 300.0, "failed width");
    close(h, 200.0, "failed height");
}

#[test]
fn a_picture_that_fails_and_then_loads_goes_back_to_the_start() {
    let mut d = doc(&format!(
        "<div style='display:grid; width:300px; height:200px'><img id=i src='{BROKEN}' style='width:103px'></div>"
    ));
    // Failed, with a width of its own: 103 wide, stretched to 200 tall.
    let (_, w, h) = rect(&d);
    close(w, 103.0, "failed width");
    close(h, 200.0, "failed height");
    // Loaded: 103 x 51.5 at the start (Chrome 157).
    let n: NodeId = d.query_selector("#i").unwrap();
    d.set_attribute(n, "src", W800);
    d.resolve_layout(800.0, 601.0);
    let (_, w, h) = rect(&d);
    close(w, 103.0, "loaded width");
    close(h, 51.5, "loaded height");
    // Failed again: stretched again.
    d.set_attribute(n, "src", BROKEN);
    d.resolve_layout(800.0, 602.0);
    let (_, w, h) = rect(&d);
    close(w, 103.0, "failed again width");
    close(h, 200.0, "failed again height");
}
