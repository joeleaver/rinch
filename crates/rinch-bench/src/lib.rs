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

/// Links in `pointer_move_inline`'s paragraph.
pub const INLINE_LINKS: usize = 1000;

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
    build_list_with(LIST_CSS)
}

fn build_list_with(css: &str) -> ListFixture {
    let mut doc = new_document();
    doc.load_css(css);
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

/// The list with every row running an infinite `transform` spin, and a class
/// on the list that changes the colour every row inherits.
const ANIMATED_LIST_CSS: &str = "
    @keyframes spin { to { transform: rotate(360deg); } }
    .row { animation: spin 1s linear infinite; }
    .list.alt { color: rgb(0, 0, 200); }
";

/// The list with all 500 rows animated, and its colour class toggled on and
/// off once.
pub fn setup_animated_list_color() -> ListFixture {
    let mut f = build_list_with(&format!("{LIST_CSS}{ANIMATED_LIST_CSS}"));
    let list = f.list;
    f.doc.set_attribute(list, "class", "list alt");
    f.layout();
    f.doc.set_attribute(list, "class", "list");
    f.layout();
    f
}

/// A colour change on the list, which every row inherits while it runs a
/// `to { transform: … }` animation, then layout: 500 cascades of an animated
/// node. The spin's stops read neither `color` nor `font-size`, so none of
/// those cascades extracts its keyframe stops again (review of #2049: before
/// the stops were keyed on what they read, each one did).
pub fn op_animated_list_color(mut f: ListFixture) -> ListFixture {
    let list = f.list;
    f.doc.set_attribute(list, "class", "list alt");
    f.layout();
    f
}

const FLEX_LABEL_CSS: &str = "
    body { margin: 0; font-family: Inter; font-size: 16px; line-height: 20px; }
    .list { display: flex; flex-direction: column; }
    .frow { display: flex; padding: 2px; }
    .frow:hover { color: rgb(200, 0, 0); }
";

/// `LIST_ROWS` × `div.frow` (`display: flex`), each holding its text
/// **directly**: a flex item's text leaf, painted from the layout its measure
/// built. The hover rule is colour-only, which paint applies to a leaf itself
/// (#904) — its first cut ran a Taffy compute per hover here instead.
fn build_flex_labels() -> ListFixture {
    let mut doc = new_document();
    doc.load_css(FLEX_LABEL_CSS);
    let body = doc.body();
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    let mut rows = Vec::with_capacity(LIST_ROWS);
    let mut texts = Vec::with_capacity(LIST_ROWS);
    for i in 0..LIST_ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "frow");
        let t = doc.create_text(&format!("Row number {i} with some text"));
        doc.append_child(row, t);
        doc.append_child(list, row);
        rows.push(row);
        texts.push(t);
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

/// The flex-label list, with row 5 hovered and un-hovered once.
pub fn setup_flex_label_hover() -> ListFixture {
    let mut f = build_flex_labels();
    let r = f.rows[5];
    f.hover(Some(r));
    f.layout();
    f.paint();
    f.hover(None);
    f.layout();
    f.paint();
    f
}

/// A colour-only `:hover` on one flex row whose text is a leaf, then the
/// frame's layout pass and paint.
pub fn op_flex_label_hover(mut f: ListFixture) -> ListFixture {
    let r = f.rows[10];
    f.hover(Some(r));
    f.layout();
    f.paint();
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
/// The other list benchmarks load only `LIST_CSS`, so they cannot see what a
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

// ── rinch-dom: a large grid of cards (#2136) ───────────────────────────────

/// Cards in [`CardGridFixture`]: #2136's asset library held 430.
pub const GRID_CARDS: usize = 430;

/// #2136's shape: a scroller holding `repeat(auto-fill, minmax(108px, 1fr))`
/// with implicit `auto` rows, each item behind a `display: contents` wrapper
/// (as an rsx `for` builds it) and a flex column of a thumbnail with an
/// absolutely positioned `max-width: 100%` pill in its corner, a name, and a
/// flex row of a badge, a spacer and a "N uses" count.
pub struct CardGridFixture {
    pub doc: RinchDocument,
    /// Each card's count text.
    pub counts: Vec<NodeId>,
    /// A text outside the grid.
    pub outside: NodeId,
}

/// The card grid, laid out and settled, with both operations run once.
pub fn setup_card_grid() -> CardGridFixture {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; }
         .scroll { width: 380px; height: 600px; overflow-y: auto; }
         .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(108px, 1fr));
                 gap: 8px; padding: 4px; }
         .card { display: flex; flex-direction: column; min-width: 0; overflow: hidden; }
         .thumb { position: relative; height: 80px; }
         .tl { position: absolute; top: 4px; left: 4px; }
         .pill { display: inline-flex; max-width: 100%; height: 18px; padding: 0 6px; }
         .name { min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
         .meta { display: flex; align-items: center; gap: 4px; min-width: 0; }
         .spacer { flex: 1 1 auto; }",
    );
    let el = |doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str| {
        let e = doc.create_element(tag);
        if !class.is_empty() {
            doc.set_attribute(e, "class", class);
        }
        doc.append_child(parent, e);
        e
    };
    let text = |doc: &mut RinchDocument, parent: NodeId, s: &str| {
        let t = doc.create_text(s);
        doc.append_child(parent, t);
        t
    };
    let body = doc.body();
    let scroll = el(&mut doc, body, "div", "scroll");
    let grid = el(&mut doc, scroll, "div", "grid");
    let mut counts = Vec::with_capacity(GRID_CARDS);
    for i in 0..GRID_CARDS {
        let wrap = el(&mut doc, grid, "div", "");
        doc.set_attribute(wrap, "style", "display: contents");
        let card = el(&mut doc, wrap, "div", "card");
        let thumb = el(&mut doc, card, "div", "thumb");
        let tl = el(&mut doc, thumb, "span", "tl");
        let pill = el(&mut doc, tl, "span", "pill");
        text(&mut doc, pill, "3d");
        let name = el(&mut doc, card, "span", "name");
        text(&mut doc, name, &format!("Asset {i}.glb"));
        let meta = el(&mut doc, card, "span", "meta");
        let badge = el(&mut doc, meta, "span", "");
        text(&mut doc, badge, "v1");
        el(&mut doc, meta, "span", "spacer");
        counts.push(text(&mut doc, meta, "3 uses"));
    }
    let other = el(&mut doc, body, "div", "");
    let outside = text(&mut doc, other, "elsewhere");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let mut f = CardGridFixture {
        doc,
        counts,
        outside,
    };
    f = op_card_count_edit(f);
    f = op_card_grid_unrelated_edit(f);
    f
}

/// One card's count goes from "3 uses" to "4 uses" (or back), then layout.
/// The grid's track sizing asks every item again; every unchanged one should
/// be answered from its cache.
pub fn op_card_count_edit(mut f: CardGridFixture) -> CardGridFixture {
    let t = f.counts[GRID_CARDS / 2];
    let next = match &f.doc.tree.nodes[t.0].kind {
        rinch_dom::node::NodeKind::Text(s) if s.content == "4 uses" => "3 uses",
        _ => "4 uses",
    };
    f.doc.set_text_content(t, next);
    f.doc.resolve_layout(VP.0, VP.1);
    f
}

/// A text outside the grid changes, then layout: nothing in the grid moves,
/// and none of its 430 percentage pills is laid out again (#2136).
pub fn op_card_grid_unrelated_edit(mut f: CardGridFixture) -> CardGridFixture {
    let next = match &f.doc.tree.nodes[f.outside.0].kind {
        rinch_dom::node::NodeKind::Text(s) if s.content == "elsewhere" => "elsewhere, changed",
        _ => "elsewhere",
    };
    f.doc.set_text_content(f.outside, next);
    f.doc.resolve_layout(VP.0, VP.1);
    f
}

// ── rinch-dom: a table with a width of its own ────────────────────────────

/// Rows in [`TableFixture`].
pub const TABLE_ROWS: usize = 200;

/// A `width: 380px` table of [`TABLE_ROWS`] three-cell rows ("Item N", a
/// pill, "3 uses"), optionally headed by one cell spanning the three columns:
/// the shape a spanning cell (or a caption) used to send through a detached
/// layout of the whole table on every edit (#2327).
pub struct TableFixture {
    pub doc: RinchDocument,
    /// Each row's count text.
    pub counts: Vec<NodeId>,
}

fn table_fixture(spanning: bool) -> TableFixture {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; }
         table { width: 380px; border-spacing: 0 } td { padding: 0 }
         .pill { display: inline-flex; max-width: 100% }",
    );
    let el = |doc: &mut RinchDocument, parent: NodeId, tag: &str| {
        let e = doc.create_element(tag);
        doc.append_child(parent, e);
        e
    };
    let text = |doc: &mut RinchDocument, parent: NodeId, s: &str| {
        let t = doc.create_text(s);
        doc.append_child(parent, t);
        t
    };
    let body = doc.body();
    let table = el(&mut doc, body, "table");
    let tbody = el(&mut doc, table, "tbody");
    if spanning {
        let tr = el(&mut doc, tbody, "tr");
        let td = el(&mut doc, tr, "td");
        doc.set_attribute(td, "colspan", "3");
        text(&mut doc, td, "All assets");
    }
    let mut counts = Vec::with_capacity(TABLE_ROWS);
    for i in 0..TABLE_ROWS {
        let tr = el(&mut doc, tbody, "tr");
        let a = el(&mut doc, tr, "td");
        text(&mut doc, a, &format!("Item {i}"));
        let b = el(&mut doc, tr, "td");
        let pill = el(&mut doc, b, "span");
        doc.set_attribute(pill, "class", "pill");
        text(&mut doc, pill, "3d");
        let c = el(&mut doc, tr, "td");
        counts.push(text(&mut doc, c, "3 uses"));
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let f = TableFixture { doc, counts };
    op_table_cell_edit(f)
}

/// The table, laid out and settled, with the edit run once.
pub fn setup_table_cell_edit() -> TableFixture {
    table_fixture(false)
}

/// The same, headed by a spanning cell.
pub fn setup_table_cell_edit_spanning() -> TableFixture {
    table_fixture(true)
}

/// One row's count goes from "3 uses" to "4 uses" (or back), then layout.
pub fn op_table_cell_edit(mut f: TableFixture) -> TableFixture {
    let t = f.counts[TABLE_ROWS / 2];
    let next = match &f.doc.tree.nodes[t.0].kind {
        rinch_dom::node::NodeKind::Text(s) if s.content == "4 uses" => "3 uses",
        _ => "4 uses",
    };
    f.doc.set_text_content(t, next);
    f.doc.resolve_layout(VP.0, VP.1);
    f
}

// ── rinch-dom: rows with an absolute badge ─────────────────────────────────

/// A 400px-high scroller of `LIST_ROWS` 20px rows, each holding a 4px leaf
/// and one absolutely positioned badge — the shape of a list of rows with an
/// unread dot or a close button.
pub struct BadgeFixture {
    pub doc: RinchDocument,
    pub scroller: NodeId,
    /// The leaf of row 7, whose margin the relayout changes.
    pub leaf: NodeId,
}

