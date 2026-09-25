//! Scenarios for rinch's instruction-count benchmarks (`benches/hot_paths.rs`).
//!
//! Every scenario is split in two, because Callgrind counts only what runs
//! inside the benchmark function:
//!
//! - a **setup** (`setup_*`) that builds the document, lays it out, paints it
//!   and performs the operation once already where that warms a cache or a
//!   lazily-initialised static — so the measured run is the *steady-state*
//!   cost a user pays on the second hover, not the first; and
//! - an **operation** (`op_*`) that does exactly one thing and returns the
//!   fixture, so dropping the document is not counted either.
//!
//! Text is set in the bundled `Inter-Regular.ttf` under its own family name,
//! and every line box is declared (`line-height`, fixed paddings). Nothing a
//! scenario draws or measures resolves through the host's font set. The
//! system collection is still *loaded* (it is built with every font context),
//! but that happens in setup.
//!
//! That does **not** make a count portable between machines. The same
//! scenarios on this repository's CI runner and on a developer workstation
//! gave 111.2M and 147.6M instructions for `resize_1px`, and 40.7M and 53.5M for
//! the keyed reorder, while most other benchmarks agreed to within 0.1%: the
//! toolchain, glibc (its `memcpy`/`memmove` variant is chosen per CPU) and the
//! CPU all move the count. Compare two counts only when one machine produced
//! both, which is what the CI job does.
//!
//! Sizes are chosen for Valgrind, which runs code roughly 50x slower than
//! native: a list is 500 rows where the whole scenario is one row's worth of
//! work (hover, toggle, text), and fewer where the operation is O(rows)
//! (the keyed reorder).
//!
//! These run under `cargo bench -p rinch-bench` (Gungraun + Valgrind); see
//! `docs/src/guide/performance.md#ci-regression-job`.

pub mod alloc;

use std::cell::RefCell;
use std::rc::Rc;

use rinch::app::RinchApp;
use rinch::font::AppFont;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::reactive::{Memo, Signal};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_platform::PlatformEvent;

/// The bundled face every scenario sets its text in.
pub const FACE: &[u8] = include_bytes!("../../rinch-dom/assets/fonts/Inter-Regular.ttf");
/// The family name `Inter-Regular.ttf` registers under.
const FAMILY: &str = "Inter";

/// The window every scenario is laid out and painted at.
pub const VP: (f32, f32) = (800.0, 600.0);
const SIZE: (u32, u32) = (800, 600);

/// Rows in the rinch-dom list and the shell scroller.
pub const LIST_ROWS: usize = 500;
/// Rows in the keyed `for` list.
pub const FOR_ROWS: usize = 200;
/// Rows in the memo-selection list.
pub const MEMO_ROWS: usize = 40;
/// Warm pointer moves per measured run.
pub const WARM_MOVES: usize = 50;

fn new_document() -> RinchDocument {
    use parley::fontique::Blob;
    alloc::reserve();
    let mut doc = RinchDocument::new();
    doc.font_cx
        .collection
        .register_fonts(Blob::new(std::sync::Arc::new(FACE)), None);
    doc
}

/// Run the measured operation under the bench arena ([`alloc`]): the only
/// thing a benchmark function calls.
#[inline(always)]
pub fn measure<T>(fixture: T, op: impl FnOnce(T) -> T) -> T {
    alloc::begin();
    let out = op(fixture);
    alloc::end();
    out
}

// ── rinch-dom: the 500-row list ────────────────────────────────────────────

const LIST_CSS: &str = "
    body { margin: 0; font-family: Inter; font-size: 16px; line-height: 20px; }
    .list { display: flex; flex-direction: column; }
    .row { display: block; padding: 2px; }
    .row:hover { background-color: rgb(200, 0, 0); }
    .row.sel { background-color: rgb(0, 0, 200); }
    .chip { display: inline-block; padding: 2px; }
";

/// `body > div.list (flex column) > LIST_ROWS × div.row`, each row holding
/// `span > text`, a `display: contents` wrapper around a second text (what
/// `rsx!` emits for every `{|| …}` and `if`/`for`/`match`) and an
/// `inline-block` chip: the shape `perf_counter_baselines.rs` pins, at 500
/// rows.
pub struct ListFixture {
    pub doc: RinchDocument,
    pub list: NodeId,
    pub rows: Vec<NodeId>,
    pub texts: Vec<NodeId>,
    pub painter: TinySkiaPainter,
}

impl ListFixture {
    fn layout(&mut self) {
        self.doc.resolve_layout(VP.0, VP.1);
    }

