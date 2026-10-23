//! Adjacent text runs share the wrapper of a mark that continues across them
//! (#1833): inline code split by bold, italic or a link is drawn as one pill.
//!
//! Each run used to have a wrapper chain of its own, so `code` split by bold
//! was `<code>a</code><code><strong>b</strong></code>`, and since an inline
//! element's padding and margin take room on its line (#1426), two pills.
//!
//! The pixel oracle: the code wrapper has a margin, so two `<code>` elements
//! leave a gap in the red background; the topmost red row (the top padding,
//! above every glyph of the x-height letters used) is one run of red for one
//! pill and two for two. The caret oracle is #1832's: the caret is drawn where
//! a typed `Z` lands.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Attrs, Mark, Node, Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

fn page() -> Page {
    let handle = crate::editor::create_editor();
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; background: white");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 600px; height: 500px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif; background: white",
        );
        root.append_child(&editor);
        let style = scope.create_element("style");
        let css = scope.create_text(
            "[data-pm-editor] p { margin: 0; padding: 0; } \
             [data-pm-editor] code { font: inherit; padding: 2px 7px; margin: 0 4px; \
               border-radius: 0; background: rgb(255, 0, 0); color: black; } \
             [data-pm-editor] a { color: black; text-decoration: none; }",
        );
        style.append_child(&css);
        root.append_child(&style);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    page
}

fn mark(handle: &crate::editor::EditorHandle, name: &str, attrs: Attrs) -> Mark {
    let st = handle.state();
    Mark::new(st.schema().mark_type(name).unwrap().clone(), attrs)
}

/// A paragraph `"xx" code("ace") code+inner("oo") code("ace") "xx"`, built
/// directly: the starter kit's `code` excludes bold, italic and links, but an
/// app's schema need not, and the view draws what the model holds.
fn load(p: &Page, inner: Mark) {
    let st = p.handle.state();
    let s = st.schema();
    let code = mark(&p.handle, "code", Attrs::default());
    let mut both = vec![code.clone(), inner];
    both.sort_by(|a, b| a.type_name().cmp(b.type_name()));
    let runs: Vec<Node> = vec![
        s.text("xx").unwrap(),
        s.text_with_marks("ace", vec![code.clone()]).unwrap(),
        s.text_with_marks("oo", both).unwrap(),
        s.text_with_marks("ace", vec![code]).unwrap(),
        s.text("xx").unwrap(),
    ];
    let para = s
        .branch("paragraph", Fragment::from_children(runs))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
    p.handle.load_doc(doc);
}

impl Page {
    /// The runs of pure red on the topmost row holding any.
    fn top_red_runs(&mut self) -> usize {
        let (px, w, h) = self.app.build_pixels(1.0, VP, false);
        let red = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            px[i] == 255 && px[i + 1] == 0 && px[i + 2] == 0
        };
        let Some(y) = (0..h).find(|&y| (0..w).any(|x| red(x, y))) else {
            return 0;
        };
        let mut runs = 0;
        let mut inside = false;
        for x in 0..w {
            let r = red(x, y);
            if r && !inside {
                runs += 1;
            }
            inside = r;
        }
        runs
    }

    fn code_elements(&self) -> usize {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-editor] code").len()
    }

    /// The painted caret's `(x, y)`, then a `Z` typed there: where its glyph
    /// is drawn, `(x, top of its line)` (#1832's oracle).
    fn caret_and_typed(&mut self) -> ((f32, f32), (f32, f32)) {
        let caret = {
            let doc = self.app.doc.as_ref().unwrap().borrow();
            let id = doc
                .query_selector_all("[data-pm-caret]")
                .into_iter()
                .next()
                .expect("a caret");
            let b = painted_element_box(&doc.tree, id.0);
            (b.0, b.1)
        };
        let head = self.handle.selection().head();
        assert!(self.handle.insert_text("Z"));
        idle(&mut self.app);
        let block = self.handle.caret_address(head).unwrap().0;
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (bx, by, ..) = painted_element_box(&doc.tree, block);
        let il = doc.tree.nodes[block].text_layout.as_ref().unwrap();
        let byte = il.text_content.find('Z').expect("the typed Z");
        let cluster = parley::layout::Cluster::from_byte_index(&il.layout, byte).unwrap();
        let x = bx + cluster.visual_offset().unwrap();
        let y = by + cluster.line().metrics().block_min_coord;
        (caret, (x, y))
    }
}

#[test]
fn code_split_by_bold_italic_or_a_link_is_one_pill() {
    for (name, attrs) in [
        ("bold", Attrs::default()),
        ("italic", Attrs::default()),
        ("link", Attrs::default().with("href", "https://x.test/")),
    ] {
        let mut p = page();
        let inner = mark(&p.handle, name, attrs);
        load(&p, inner);
        idle(&mut p.app);
        assert_eq!(p.code_elements(), 1, "{name}: one <code>");
        assert_eq!(p.top_red_runs(), 1, "{name}: one pill");
    }
}

/// The positive control for the pixel oracle: two code runs with plain text
/// between them are two pills, and the scan sees two.
#[test]
fn two_code_runs_apart_are_two_pills() {
    let mut p = page();
    let html = "<p>xx<code>ace</code>oo<code>ace</code>xx</p>";
    assert!(p.handle.load_html(html));
    idle(&mut p.app);
    assert_eq!(p.code_elements(), 2);
    assert_eq!(p.top_red_runs(), 2);
}

/// At each seam inside the shared pill, and at its outer edges, the caret is
/// drawn where a typed `Z` lands: the shared `<code>`'s edges are not at the
/// seam, so the caret is not drawn at the far end of the merged wrapper.
#[test]
fn the_caret_at_a_seam_in_a_shared_pill_is_where_z_lands() {
    // 0[p 1 xx 3 ace 6 oo 8 ace 11 xx 13]
    for pos in [3, 6, 8, 11] {
        let mut p = page();
        let inner = mark(&p.handle, "bold", Attrs::default());
        load(&p, inner);
        idle(&mut p.app);
        p.handle.set_selection(Selection::cursor(Pos(pos)));
        idle(&mut p.app);
        let (caret, typed) = p.caret_and_typed();
        assert!(
            (caret.0 - typed.0).abs() <= 1.0 && (caret.1 - typed.1).abs() <= 3.0,
            "pos {pos}: the caret is drawn at {caret:?}, a typed Z lands at {typed:?}"
        );
    }
}