/// - `direct`: each row is `position: relative` and the badge is its child.
///   Taffy resolves it correctly by itself, so none of the passes that
///   resolve an absolute box against a containing block Taffy does not know
///   (`rinch_dom`'s `out_of_flow`, #386) may look at it — on a layout or on a
///   scroll. The review of PR #1409 measured +12% on this relayout when they
///   did.
/// - `ancestor`: the same row with a static wrapper around the badge, so the
///   row is a containing block Taffy does not know.
/// - `icb`: no row is positioned, so every badge resolves against the
///   initial containing block from inside the scroller.
fn build_badges(variant: &str) -> BadgeFixture {
    let mut doc = new_document();
    let row_position = if variant == "icb" {
        ""
    } else {
        "position: relative;"
    };
    doc.load_css(&format!(
        ".s {{ height: 400px; width: 500px; overflow: auto; }}
         .row {{ {row_position} height: 20px; }}
         .leaf {{ height: 4px; }}
         .badge {{ position: absolute; right: 5px; top: 2px; width: 10px; height: 10px; }}"
    ));
    let body = doc.body();
    let scroller = doc.create_element("div");
    doc.set_attribute(scroller, "class", "s");
    let mut leaf = None;
    for i in 0..LIST_ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let l = doc.create_element("div");
        doc.set_attribute(l, "class", "leaf");
        doc.append_child(row, l);
        if i == 7 {
            leaf = Some(l);
        }
        let holder = if variant == "ancestor" {
            let w = doc.create_element("div");
            doc.append_child(row, w);
            w
        } else {
            row
        };
        let badge = doc.create_element("div");
        doc.set_attribute(badge, "class", "badge");
        doc.append_child(holder, badge);
        doc.append_child(scroller, row);
    }
    doc.append_child(body, scroller);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let mut f = BadgeFixture {
        doc,
        scroller,
        leaf: leaf.expect("row 7 exists"),
    };
    // Warm the operations once — the bars too, as the paint after that
    // scroll has read them by the time the pointer next moves.
    f.doc.set_style(f.leaf, "margin-left", "1px");
    f.doc.resolve_layout(VP.0, VP.1);
    f.doc.set_scroll_top(f.scroller, 7.0);
    f = op_badge_extents(f);
    f
}

/// Badges that are children of their own positioned row.
pub fn setup_badges_direct() -> BadgeFixture {
    build_badges("direct")
}

/// Badges under a static wrapper in their positioned row.
pub fn setup_badges_ancestor() -> BadgeFixture {
    build_badges("ancestor")
}

/// Badges with no positioned ancestor, inside the scroller.
pub fn setup_badges_icb() -> BadgeFixture {
    build_badges("icb")
}

/// One leaf's margin changes, then layout: no containing block changes size
/// and no badge moves.
pub fn op_badge_relayout(mut f: BadgeFixture) -> BadgeFixture {
    f.doc.set_style(f.leaf, "margin-left", "2px");
    f.doc.resolve_layout(VP.0, VP.1);
    f
}

/// The scroller scrolls by one notch's worth; no layout runs.
pub fn op_badge_scroll(mut f: BadgeFixture) -> BadgeFixture {
    f.doc.set_scroll_top(f.scroller, 60.0);
    f
}

/// The bars of the scroller and of the body, as every paint and every
/// pointer move's hit test asks for them (`scrollbars` runs `content_extents`,
/// which counts each badge in the scroller its containing block sits in, #770).
pub fn op_badge_extents(f: BadgeFixture) -> BadgeFixture {
    let body = f.doc.body();
    std::hint::black_box(rinch_dom::paint::scrollbar::scrollbars(
        &f.doc.tree,
        f.scroller.0,
        1.0,
    ));
    std::hint::black_box(rinch_dom::paint::scrollbar::scrollbars(
        &f.doc.tree,
        body.0,
        1.0,
    ));
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
    full_paint_page("")
}

/// The text page of [`setup_full_paint`] on a white page, painted with
/// subpixel (LCD) text (#2286 stage 3), RGB through FreeType's default
/// filter: each glyph blitted per channel. Warm, as the grey one.
pub fn setup_full_paint_subpixel() -> PaintFixture {
    subpixel_policy();
    full_paint_page("background: #fff;")
}

/// The thread's text policy set to subpixel (LCD) text, RGB, lcddefault,
/// for the measured process (each benchmark is its own process).
fn subpixel_policy() {
    use rinch_dom::paint::text_rasterization::{
        LcdFilter, SubpixelOrder, SystemTextSettings, TextAntialias, TextRasterization,
        set_thread_policy,
    };
    set_thread_policy(
        TextRasterization::default()
            .antialias(TextAntialias::Subpixel)
            .subpixel_order(Some(SubpixelOrder::Rgb))
            .lcd_filter(Some(LcdFilter::Default))
            .resolve(&SystemTextSettings::default()),
    );
}

fn full_paint_page(body_css: &str) -> PaintFixture {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; font-size: 14px; line-height: 18px; \
         color: rgb(30, 30, 40); {body_css} }} p {{ margin: 0 0 2px 0; }}"
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

/// Words in [`setup_long_paragraph_paint`]'s paragraph.
pub const PARAGRAPH_WORDS: usize = 3000;

/// One paragraph of [`PARAGRAPH_WORDS`] words in a 200px scroller, scrolled
/// into its middle, laid out and painted once (glyph cache warm): one node
/// whose lines are nearly all clipped away, which paint drew whole until it
/// culled lines (#2314).
fn long_paragraph_page() -> RinchDocument {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; font-size: 14px; line-height: 18px; \
         color: rgb(30, 30, 40); }} .s {{ height: 200px; overflow-y: auto; }} \
         p {{ margin: 0; }}"
    ));
    let words: String = (0..PARAGRAPH_WORDS).map(|i| format!("word{i} ")).collect();
    let body = doc.body();
    doc.set_inner_html(body, &format!("<div class=s><p>{words}</p></div>"));
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.query_selector(".s").expect("the scroller");
    doc.set_scroll_top(s, 4000.0);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