    fn paint(&mut self) {
        let doc = &mut self.doc;
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut self.painter,
            1.0,
            VP,
            &mut doc.font_cx,
            &mut doc.layout_cx,
        );
    }

    fn hover(&mut self, row: Option<NodeId>) {
        let mut changed = false;
        self.doc.update_hover(row.map(|r| r.0), &mut changed);
    }

    fn new_row(&mut self, label: &str) -> NodeId {
        let doc = &mut self.doc;
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let t = doc.create_text(label);
        doc.append_child(row, t);
        row
    }
}

fn build_list() -> ListFixture {
    let mut doc = new_document();
    doc.load_css(LIST_CSS);
    let body = doc.body();
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    let mut rows = Vec::with_capacity(LIST_ROWS);
    let mut texts = Vec::with_capacity(LIST_ROWS);
    for i in 0..LIST_ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let span = doc.create_element("span");
        let t = doc.create_text(&format!("Row number {i} with some text"));
        doc.append_child(span, t);
        doc.append_child(row, span);
        let w = doc.create_element("span");
        doc.set_attribute(w, "style", "display: contents");
        let t2 = doc.create_text(&format!(" ({i})"));
        doc.append_child(w, t2);
        doc.append_child(row, w);
        texts.push(t2);
        let chip = doc.create_element("span");
        doc.set_attribute(chip, "class", "chip");
        let t3 = doc.create_text("chip");
        doc.append_child(chip, t3);
        doc.append_child(row, chip);
        doc.append_child(list, row);
        rows.push(row);
    }
    doc.append_child(body, list);
    let mut f = ListFixture {
        doc,
        list,
        rows,
        texts,
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.layout();
    f.layout();
    f.paint();
    f
}

/// The list, with row 5 hovered and un-hovered once (warms the hover path).
pub fn setup_hover() -> ListFixture {
    let mut f = build_list();
    let r = f.rows[5];
    f.hover(Some(r));
    f.layout();
    f.hover(None);
    f.layout();
    f
}

/// `:hover` on one row (a colour-only rule), then the frame's layout pass.
pub fn op_hover(mut f: ListFixture) -> ListFixture {
    let r = f.rows[10];
    f.hover(Some(r));
    f.layout();
    f
}

/// The list, with row 5's class toggled on and off once.
pub fn setup_class_toggle() -> ListFixture {
    let mut f = build_list();
    let r = f.rows[5];
    f.doc.set_attribute(r, "class", "row sel");
    f.layout();
    f.doc.set_attribute(r, "class", "row");
    f.layout();
    f
}

/// `.row` → `.row.sel` (colour-only) on one row, then layout.
pub fn op_class_toggle(mut f: ListFixture) -> ListFixture {
    let r = f.rows[10];
    f.doc.set_attribute(r, "class", "row sel");
    f.layout();
    f
}

/// The list, after one row was appended and laid out.
pub fn setup_append_row() -> ListFixture {
    let mut f = build_list();
    let row = f.new_row("warm-up row");
    let list = f.list;
    f.doc.append_child(list, row);
    f.layout();
    f
}

/// Append one row to the list, then layout.
pub fn op_append_row(mut f: ListFixture) -> ListFixture {
    let row = f.new_row("new row");
    let list = f.list;
    f.doc.append_child(list, row);
    f.layout();
    f
}

/// The list, after one row was removed and laid out.
pub fn setup_remove_row() -> ListFixture {
    let mut f = build_list();
    let (list, r) = (f.list, f.rows[5]);
    f.doc.remove_child(list, r);
    f.layout();
    f
}

/// Remove one row from the middle of the list, then layout.
pub fn op_remove_row(mut f: ListFixture) -> ListFixture {
    let (list, r) = (f.list, f.rows[LIST_ROWS / 2]);
    f.doc.remove_child(list, r);
    f.layout();
    f
}

/// The list, resized 1px and back once.
pub fn setup_resize() -> ListFixture {
    let mut f = build_list();
    f.doc.resolve_layout(VP.0 + 1.0, VP.1);
    f.layout();
    f
}

/// A 1px viewport resize: layout at the new width (no rule reads it).
pub fn op_resize(mut f: ListFixture) -> ListFixture {
    f.doc.resolve_layout(VP.0 + 1.0, VP.1);
    f
}

/// The list, after one text node's content was changed once.
pub fn setup_set_text() -> ListFixture {
    let mut f = build_list();
    let t = f.texts[5];
    f.doc.set_text_content(t, " (warm)");
    f.layout();
    f
}

