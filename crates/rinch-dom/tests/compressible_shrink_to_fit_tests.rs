//! A compressible replaced element (css-sizing-3 §5.2.2: a picture whose
//! `width` or `max-width` is a percentage) in a box sized to fit its content.
//! Its min-content contribution is computed with those percentages resolved
//! against zero, so a shrink-to-fit box around it takes
//! `min(max-content, max(min-content, room))`: the room. Email wraps
//! pictures like this (`<img width=800 style="max-width: 100%">`) often.
//!
//! - #2424: a float, an `inline-block`, a `width: fit-content` block and an
//!   absolutely positioned box holding such a picture in a 300px container
//!   were 800 wide (the picture's length `width`, or its natural width).
//! - #2431: a flex or grid item picture with a length `width` gave a
//!   min-content measure 800.
//! - #2432: an inline picture with a percentage padding gave a min-content
//!   measure its natural width.
//!
//! Every expected number is Chrome 157's for the same markup in a 696px
//! viewport (the picture is an 800x400 PNG).
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
    d.set_inner_html(body, &html.replace("W800", W800));
    d.resolve_layout(696.0, 800.0);
    d
}

/// The border-box width and height of `#id`.
fn size(d: &RinchDocument, id: &str) -> (f32, f32) {
    let n: NodeId = d.query_selector(&format!("#{id}")).expect(id);
    let l = d.tree.get(n.0).unwrap().layout;
    (l.width, l.height)
}

fn close(d: &RinchDocument, id: &str, want: (f32, f32)) {
    let got = size(d, id);
    assert!(
        (got.0 - want.0).abs() < 0.6 && (got.1 - want.1).abs() < 0.6,
        "#{id}: {got:?}, Chrome {want:?}"
    );
}

/// A block picture with a length `width` beside its percentage `max-width`.
const PICTURE: &str = "<img id=i src='W800' style='display:block; width:800px; max-width:100%'>";

#[test]
fn a_float_fits_a_length_width_picture_to_the_room_issue_2424() {
    // rinch: 800x400 (the picture's `width`).
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='float:left'>{PICTURE}</div></div>"
    ));
    close(&d, "f", (300.0, 150.0));
    close(&d, "i", (300.0, 150.0));
}

#[test]
fn an_inline_block_fits_a_length_width_picture_to_the_room() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='display:inline-block'>{PICTURE}</div></div>"
    ));
    close(&d, "f", (300.0, 150.0));
    close(&d, "i", (300.0, 150.0));
}

#[test]
fn a_fit_content_block_fits_a_length_width_picture_to_the_room() {
    let d = doc(&format!(
        "<div style='width:300px'><div id=f style='width:fit-content'>{PICTURE}</div></div>"
    ));
    close(&d, "f", (300.0, 150.0));
    close(&d, "i", (300.0, 150.0));
}

#[test]
fn an_absolute_box_fits_a_length_width_picture_to_the_room() {
    let d = doc(&format!(
        "<div style='width:300px; height:160px; position:relative'>         <div id=f style='position:absolute; left:0; top:0'>{PICTURE}</div></div>"
    ));
    close(&d, "f", (300.0, 150.0));
    close(&d, "i", (300.0, 150.0));
}

#[test]
fn a_float_fits_a_natural_width_picture_to_the_room() {
    // `width: auto`: the picture's natural 800 was the float's width.
    let d = doc(
        "<div style='width:300px'><div id=f style='float:left'>         <img id=i src='W800' style='display:block; max-width:100%'></div></div>",
    );
    close(&d, "f", (300.0, 150.0));
    close(&d, "i", (300.0, 150.0));
}

#[test]
fn a_float_fits_an_inline_picture_to_the_room() {
    // #2424's second shape: the picture on a line (rinch: 800x405).
    let d = doc(
        "<div style='width:300px'><div id=f style='float:left'>         <img id=i src='W800' width=800 style='max-width:100%'></div></div>",
    );
    close(&d, "f", (300.0, 155.0));
    close(&d, "i", (300.0, 150.0));
}

#[test]
fn a_flex_item_picture_gives_no_min_content_width_issue_2431() {
    // A `width: min-content` row: Chrome 0x0 (rinch: 800x400). In a float
    // in 300px the row takes the room.
    let d = doc(
        "<div id=c style='display:flex; width:min-content'><img id=i src='W800' style='width:800px; max-width:100%'></div>         <div style='width:300px'><div id=f style='float:left'><div style='display:flex'><img id=j src='W800' style='width:800px; max-width:100%'></div></div></div>",
    );
    close(&d, "c", (0.0, 0.0));
    close(&d, "i", (0.0, 0.0));
    close(&d, "f", (300.0, 150.0));
    close(&d, "j", (300.0, 150.0));
}

#[test]
fn a_grid_item_picture_gives_no_min_content_width() {
    // A `width: min-content` grid, a grid in a float in 300px, and a
    // `min-content` track beside a `1fr` one (rinch: 800 each, and the
    // `1fr` track 8).
    let d = doc(
        "<div id=c style='display:grid; width:min-content'><img id=i src='W800' style='width:800px; max-width:100%'></div>         <div style='width:300px'><div id=f style='float:left'><div style='display:grid'><img id=j src='W800' style='width:800px; max-width:100%'></div></div></div>         <div style='clear:both; display:grid; grid-template-columns:min-content 1fr; width:300px'><img id=k src='W800' style='width:800px; max-width:100%'><div id=x>x</div></div>",
    );
    close(&d, "c", (0.0, 0.0));
    close(&d, "i", (0.0, 0.0));
    close(&d, "f", (300.0, 150.0));
    close(&d, "j", (300.0, 150.0));
    assert!(
        size(&d, "k").0.abs() < 0.6,
        "#k: {:?}, Chrome 0",
        size(&d, "k")
    );
    assert!(
        (size(&d, "x").0 - 300.0).abs() < 0.6,
        "#x: {:?}, Chrome 300",
        size(&d, "x")
    );
}

#[test]
fn an_inline_picture_with_percentage_padding_gives_no_min_content_width_issue_2432() {
    // A percentage padding resolves against zero in a min-content measure,
    // a `calc()` one counts its length. Chrome: the float and the table
    // keep 300 (rinch: 792 and 806 for the floats, 792 for the table).
    // (The picture's own box in 300px, 330 wide in Chrome, is not pinned
    // here: rinch lays it out 300 wide in a plain 300px block too, #2442.)
    let d = doc(
        "<div style='width:300px'><div id=f style='float:left'>         <img src='W800' style='max-width:100%; padding:5%'></div></div>         <div style='clear:both; width:300px'><div id=g style='float:left'>         <img src='W800' style='max-width:100%; padding:calc(2% + 3px)'></div></div>         <table id=t cellpadding=0 cellspacing=0 style='clear:both; width:300px'><tr><td>         a <img src='W800' style='max-width:100%; padding:5%'> b</td></tr></table>",
    );
    assert!(
        (size(&d, "f").0 - 300.0).abs() < 0.6,
        "#f: {:?}, Chrome 300",
        size(&d, "f")
    );
    assert!(
        (size(&d, "g").0 - 300.0).abs() < 0.6,
        "#g: {:?}, Chrome 300",
        size(&d, "g")
    );
    assert!(
        (size(&d, "t").0 - 300.0).abs() < 0.6,
        "#t: {:?}, Chrome 300",
        size(&d, "t")
    );
}