/// The long paragraph page (`long_paragraph_page`) with the software painter.
pub fn setup_long_paragraph_paint() -> PaintFixture {
    let mut f = PaintFixture {
        doc: long_paragraph_page(),
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// The long paragraph page (`long_paragraph_page`) with the Vello painter (the
/// encoding alone).
pub fn setup_vello_long_paragraph() -> VelloPaintFixture {
    let mut f = VelloPaintFixture {
        doc: long_paragraph_page(),
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: None,
    };
    f.paint();
    f
}

/// A game HUD of [`HUD_NUMERALS`] numerals (#2719): each a gradient
/// background clipped to its text (`background-clip: text`, `color:
/// transparent`), a dark `-webkit-text-stroke` under the fill (`paint-order:
/// stroke fill`) and a hard `text-shadow`, the style Braynard Racing's HUD
/// draws its lap times, positions and speed in. Laid out and painted once
/// (glyph cache warm).
fn hud_page() -> RinchDocument {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; }} \
         .n {{ display: inline-block; margin: 6px 10px; font-size: 41px; line-height: 48px; \
               background: linear-gradient(#fff6b0 8%, #ffb000 46%, #e04000 54%, #7a1000 93%); \
               -webkit-background-clip: text; background-clip: text; color: transparent; \
               -webkit-text-stroke: 4.5px #1a1a2e; paint-order: stroke fill; \
               text-shadow: 2px 3px 0 #000; }}"
    ));
    let labels = ["1:23.456", "P3/12", "213", "LAP 7", "+0.84"];
    let html: String = (0..HUD_NUMERALS)
        .map(|i| format!("<span class=n>{}</span>", labels[i % labels.len()]))
        .collect();
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

/// Numerals on [`setup_hud_paint`]'s page.
pub const HUD_NUMERALS: usize = 24;

/// The HUD page (`hud_page`) with the software painter.
pub fn setup_hud_paint() -> PaintFixture {
    let mut f = PaintFixture {
        doc: hud_page(),
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// The HUD page (`hud_page`) with the Vello painter (the encoding alone).
pub fn setup_vello_hud() -> VelloPaintFixture {
    let mut f = VelloPaintFixture {
        doc: hud_page(),
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: None,
    };
    f.paint();
    f
}

/// The drawer list of [`setup_drawer_toggle`] — 500 rows under a **closed**
/// `Drawer`, with the theme and component stylesheets — laid out and painted
/// once, glyph cache warm. Built closed and never opened: a close holds the
/// root `visible` for its 300ms (`transition: visibility 0s linear 300ms`),
/// and no clock runs here to end it. A closed overlay is `visibility: hidden`
/// with nothing visible in it, so paint and the stacking collector skip it
/// whole (#2137).
pub fn setup_closed_drawer_paint() -> PaintFixture {
    let DrawerFixture { list, .. } = build_drawer_list();
    let mut f = PaintFixture {
        doc: list.doc,
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

/// A page painted by the **Vello** painter, the GPU window's: the encoding
/// walk alone, no GPU. CI's other paint benchmarks measure the software
/// painter only (#1894 review), and the split-layer path (#1865) lives in
/// this one: with no GPU surface it must cost nothing, and with one split it
/// routes every draw over the surface.
pub struct VelloPaintFixture {
    pub doc: RinchDocument,
    pub painter: rinch_dom::paint::vello_painter::VelloPainter,
    /// The window the frame is opened for slicing over, when it is.
    pub slicing: Option<peniko::kurbo::Rect>,
}

impl VelloPaintFixture {
    /// Paint, and how many surfaces the frame split.
    fn paint(&mut self) -> usize {
        use rinch_dom::paint::painter::Painter;
        self.painter.reset();
        if let Some(window) = self.slicing {
            self.painter.begin_slicing(window);
        }
        let doc = &mut self.doc;
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut self.painter,
            1.0,
            VP,
            &mut doc.font_cx,
            &mut doc.layout_cx,
        );
        std::hint::black_box(self.painter.take_slices()).len()
    }
}

/// The text page of [`setup_full_paint`], painted by the Vello painter on a
/// frame opened for slicing with no GPU surface in it: the cost of the
/// split-layer bookkeeping where it does nothing.
pub fn setup_vello_paint() -> VelloPaintFixture {
    let PaintFixture { doc, .. } = setup_full_paint();
    let mut f = VelloPaintFixture {
        doc,
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: Some(peniko::kurbo::Rect::new(
            0.0,
            0.0,
            SIZE.0 as f64,
            SIZE.1 as f64,
        )),
    };
    f.paint();
    f
}

/// A 640x360 GPU surface split out of the scene, 70 boxes painted over it
/// (rounded, some translucent) and 70 beside it: every draw after the split
/// is routed. The frame stays installed for the measured paint.
pub fn setup_vello_paint_split() -> VelloPaintFixture {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; } .b { position: absolute; width: 30px; height: 20px; \
         border-radius: 4px; background: rgba(20, 30, 40, 0.6); }",
    );
    let mut html = String::from(
        "<div data-render-surface=\"1\" style=\"position: absolute; left: 80px; top: 60px; \
         width: 640px; height: 360px\"></div>",
    );
    for i in 0..140 {
        let (x, y) = if i < 70 {
            (90 + (i % 10) * 62, 70 + (i / 10) * 48)
        } else {
            (10 + (i % 10) * 70, 450 + ((i - 70) / 10) * 20)
        };
        html.push_str(&format!(
            "<div class=\"b\" style=\"left: {x}px; top: {y}px\"></div>"
        ));
    }
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    rinch_dom::paint::set_surface_gpu_images(Some(std::collections::HashMap::from([(
        1,
        peniko::ImageData {
            data: peniko::Blob::new(std::sync::Arc::new([])),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::AlphaPremultiplied,
            width: 640,
            height: 360,
        },
    )])));
    let mut f = VelloPaintFixture {
        doc,
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: Some(peniko::kurbo::Rect::new(
            0.0,
            0.0,
            SIZE.0 as f64,
            SIZE.1 as f64,
        )),
    };
    f.paint();
    f
}

/// The text page of [`setup_vello_paint`] on a frame not opened for slicing:
/// a window with no GPU surface, which must cost what it did before #1865
/// (#1899 review).
pub fn setup_vello_paint_unsliced() -> VelloPaintFixture {
    let mut f = setup_vello_paint();
    f.slicing = None;
    f.paint();
    f
}

/// [`setup_vello_paint_unsliced`] with subpixel (LCD) text (#2286 stage 3):
/// every glyph a mask-mode-2 image. Warm.
pub fn setup_vello_paint_subpixel() -> VelloPaintFixture {
    subpixel_policy();
    setup_vello_paint_unsliced()
}

/// The page of [`setup_vello_paint_split`] on a frame not opened for slicing:
/// the surface drawn in the scene, as before #1865 (#1899 review).
/// 140 boxes of 50x30 with a 6px radius, each styled by `style`, painted by
/// the Vello painter with no slicing (#2321: rounded borders and shadows are
/// curved paths the painter flattens).
fn vello_rounded_boxes(style: &str) -> VelloPaintFixture {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; background: #fff; }} .b {{ position: absolute; width: 50px; \
         height: 30px; border-radius: 6px; background: #fff; {style} }}"
    ));
    let mut html = String::new();
    for i in 0..140 {
        let (x, y) = (12 + (i % 10) * 76, 12 + (i / 10) * 40);
        html.push_str(&format!(
            "<div class=\"b\" style=\"left: {x}px; top: {y}px\"></div>"
        ));
    }
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let mut f = VelloPaintFixture {
        doc,
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: None,
    };
    f.paint();
    f
}

/// `vello_rounded_boxes` with a 1px solid border: a filled ring each.
pub fn setup_vello_paint_rounded_border() -> VelloPaintFixture {
    vello_rounded_boxes("border: 1px solid rgb(200, 200, 200);")
}

/// `vello_rounded_boxes` with a blurred shadow (eight soft layers each).
pub fn setup_vello_paint_soft_shadow() -> VelloPaintFixture {
    vello_rounded_boxes("box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);")
}

/// `vello_rounded_boxes` with a hard offset shadow.
pub fn setup_vello_paint_hard_shadow() -> VelloPaintFixture {
    vello_rounded_boxes("box-shadow: 2px 2px 0 rgba(0, 0, 0, 0.3);")
}

/// `vello_rounded_boxes` as cards: a border and two blurred shadows.
pub fn setup_vello_paint_cards() -> VelloPaintFixture {
    vello_rounded_boxes(
        "border: 1px solid rgb(200, 200, 200); \
         box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12), 0 1px 2px rgba(0, 0, 0, 0.24);",
    )
}

pub fn setup_vello_paint_split_unsliced() -> VelloPaintFixture {
    let mut f = setup_vello_paint_split();
    f.slicing = None;
    f.paint();
    f
}

/// A game HUD drawn with filters that move pixels (#2719): a 300x96 panel
/// whose glow is a `blur(6px)` box behind it, a speed readout in 52px bold
/// numerals under `drop-shadow(2px 3px 2px)`, and a row of six 40px item
/// slots each under `drop-shadow(0 2px 3px)`. Eight offscreen rasters a
/// frame, each the part of its box that shows grown by the filter's reach.
fn hud_filter_page(doc: &mut RinchDocument) {
    doc.load_css(&format!(
        "body {{ margin: 0; background: rgb(20, 28, 40); font-family: {FAMILY}; }}          #glow {{ position: absolute; left: 40px; top: 40px; width: 300px; height: 96px;            background: rgba(80, 160, 255, 0.5); filter: blur(6px); }}          #panel {{ position: absolute; left: 40px; top: 40px; width: 300px; height: 96px;            background: rgba(10, 14, 22, 0.85); border-radius: 10px; }}          #speed {{ position: absolute; left: 24px; top: 14px; font-size: 52px; line-height: 64px;            font-weight: bold; color: #fc3; filter: drop-shadow(2px 3px 2px rgba(0, 0, 0, 0.6)); }}          .slot {{ position: absolute; top: 160px; width: 40px; height: 40px; border-radius: 8px;            background: rgb(60, 70, 90); filter: drop-shadow(0 2px 3px rgba(0, 0, 0, 0.7)); }}"
    ));
    let mut html = String::from(
        "<div id=\"glow\"></div><div id=\"panel\"><div id=\"speed\">218 km/h</div></div>",
    );
    for i in 0..6 {
        html.push_str(&format!(
            "<div class=\"slot\" style=\"left: {}px\"></div>",
            40 + i * 52
        ));
    }
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
}

/// The HUD of `hud_filter_page` painted by the software painter, warm.
pub fn setup_hud_filter_paint() -> PaintFixture {
    let mut doc = new_document();
    hud_filter_page(&mut doc);
    let mut f = PaintFixture {
        doc,
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// The HUD of `hud_filter_page` painted by the Vello painter (the encoding walk
/// and the filter rasters it draws on the CPU), warm.
pub fn setup_vello_hud_filter_paint() -> VelloPaintFixture {
    let mut doc = new_document();
    hud_filter_page(&mut doc);
    let mut f = VelloPaintFixture {
        doc,
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: None,
    };
    f.paint();
    f
}

/// A game HUD's frosted-glass panel (#2719): a 300x100 rounded panel with
/// `backdrop-filter: blur(10px) saturate(1.4)` and a speed readout in it,
/// over a busy page (stripes and 30 rows of text) — one backdrop a frame:
/// what is behind the panel painted a second time, blurred and saturated.
fn hud_backdrop_page(doc: &mut RinchDocument) {
    hud_backdrop_page_with(doc, true);
}

/// [`hud_backdrop_page`], behind the panel 30 rows of text (`text_rows`) or
/// 30 coloured bars.
fn hud_backdrop_page_with(doc: &mut RinchDocument, text_rows: bool) {
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; font-size: 14px; line-height: 18px; \
           background: repeating-linear-gradient(90deg, rgb(20, 40, 70) 0 9px, rgb(240, 200, 60) 9px 17px); }} \
         #glass {{ position: absolute; left: 60px; top: 70px; width: 300px; height: 100px; \
           border-radius: 14px; background: rgba(255, 255, 255, 0.18); \
           backdrop-filter: blur(10px) saturate(1.4); }} \
         #speed {{ position: absolute; left: 20px; top: 18px; font-size: 52px; line-height: 64px; \
           font-weight: bold; color: #fff; }}"
    ));
    let mut html = String::new();
    for i in 0..30 {
        if text_rows {
            html.push_str(&format!("<p style=\"margin: 0\">{i}: {LOREM}</p>"));
        } else {
            html.push_str(&format!(
                "<div style=\"height: 14px; margin: 0 0 4px {}px; background: rgb({}, 90, 160)\"></div>",
                (i * 37) % 200,
                (i * 53) % 255
            ));
        }
    }
    html.push_str("<div id=\"glass\"><div id=\"speed\">218 km/h</div></div>");
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
}

/// The panel of `hud_backdrop_page` painted by the software painter, warm.
pub fn setup_hud_backdrop_paint() -> PaintFixture {
    let mut doc = new_document();
    hud_backdrop_page(&mut doc);
    let mut f = PaintFixture {
        doc,
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// The panel of `hud_backdrop_page` painted by the Vello painter, warm.
pub fn setup_vello_hud_backdrop_paint() -> VelloPaintFixture {
    let mut doc = new_document();
    hud_backdrop_page(&mut doc);
    let mut f = VelloPaintFixture {
        doc,
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: None,
    };
    f.paint();
    f
}

/// The HUD of `hud_filter_page` as the embed overlay draws it (#2719): every
/// frame a whole-scene Vello paint (`build_scene`), the dirty lists consumed
/// before it, as `build_scene_with` does. The speed readout's text ticks each
/// frame; the glow and the six slots do not.
pub struct HudTickFixture {
    pub doc: RinchDocument,
    pub painter: rinch_dom::paint::vello_painter::VelloPainter,
    pub scale: f64,
    /// The readout's text node.
    pub text: NodeId,
}

impl HudTickFixture {
    /// One frame in the shell's order: the change lists drained, then a
    /// whole-scene paint (`RinchApp::build_scene_with`).
    fn frame(&mut self) {
        use rinch_dom::paint::painter::Painter;
        self.doc.tree.consume_paint_dirty();
        self.painter.reset();
        let doc = &mut self.doc;
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut self.painter,
            self.scale,
            VP,
            &mut doc.font_cx,
            &mut doc.layout_cx,
        );
    }
}

fn hud_tick(scale: f64) -> HudTickFixture {
    let mut doc = new_document();
    hud_filter_page(&mut doc);
    let speed = doc.query_selector_all("#speed")[0];
    let text = doc.get_children(speed)[0];
    let mut f = HudTickFixture {
        doc,
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        scale,
        text,
    };
    // Two frames: the first rasterises everything, the second is warm.
    f.frame();
    f.frame();
    f
}

/// The glass panel of `hud_backdrop_page` as the embed overlay draws it,
/// its readout (inside the panel) ticking: what is behind the panel does not
/// change, so its backdrop is kept (#2719).
fn hud_backdrop_tick(scale: f64, text_rows: bool) -> HudTickFixture {
    let mut doc = new_document();
    hud_backdrop_page_with(&mut doc, text_rows);
    let speed = doc.query_selector_all("#speed")[0];
    let text = doc.get_children(speed)[0];
    let mut f = HudTickFixture {
        doc,
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        scale,
        text,
    };
    f.frame();
    f.frame();
    f
}

/// The ticking glass panel over text at 1x, warm. Every restyle marks every
/// text node paint-dirty (pre-existing), so the text behind the panel
/// invalidates its kept backdrop each frame.
pub fn setup_hud_backdrop_tick_1x() -> HudTickFixture {
    hud_backdrop_tick(1.0, true)
}

/// The ticking glass panel over text at 2x, warm.
pub fn setup_hud_backdrop_tick_2x() -> HudTickFixture {
    hud_backdrop_tick(2.0, true)
}

/// The ticking glass panel over coloured bars at 1x, warm: nothing behind
/// it changes, so its backdrop is drawn from the one kept.
pub fn setup_hud_backdrop_bars_tick_1x() -> HudTickFixture {
    hud_backdrop_tick(1.0, false)
}

/// The same at 2x. It measures below the 1x one, and that is the text, not
/// the backdrop (which is kept at both): the readout's new digit is a glyph
/// the frame has not drawn before, rasterised into the glyph-mask atlas at
/// 52px (1x), while at 104px (2x) it is past the atlas's size limit and Vello
/// draws it as an outline, with nothing to rasterise.
pub fn setup_hud_backdrop_bars_tick_2x() -> HudTickFixture {
    hud_backdrop_tick(2.0, false)
}

/// [`HudTickFixture`] at 1x, warm.
pub fn setup_hud_tick_1x() -> HudTickFixture {
    hud_tick(1.0)
}

/// [`HudTickFixture`] at 2x, warm.
pub fn setup_hud_tick_2x() -> HudTickFixture {
    hud_tick(2.0)
}

/// One frame of the ticking HUD: the readout's text changes, layout, a
/// whole-scene Vello paint, the dirty lists consumed.
pub fn op_hud_tick(mut f: HudTickFixture) -> HudTickFixture {
    f.doc.set_text_content(f.text, "219 km/h");
    f.doc.resolve_layout(VP.0, VP.1);
    f.frame();
    f
}

/// One Vello paint of the fixture's page.
pub fn op_vello_paint(mut f: VelloPaintFixture) -> VelloPaintFixture {
    std::hint::black_box(f.paint());
    f
}

/// 500 small boxes with dashed and dotted borders (#2371, #2372; review of
/// #2391): 1px dashed, 2px dotted (square dots), 3px dashed, 5px dotted
/// (round dots) and a rounded 2px dashed, a hundred of each, laid out and
/// painted once. Every dash and dot is laid out by `paint::border_dashes`
/// and drawn as a fill, so this measures that layout and the path it hands
/// the painter.
fn dashed_borders_page() -> RinchDocument {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; } div { position: absolute; box-sizing: border-box; \
         width: 30px; height: 26px; } \
         .a { border: 1px dashed rgb(40, 40, 40); } \
         .b { border: 2px dotted rgb(40, 40, 40); } \
         .c { border: 3px dashed rgb(40, 40, 40); } \
         .d { border: 5px dotted rgb(40, 40, 40); } \
         .e { border: 2px dashed rgb(40, 40, 40); border-radius: 6px; }",
    );
    let mut html = String::new();
    for i in 0..500 {
        let (x, y) = ((i % 25) * 32, (i / 25) * 29);
        let class = ["a", "b", "c", "d", "e"][i % 5];
        html.push_str(&format!(
            "<div class=\"{class}\" style=\"left: {x}px; top: {y}px\"></div>"
        ));
    }
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

