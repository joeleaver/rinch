//! #2667, the web half of #2666: the editor's selection over a block with an
//! opaque background. The selection boxes (`[data-pm-selection]`) are
//! `z-index: -1` in the editor's stacking context, which a browser paints
//! before every in-flow background there, so a `pre`, an inline `code` or
//! `mark` and a `th` with a background hid the selection whole.
//!
//! The web now paints a line of text as a CSS Custom Highlight
//! (`CSS.highlights`, a `::highlight()` rule with the box's colour: over the
//! backgrounds, under the glyphs) and a cell's wash as an inset `box-shadow`
//! on the cell (over its background, under its content), and hides the box.
//! These assert that state, which is what the browser paints from: a
//! highlight that covers the selected text in the selection's colour and sets
//! no text colour (the glyphs keep theirs), the boxes hidden (no second
//! tint), and nothing left behind once the selection collapses. Measured by
//! screenshot in Chrome 157 and Firefox 155 with the same structure: the
//! highlight shows over the `pre`, `code`, `mark` and `th` backgrounds, the
//! `z-index: -1` boxes do not.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_selection_highlight_2667
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
export function highlight_over(el) {
    // The names of the registered highlights with a range over a text node
    // of `el`, comma-separated ("" when none, or no `CSS.highlights`).
    if (!window.CSS || !CSS.highlights) return "";
    const texts = [];
    const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    while (walk.nextNode()) if (walk.currentNode.data.trim()) texts.push(walk.currentNode);
    const names = [];
    CSS.highlights.forEach((h, name) => {
        for (const r of h) {
            if (r.collapsed) continue;
            if (texts.some(t => r.intersectsNode(t))) { names.push(name); break; }
        }
    });
    return names.join(",");
}
export function highlight_ranges() {
    if (!window.CSS || !CSS.highlights) return 0;
    let n = 0;
    CSS.highlights.forEach(h => { n += h.size; });
    return n;
}
export function highlight_rule(name) {
    // The declarations of the `::highlight(name)` rule, as written.
    for (const sheet of document.styleSheets) {
        let rules;
        try { rules = sheet.cssRules; } catch (_) { continue; }
        for (const rule of rules) {
            if (rule.selectorText === `::highlight(${name})`) return rule.style.cssText;
        }
    }
    return "";
}
export function registered_highlights() {
    return (window.CSS && CSS.highlights) ? CSS.highlights.size : 0;
}
export function has_highlights() { return !!(window.CSS && CSS.highlights); }
"#)]
extern "C" {
    fn highlight_over(el: &web_sys::Element) -> String;
    fn highlight_ranges() -> u32;
    fn highlight_rule(name: &str) -> String;
    fn has_highlights() -> bool;
    fn registered_highlights() -> u32;
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-selection-highlight-2667";

/// The opaque backgrounds of #2666's report, written by the test so they do
/// not depend on the editor's default sheet.
const SHEET: &str = "pre { background: rgb(246, 248, 250); padding: 4px; margin: 0; } \
    code { background: rgb(240, 240, 240); } \
    mark { background: rgb(255, 255, 0); } \
    table { border-collapse: collapse; } \
    th, td { border: 1px solid #888; padding: 4px; } \
    th { background: rgb(230, 230, 230); box-shadow: rgb(0, 128, 0) 2px 2px 0px 0px; }";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    fn mount(html: &str, extra_css: &str) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        rinch_web::__reset_activation_state();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        let sheet = document().create_element("style").unwrap();
        let scoped: String = format!("{SHEET} {extra_css}")
            .split('}')
            .filter(|r| !r.trim().is_empty())
            .map(|r| format!("[{HOST_MARKER}] {} }}", r.trim()))
            .collect();
        sheet.set_text_content(Some(&scoped));
        host.append_child(&sheet).unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(html));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let wrap = scope.create_element("div");
                wrap.set_attribute("style", "width: 500px");
                wrap.append_child(&mounted.mount(scope));
                wrap
            },
        );
        handle.focus();
        Self { root, host, handle }
    }

    fn el(&self, selector: &str) -> web_sys::Element {
        self.host
            .query_selector(&format!("[data-pm-editor] {selector}"))
            .unwrap()
            .unwrap_or_else(|| panic!("no `{selector}` in the editor"))
    }

    async fn select(&self, sel: Selection) {
        self.handle.set_selection(sel);
        next_task().await;
    }

    /// The selection boxes, with their computed `visibility` and
    /// `background-color`.
    fn boxes(&self) -> Vec<(String, String)> {
        let list = self
            .host
            .query_selector_all("[data-pm-editor] [data-pm-selection]")
            .unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
            .map(|el| {
                let cs = web_sys::window()
                    .unwrap()
                    .get_computed_style(&el)
                    .unwrap()
                    .unwrap();
                (
                    cs.get_property_value("visibility").unwrap(),
                    cs.get_property_value("background-color").unwrap(),
                )
            })
            .collect()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn computed(el: &web_sys::Element, prop: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap()
        .get_property_value(prop)
        .unwrap()
}

async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

async fn next_task() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The text of `el` is painted with the selection: a highlight covers it in
/// `colour` and leaves the glyphs' colour alone, and no box is visible (a
/// visible `z-index: -1` box is what an opaque background hides, and over a
/// transparent one it would tint twice).
#[track_caller]
fn assert_text_highlighted(f: &Fixture, el: &web_sys::Element, colour: &str, what: &str) {
    assert!(has_highlights(), "this browser has `CSS.highlights`");
    let boxes = f.boxes();
    assert!(
        !boxes.is_empty(),
        "{what}: positive control, no selection boxes"
    );
    let names = highlight_over(el);
    assert!(
        !names.is_empty(),
        "{what}: no highlight over the text; the selection is only the boxes {boxes:?}, \
         under the block's background"
    );
    for name in names.split(',') {
        let rule = highlight_rule(name);
        assert!(
            rule.contains(&format!("background-color: {colour}")),
            "{what}: `::highlight({name})` is `{rule}`, want background {colour}"
        );
        assert!(
            !rule.replace("background-color", "").contains("color"),
            "{what}: the highlight sets a text colour: `{rule}`"
        );
    }
    for (visibility, _) in &boxes {
        assert_eq!(
            visibility, "hidden",
            "{what}: a box is still painted: {boxes:?}"
        );
    }
}

const BLUE: &str = "rgba(26, 115, 232, 0.3)";

#[wasm_bindgen_test]
async fn a_selection_in_a_code_block_is_painted_over_its_background() {
    let f = Fixture::mount("<pre><code>let x = 1;</code></pre>", "");
    next_task().await;
    let pre = f.el("pre");
    assert_eq!(computed(&pre, "background-color"), "rgb(246, 248, 250)");
    let ink = computed(&pre, "color");
    f.select(Selection::text(Pos(1), Pos(8))).await;
    assert_text_highlighted(&f, &pre, BLUE, "pre");
    assert_eq!(computed(&pre, "color"), ink, "the text keeps its colour");
    // Collapsing it leaves nothing painted.
    f.select(Selection::cursor(Pos(3))).await;
    assert_eq!(
        highlight_over(&pre),
        "",
        "a collapsed selection paints nothing"
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_selection_over_inline_code_and_mark_is_painted_over_their_backgrounds() {
    // "a " (2) + "code" (4) + " " + "mark" (4)
    let f = Fixture::mount("<p>a <code>code</code> <mark>mark</mark></p>", "");
    next_task().await;
    let code = f.el("code");
    let mark = f.el("mark");
    // Opaque, from this sheet or the editor's own.
    for el in [&code, &mark] {
        let bg = computed(el, "background-color");
        assert!(
            bg.starts_with("rgb("),
            "positive control: an opaque background, {bg}"
        );
    }
    f.select(Selection::text(Pos(1), Pos(12))).await;
    assert_text_highlighted(&f, &code, BLUE, "inline code");
    assert_text_highlighted(&f, &mark, BLUE, "mark");
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_selection_in_a_table_header_is_painted_over_its_background() {
    // table 0, row 1, th 2, p 3, "head" 4..8
    let f = Fixture::mount(
        "<table><tr><th><p>head</p></th><td><p>cell</p></td></tr></table>",
        "",
    );
    next_task().await;
    let th = f.el("th");
    f.select(Selection::text(Pos(4), Pos(8))).await;
    assert_text_highlighted(&f, &th, BLUE, "th text");
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_cell_selections_wash_is_painted_over_the_cells_background() {
    let f = Fixture::mount(
        "<table><tr><th><p>head</p></th><td><p>cell</p></td></tr></table>",
        "",
    );
    next_task().await;
    let th = f.el("th");
    let own = computed(&th, "box-shadow");
    assert!(own.contains("rgb(0, 128, 0)"), "positive control: {own}");
    f.select(Selection::cell(Pos(2), Pos(2))).await;
    let boxes = f.boxes();
    assert_eq!(boxes.len(), 1, "positive control: one wash box {boxes:?}");
    let shadow = computed(&th, "box-shadow");
    assert!(
        shadow.contains(BLUE) && shadow.contains("inset"),
        "the wash is not painted on the cell (box-shadow `{shadow}`), only as a box \
         under its background: {boxes:?}"
    );
    assert!(
        shadow.contains("rgb(0, 128, 0)"),
        "the cell's own shadow is kept: `{shadow}`"
    );
    assert_eq!(boxes[0].0, "hidden", "the wash box is still painted");
    // A text selection again: the cell is as it was.
    f.select(Selection::text(Pos(4), Pos(6))).await;
    assert_eq!(
        computed(&th, "box-shadow"),
        own,
        "the cell's shadow is put back"
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn the_selection_colour_is_the_boxes_restyled_colour() {
    let f = Fixture::mount(
        "<pre>restyled</pre>",
        "[data-pm-selection] { background-color: rgba(200, 0, 0, 0.5) !important; }",
    );
    next_task().await;
    let pre = f.el("pre");
    f.select(Selection::text(Pos(1), Pos(5))).await;
    assert_text_highlighted(&f, &pre, "rgba(200, 0, 0, 0.5)", "restyled");
    f.teardown();
}

#[wasm_bindgen_test]
async fn unmounting_a_selected_editor_leaves_no_highlight() {
    let f = Fixture::mount("<pre>gone</pre>", "");
    next_task().await;
    let before = highlight_ranges();
    f.select(Selection::text(Pos(1), Pos(4))).await;
    assert!(
        highlight_ranges() > before,
        "positive control: a range is registered"
    );
    f.teardown();
    next_task().await;
    assert_eq!(
        highlight_ranges(),
        before,
        "the page's registry keeps a range"
    );
}

#[wasm_bindgen_test]
async fn the_highlight_follows_the_text_when_a_mark_rewraps_it() {
    // Bold over the selection rebuilds the text's DOM; the selection is the
    // same positions, and its highlight must still be over the text.
    let f = Fixture::mount("<p>alpha <mark>bravo</mark> tail</p>", "");
    next_task().await;
    f.select(Selection::text(Pos(7), Pos(12))).await;
    assert!(
        f.handle.command("toggleBold"),
        "positive control: bold applied"
    );
    next_frame().await;
    next_frame().await;
    let p = f.el("p");
    assert!(
        p.query_selector("strong, b").unwrap().is_some(),
        "positive control: the text is rewrapped: {}",
        p.inner_html()
    );
    let bold = p.query_selector("strong, b").unwrap().unwrap();
    assert_text_highlighted(&f, &bold, BLUE, "after bold");
    f.teardown();
}

// From the review of #2673.

#[wasm_bindgen_test]
/// A wash recoloured while its cell stays selected (an app rule that
/// depends on focus or a theme) used to save its own first colour as the
/// cell's shadow, and put it back for good when the selection left.
async fn a_recoloured_cell_wash_leaves_no_wash_behind() {
    let f = Fixture::mount(
        "<table><tr><th><p>head</p></th><td><p>cell</p></td></tr></table>",
        "",
    );
    next_task().await;
    let th = f.el("th");
    let own = computed(&th, "box-shadow");
    f.select(Selection::cell(Pos(2), Pos(2))).await;
    let blue = computed(&th, "box-shadow");
    assert!(blue.contains(BLUE), "positive control {blue}");
    // The app's rule changes the colour (a focus-dependent rule, a theme).
    let red = document().create_element("style").unwrap();
    red.set_text_content(Some(
        "[data-pm-editor] [data-pm-selection] { background-color: rgb(200, 0, 0) !important; }",
    ));
    document().head().unwrap().append_child(&red).unwrap();
    // The same cell stays selected; the selection grows by one cell.
    f.select(Selection::cell(Pos(2), Pos(10))).await;
    let during = computed(&th, "box-shadow");
    let td = f.el("td");
    let td_during = computed(&td, "box-shadow");
    red.remove();
    f.select(Selection::text(Pos(4), Pos(6))).await;
    let after = computed(&th, "box-shadow");
    f.teardown();
    assert!(
        during.contains("rgb(200, 0, 0)") && after == own,
        "th during: `{during}` (td: `{td_during}`); th after: `{after}`, own `{own}`"
    );
}

#[wasm_bindgen_test]
async fn a_selection_across_blocks_and_a_second_editor_each_keep_their_highlight() {
    let f = Fixture::mount("<p>abc</p><p dir=\"rtl\">שלום עולם</p><pre>xyz</pre>", "");
    let g = {
        // A second editor on the page, mounted the same way.
        let host2 = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host2).unwrap();
        let handle = create_editor();
        assert!(handle.load_html("<pre>second</pre>"));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host2,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        (root, host2, handle)
    };
    next_task().await;
    let base = highlight_ranges();
    // abc (1..4), rtl p (6..6+19 bytes? positions are chars), pre.
    f.select(Selection::text(Pos(2), Pos(14))).await;
    let ps = f.host.query_selector_all("[data-pm-editor] p").unwrap();
    let p0: web_sys::Element = ps.item(0).unwrap().dyn_into().unwrap();
    let p1: web_sys::Element = ps.item(1).unwrap().dyn_into().unwrap();
    assert_text_highlighted(&f, &p0, BLUE, "first block");
    assert_text_highlighted(&f, &p1, BLUE, "rtl block");
    g.2.focus();
    g.2.set_selection(Selection::text(Pos(1), Pos(4)));
    next_task().await;
    let second_pre = g.1.query_selector("pre").unwrap().unwrap();
    assert!(!highlight_over(&second_pre).is_empty(), "second editor paints its range");
    // Unmount the first editor: the second's highlight stays.
    f.teardown();
    next_task().await;
    assert!(!highlight_over(&second_pre).is_empty(), "second editor's highlight survives");
    g.0.unmount();
    g.1.remove();
    next_task().await;
    assert_eq!(highlight_ranges(), base, "everything released");
}

/// An inline decoration over 3..13, switched on by the cell.
struct ToggleDeco(std::rc::Rc<std::cell::Cell<bool>>);
impl rinch_editor_core::Plugin for ToggleDeco {
    fn key(&self) -> rinch_editor_core::PluginKey {
        rinch_editor_core::PluginKey("review2673.deco")
    }
    fn decorations(
        &self,
        _state: &rinch_editor_core::EditorState,
    ) -> rinch_editor_core::decoration::DecorationSet {
        use rinch_editor_core::decoration::{Decoration, DecorationSet};
        if !self.0.get() {
            return DecorationSet::new(vec![]);
        }
        DecorationSet::new(vec![Decoration::inline(
            Pos(3),
            Pos(13),
            rinch_editor_core::Attrs::new().with("class", "probe-deco"),
        )])
    }
}

#[wasm_bindgen_test]
/// An inline decoration re-applied with no document change (a late
/// spellcheck result, find-as-you-type) rebuilds the runs under the
/// selection; the highlight used to stay collapsed until the selection moved.
async fn a_decoration_change_keeps_the_highlight() {
    let on = std::rc::Rc::new(std::cell::Cell::new(false));
    let f = Fixture::mount("<pre>Hello world one</pre>", "");
    assert!(f.handle.add_plugin(std::rc::Rc::new(ToggleDeco(on.clone()))));
    next_task().await;
    let sel = Selection::text(Pos(5), Pos(10));
    f.select(sel.clone()).await;
    let pre = f.el("pre");
    assert_text_highlighted(&f, &pre, BLUE, "before");
    on.set(true);
    // The same selection again: a transaction with no document change.
    f.select(sel).await;
    next_frame().await;
    let html = pre.inner_html();
    assert!(html.contains("probe-deco"), "positive control: decorated {html}");
    let names = highlight_over(&pre);
    let text = js_sys::eval("(() => { let s=''; CSS.highlights.forEach((h,n)=>{ for (const r of h) s += n+':'+JSON.stringify(r.toString())+';'; }); return s; })()").unwrap().as_string().unwrap();
    f.teardown();
    assert!(!names.is_empty(), "highlight lost after a decoration-only change: {text} / {html}");
    assert!(text.contains("\"o wor\""), "highlight text is {text}");
}

/// Each colour the boxes take gets a highlight; one no range uses any more is
/// unregistered, so a colour that keeps changing does not grow the page's
/// registry (a `transition` on the boxes, a theme switching back and forth).
#[wasm_bindgen_test]
async fn colours_no_longer_used_leave_the_registry() {
    let f = Fixture::mount("<pre>colours</pre>", "");
    next_task().await;
    let base = registered_highlights();
    let pre = f.el("pre");
    let sheet = document().create_element("style").unwrap();
    document().head().unwrap().append_child(&sheet).unwrap();
    let mut names = Vec::new();
    for i in 0..5 {
        sheet.set_text_content(Some(&format!(
            "[data-pm-editor] [data-pm-selection] {{ background-color: rgb({}, 0, 0) !important; }}",
            100 + i
        )));
        f.select(Selection::text(Pos(1), Pos(3 + i))).await;
        names.push(highlight_over(&pre));
    }
    let in_use = registered_highlights();
    // Cleaned up before asserting: the colour rule must not reach the next
    // test.
    sheet.remove();
    f.select(Selection::cursor(Pos(2))).await;
    let after = registered_highlights();
    f.teardown();
    assert!(names.iter().all(|n| !n.is_empty()), "positive control: {names:?}");
    assert_eq!(in_use, base + 1, "only the colour in use is registered ({names:?})");
    assert_eq!(after, base, "no colour in use");
}