/// `set_text_content` on one row's wrapped text, then layout.
pub fn op_set_text(mut f: ListFixture) -> ListFixture {
    let t = f.texts[10];
    f.doc.set_text_content(t, " (changed text)");
    f.layout();
    f
}

// ── rinch-dom: the component library's stylesheet, under a closed Drawer ───

/// The 500-row list inside a hand-built **closed** `Drawer`
/// (`div.rinch-drawer__root.rinch-drawer__root--hidden > div.rinch-drawer`),
/// with the default theme's CSS and `rinch_components::generate_component_css()`
/// loaded ahead of the list's own sheet — what every app built with the
/// `components` feature carries.
///
/// The other list benchmarks load only [`LIST_CSS`], so they cannot see what a
/// component-library selector costs. This one exists because PR #929 first
/// added `*::before` / `*::after` rules under the closed overlays: rinch-dom
/// matches pseudo-element rules with no ancestor bloom filter (#935), so a rule
/// whose rightmost compound is universal is tried against every element's
/// ancestor chain, and its review measured +71% of style instructions under a
/// closed drawer (+10% with no overlay at all) while every perf-counter
/// baseline — which count cascades, not selector work — stayed identical.
pub struct DrawerFixture {
    pub list: ListFixture,
    /// The drawer root, whose class opens and closes it.
    pub root: NodeId,
}

const DRAWER_CLOSED: &str = "rinch-drawer__root rinch-drawer__root--hidden";
const DRAWER_OPEN: &str = "rinch-drawer__root";

fn build_drawer_list() -> DrawerFixture {
    let mut doc = new_document();
    doc.load_css(&rinch_theme::generate_theme_css(
        &rinch_theme::Theme::default(),
    ));
    doc.load_css(&rinch_components::generate_component_css());
    doc.load_css(LIST_CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", DRAWER_CLOSED);
    let panel = doc.create_element("div");
    doc.set_attribute(panel, "class", "rinch-drawer rinch-drawer--left");
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    let mut rows = Vec::with_capacity(LIST_ROWS);
    for i in 0..LIST_ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let span = doc.create_element("span");
        let t = doc.create_text(&format!("Row number {i} with some text"));
        doc.append_child(span, t);
        doc.append_child(row, span);
        doc.append_child(list, row);
        rows.push(row);
    }
    doc.append_child(panel, list);
    doc.append_child(root, panel);
    doc.append_child(body, root);
    let mut list = ListFixture {
        doc,
        list,
        rows,
        texts: Vec::new(),
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    list.layout();
    list.layout();
    list.paint();
    DrawerFixture { list, root }
}

impl DrawerFixture {
    fn set_open(&mut self, open: bool) {
        let class = if open { DRAWER_OPEN } else { DRAWER_CLOSED };
        self.list.doc.set_attribute(self.root, "class", class);
        self.list.layout();
    }
}

/// The drawer list, opened and closed once (warms both cascades).
pub fn setup_drawer_toggle() -> DrawerFixture {
    let mut f = build_drawer_list();
    f.set_open(true);
    f.set_open(false);
    f
}

/// Open the drawer and lay out, then close it and lay out: each re-cascades
/// the whole 500-row subtree against the full component stylesheet.
pub fn op_drawer_toggle(mut f: DrawerFixture) -> DrawerFixture {
    f.set_open(true);
    f.set_open(false);
    f
}

// ── rinch-dom: an absolute panel moved by its insets ──────────────────────

/// The 500-row list plus an absolutely positioned panel (a `FloatingPanel`
/// drag's shape) in a `position: relative` wrapper after it.
pub struct InsetFixture {
    pub list: ListFixture,
    pub panel: NodeId,
}

/// The list and the panel, after one warm-up move laid out.
pub fn setup_inset_move() -> InsetFixture {
    let mut list = build_list();
    let doc = &mut list.doc;
    let body = doc.body();
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "position: relative; height: 0");
    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; left: 10px; top: 10px; width: 200px; height: 120px; \
         box-shadow: 0 4px 12px rgb(0, 0, 0)",
    );
    let t = doc.create_text("Panel");
    doc.append_child(panel, t);
    doc.append_child(wrapper, panel);
    doc.append_child(body, wrapper);
    list.layout();
    list.doc
        .set_styles(panel, &[("left", "20px"), ("top", "15px")]);
    list.layout();
    InsetFixture { list, panel }
}