/// 500 small boxes whose dashed top and right sides meet solid bottom and
/// left ones, every side `rgba(40, 40, 40, 0.5)` (review of #2391): each
/// border is painted opaque into a layer of its alpha, as Blink's opacity
/// groups paint it, so its corners do not darken twice. This measures that
/// layer.
fn dashed_translucent_page() -> RinchDocument {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; } div { position: absolute; box-sizing: border-box; \
         width: 30px; height: 26px; border: 3px solid rgba(40, 40, 40, 0.5); \
         border-top-style: dashed; border-right-style: dashed; }",
    );
    let mut html = String::new();
    for i in 0..500 {
        let (x, y) = ((i % 25) * 32, (i / 25) * 29);
        html.push_str(&format!("<div style=\"left: {x}px; top: {y}px\"></div>"));
    }
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

/// `dashed_translucent_page` on the software painter.
pub fn setup_dashed_translucent_paint() -> PaintFixture {
    let mut f = PaintFixture {
        doc: dashed_translucent_page(),
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// 200 cards whose rounded border differs in colour by side (#2375, the
/// review of #2606): `border` is the cards' border. Each corner where two
/// colours meet is split on its diagonal; the rest of the ring is straight
/// runs and same-colour corners.
fn multi_colour_cards_page(border: &str) -> RinchDocument {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; }} div {{ position: absolute; box-sizing: border-box; \
         width: 36px; height: 56px; {border} }}"
    ));
    let mut html = String::new();
    for i in 0..200 {
        let (x, y) = ((i % 20) * 40, (i / 20) * 60);
        html.push_str(&format!("<div style=\"left: {x}px; top: {y}px\"></div>"));
    }
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

