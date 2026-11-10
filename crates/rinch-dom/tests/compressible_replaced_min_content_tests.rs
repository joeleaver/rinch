//! A replaced element whose `width` or `max-width` is a percentage is
//! *compressible* (css-sizing-3 §5.2.2): its min-content contribution is
//! computed with those percentages resolved against zero, so it gives a table
//! column, a float or any min-content measure no width of its own. Two shapes
//! did not: a block picture with a **length** `width` beside its percentage
//! `max-width` (Taffy's block algorithm took the length as the box's
//! min-content width before asking its measure), and an **inline** picture
//! (the inline formatting context lined it up at its natural width in every
//! measure, #2398).
//!
//! Every expected number is Chrome 156's for the same markup in a 696px
//! container (the pictures are flat PNGs of the same sizes). Postrinch's
//! mail 033 had the first shape on every picture of a `max-width: 640px`
//! card and was 1,120px taller than in Chrome; mail 030 the second.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// An 800x400 PNG.
const W800: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAyAAAAGQCAIAAADZR5NjAAAF6ElEQVR42u3WMQ0AAAgEMcC/sJeFBhLGVsJN10kKAIA/IwEAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAIMlAQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAGCwDAYAEAGCwAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAMFgAABgsAwGABABgsAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQAYLAAADBYAgMECADBYAAAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgAwWAAAGCwAAIMFAGCwAAAwWAAABgsAwGABAGCwAAAMFgCAwQIAMFgAABgsAACDBQBgsAAAMFgAAAYLAMBgAQBgsAAADBYAgMECAMBgAQAYLAAAgwUAYLAAADBYAAAGCwDAYAEAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAwGABAGCwAAAMFgCAwQIAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAgMECAMBgAQAYLAAAgwUAgMECADBYAAAGCwAAgwUAYLAAAAwWAIDBAgDAYAEAGCwAAIMFAIDBAgAwWAAABgsAAIMFAGCwAAAMFgAABgsAwGABABgsAACDBQCAwQIAMFgAAAYLAACDBQBgsAAADBYAAAYLAMBgAQAYLAAADBYAgMECADBYAAAGCwAAgwUAYLAAAAwWAAAXC4kQBXhhKf6rAAAAAElFTkSuQmCC";
/// An 88x88 PNG.
const W88: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAFgAAABYCAIAAAD+96djAAAAgElEQVR42u3QMQEAAAgDoGn/YItlCj+IwLQNySoQIUKECBEiRIgQIUKECBEiRIgQIUKECBEiRIgQIUKECBEiRIgQIUKECESIECFChAgRIkSIECFChAgRIkSIECFChAgRIkSIECFChAgRIkSIECFCBCJEiBAhQoQIESJEiBAh4tMBNUMDCPpXMBMAAAAASUVORK5CYII=";

const CSS: &str =
    "body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(696.0, 800.0);
    d
}

/// The border box of `#id`, in document coordinates.
fn abs(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n: NodeId = d.query_selector(&format!("#{id}")).expect(id);
    let l = d.tree.get(n.0).unwrap().layout;
    let (mut x, mut y, mut cur) = (0.0, 0.0, Some(n.0));
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        y += node.layout.y;
        cur = node.parent;
    }
    (x, y, l.width, l.height)
}

fn close(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() < 0.6, "{what}: {got}, Chrome {want}");
}

#[test]
fn a_length_width_picture_does_not_widen_a_max_width_table() {
    // Postrinch mail 033's card: a centred `width: 100%; max-width: 640px`
    // table whose `width=640` cell holds a padded `width: 800px;
    // max-width: 100%` picture. Chrome: the table is 640 wide at x = 28 and
    // the picture scales to 568x284. rinch made the table 872 wide (the
    // picture's 800 plus the padding) and drew the picture at 800x400.
    let d = doc(&format!(
        "<center><table id=t cellpadding=0 cellspacing=0 style='border-collapse:separate; width:100%; margin:0 auto; max-width:640px'><tr>         <td width=640 style='vertical-align:top; box-sizing:border-box'><div style='padding:18px 36px'>         <img id=i src='{W800}' width=800 style='display:block; max-width:100%; width:800px; height:auto'>         </div></td></tr></table></center>"
    ));
    let t = abs(&d, "t");
    close(t.0, 28.0, "table x");
    close(t.2, 640.0, "table width");
    close(t.3, 320.0, "table height");
    let i = abs(&d, "i");
    close(i.0, 64.0, "picture x");
    close(i.2, 568.0, "picture width");
    close(i.3, 284.0, "picture height");
}