/// One drag step: `left` and `top` through `set_styles` (the inset fast
/// path, #280 / #277), then layout.
pub fn op_inset_move(mut f: InsetFixture) -> InsetFixture {
    f.list
        .doc
        .set_styles(f.panel, &[("left", "40px"), ("top", "25px")]);
    f.list.layout();
    f
}

// ── rinch-dom: a full software paint ───────────────────────────────────────

const LOREM: &str = "The quick brown fox jumps over the lazy dog while a sphinx of black \
    quartz judges my vow; pack my box with five dozen liquor jugs, then 0123456789.";

/// A page of 40 wrapped paragraphs and the painter that already painted it.
pub struct PaintFixture {
    pub doc: RinchDocument,
    pub painter: TinySkiaPainter,
}

impl PaintFixture {
    fn paint(&mut self) {
        let doc = &mut self.doc;
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut self.painter,
            1.0,
            VP,
            &mut doc.font_cx,
            &mut doc.layout_cx,
        );
    }
}

/// The text page, laid out and painted once, so the glyph cache is **warm**:
/// the measured paint rasterises no glyph (a cold cache is a first frame,
/// dominated by rasterisation, and would hide a regression in the walk).
pub fn setup_full_paint() -> PaintFixture {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; font-size: 14px; line-height: 18px; \
         color: rgb(30, 30, 40); }} p {{ margin: 0 0 2px 0; }}"
    ));
    let mut html = String::new();
    for i in 0..40 {
        html.push_str(&format!("<p>{i}: {LOREM}</p>"));
    }
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let mut f = PaintFixture {
        doc,
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// Paint the whole page with the software painter, glyph cache warm.
pub fn op_full_paint(mut f: PaintFixture) -> PaintFixture {
    f.paint();
    f
}

// ── rinch (shell): a RinchApp on the software painter ──────────────────────

fn mount(component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    alloc::reserve();
    let mut app = RinchApp::new(component);
    app.register_app_font(AppFont::new(FACE));
    app.mount_component(VP.0, VP.1);
    frame(&mut app);
    app
}

/// One redraw, as `RinchRuntime::paint` performs it.
fn frame(app: &mut RinchApp) {
    app.resolve_and_repaint(VP.0, VP.1);
    let _ = app.build_pixels(1.0, SIZE, false);
    let _ = app.end_perf_frame();
}

fn pointer_move(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
}

fn style_element(scope: &mut RenderScope, css: &str) -> NodeHandle {
    let style = scope.create_element("style");
    let t = scope.create_text(css);
    style.append_child(&t);
    style
}

/// A shell app plus whatever signal a scenario drives.
pub struct ShellFixture<S> {
    pub app: RinchApp,
    pub state: S,
}

const SCROLLER_CSS: &str = "
    body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; }
    .scroller { height: 400px; overflow-y: auto; }
    .srow { height: 20px; }
    .srow:hover { background-color: rgb(200, 0, 0); }
";

/// A 400px scroller of `LIST_ROWS` 20px rows (each `div > span > text`),
/// scrolled to the middle, with a `:hover` rule on the rows: the shape
/// `perf_stats_tests.rs` pins the pointer-move path on.
fn mount_scroller() -> RinchApp {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = style_element(scope, SCROLLER_CSS);
        root.append_child(&style);
        let scroller = scope.create_element("div");
        scroller.set_attribute("class", "scroller");
        for i in 0..LIST_ROWS {
            let row = scope.create_element("div");
            row.set_attribute("class", "srow");
            let span = scope.create_element("span");
            let t = scope.create_text(&format!("row {i}"));
            span.append_child(&t);
            row.append_child(&span);
            scroller.append_child(&row);
        }
        root.append_child(&scroller);
        *out2.borrow_mut() = Some(scroller.clone());
        root
    });
    let scroller = out.borrow().clone().expect("mounted");
    scroller.set_scroll_top(4000.0);
    frame(&mut app);
    app
}

/// The scroller, with the pointer already over a row and the hit cache warm.
pub fn setup_pointer_move_warm() -> RinchApp {
    let mut app = mount_scroller();
    pointer_move(&mut app, 50.0, 105.0);
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    frame(&mut app);
    pointer_move(&mut app, 51.0, 105.0);
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    app
}

/// `WARM_MOVES` pointer moves inside one row (so hover does not change and
/// nothing re-lays out), each followed by `AboutToWait` as the shell's
/// coalescer delivers them.
pub fn op_pointer_move_warm(mut app: RinchApp) -> RinchApp {
    for i in 0..WARM_MOVES {
        pointer_move(&mut app, 52.0 + i as f32, 106.0);
        app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    }
    app
}