/// `multi_colour_cards_page` with one coloured side on a grey border.
pub fn setup_rounded_one_side_paint() -> PaintFixture {
    let mut f = PaintFixture {
        doc: multi_colour_cards_page(
            "border: 2px solid #ccc; border-left-color: #36f; border-radius: 8px",
        ),
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// `multi_colour_cards_page` in four colours, 3px, radius 12.
pub fn setup_rounded_four_colour_paint() -> PaintFixture {
    let mut f = PaintFixture {
        doc: multi_colour_cards_page(
            "border: 3px solid; border-color: #c00 #0a0 #00c #888; border-radius: 12px",
        ),
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// `dashed_borders_page` on the software painter.
pub fn setup_dashed_borders_paint() -> PaintFixture {
    let mut f = PaintFixture {
        doc: dashed_borders_page(),
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// A full window whose background is a 16x16 `url()` image tiled at
/// `background-size: 24px` (1.5x) with `position`, laid out and painted once
/// (the tile's raster is cached, warm). At `0 0` the tile is whole device
/// pixels on the device grid and is filled nearest from a raster of the
/// image's bilinear samples; at a fractional position each pixel is one
/// bilinear sample of the image (#1753, the #2528 review: that is 5x the
/// cost, paid only where a fraction asks for it).
fn tiled_background_paint(position: &str) -> PaintFixture {
    let mut doc = new_document();
    let mut px = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u8 {
        for x in 0..16u8 {
            px.extend_from_slice(&[x * 16, y * 16, (x ^ y) * 16, 255]);
        }
    }
    doc.tree.image_cache.insert_decoded(
        "tile.png".into(),
        rinch_dom::image_cache::DecodedImage::new(px, 16, 16),
    );
    doc.load_css(&format!(
        "html, body {{ margin: 0; }} body {{ height: 600px; \
         background: url(tile.png) {position} / 24px 24px; }}"
    ));
    doc.resolve_layout(VP.0, VP.1);
    let mut f = PaintFixture {
        doc,
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.paint();
    f
}

/// A `w` x `h` photo-like picture (a gradient under deterministic noise)
/// drawn at 500x400, about 1/3 (#2456, #2534): the draw samples its half-size
/// mipmap level, made on the first paint and kept in `image_minify`'s cache.
fn minified_picture_page((w, h): (u32, u32)) -> PaintFixture {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    let mut seed = 0x2534_u32;
    for y in 0..h {
        for x in 0..w {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let n = seed & 63;
            px.extend_from_slice(&[
                ((x * 191 / w) + n) as u8,
                ((y * 191 / h) + n) as u8,
                (((x + y) * 95 / (w + h)) + n) as u8,
                255,
            ]);
        }
    }
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; background: #fff; } img { display: block; width: 500px; height: 400px; }",
    );
    doc.tree.image_cache.insert_decoded(
        "photo.png".to_string(),
        rinch_dom::image_cache::DecodedImage::new(px, w, h),
    );
    let body = doc.body();
    doc.set_inner_html(body, "<img src=\"photo.png\">");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    PaintFixture {
        doc,
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    }
}

/// `minified_picture_page` not painted yet: the measured paint makes the
/// mipmap level (one 2x2 halving of the opaque picture) and draws it. A
/// one-time cost per drawn size.
pub fn setup_minified_picture_first_paint() -> PaintFixture {
    minified_picture_page((1500, 1200))
}

/// The same with a 1499x1199 picture (#2629 review): its level 1 is
/// `ceil`-sized, 750x600, which no halving reaches, so the measured paint
/// resamples the picture to it bilinearly, as Chrome fills it.
pub fn setup_minified_picture_odd_first_paint() -> PaintFixture {
    minified_picture_page((1499, 1199))
}

/// `minified_picture_page` painted once: the measured paint finds the
/// level in the cache and only draws it. The per-frame cost.
pub fn setup_minified_picture_cached_paint() -> PaintFixture {
    let mut f = minified_picture_page((1500, 1200));
    f.paint();
    f
}

/// `tiled_background_paint` on the device grid.
pub fn setup_tiled_background_paint() -> PaintFixture {
    tiled_background_paint("0 0")
}

/// `tiled_background_paint` at a fractional position.
pub fn setup_tiled_background_fraction_paint() -> PaintFixture {
    tiled_background_paint("0.5px 0.5px")
}

/// `dashed_borders_page` on the Vello painter (the encoding alone).
pub fn setup_vello_dashed_borders() -> VelloPaintFixture {
    let mut f = VelloPaintFixture {
        doc: dashed_borders_page(),
        painter: rinch_dom::paint::vello_painter::VelloPainter::new(),
        slicing: None,
    };
    f.paint();
    f
}

/// 40 form fields styled the way an app styles a text input — a 1px border,
/// a small radius and a blurred `inset` shadow — laid out and painted once
/// (#974). An inset shadow builds and draws a blurred mask per box per paint,
/// so this is where its cost shows.
pub fn setup_shadow_paint() -> PaintFixture {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; } \
         div { position: absolute; width: 280px; height: 34px; background: rgb(255, 255, 255); \
         border: 1px solid rgb(200, 200, 200); border-radius: 6px; \
         box-shadow: inset 0 1px 3px rgba(0, 0, 0, 0.2); }",
    );
    let mut html = String::new();
    for i in 0..40 {
        let (x, y) = ((i % 2) * 300 + 10, (i / 2) * 40 + 5);
        html.push_str(&format!("<div style=\"left: {x}px; top: {y}px\"></div>"));
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

/// 20 cards, each under four nested `overflow: hidden; border-radius: 3px`
/// scrollers and faded to `opacity: 0.9`, each holding a `position: fixed`
/// box: `none` (no fixed box), `open` (one drawn just below its card, a
/// dropdown under its trigger), `open_far` (one drawn along the window's top
/// edge, far from its card) or `closed` (one at `opacity: 0; visibility:
/// hidden`, a closed `Popover`) (#549). A fixed box escapes the four clips:
/// the software painter lifts them in place around it (on a painter that
/// popped and pushed them, every rounded clip pushed again was a mask
/// rasterised again, #2057), and the fixed box is composited with its card as
/// one group: `open_far` is what a group whose content is spread out costs,
/// which the layer's drawn rects keep to the card and the box.
pub fn setup_fixed_under_clips(fixed: &str) -> PaintFixture {
    let mut doc = new_document();
    let mut html = String::new();
    for s in 0..20 {
        for _ in 0..4 {
            html.push_str(
                "<div style=\"overflow: hidden; border-radius: 3px; padding: 1px; width: 300px\">",
            );
        }
        html.push_str("<div style=\"opacity: 0.9; height: 10px; background: rgb(200, 0, 0)\">");
        match fixed {
            // Each stack is 18px tall (10px and four 1px paddings a side), the
            // card 4px into it.
            "open" => html.push_str(&format!(
                "<div style=\"position: fixed; left: {}px; top: {}px; width: 10px; height: 10px; \
                 background: rgb(0, 200, 0)\"></div>",
                20 + s * 12,
                s * 18 + 14
            )),
            "open_far" => html.push_str(&format!(
                "<div style=\"position: fixed; left: {}px; top: 5px; width: 10px; height: 10px; \
                 background: rgb(0, 200, 0)\"></div>",
                s * 12
            )),
            "closed" => html.push_str(&format!(
                "<div style=\"position: fixed; left: {}px; top: 5px; width: 10px; height: 10px; \
                 background: rgb(0, 200, 0); opacity: 0; visibility: hidden\"></div>",
                s * 12
            )),
            _ => {}
        }
        html.push_str("</div>");
        for _ in 0..4 {
            html.push_str("</div>");
        }
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

/// [`setup_fixed_under_clips`] with no fixed box: the baseline the other two
/// are read against.
pub fn setup_fixed_under_clips_none() -> PaintFixture {
    setup_fixed_under_clips("none")
}

/// [`setup_fixed_under_clips`] with a drawn fixed box in every card.
pub fn setup_fixed_under_clips_open() -> PaintFixture {
    setup_fixed_under_clips("open")
}

/// [`setup_fixed_under_clips`] with a drawn fixed box in every card, along the
/// window's top edge.
pub fn setup_fixed_under_clips_open_far() -> PaintFixture {
    setup_fixed_under_clips("open_far")
}

/// [`setup_fixed_under_clips`] with a closed popover in every card.
pub fn setup_fixed_under_clips_closed() -> PaintFixture {
    setup_fixed_under_clips("closed")
}

/// Paint the 40 fields with the software painter.
pub fn op_shadow_paint(mut f: PaintFixture) -> PaintFixture {
    f.paint();
    f
}

/// [`setup_full_paint`]'s 40 paragraphs with `text-shadow: 0 1px {blur}px` on
/// every one (#980), painted once — which rasterises, blurs and caches every
/// shadow's mask.
fn text_shadow_page(blur: &str) -> PaintFixture {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; font-size: 14px; line-height: 18px; \
         color: rgb(30, 30, 40); }} p {{ margin: 0 0 2px 0; \
         text-shadow: 0 1px {blur}px rgba(0, 0, 0, 0.4); }}"
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

/// The 40 shadowed paragraphs, painted once: the measured paint is the
/// steady state, every blurred mask served from the cache.
pub fn setup_text_shadow_paint() -> PaintFixture {
    text_shadow_page("4")
}

/// Paint the 40 shadowed paragraphs with the software painter.
pub fn op_text_shadow_paint(mut f: PaintFixture) -> PaintFixture {
    f.paint();
    f
}

/// A page whose lines are made of many text nodes, with a blurred
/// `text-shadow` on each (#1060): every IFC holds more than one text
/// fragment, so the shadow's paint order is decided per fragment. `spans`
/// is 30 `nowrap` lines of 12 spans (a syntax-highlighted listing); `count`
/// is 30 lines of `Count: <span>N</span> items…` (a label and a reactive
/// value). Painted once, so the measured paint is the warm steady state.
fn text_shadow_fragments_page(count: bool) -> PaintFixture {
    text_shadow_fragments_page_in(count, false)
}

/// [`text_shadow_fragments_page`], with each span in one of six
/// highlighter colours when `coloured`.
fn text_shadow_fragments_page_in(count: bool, coloured: bool) -> PaintFixture {
    const COLOURS: [&str; 6] = [
        "rgb(170, 13, 145)",
        "rgb(28, 0, 207)",
        "rgb(196, 26, 22)",
        "rgb(0, 116, 0)",
        "rgb(63, 110, 116)",
        "rgb(92, 38, 153)",
    ];
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; font-size: 14px; line-height: 18px; \
         color: rgb(30, 30, 40); }} p {{ margin: 0 0 2px 0; \
         text-shadow: 0 1px 4px rgba(0, 0, 0, 0.4); }}"
    ));
    let words: Vec<&str> = LOREM.split(' ').collect();
    let mut html = String::new();
    for i in 0..30 {
        if count {
            html.push_str(&format!(
                "<p>Count: <span>{i}</span> items in the basket today</p>"
            ));
        } else {
            html.push_str("<p style=\"white-space: nowrap\">");
            for (k, w) in words.iter().take(12).enumerate() {
                if coloured {
                    let c = COLOURS[k % COLOURS.len()];
                    html.push_str(&format!("<span style=\"color: {c}\">{w}{i}{k}</span> "));
                } else {
                    html.push_str(&format!("<span>{w}{i}{k}</span> "));
                }
            }
            html.push_str("</p>");
        }
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

/// `text_shadow_fragments_page` with 12 spans a line.
pub fn setup_text_shadow_spans() -> PaintFixture {
    text_shadow_fragments_page(false)
}

/// `text_shadow_fragments_page` with 12 spans a line in six colours (a
/// syntax-highlighted listing).
pub fn setup_text_shadow_coloured() -> PaintFixture {
    text_shadow_fragments_page_in(false, true)
}

/// `text_shadow_fragments_page` with a label and a value a line.
pub fn setup_text_shadow_count() -> PaintFixture {
    text_shadow_fragments_page(true)
}

/// One paragraph of 200 spans, each casting a hard `text-shadow` list of its
/// own (#1066): 200 shadow passes over the lines they cast on, painted once.
/// Each pass used to walk its lines' items again, quadratic in the glyph runs
/// of a line; the items are collected once per paint now.
pub fn setup_text_shadow_distinct() -> PaintFixture {
    let mut doc = new_document();
    doc.load_css(&format!(
        "body {{ margin: 0; font-family: {FAMILY}; font-size: 14px; line-height: 18px; \
         color: rgb(30, 30, 40); }} p {{ margin: 0; }}"
    ));
    let mut html = String::from("<p>");
    for i in 0..200 {
        html.push_str(&format!(
            "<span style=\"text-shadow: 0 2px 0 rgb({}, {}, 0)\">tok{i} </span>",
            (i * 53) % 256,
            (i * 97) % 256
        ));
    }
    html.push_str("</p>");
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

/// The same page painted once at a 4px blur, then restyled to 4.01px — a blur
/// radius is in a mask's cache key — so the measured paint rasterises and
/// blurs every shadow with the glyph caches warm. It needs no API of its own,
/// so the base of a Perf comparison can build it.
pub fn setup_text_shadow_paint_cold() -> PaintFixture {
    let mut f = text_shadow_page("4");
    f.doc
        .load_css("p { text-shadow: 0 1px 4.01px rgba(0, 0, 0, 0.4); }");
    // The restyle is forced here so the setup, not the measured paint, pays
    // it. Before #884 `load_css` queued no restyle at all, and without this
    // the paragraphs kept their 4px blur and every mask was a cache hit
    // (review of #1020, round 3, F1); a Perf base older than #884 still
    // needs it.
    f.doc.recompute_all_styles_full();
    f.doc.resolve_layout(VP.0, VP.1);
    f
}

#[cfg(test)]
mod text_shadow_bench_tests {
    use rinch_dom::perf::Counter;

    /// The cold benchmark is cold: its measured paint rasterises the mask of
    /// every line on the 800x600 page, and the warm one rasterises none. The
    /// page shows 17 of the 40 paragraphs, two lines each; a line's blurred
    /// shadow reaches 5px up into the line above, so each line is painted as
    /// its own fragment with its own mask (#1060). The second lines are the
    /// end of the same text, so most of them draw the same mask and are
    /// served from the cache: 19 rasterised for 34 lines.
    ///
    /// Kills: the restyle left out of the cold setup (it measured the warm
    /// paint, 0 masks rasterised).
    #[test]
    fn the_cold_text_shadow_paint_rasterises_every_mask() {
        let mut f = super::setup_text_shadow_paint_cold();
        f.doc.tree.perf.reset();
        let mut f = super::op_text_shadow_paint(f);
        let n = f.doc.tree.perf.get(Counter::TextShadowMasksRasterised);
        assert_eq!(n, 19, "the cold paint rasterised {n} masks");
        f.doc.tree.perf.reset();
        let f = super::op_text_shadow_paint(f);
        assert_eq!(f.doc.tree.perf.get(Counter::TextShadowMasksRasterised), 0);
    }

    /// The one-colour multi-fragment pages keep the one-pass order: a cold
    /// paint rasterises one mask a line (30), not one a fragment (#1060).
    /// The highlighted page goes per fragment: a 40%-black 4px blur over
    /// `rgb(28, 0, 207)` stays above the 2-level order tolerance for about
    /// 3.95px, past the 3.91px space before the next span, so it rasterises a
    /// mask a fragment: 410.
    ///
    /// It was 377 before the glyph-mask atlas (#2286): glyphs were drawn at
    /// whole pixels, so fragments of the same text at different fractions of
    /// a pixel had the same coverage and shared a mask. The atlas draws each
    /// glyph at its quarter-pixel position, so those fragments' coverage now
    /// differs, and the mask key (`CallHasher::glyph_x`) says so: 33 more
    /// fragments rasterise a mask of their own. Measured by taking the key
    /// apart: 6 of them are fragments at other quarter-pixel positions, and
    /// 27 sit within 1/64 px of a quarter boundary, where the key holds the
    /// exact position so that float error cannot round it to the neighbouring
    /// bucket (the old key did the same at the half pixel only). The
    /// whole-pixel key with the quarter-boundary guard gives 404; the old key,
    /// whole pixel and half-pixel guard, gives 377 again. That key shares
    /// masks whose pixels differ: a shadow drawn up to a quarter pixel off
    /// its text before the blur.
    ///
    /// Kills: the colour difference left out (the one-colour pages go per
    /// fragment); a tolerance that lets the highlighted page keep one pass
    /// without saying so here.
    #[test]
    fn the_multi_fragment_pages_rasterise_the_masks_their_order_needs() {
        for (name, setup, masks) in [
            (
                "spans",
                super::setup_text_shadow_spans as fn() -> super::PaintFixture,
                30,
            ),
            ("coloured", super::setup_text_shadow_coloured, 410),
            ("count", super::setup_text_shadow_count, 30),
        ] {
            let mut f = setup();
            rinch_dom::paint::clear_text_shadow_cache();
            f.doc.tree.perf.reset();
            let f = super::op_text_shadow_paint(f);
            let n = f.doc.tree.perf.get(Counter::TextShadowMasksRasterised);
            assert_eq!(n, masks, "{name}: {n} masks");
        }
    }
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

/// A paragraph of `INLINE_LINKS` links (`<a>` with `style`), wrapped over
/// lines 22px apart, with the pointer resting on it and the hit cache warm.
fn mount_inline_links(style: &'static str) -> RinchApp {
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style_el = style_element(
            scope,
            "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 22px; }
             p { margin: 0; width: 780px; }
             a:hover { color: rgb(200, 0, 0); }",
        );
        root.append_child(&style_el);
        let p = scope.create_element("p");
        for i in 0..INLINE_LINKS {
            let a = scope.create_element("a");
            a.set_attribute("href", "#");
            a.set_attribute("style", style);
            let t = scope.create_text(&format!("w{i}"));
            a.append_child(&t);
            p.append_child(&a);
            let gap = scope.create_text(" ");
            p.append_child(&gap);
        }
        root.append_child(&p);
        root
    });
    pointer_move(&mut app, 50.0, 105.0);
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    frame(&mut app);
    app
}

/// The links paragraph, bare links.
pub fn setup_pointer_move_inline_bare() -> RinchApp {
    mount_inline_links("")
}

/// The links paragraph, each link padded (a box past its glyphs, #2645).
pub fn setup_pointer_move_inline_padded() -> RinchApp {
    mount_inline_links("padding: 3px 4px")
}

/// `WARM_MOVES` pointer moves over the links paragraph, on glyphs, between
/// them and between lines, each followed by `AboutToWait`: hover follows
/// the link under the pointer (#2648), which asks the paragraph's glyphs
/// and, off them, its links' boxes.
pub fn op_pointer_move_inline(mut app: RinchApp) -> RinchApp {
    for i in 0..WARM_MOVES {
        let x = 20.0 + (i * 37 % 740) as f32;
        let y = 100.0 + (i * 7 % 60) as f32 + 0.5;
        pointer_move(&mut app, x, y);
        app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0);
    }
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

/// Paragraphs in `relayout_beside_text`'s page.
pub const TEXT_PARAGRAPHS: usize = 200;

/// A page of `TEXT_PARAGRAPHS` paragraphs with an absolutely positioned
/// gauge at its corner, mounted and painted once: its paragraphs are freshly
/// shaped, as after any change that rebuilt their lines.
pub fn setup_relayout_beside_text() -> ShellFixture<NodeHandle> {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = style_element(
            scope,
            "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; }
             p { margin: 0; }
             .gauge { position: absolute; left: 600px; top: 560px; width: 100px;
                      height: 10px; background: rgb(0, 0, 200); }",
        );
        root.append_child(&style);
        for i in 0..TEXT_PARAGRAPHS {
            let p = scope.create_element("p");
            let t = scope.create_text(&format!("Paragraph {i} of the page's text"));
            p.append_child(&t);
            root.append_child(&p);
        }
        let gauge = scope.create_element("div");
        gauge.set_attribute("class", "gauge");
        root.append_child(&gauge);
        *out2.borrow_mut() = Some(gauge.clone());
        root
    });
    let gauge = out.borrow().clone().expect("mounted");
    ShellFixture { app, state: gauge }
}

/// The gauge's width changes: one restyle and relayout, then the frame's
/// software repaint. Only the gauge changed (#2734: every paragraph's text was
/// named as damage, and the frame repainted in full).
pub fn op_relayout_beside_text(mut f: ShellFixture<NodeHandle>) -> ShellFixture<NodeHandle> {
    f.state.set_style("width", "111px");
    frame(&mut f.app);
    f
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

/// Rows in the branch [`setup_branch_hide`] builds.
pub const BRANCH_ROWS: usize = 1_000;

// ── rinch (shell): a popup's document opened beside a bar's (#2552) ───────

/// Items in [`setup_popup_open`]'s menu.
pub const POPUP_ITEMS: usize = 10;
/// The menu's largest size (a popup is mounted at it, then measured).
const POPUP_MAX: (u32, u32) = (300, 400);

/// A bar's document, kept open while popups come and go.
pub struct PopupFixture {
    pub bar: RinchApp,
    /// The menu the measured operation opened.
    pub popup: Option<RinchApp>,
}

/// A menu of [`POPUP_ITEMS`] rows of text, the shape `examples/popup-menu`
/// opens.
fn popup_menu(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    let style = style_element(
        scope,
        "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; } \
         .menu { display: flex; flex-direction: column; padding: 4px; } \
         .item { padding: 4px 12px; } .sep { height: 1px; margin: 4px 0; }",
    );
    root.append_child(&style);
    let menu = scope.create_element("div");
    menu.set_attribute("class", "menu");
    for i in 0..POPUP_ITEMS {
        if i == 5 {
            let sep = scope.create_element("div");
            sep.set_attribute("class", "sep");
            menu.append_child(&sep);
        }
        let item = scope.create_element("div");
        item.set_attribute("class", "item");
        item.append_child(&scope.create_text(&format!("Menu item {i}")));
        menu.append_child(&item);
    }
    root.append_child(&menu);
    root
}

/// What the shell does for a popup before its OS window exists, and its
/// first frame: a new document (its font context, the stylesheets, Stylo's
/// setup), mounted at its largest size, laid out and painted.
fn open_popup_menu() -> RinchApp {
    let mut app = RinchApp::new(popup_menu);
    app.register_app_font(AppFont::new(FACE));
    let (w, h) = (POPUP_MAX.0 as f32, POPUP_MAX.1 as f32);
    app.mount_component(w, h);
    app.resolve_and_repaint(w, h);
    let _ = app.build_pixels(1.0, POPUP_MAX, false);
    let _ = app.end_perf_frame();
    app
}

/// A 40 px bar, and one menu opened and closed: what a process has paid
/// before its next menu (the system font scan, the first document's one-off
/// work).
pub fn setup_popup_open() -> PopupFixture {
    let bar = mount(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = style_element(
            scope,
            "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; } \
             .bar { height: 40px; }",
        );
        root.append_child(&style);
        let bar = scope.create_element("div");
        bar.set_attribute("class", "bar");
        bar.append_child(&scope.create_text("Menu"));
        root.append_child(&bar);
        root
    });
    drop(open_popup_menu());
    PopupFixture { bar, popup: None }
}

/// Open the next menu: its document made, mounted, laid out and painted.
pub fn op_popup_open(mut f: PopupFixture) -> PopupFixture {
    f.popup = Some(open_popup_menu());
    f
}

/// An `if` branch holding a keyed `for` of [`BRANCH_ROWS`] rows (each
/// `div > text`) and a captured handle nested in the branch's own markup, shown
/// and painted. The state is the branch's condition.
pub fn setup_branch_hide() -> ShellFixture<Signal<bool>> {
    let open: Rc<RefCell<Option<Signal<bool>>>> = Rc::new(RefCell::new(None));
    let open2 = open.clone();
    let mut app = mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = style_element(
            scope,
            "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; }",
        );
        root.append_child(&style);
        let panel = scope.create_element("aside");
        panel.append_child(&scope.create_text("captured"));
        let visible = Signal::new(true);
        *open2.borrow_mut() = Some(visible);
        rinch_core::show_dom(
            scope,
            &root,
            move || visible.get(),
            move |s: &mut RenderScope| {
                let wrap = s.create_element("section");
                wrap.append_child(&panel);
                let list = s.create_element("div");
                wrap.append_child(&list);
                rinch_core::for_each_dom_typed(
                    s,
                    &list,
                    || (0..BRANCH_ROWS as u32).collect::<Vec<_>>(),
                    |i: &u32| i.to_string(),
                    |i: u32, rs: &mut RenderScope| {
                        let row = rs.create_element("div");
                        row.append_child(&rs.create_text(&format!("row {i}")));
                        row
                    },
                );
                wrap
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        root
    });
    let open = open.borrow().expect("mounted");
    // Warm the hide and the show once, then paint the shown branch.
    open.set(false);
    open.set(true);
    frame(&mut app);
    ShellFixture { app, state: open }
}

/// Hide the branch: dispose its scopes, detach the captured handle, discard
/// the rest (issue #732's walk), then lay out and repaint.
pub fn op_branch_hide(mut f: ShellFixture<Signal<bool>>) -> ShellFixture<Signal<bool>> {
    f.state.set(false);
    frame(&mut f.app);
    f
}

// ── rinch-editor-collab: a keystroke in a large collaborating document ─────

/// Paragraphs in the collaborating document.
pub const COLLAB_BLOCKS: usize = 2_000;

/// A collaboration session over [`COLLAB_BLOCKS`] paragraphs, and the document one
/// keystroke in the middle paragraph leads to: what [`op_collab_keystroke`] projects.
pub struct CollabFixture {
    pub schema: Rc<rinch_editor_core::Schema>,
    pub before: rinch_editor_core::Node,
    pub after: rinch_editor_core::Node,
    pub session: rinch_editor_collab::CollabSession,
}

fn collab_state(schema: &Rc<rinch_editor_core::Schema>) -> rinch_editor_core::EditorState {
    use rinch_editor_core::{EditorState, Fragment, Plugin, default_plugins};
    let blocks: Vec<_> = (0..COLLAB_BLOCKS)
        .map(|i| {
            let text = schema
                .text(&format!("paragraph number {i} with some text in it"))
                .expect("text");
            schema
                .branch("paragraph", Fragment::from_node(text))
                .expect("paragraph")
        })
        .collect();
    let doc = schema
        .branch("doc", Fragment::from_children(blocks))
        .expect("doc");
    let mut plugins: Vec<Rc<dyn Plugin>> = default_plugins();
    plugins.push(Rc::new(rinch_editor_collab::CollabPlugin));
    EditorState::create(schema.clone(), doc, plugins)
}

/// Type one character into the middle paragraph of `state`.
fn collab_type(state: &rinch_editor_core::EditorState) -> rinch_editor_core::EditorState {
    use rinch_editor_core::{Pos, Selection};
    let at: usize = (0..COLLAB_BLOCKS / 2)
        .map(|i| state.doc.child(i).node_size())
        .sum::<usize>()
        + 3;
    let mut tr = state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").expect("insert");
    state.apply(tr)
}

/// [`COLLAB_BLOCKS`] paragraphs under a live session, one keystroke already projected
/// (so the measured one is the second), and the next keystroke's document ready.
///
/// The document holds no void container (`rinch_editor_collab::projection`): the
/// common case, which reads back the paragraph it changed and nothing else. #1229's
/// first round read back all 2,000 on every keystroke (about 770x the time).
pub fn setup_collab_keystroke() -> CollabFixture {
    alloc::reserve();
    let schema = Rc::new(rinch_editor_core::Schema::starter_kit());
    let state = collab_state(&schema);
    let mut session = rinch_editor_collab::CollabSession::new(&state).expect("session");
    let warm = collab_type(&state);
    session
        .record_local(&schema, &state.doc, &warm.doc)
        .expect("projects");
    session.save_incremental().expect("drain");
    let next = collab_type(&warm);
    CollabFixture {
        schema,
        before: warm.doc.clone(),
        after: next.doc.clone(),
        session,
    }
}

/// Project the keystroke onto the CRDT.
pub fn op_collab_keystroke(mut f: CollabFixture) -> CollabFixture {
    f.session
        .record_local(&f.schema, &f.before, &f.after)
        .expect("projects");
    f
}

// ── rinch-editor-collab: a keystroke in one cell of a large table (#1241) ──

/// Columns of the table [`setup_collab_table_keystroke`] types into.
pub const COLLAB_TABLE_COLS: usize = 4;

fn collab_table_state(
    schema: &Rc<rinch_editor_core::Schema>,
    rows: usize,
) -> rinch_editor_core::EditorState {
    use rinch_editor_core::{EditorState, Fragment, Plugin, default_plugins};
    let para = |text: &str| {
        let text = schema.text(text).expect("text");
        schema
            .branch("paragraph", Fragment::from_node(text))
            .expect("paragraph")
    };
    let branch = |name: &str, children: Vec<rinch_editor_core::Node>| {
        schema
            .branch(name, Fragment::from_children(children))
            .expect(name)
    };
    let table = branch(
        "table",
        (0..rows)
            .map(|r| {
                branch(
                    "table_row",
                    (0..COLLAB_TABLE_COLS)
                        .map(|c| branch("table_cell", vec![para(&format!("row {r} column {c}"))]))
                        .collect(),
                )
            })
            .collect(),
    );
    let doc = branch(
        "doc",
        vec![para("before the table"), table, para("after it")],
    );
    let mut plugins: Vec<Rc<dyn Plugin>> = default_plugins();
    plugins.push(Rc::new(rinch_editor_collab::CollabPlugin));
    EditorState::create(schema.clone(), doc, plugins)
}

/// Type one character into the third cell of the middle row of the table (block 1).
fn collab_table_type(state: &rinch_editor_core::EditorState) -> rinch_editor_core::EditorState {
    use rinch_editor_core::{Pos, Selection};
    let doc = &state.doc;
    let table = doc.child(1);
    let row = table.child_count() / 2;
    // Into the table, the row, the cell and its paragraph: one opening token each.
    let mut at = doc.child(0).node_size() + 1;
    at += (0..row).map(|r| table.child(r).node_size()).sum::<usize>() + 1;
    let cells = table.child(row);
    at += (0..2).map(|k| cells.child(k).node_size()).sum::<usize>() + 1;
    at += 1 + 3;
    let mut tr = state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").expect("insert");
    state.apply(tr)
}

/// A table of `rows` × [`COLLAB_TABLE_COLS`] cells between two paragraphs, under a live
/// session, one keystroke in a middle cell already projected (so the measured one is the
/// second, in the same cell), and the next keystroke's document ready.
///
/// What the keystroke costs should not depend on `rows`: it changes one cell. #1241
/// measured it re-reading and re-comparing the whole table (26 ms at 1000 rows).
pub fn setup_collab_table_keystroke(rows: usize) -> CollabFixture {
    alloc::reserve();
    let schema = Rc::new(rinch_editor_core::Schema::starter_kit());
    let state = collab_table_state(&schema, rows);
    let mut session = rinch_editor_collab::CollabSession::new(&state).expect("session");
    let warm = collab_table_type(&state);
    session
        .record_local(&schema, &state.doc, &warm.doc)
        .expect("projects");
    session.save_incremental().expect("drain");
    let next = collab_table_type(&warm);
    CollabFixture {
        schema,
        before: warm.doc.clone(),
        after: next.doc.clone(),
        session,
    }
}

// ── rinch-editor-collab: a transaction written place by place (#1885, #2526) ─

/// A collaboration session, and one transaction from `before` to `after` with
/// its position mapping: what [`op_collab_mapped`] records through
/// `CollabSession::record_local_mapped`, the path the editor records every
/// transaction on.
pub struct CollabMappedFixture {
    pub schema: Rc<rinch_editor_core::Schema>,
    pub before: rinch_editor_core::Node,
    pub after: rinch_editor_core::Node,
    pub mapping: rinch_editor_core::Mapping,
    pub session: rinch_editor_collab::CollabSession,
}

/// `blocks` paragraphs of 80 chars under a live session, and a transaction that
/// replaces five chars with three at two places (offsets 10 and 50) of each of the
/// first `edited` paragraphs, one step per place, first paragraph first with each
/// position mapped through the steps before it (as a find-and-replace does).
pub fn setup_collab_mapped((blocks, edited): (usize, usize)) -> CollabMappedFixture {
    use rinch_editor_core::{EditorState, Fragment, Plugin, default_plugins};
    alloc::reserve();
    let schema = Rc::new(rinch_editor_core::Schema::starter_kit());
    let paragraphs: Vec<_> = (0..blocks)
        .map(|i| {
            let mut t = format!("p{i:05} ");
            while t.len() < 80 {
                t.push_str("lorem ipsum ");
            }
            let text = schema.text(&t).expect("text");
            schema
                .branch("paragraph", Fragment::from_node(text))
                .expect("paragraph")
        })
        .collect();
    let doc = schema
        .branch("doc", Fragment::from_children(paragraphs))
        .expect("doc");
    let mut plugins: Vec<Rc<dyn Plugin>> = default_plugins();
    plugins.push(Rc::new(rinch_editor_collab::CollabPlugin));
    let state = EditorState::create(schema.clone(), doc, plugins);
    let session = rinch_editor_collab::CollabSession::new(&state).expect("session");
    let mut tr = state.tr();
    let mut start = 1;
    for i in 0..edited {
        for off in [10, 50] {
            let from = tr.mapping().map(start + off, 1);
            let to = tr.mapping().map(start + off + 5, -1);
            let xyz = Fragment::from_node(schema.text("XYZ").expect("text"));
            tr.replace_with(from, to, xyz).expect("replace");
        }
        start += state.doc.child(i).node_size();
    }
    let mapping = tr.mapping().clone();
    let next = state.apply(tr);
    CollabMappedFixture {
        schema,
        before: state.doc.clone(),
        after: next.doc.clone(),
        mapping,
        session,
    }
}

/// Record the transaction on the CRDT, place by place.
pub fn op_collab_mapped(mut f: CollabMappedFixture) -> CollabMappedFixture {
    f.session
        .record_local_mapped(&f.schema, &f.before, &f.after, &f.mapping)
        .expect("projects");
    f
}

// ── rinch-editor-core: typing over a selection in one cell of a large table ─

/// Columns of the table [`setup_range_type_in_table`] types into.
pub const RANGE_TABLE_COLS: usize = 10;

/// An editor state, and the state one operation over it led to (kept, so the
/// measured operation does not also pay to drop it).
pub struct EditorFixture {
    pub state: rinch_editor_core::EditorState,
    pub next: Option<rinch_editor_core::EditorState>,
}

/// A table of `rows` × [`RANGE_TABLE_COLS`] cells, and a text selection
/// `len` characters long (`0`: a caret) inside one cell of its middle row.
///
/// Typing over a range replaces the selection, which checks the edit for a
/// table it left ragged (#1847). An edit inside one cell cannot change any
/// table's grid, so the check reads no table here: the first round of it
/// walked the whole table (1.0 ms at 1,024 rows, 24.6 ms at 4,096). The
/// caret variants are the yardstick: a keystroke anywhere in the table is
/// linear in its rows already (resolving a position walks the row list, and
/// the edit copies it), and the check adds nothing linear to that: it reads
/// the start's resolve that the inherited marks need anyway.
pub fn setup_range_type_in_table((rows, len): (usize, usize)) -> EditorFixture {
    use rinch_editor_core::{EditorState, Fragment, Pos, Selection, default_plugins};
    alloc::reserve();
    let schema = Rc::new(rinch_editor_core::Schema::starter_kit());
    let para = |text: &str| {
        let text = schema.text(text).expect("text");
        schema
            .branch("paragraph", Fragment::from_node(text))
            .expect("paragraph")
    };
    let branch = |name: &str, children: Vec<rinch_editor_core::Node>| {
        schema
            .branch(name, Fragment::from_children(children))
            .expect(name)
    };
    let table = branch(
        "table",
        (0..rows)
            .map(|_| {
                branch(
                    "table_row",
                    (0..RANGE_TABLE_COLS)
                        .map(|_| branch("table_cell", vec![para("abcd")]))
                        .collect(),
                )
            })
            .collect(),
    );
    let doc = branch("doc", vec![para("before the table"), table]);
    // Into the table, the middle row, its third cell and its paragraph; "bc"
    // of "abcd".
    let tbl = doc.child(1);
    let row = tbl.child_count() / 2;
    let mut at = doc.child(0).node_size() + 1;
    at += (0..row).map(|r| tbl.child(r).node_size()).sum::<usize>() + 1;
    at += (0..2)
        .map(|k| tbl.child(row).child(k).node_size())
        .sum::<usize>()
        + 1;
    at += 1 + 1;
    let mut state = EditorState::create(schema.clone(), doc, default_plugins());
    let mut tr = state.tr();
    tr.set_selection(Selection::text(Pos(at), Pos(at + len)));
    state = state.apply(tr);
    EditorFixture { state, next: None }
}

/// Type one character over the selection, and land the state.
pub fn op_range_type_in_table(mut f: EditorFixture) -> EditorFixture {
    let mut tr = f.state.tr();
    tr.insert_text("Q").expect("insert");
    f.next = Some(f.state.apply(tr));
    f
}

// ── rinch-editor-core: an undo in a large table (#1937) ─────────────────────

/// A keystroke typed in a cell of the middle row of a table of `rows` ×
/// [`RANGE_TABLE_COLS`] cells, recorded: what [`op_undo`] undoes. With `peer`, a
/// peer's keystroke follows in the first cell of the table (unrecorded, as a
/// collaboration integrate applies it), so the undo maps its step through it.
///
/// An undo repairs a table it left ragged (#1937). Only an undo mapped through a
/// change the history did not record can leave one, so a local undo checks
/// nothing; the peer variant resolves its step's start once, which a keystroke
/// anywhere in the table already does, and reads no table.
pub fn setup_undo_in_table((rows, peer): (usize, bool)) -> EditorFixture {
    use rinch_editor_core::{Pos, Selection};
    let f = op_range_type_in_table(setup_range_type_in_table((rows, 0)));
    let mut state = f.next.expect("typed");
    if peer {
        // Inside the first cell's paragraph: the table, its row, the cell and
        // the paragraph open after the paragraph before the table.
        let at = state.doc.child(0).node_size() + 4 + 1;
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.insert_text("p").expect("insert");
        tr.set_add_to_history(false);
        state = state.apply(tr);
    }
    EditorFixture { state, next: None }
}

/// Undo the newest event.
pub fn op_undo(mut f: EditorFixture) -> EditorFixture {
    f.next = f.state.run("undo");
    f
}

// ── rinch-editor-core: a cell selection in a large table (#1847) ───────────

/// A table of `rows` × [`RANGE_TABLE_COLS`] cells, each of two paragraphs,
/// and in its middle row either (`kind` 0) a text range from the first
/// paragraph of the third cell into its second, or (`kind` 1) a cell
/// selection of the third and fourth cells. Adopted from the second review
/// of #1931.
pub fn setup_cells_in_table((rows, kind): (usize, usize)) -> EditorFixture {
    use rinch_editor_core::{EditorState, Fragment, Pos, Selection, default_plugins};
    alloc::reserve();
    let schema = Rc::new(rinch_editor_core::Schema::starter_kit());
    let para = |text: &str| {
        schema
            .branch(
                "paragraph",
                Fragment::from_node(schema.text(text).expect("text")),
            )
            .expect("paragraph")
    };
    let branch = |name: &str, children: Vec<rinch_editor_core::Node>| {
        schema
            .branch(name, Fragment::from_children(children))
            .expect(name)
    };
    let table = branch(
        "table",
        (0..rows)
            .map(|_| {
                branch(
                    "table_row",
                    (0..RANGE_TABLE_COLS)
                        .map(|_| branch("table_cell", vec![para("abcd"), para("efgh")]))
                        .collect(),
                )
            })
            .collect(),
    );
    let doc = branch("doc", vec![para("before the table"), table]);
    let tbl = doc.child(1);
    let row = tbl.child_count() / 2;
    let row_start = doc.child(0).node_size()
        + 1
        + (0..row).map(|r| tbl.child(r).node_size()).sum::<usize>()
        + 1;
    let cell2 = row_start
        + (0..2)
            .map(|k| tbl.child(row).child(k).node_size())
            .sum::<usize>();
    let cell3 = cell2 + tbl.child(row).child(2).node_size();
    let sel = if kind == 0 {
        // "a|bcd" .. "ef|gh" in the third cell.
        Selection::text(Pos(cell2 + 3), Pos(cell2 + 1 + 6 + 3))
    } else {
        Selection::cell(Pos(cell2), Pos(cell3))
    };
    let mut state = EditorState::create(schema.clone(), doc, default_plugins());
    let mut tr = state.tr();
    tr.set_selection(sel.clone());
    state = state.apply(tr);
    assert_eq!(state.selection, sel);
    // The toolbar's first ask, which computes the table's map; the measured
    // one is the next mark's.
    let bold = schema.mark_type("bold").expect("bold").clone();
    std::hint::black_box(rinch_editor_core::commands::is_mark_active(&state, &bold));
    EditorFixture { state, next: None }
}

/// Whether bold is active over the selection: what a toolbar asks once per
/// mark on every update.
pub fn op_bold_active(f: EditorFixture) -> EditorFixture {
    let bold = f.state.schema().mark_type("bold").expect("bold").clone();
    std::hint::black_box(rinch_editor_core::commands::is_mark_active(&f.state, &bold));
    f
}

/// Run `toggleBold` over the selection.
pub fn op_toggle_bold(mut f: EditorFixture) -> EditorFixture {
    f.next = f.state.run("toggleBold");
    f
}

/// Type one character into cell `col` of the middle row of the table (block 1).
fn collab_table_type_in(
    state: &rinch_editor_core::EditorState,
    col: usize,
) -> rinch_editor_core::EditorState {
    use rinch_editor_core::{Pos, Selection};
    let doc = &state.doc;
    let table = doc.child(1);
    let row = table.child_count() / 2;
    let mut at = doc.child(0).node_size() + 1;
    at += (0..row).map(|r| table.child(r).node_size()).sum::<usize>() + 1;
    let cells = table.child(row);
    at += (0..col).map(|k| cells.child(k).node_size()).sum::<usize>() + 1;
    at += 1 + 3;
    let mut tr = state.tr();
    tr.set_selection(Selection::cursor(Pos(at)));
    tr.insert_text("x").expect("insert");
    state.apply(tr)
}

/// Two peers on a table of `rows` × [`COLLAB_TABLE_COLS`] cells: the host and a guest
/// joined from its snapshot.
fn collab_table_peers(
    rows: usize,
) -> (
    Rc<rinch_editor_core::Schema>,
    (
        rinch_editor_core::EditorState,
        rinch_editor_collab::CollabSession,
    ),
    (
        rinch_editor_core::EditorState,
        rinch_editor_collab::CollabSession,
    ),
) {
    use rinch_editor_core::{EditorState, Plugin, default_plugins};
    let schema = Rc::new(rinch_editor_core::Schema::starter_kit());
    let host = collab_table_state(&schema, rows);
    let host_session = rinch_editor_collab::CollabSession::new(&host).expect("session");
    let guest_session =
        rinch_editor_collab::CollabSession::from_bytes(&host_session.snapshot()).expect("join");
    let mut plugins: Vec<Rc<dyn Plugin>> = default_plugins();
    plugins.push(Rc::new(rinch_editor_collab::CollabPlugin));
    let guest = EditorState::create(
        schema.clone(),
        guest_session.projected_doc(&schema).expect("read"),
        plugins,
    );
    (schema, (host, host_session), (guest, guest_session))
}

/// The guest types in `col`, records it, and hands over its delta.
fn collab_table_peer_types(
    schema: &rinch_editor_core::Schema,
    guest: &mut (
        rinch_editor_core::EditorState,
        rinch_editor_collab::CollabSession,
    ),
    col: usize,
) -> Vec<u8> {
    let next = collab_table_type_in(&guest.0, col);
    guest
        .1
        .record_local(schema, &guest.0.doc, &next.doc)
        .expect("projects");
    guest.0 = next;
    guest.1.save_incremental().expect("drain")
}

/// A peer's keystroke for [`op_collab_integrate`] to integrate: the receiving
/// editor's state and session, and the delta.
pub struct CollabIntegrateFixture {
    pub state: rinch_editor_core::EditorState,
    pub session: rinch_editor_collab::CollabSession,
    pub delta: Vec<u8>,
}

/// A table of `rows` × [`COLLAB_TABLE_COLS`] cells shared by two peers. The guest
/// types in a cell of the middle row and the host integrates it, once to warm up,
/// then the guest types again: the host's integrate of that second keystroke is
/// what [`op_collab_integrate`] measures.
///
/// It should not depend on `rows`: the peer changed one cell. #1241 measured it
/// reading back the whole document (18 ms at 1000 rows, release).
pub fn setup_collab_table_integrate(rows: usize) -> CollabIntegrateFixture {
    alloc::reserve();
    let (schema, mut host, mut guest) = collab_table_peers(rows);
    let delta = collab_table_peer_types(&schema, &mut guest, 2);
    host.0 = host
        .1
        .integrate_incremental(&host.0, &delta)
        .expect("integrates")
        .expect("changes");
    let delta = collab_table_peer_types(&schema, &mut guest, 2);
    CollabIntegrateFixture {
        state: host.0,
        session: host.1,
        delta,
    }
}

/// Integrate the peer's keystroke into the editor: the converged document as a
/// transaction, applied (the history's map of it included).
pub fn op_collab_integrate(mut f: CollabIntegrateFixture) -> CollabIntegrateFixture {
    f.state = f
        .session
        .integrate_incremental(&f.state, &f.delta)
        .expect("integrates")
        .expect("changes");
    f
}

/// Live collaboration in a table of `rows` × [`COLLAB_TABLE_COLS`] cells: the host
/// has typed in one cell of the middle row, the guest in another, and the host has
/// just integrated the guest's keystroke. What [`op_collab_keystroke`] then
/// projects is the host's next keystroke in its own cell, the first after a peer's
/// change (#1241: any remote change used to make it read the whole table again).
pub fn setup_collab_table_keystroke_after_remote(rows: usize) -> CollabFixture {
    alloc::reserve();
    let (schema, mut host, mut guest) = collab_table_peers(rows);
    for _ in 0..2 {
        let next = collab_table_type_in(&host.0, 1);
        host.1
            .record_local(&schema, &host.0.doc, &next.doc)
            .expect("projects");
        host.0 = next;
        let mine = host.1.save_incremental().expect("drain");
        let theirs = collab_table_peer_types(&schema, &mut guest, 2);
        guest.0 = guest
            .1
            .integrate_incremental(&guest.0, &mine)
            .expect("integrates")
            .expect("changes");
        host.0 = host
            .1
            .integrate_incremental(&host.0, &theirs)
            .expect("integrates")
            .expect("changes");
    }
    let next = collab_table_type_in(&host.0, 1);
    CollabFixture {
        schema,
        before: host.0.doc.clone(),
        after: next.doc.clone(),
        session: host.1,
    }
}

// ── list items with outside markers (#1356, review of #2271) ───────────────

/// A `<ul>` of 500 `<li><p>` items, Outlook's and Gmail's list shape: each
/// item has an outside marker that `rinch_dom`'s marker pass places beside
/// its paragraph's first line. The div-row lists above have no marker, so
/// they cannot see what that pass costs. `other` is a box after the list.
pub struct MarkerListFixture {
    pub doc: RinchDocument,
    pub list: NodeId,
    pub other: NodeId,
    pub grow: f32,
}

fn build_marker_list() -> MarkerListFixture {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; font-family: Inter; font-size: 16px; line-height: 20px; }
         p { margin: 0; }",
    );
    let body = doc.body();
    let list = doc.create_element("ul");
    for i in 0..LIST_ROWS {
        let li = doc.create_element("li");
        let p = doc.create_element("p");
        let t = doc.create_text(&format!("Row number {i} with some text"));
        doc.append_child(p, t);
        doc.append_child(li, p);
        doc.append_child(list, li);
    }
    doc.append_child(body, list);
    let other = doc.create_element("div");
    let t = doc.create_text("after the list");
    doc.append_child(other, t);
    doc.append_child(body, other);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    MarkerListFixture {
        doc,
        list,
        other,
        grow: 10.0,
    }
}

/// The marker list, after one item was appended and laid out.
pub fn setup_marker_list_append() -> MarkerListFixture {
    op_marker_list_append(build_marker_list())
}

/// Append one `<li><p>` item to the marker list, then layout: one new
/// marker to place.
pub fn op_marker_list_append(mut f: MarkerListFixture) -> MarkerListFixture {
    let doc = &mut f.doc;
    let li = doc.create_element("li");
    let p = doc.create_element("p");
    let t = doc.create_text("new row");
    doc.append_child(p, t);
    doc.append_child(li, p);
    doc.append_child(f.list, li);
    doc.resolve_layout(VP.0, VP.1);
    f
}

/// The marker list, after the box below it changed height once.
pub fn setup_marker_list_unrelated() -> MarkerListFixture {
    op_marker_list_unrelated(build_marker_list())
}

/// Grow the box after the list, then layout: no marker can have moved.
pub fn op_marker_list_unrelated(mut f: MarkerListFixture) -> MarkerListFixture {
    f.grow += 1.0;
    let style = format!("height: {}px", f.grow);
    f.doc.set_attribute(f.other, "style", &style);
    f.doc.resolve_layout(VP.0, VP.1);
    f
}

// ── rinch-dom: font stacks naming missing families ────────────────────────

/// Mail and web `font-family` stacks: ten family names, most missing on a
/// Linux host, in twenty different orders and combinations.
const MAIL_STACKS: [&str; 20] = [
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif",
    "'Helvetica Neue', Helvetica, 'Lucida Grande', Tahoma, Verdana, Arial, sans-serif",
    "'Segoe UI', Tahoma, Geneva, Verdana, sans-serif",
    "-apple-system, 'Segoe UI', 'Helvetica Neue', Arial, sans-serif",
    "BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif",
    "'Lucida Grande', 'Segoe UI', Geneva, sans-serif",
    "Roboto, 'Helvetica Neue', Arial, sans-serif",
    "Verdana, Geneva, Tahoma, sans-serif",
    "Oxygen, Cantarell, 'Fira Sans', 'Droid Sans', 'Helvetica Neue', sans-serif",
    "Tahoma, Verdana, Segoe, sans-serif",
    "'Segoe UI', -apple-system, Roboto, sans-serif",
    "'Helvetica Neue', 'Segoe UI', Tahoma, sans-serif",
    "Geneva, 'Lucida Grande', Verdana, sans-serif",
    "Cantarell, Oxygen, Roboto, -apple-system, sans-serif",
    "'Droid Sans', 'Fira Sans', Ubuntu, sans-serif",
    "Ubuntu, 'Segoe UI', Roboto, sans-serif",
    "-apple-system, BlinkMacSystemFont, sans-serif",
    "Arial, 'Helvetica Neue', Helvetica, sans-serif",
    "'Fira Sans', Geneva, 'Segoe UI', sans-serif",
    "Segoe, 'Lucida Grande', Tahoma, Arial, sans-serif",
];

/// The host's font context (fontconfig on Linux), with the per-thread stack
/// and system-match caches empty.
pub fn setup_font_stack_resolve() -> parley::FontContext {
    alloc::reserve();
    let cx = rinch_dom::fonts::new_font_context();
    // Drops what building the context asked (the generic defaults).
    rinch_dom::fonts::set_font_defaults(Default::default());
    cx
}

/// The first layout of each of `MAIL_STACKS`: every missing family name is
/// looked up once, and on Linux asked of fontconfig once (#2336, review of
/// #2349: per stack, 100 such stacks cost 526–877 ms on the UI thread).
pub fn op_font_stack_resolve(mut cx: parley::FontContext) -> parley::FontContext {
    for stack in MAIL_STACKS {
        std::hint::black_box(rinch_dom::fonts::parley_font_family(&mut cx, stack));
    }
    cx
}

// ── rinch-dom: a reflow that shifts a long list's rows (#2222) ─────────────

/// Rows in the shifted list.
pub const SHIFT_ROWS: usize = 2000;
/// The row removed, near the top: every row after it moves up.
pub const SHIFT_REMOVED: usize = 13;

/// A 2000-row list, 150px wide so the frame stays partial, with a row near
/// the top removed and laid out: the measured damage, partial paint and
/// consume of the frame that follows.
pub struct ShiftFixture {
    pub doc: RinchDocument,
    pub painter: TinySkiaPainter,
}

/// `body > div (150px) > SHIFT_ROWS × div > span > text`, painted and
/// consumed, then a row near the top removed and laid out. A first removal,
/// painted and consumed, warms the paths.
pub fn setup_row_shift_damage() -> ShiftFixture {
    let mut doc = new_document();
    doc.load_css(
        "body { margin: 0; font-family: Inter; font-size: 16px; line-height: 20px; }
         .list { width: 150px; }
         .row { padding: 2px; }",
    );
    let body = doc.body();
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    let mut rows = Vec::with_capacity(SHIFT_ROWS);
    for i in 0..SHIFT_ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let span = doc.create_element("span");
        let t = doc.create_text(&format!("Row {i}"));
        doc.append_child(span, t);
        doc.append_child(row, span);
        doc.append_child(list, row);
        rows.push(row);
    }
    doc.append_child(body, list);
    let mut f = ShiftFixture {
        doc,
        painter: TinySkiaPainter::new(SIZE.0, SIZE.1),
    };
    f.doc.resolve_layout(VP.0, VP.1);
    f.doc.resolve_layout(VP.0, VP.1);
    f.painter.fill_white();
    let doc = &mut f.doc;
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut f.painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
    f.doc.tree.consume_paint_dirty();
    f.doc.remove_child(list, rows[SHIFT_REMOVED + 1]);
    f.doc.resolve_layout(VP.0, VP.1);
    f = op_row_shift_damage(f);
    f.doc.remove_child(list, rows[SHIFT_REMOVED]);
    f.doc.resolve_layout(VP.0, VP.1);
    f
}

/// The frame after the removal: its damage, the partial repaint of it, and
/// the consume that records what was painted.
pub fn op_row_shift_damage(mut f: ShiftFixture) -> ShiftFixture {
    let doc = &mut f.doc;
    let damage = rinch_dom::paint::compute_damage(&doc.tree, 1.0, VP.0 as f64, VP.1 as f64);
    let rects = damage.rects().to_vec();
    rinch_dom::paint::set_dirty_rects(Some(&rects));
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut f.painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
    rinch_dom::paint::set_dirty_region(None);
    doc.tree.consume_paint_dirty();
    f
}
