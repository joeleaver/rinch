//! An inline picture with `max-width: 100%` and a percentage (or length)
//! padding or margin, in a plain block (#2442). The `max-width` resolves
//! against the containing block for the picture's **content** box
//! (`box-sizing: content-box`), so the padding and margins are added outside
//! the 100% and overflow the container. rinch took them out of the container
//! first, so the picture was narrower than in Chrome.
//!
//! Every expected number is Chrome 157's for the same markup (an 800x400 PNG
//! in a 317px block, so nothing lands on a whole or half pixel), measured
//! with `getBoundingClientRect`. Only sizes are compared: an atomic inline's
//! horizontal margins do not offset it on its line yet (#1405), so a
//! picture with margins sits at x = 0 where Chrome puts it at its margin.
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

/// `#id`'s border box size, and its parent's width.
fn size(d: &RinchDocument, id: &str) -> (f32, f32, f32) {
    let n: NodeId = d.query_selector(&format!("#{id}")).expect(id);
    let node = d.tree.get(n.0).unwrap();
    let parent = d.tree.get(node.parent.unwrap()).unwrap();
    (node.layout.width, node.layout.height, parent.layout.width)
}

#[track_caller]
fn close(got: f32, want: f32, what: &str) {
    assert!(
        (got - want).abs() <= 0.6,
        "{what}: rinch {got}, Chrome {want}"
    );
}

/// The picture `<img id=i style={style}>` in a 317px block: its border box
/// is `want` (Chrome 157) and the block stays 317 wide.
#[track_caller]
fn check(style: &str, want: (f32, f32)) {
    let d = doc(&format!(
        "<div style='width:317px'><img id=i src='{W800}' style='{style}'></div>"
    ));
    let (w, h, pw) = size(&d, "i");
    close(w, want.0, &format!("width of `{style}`"));
    close(h, want.1, &format!("height of `{style}`"));
    close(pw, 317.0, &format!("container of `{style}`"));
}

#[test]
fn percentage_padding_goes_outside_the_100_percent() {
    // The content box is 317 x 158.5, the 7% padding (22.19 on every side)
    // outside it. rinch was 317 x 181: the padding inside the 317.
    check("max-width:100%; padding:7%", (361.375, 202.875));
}

#[test]
fn percentage_margins_go_outside_the_100_percent() {
    // rinch was 279 x 139: fitted to the 317 less its margins.
    check("max-width:100%; margin:0 6%", (317.0, 158.5));
}

#[test]
fn length_padding_goes_outside_the_100_percent() {
    // rinch was 317 x 168.
    check("max-width:100%; padding:13px 17px", (351.0, 184.5));
}

#[test]
fn padding_margins_and_border_together() {
    // rinch was 292 x 158.
    check(
        "max-width:100%; padding:3%; margin:0 4%; border:3px solid red",
        (342.0, 183.5),
    );
}

#[test]
fn display_inline_block_on_the_picture_changes_nothing() {
    // A replaced element ignores its inner display: rinch was 317 x 181.
    check(
        "display:inline-block; max-width:100%; padding:7%",
        (361.375, 202.875),
    );
}

#[test]
fn border_box_keeps_the_padding_inside() {
    // `box-sizing: border-box`: the 100% is the border box. Right before.
    check(
        "max-width:100%; padding:7%; box-sizing:border-box",
        (317.0, 180.688),
    );
}

#[test]
fn a_block_picture_and_a_percentage_width_were_already_right() {
    check(
        "display:block; max-width:100%; padding:7%",
        (361.375, 202.875),
    );
    check("display:block; max-width:100%; margin:0 6%", (317.0, 158.5));
    check("width:100%; padding:7%", (361.375, 202.875));
}

#[test]
fn a_canvas_too() {
    // A 500x100 canvas: content 317 x 63.4, the padding beside it.
    let d = doc(
        "<div style='width:317px'><canvas id=c width=500 height=100 style='max-width:100%; padding:0 7%'></canvas></div>",
    );
    let (w, h, _) = size(&d, "c");
    close(w, 361.328, "canvas width");
    close(h, 63.391, "canvas height");
}

/// `#box` holding the picture, in a 317px block, laid out three times with
/// only the viewport height changed: the box is `want_box` wide and the
/// picture `want` every time.
#[track_caller]
fn check_fitted(box_style: &str, img_style: &str, want_box: f32, want: (f32, f32)) {
    let mut d = doc(&format!(
        "<div style='width:317px'><div id=box style='{box_style}'><img id=i src='{W800}' style='{img_style}'></div></div>"
    ));
    for (round, height) in [600.0, 601.0, 602.0].into_iter().enumerate() {
        d.resolve_layout(800.0, height);
        let what = format!("`{box_style}` round {round}");
        close(size(&d, "box").0, want_box, &format!("{what}: box width"));
        let (w, h, _) = size(&d, "i");
        close(w, want.0, &format!("{what}: picture width"));
        close(h, want.1, &format!("{what}: picture height"));
    }
}

#[test]
fn a_float_around_a_picture_with_margins_keeps_its_width() {
    // The issue's float: rinch shrank it round by round (243 where Chrome
    // gives 300 in a 300px block).
    check_fitted(
        "float:left",
        "max-width:100%; margin:0 6%",
        317.0,
        (317.0, 158.5),
    );
}

#[test]
fn a_float_around_a_padded_picture() {
    check_fitted(
        "float:left",
        "max-width:100%; padding:7%",
        317.0,
        (361.375, 202.875),
    );
}

#[test]
fn an_inline_block_around_a_picture_with_margins() {
    check_fitted(
        "display:inline-block",
        "max-width:100%; margin:0 6%",
        317.0,
        (317.0, 158.5),
    );
}