/// The scroller, freshly laid out and painted: the hit cache is cold.
pub fn setup_pointer_move_cold() -> RinchApp {
    let mut app = mount_scroller();
    // Warm the code path (and anything lazily initialised on it) on a first
    // move, then lay out again so the next move finds the cache cold.
    pointer_move(&mut app, 50.0, 105.0);
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    frame(&mut app);
    pointer_move(&mut app, 50.0, 300.0);
    frame(&mut app);
    app
}

/// The first pointer move after a layout: it computes the subtree extents
/// the pruned hit test consults, and moves hover to a new row.
pub fn op_pointer_move_cold(mut app: RinchApp) -> RinchApp {
    pointer_move(&mut app, 50.0, 105.0);
    app
}

/// The scroller with the pointer over one row and that frame painted.
pub fn setup_hover_frame() -> RinchApp {
    let mut app = mount_scroller();
    pointer_move(&mut app, 50.0, 105.0);
    frame(&mut app);
    pointer_move(&mut app, 50.0, 145.0);
    frame(&mut app);
    app
}

/// The whole hover frame: a move onto another row, then layout and a
/// **partial** software repaint of the two rows' damage.
pub fn op_hover_frame(mut app: RinchApp) -> RinchApp {
    pointer_move(&mut app, 50.0, 185.0);
    frame(&mut app);
    app
}

/// A keyed `for` over `FOR_ROWS` items.
pub fn setup_keyed_reorder() -> ShellFixture<Signal<Vec<u32>>> {
    let items: Rc<RefCell<Option<Signal<Vec<u32>>>>> = Rc::new(RefCell::new(None));
    let items2 = items.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = style_element(
            scope,
            "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; }
             .item { padding: 1px; }",
        );
        root.append_child(&style);
        let list = scope.create_element("div");
        root.append_child(&list);
        let items = Signal::new((0..FOR_ROWS as u32).collect::<Vec<_>>());
        *items2.borrow_mut() = Some(items);
        rinch_core::for_each_dom_typed(
            scope,
            &list,
            move || items.get(),
            |i: &u32| i.to_string(),
            |i: u32, scope: &mut RenderScope| {
                let row = scope.create_element("div");
                row.set_attribute("class", "item");
                let t = scope.create_text(&format!("item {i}"));
                row.append_child(&t);
                row
            },
        );
        root
    });
    let items = items.borrow().expect("mounted");
    // Warm the reconcile once (a rotation) and paint it.
    items.update(|v| v.rotate_left(1));
    frame(&mut app);
    ShellFixture { app, state: items }
}

/// Reverse the list (every row but one moves; none is re-rendered), then
/// lay out and repaint.
pub fn op_keyed_reorder(mut f: ShellFixture<Signal<Vec<u32>>>) -> ShellFixture<Signal<Vec<u32>>> {
    f.state.update(|v| v.reverse());
    frame(&mut f.app);
    f
}

/// `MEMO_ROWS` rows, each with a `Memo<bool>` over one shared selection
/// signal and an effect that writes the row's class from it.
pub fn setup_memo_selection() -> ShellFixture<Signal<usize>> {
    let sel: Rc<RefCell<Option<Signal<usize>>>> = Rc::new(RefCell::new(None));
    let sel2 = sel.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = style_element(
            scope,
            "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; }
             .sel { background-color: rgb(0, 0, 200); }",
        );
        root.append_child(&style);
        let selected = Signal::new(0usize);
        *sel2.borrow_mut() = Some(selected);
        for i in 0..MEMO_ROWS {
            let row = scope.create_element("div");
            let t = scope.create_text(&format!("row {i}"));
            row.append_child(&t);
            let is_sel = Memo::new(move || selected.get() == i);
            let target = row.clone();
            scope.create_effect(move || {
                target.set_attribute("class", if is_sel.get() { "row sel" } else { "row" });
            });
            root.append_child(&row);
        }
        root
    });
    let selected = sel.borrow().expect("mounted");
    selected.set(1);
    frame(&mut app);
    ShellFixture {
        app,
        state: selected,
    }
}

/// Move the selection: the write, and the effect flush it triggers (40 memos
/// re-evaluated, the two rows whose answer changed re-run their effect).
pub fn op_memo_selection(f: ShellFixture<Signal<usize>>) -> ShellFixture<Signal<usize>> {
    f.state.set(2);
    f
}