#[test]
fn a_length_width_picture_keeps_a_tables_declared_width() {
    // Chrome: 300 wide, the picture 300x150 (rinch: 800 and 800x400).
    let d = doc(&format!(
        "<table id=t cellpadding=0 cellspacing=0 style='width:300px'><tr><td>         <img id=i src='{W800}' style='display:block; width:800px; max-width:100%'></td></tr></table>"
    ));
    close(abs(&d, "t").2, 300.0, "table width");
    let i = abs(&d, "i");
    close(i.2, 300.0, "picture width");
    close(i.3, 150.0, "picture height");
}

#[test]
fn a_compressed_pictures_length_min_width_still_holds() {
    // The percentage resolves against zero, the length `min-width` stays:
    // Chrome gives the picture's column 100 and the other column the rest
    // (rinch: 400 and 296).
    let d = doc(&format!(
        "<table cellpadding=0 cellspacing=0><tr>         <td id=a><img id=i src='{W800}' style='display:block; width:400px; min-width:100px; max-width:100%'></td>         <td id=b style='width:900px'>y</td></tr></table>"
    ));
    close(abs(&d, "a").2, 100.0, "picture column");
    let i = abs(&d, "i");
    close(i.2, 100.0, "picture width");
    close(i.3, 50.0, "picture height");
    let b = abs(&d, "b");
    close(b.0, 100.0, "other column x");
    close(b.2, 596.0, "other column width");
}

#[test]
fn an_inline_picture_does_not_widen_its_table() {
    // An inline picture between two words, in a 300px table. Chrome: the
    // table stays 300 and the picture is 300x150 on a line of its own
    // (rinch: 800 and 800x400).
    let d = doc(&format!(
        "<table id=t cellpadding=0 cellspacing=0 style='width:300px'><tr><td>         a <img id=i src='{W800}' style='width:800px; max-width:100%'> b</td></tr></table>"
    ));
    close(abs(&d, "t").2, 300.0, "table width");
    let i = abs(&d, "i");
    close(i.2, 300.0, "picture width");
    close(i.3, 150.0, "picture height");
}

#[test]
fn an_inline_picture_keeps_its_cells_declared_width_issue_2398() {
    // #2398's repro: an 88px logo with `max-width: 100%` in a `width: 45pt`
    // cell beside a `width: 200pt` one. Chrome: the cell is 60 wide and the
    // logo 60x60; in a 400px table the excess is shared 60 : 266.66, so
    // 73.47. rinch: 88 and 88x88, then 99.
    let d = doc(&format!(
        "<table style='border-collapse:collapse'><tr>         <td id=a style='padding:0; width:45pt; vertical-align:top'><img id=i src='{W88}' style='max-width:100%'></td>         <td id=b style='padding:0; width:200pt'>x</td></tr></table>         <table style='border-collapse:collapse; width:400px'><tr>         <td id=c style='padding:0; width:45pt; vertical-align:top'><img id=j src='{W88}' style='max-width:100%'></td>         <td style='padding:0; width:200pt'>x</td></tr></table>"
    ));
    close(abs(&d, "a").2, 60.0, "#a width");
    let i = abs(&d, "i");
    close(i.2, 60.0, "#i width");
    close(i.3, 60.0, "#i height");
    close(abs(&d, "b").0, 60.0, "#b x");
    close(abs(&d, "c").2, 73.47, "#c width");
    close(abs(&d, "j").2, 73.47, "#j width");
}
