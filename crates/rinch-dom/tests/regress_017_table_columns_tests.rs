//! Regression 017 (Postrinch): a 3-column product grid in a mail drew its
//! columns at the widths the pictures happened to arrive in, sometimes two
//! columns at a picture's natural width with the third pushed out of view.
//!
//! Each grid row is nine cells, `16 | picture | 16` three times, the pictures
//! `width: 100%` inside a `display: block` link, the rows between spacer rows
//! of one empty `<td height=16>`. A table's auto columns share the free width
//! by their cells' max-content widths, and a `width: 100%` picture's
//! max-content contribution is its natural width (its percentage is cyclic
//! there, so it counts as `auto`). The pictures are all 284 wide, so Chrome
//! splits the 524px left after the 16px cells equally: 174.66px a column.
//!
//! rinch lined a percentage-sized atomic inline up at the width it had last
//! been resolved to in a max-content measure. A picture that arrived after
//! the first layout was measured at its natural width, while the ones before
//! it had been resolved to the narrow column they were in, so each column's
//! share depended on the order the pictures loaded in, and stayed so (#2136's
//! record kept it from being measured again at the same width).
//!
//! Every expected number is Chrome 156's for the same markup (headless, at
//! 720 wide, body margin 0), with the pictures in the page from the start.
//! The pictures arrive here one at a time through a loader that answers only
//! once the test releases a source, a layout after each: deterministic, and
//! the order Postrinch's loads happened to finish in.
use rinch_core::dom::DomDocument;
use rinch_core::image::{ImageLoadResult, ImageLoader};
use rinch_dom::RinchDocument;
use std::sync::{Arc, Mutex};

/// The sources a [`Gated`] loader may answer.
type Released = Arc<Mutex<Vec<String>>>;

/// A loader that answers a source with a 284x442 picture once it has been
/// released, and not before.
struct Gated(Released);

impl ImageLoader for Gated {
    fn load(&self, src: &str) -> ImageLoadResult {
        let start = std::time::Instant::now();
        while !self.0.lock().unwrap().iter().any(|s| s == src) {
            if start.elapsed().as_secs() > 30 {
                return ImageLoadResult::Failed("never released".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        ImageLoadResult::Loaded(picture())
    }
}

/// A 284x442 PNG, the size of Postrinch's product photos.
fn picture() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(284, 442, image::Rgb([200, 60, 60]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

const W: f32 = 720.0;
const H: f32 = 2000.0;

/// The grid, its nine pictures `q0`..`q8` (row by row). With `order`, they
/// arrive one at a time in that order, a layout after each; with `None`,
/// they are decoded before the first layout.
fn grid(order: Option<&[usize]>) -> RinchDocument {
    let released: Released = Arc::default();
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0; font: 16px/20px sans-serif }");
    d.tree.image_loader = Some(Arc::new(Gated(released.clone())));
    if order.is_none() {
        for i in 0..9 {
            let img = rinch_dom::image_cache::decode_image_bytes(&picture()).unwrap();
            d.tree.image_cache.insert_decoded(format!("q{i}"), img);
        }
    }
    let side = "<td width=16><div></div></td>";
    let spacer = "<tr><td height=16></td></tr>";
    let mut rows = String::new();
    for r in 0..3 {
        rows.push_str(spacer);
        rows.push_str("<tr>");
        for k in 0..3 {
            rows.push_str(&format!(
                "{side}<td id=c{r}{k}><a href=\"#\" \
                 style=\"display:block;overflow:hidden;border-radius:2px\">\
                 <img id=i{r}{k} src=\"q{}\" style=\"width:100%\"></a></td>{side}",
                r * 3 + k
            ));
        }
        rows.push_str("</tr>");
    }
    rows.push_str(spacer);
    let body = d.body();
    d.set_inner_html(
        body,
        &format!(
            "<div style=\"width:620px;max-width:620px\"><table id=t cellspacing=0 \
             cellpadding=0 border=0 width=\"100%\">{rows}</table></div>"
        ),
    );
    d.resolve_layout(W, H);
    for &i in order.unwrap_or(&[]) {
        let src = format!("q{i}");
        released.lock().unwrap().push(src.clone());
        let start = std::time::Instant::now();
        while d.tree.image_cache.get(&src).is_none() {
            assert!(start.elapsed().as_secs() < 30, "{src} never landed");
            std::thread::sleep(std::time::Duration::from_millis(2));
            d.resolve_layout(W, H);
        }
        d.resolve_layout(W, H);
    }
    d
}

/// The border box of `#id`, in page coordinates.
fn abs(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let l = d.tree.get(n.0).unwrap().layout;
    let (mut x, mut y) = (0.0, 0.0);
    let mut cur = Some(n.0);
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        y += node.layout.y;
        cur = node.parent;
    }
    (x, y, l.width, l.height)
}

/// Chrome's grid: each picture cell at its x, 174.66 wide, its picture as
/// wide and 271.8 tall (442/284 of that), the table 894.5 tall. Within a
/// pixel, as rinch rounds the boxes it lays out.
fn assert_chromes_grid(d: &RinchDocument, case: &str) {
    const X: [f32; 3] = [16.0, 222.66, 429.31];
    const COL: f32 = 174.66;
    let near = |got: f32, want: f32, what: &str| {
        assert!(
            (got - want).abs() <= 1.0,
            "{case}: {what} is {got}, Chrome {want}"
        );
    };
    for r in 0..3 {
        for (k, x) in X.iter().enumerate() {
            let (cx, _, cw, _) = abs(d, &format!("c{r}{k}"));
            near(cx, *x, &format!("c{r}{k} x"));
            near(cw, COL, &format!("c{r}{k} width"));
            let (_, _, iw, ih) = abs(d, &format!("i{r}{k}"));
            near(iw, COL, &format!("i{r}{k} width"));
            near(ih, COL * 442.0 / 284.0, &format!("i{r}{k} height"));
        }
    }
    near(abs(d, "t").3, 894.5, "table height");
}

/// Run `f` on a thread with room for a debug build's layout recursion.
fn big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

/// The control: every picture decoded before the first layout.
#[test]
fn pictures_decoded_up_front_split_the_columns_equally() {
    big_stack(|| assert_chromes_grid(&grid(None), "decoded up front"));
}

/// Row by row: rinch drew the columns 85, 155 and 284 wide.
#[test]
fn pictures_arriving_in_order_split_the_columns_equally() {
    big_stack(|| {
        assert_chromes_grid(&grid(Some(&[0, 1, 2, 3, 4, 5, 6, 7, 8])), "in order");
    });
}

/// The other way round: rinch drew 284, 155 and 85, the third column past
/// the 620px wrapper.
#[test]
fn pictures_arriving_in_reverse_split_the_columns_equally() {
    big_stack(|| {
        assert_chromes_grid(&grid(Some(&[8, 7, 6, 5, 4, 3, 2, 1, 0])), "reverse");
    });
}

/// Column by column: rinch drew 90, 150 and 284.
#[test]
fn pictures_arriving_column_by_column_split_the_columns_equally() {
    big_stack(|| {
        assert_chromes_grid(&grid(Some(&[0, 3, 6, 1, 4, 7, 2, 5, 8])), "by column");
    });
}
