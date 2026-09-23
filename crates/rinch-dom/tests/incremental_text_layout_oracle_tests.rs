//! Incremental text layout against a fresh layout of the final state.
//!
//! Every fixture here builds a document in state **A**, lays it out, changes
//! it into state **B** through the DOM API, lays it out again, and compares
//! the result with a *second* document built directly in state B and laid
//! out once. The two must agree on every box and on every IFC root's shaped
//! text — size, line count, and the brush, font size, position and
//! decorations of each glyph run. The fresh twin is the oracle: it has no
//! cache to be stale, so any difference is an invalidation the incremental
//! path missed.
//!
//! Each fixture also checks the **counter-oracle** — state A's layout differs
//! from state B's — so a change that happens to move nothing on this host
//! fails loudly rather than passing vacuously.
//!
//! These exist because layout keeps two derived text caches across frames —
//! each IFC root's `text_layout` (the Parley layout paint uses) and the
//! per-root `ifc_measure_cache` (the size Taffy was given) — and neither is
//! dropped wholesale any more: a restyle drops only the roots whose text
//! inputs changed (`ComputedStyle::same_text_layout_inputs`), and a
//! structural change only the roots whose content changed (the IFC content
//! signature). One fixture per input those two rules have to cover.
//!
//! Every value is moved **off** its natural zero or default (letter-spacing
//! 1px → 3px, not 0 → 3px; a colour to another non-black colour), so a
//! mutant that confuses "unchanged" with "default" cannot pass by accident.
//! `line-height` is declared everywhere so no line box is derived from a font
//! metric.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (800.0, 600.0);

const BASE_CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px;
           color: rgb(10, 20, 30); letter-spacing: 1px; word-spacing: 1px; }
    .box { width: 180px; }
    .chip { display: inline-block; padding: 2px; }
    .list { display: flex; flex-direction: column; }
    .row { display: block; padding: 2px; }
";

// ── Snapshot ────────────────────────────────────────────────────────────────

/// A structural description of everything layout produced, in DOM pre-order,
/// free of node ids so two documents can be compared.
fn snapshot(doc: &RinchDocument) -> Vec<String> {
    let mut out = Vec::new();
    walk(doc, doc.tree.html_id, 0, &mut out);
    out
}

fn r(v: f32) -> f32 {
    (v * 64.0).round() / 64.0
}

fn walk(doc: &RinchDocument, id: usize, depth: usize, out: &mut Vec<String>) {
    let Some(node) = doc.tree.get(id) else {
        return;
    };
    let what = match node.tag() {
        Some(t) => format!(
            "<{t}{}>",
            if node.is_pseudo_element {
                "::pseudo"
            } else {
                ""
            }
        ),
        None => match &node.kind {
            rinch_dom::node::NodeKind::Text(t) => format!("{:?}", t.content),
            _ => "?".to_string(),
        },
    };
    let l = &node.layout;
    // A text node inside an IFC owns no box — its glyphs are the IFC's, and
    // are compared below — so its `layout` is whatever Taffy last left on a
    // detached leaf, which differs between a first and a later structural
    // pass without anything reading it.
    let is_text = node.tag().is_none();
    out.push(format!(
        "{:indent$}{what}{}",
        "",
        if is_text {
            String::new()
        } else {
            format!(" @({}, {}) {}x{}", r(l.x), r(l.y), r(l.width), r(l.height))
        },
        indent = depth * 2
    ));
    if let Some(tl) = &node.text_layout {
        out.push(format!(
            "{:indent$}  ifc {:?} {}x{} lines={}",
            "",
            tl.text_content,
            r(tl.layout.width()),
            r(tl.layout.height()),
            tl.layout.len(),
            indent = depth * 2
        ));
        for bg in &tl.background_spans {
            out.push(format!(
                "{:indent$}  bg {}..{} {:?} pad-left={}",
                "",
                bg.start,
                bg.end,
                bg.color,
                r(bg.padding_left),
                indent = depth * 2
            ));
        }
        for (li, line) in tl.layout.lines().enumerate() {
            for item in line.items() {
                if let parley::layout::PositionedLayoutItem::GlyphRun(gr) = item {
                    let style = gr.style();
                    out.push(format!(
                        "{:indent$}  line{li} run x={} base={} adv={} glyphs={} size={} brush={:?} ul={} st={}",
                        "",
                        r(gr.offset()),
                        r(gr.baseline()),
                        r(gr.advance()),
                        gr.glyphs().count(),
                        r(gr.run().font_size()),
                        style.brush,
                        style.underline.is_some(),
                        style.strikethrough.is_some(),
                        indent = depth * 2
                    ));
                }
            }
        }
    }
    for &c in &node.children {
        walk(doc, c, depth + 1, out);
    }
}

fn settle(doc: &mut RinchDocument) {
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
}

/// Build state A with `build(doc, false)`, lay it out, `change` it into state
/// B, lay it out, and compare with `build(doc, true)` laid out fresh.
#[track_caller]
fn twin<H>(
    name: &str,
    css: &str,
    build: impl Fn(&mut RinchDocument, bool) -> H,
    change: impl FnOnce(&mut RinchDocument, &H),
) {
    let make = |state: bool| {
        let mut doc = RinchDocument::new();
        doc.load_css(BASE_CSS);
        doc.load_css(css);
        let h = build(&mut doc, state);
        settle(&mut doc);
        (doc, h)
    };
    let (mut doc, h) = make(false);
    let before = snapshot(&doc);
    change(&mut doc, &h);
    doc.resolve_layout(VP.0, VP.1);
    let incremental = snapshot(&doc);
    let (fresh_doc, _) = make(true);
    let fresh = snapshot(&fresh_doc);

    assert_ne!(
        before, fresh,
        "{name}: states A and B lay out identically, so this fixture cannot see anything"
    );
    if incremental != fresh {
        let mut diff = String::new();
        for (i, (a, b)) in incremental.iter().zip(fresh.iter()).enumerate() {
            if a != b {
                diff.push_str(&format!(
                    "  #{i}\n    incremental: {a}\n    fresh:       {b}\n"
                ));
            }
        }
        if incremental.len() != fresh.len() {
            diff.push_str(&format!(
                "  lengths differ: incremental {} vs fresh {}\n",
                incremental.len(),
                fresh.len()
            ));
        }
        panic!(
            "{name}: the incremental layout differs from a fresh layout of the final state\n{diff}"
        );
    }
}

// ── Typography reaching an IFC from an ancestor ───────────────────────────────

/// `body > div.wrap > p.box > ["alpha … theta ", span > "iota kappa lambda", " mu nu"]`.
/// State B adds `on` to the wrapper, so every rule of the form `.on …` is an
/// **ancestor** restyle whose values reach the paragraph and the span inside
/// it by inheritance (or, for the `span`-only rules, reach only the span).
fn ifc_doc(doc: &mut RinchDocument, on: bool) -> NodeId {
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", if on { "wrap on" } else { "wrap" });
    let p = doc.create_element("p");
    doc.set_attribute(p, "class", "box");
    let t1 = doc.create_text("alpha beta gamma delta epsilon zeta eta theta ");
    doc.append_child(p, t1);
    let span = doc.create_element("span");
    let t2 = doc.create_text("iota kappa lambda");
    doc.append_child(span, t2);
    doc.append_child(p, span);
    let t3 = doc.create_text(" mu nu xi omicron");
    doc.append_child(p, t3);
    doc.append_child(wrap, p);
    doc.append_child(body, wrap);
    wrap
}

fn toggle_on(doc: &mut RinchDocument, wrap: &NodeId) {
    doc.set_attribute(*wrap, "class", "wrap on");
}

macro_rules! ancestor_typography {
    ($name:ident, $css:expr) => {
        #[test]
        fn $name() {
            twin(stringify!($name), $css, ifc_doc, toggle_on);
        }
    };
}

ancestor_typography!(font_size_from_an_ancestor, ".on { font-size: 23px; }");
ancestor_typography!(font_weight_from_an_ancestor, ".on { font-weight: 700; }");
ancestor_typography!(
    font_family_from_an_ancestor,
    ".on { font-family: monospace; }"
);
ancestor_typography!(font_style_from_an_ancestor, ".on { font-style: italic; }");
ancestor_typography!(line_height_from_an_ancestor, ".on { line-height: 31px; }");
ancestor_typography!(
    letter_spacing_from_an_ancestor,
    ".on { letter-spacing: 3px; }"
);
ancestor_typography!(word_spacing_from_an_ancestor, ".on { word-spacing: 7px; }");
ancestor_typography!(colour_from_an_ancestor, ".on { color: rgb(200, 100, 50); }");
ancestor_typography!(
    text_transform_from_an_ancestor,
    ".on { text-transform: uppercase; }"
);
ancestor_typography!(white_space_from_an_ancestor, ".on { white-space: nowrap; }");
ancestor_typography!(text_align_from_an_ancestor, ".on { text-align: right; }");
ancestor_typography!(
    underline_on_the_root,
    ".on .box { text-decoration: underline; }"
);
ancestor_typography!(
    line_through_on_the_span,
    ".on span { text-decoration: line-through; }"
);
ancestor_typography!(
    available_width_from_an_ancestor_rule,
    ".on .box { width: 120px; }"
);

// Only the span's typography changes; the paragraph that holds the IFC
// restyles to exactly the style it had. The IFC must still be re-measured,
// through the span's own comparison — and the paragraph's Taffy node marked,
// since the span has none of its own in the tree.
ancestor_typography!(font_size_on_the_span_only, ".on span { font-size: 27px; }");
ancestor_typography!(
    colour_on_the_span_only,
    ".on span { color: rgb(0, 150, 60); }"
);
ancestor_typography!(
    inline_background_on_the_span_only,
    ".on span { background-color: rgb(250, 240, 10); padding-left: 9px; }"
);

/// The measure cache keeps one size per available width. A typography change
/// has to drop **all** of them, not just the one at the width the next compute
/// happens to ask about: here the root is measured at 180px, then at 120px,
/// then its font changes while it is 120px wide, then it goes back to 180px —
/// where a size kept from before the font change would be served.
#[test]
fn a_font_change_drops_the_sizes_cached_at_other_widths() {
    let css = ".box { width: 180px; } .narrow .box { width: 120px; } .big { font-size: 23px; }";
    let make = |final_state: bool| {
        let mut doc = RinchDocument::new();
        doc.load_css(BASE_CSS);
        doc.load_css(css);
        let wrap = ifc_doc(&mut doc, false);
        if final_state {
            doc.set_attribute(wrap, "class", "wrap big");
        }
        settle(&mut doc);
        (doc, wrap)
    };
    let (mut doc, wrap) = make(false);
    let before = snapshot(&doc);
    doc.set_attribute(wrap, "class", "wrap narrow");
    doc.resolve_layout(VP.0, VP.1);
    doc.set_attribute(wrap, "class", "wrap narrow big");
    doc.resolve_layout(VP.0, VP.1);
    doc.set_attribute(wrap, "class", "wrap big");
    doc.resolve_layout(VP.0, VP.1);
    let incremental = snapshot(&doc);
    let (fresh_doc, _) = make(true);
    let fresh = snapshot(&fresh_doc);
    assert_ne!(
        before, fresh,
        "counter-oracle: the font change must move something"
    );
    assert_eq!(
        incremental, fresh,
        "a size cached at 180px before the font changed was served after it"
    );
}

/// `overflow-wrap: anywhere` breaks a word longer than the line; the default
/// overflows instead.
#[test]
fn overflow_wrap_from_an_ancestor() {
    twin(
        "overflow_wrap_from_an_ancestor",
        ".on { overflow-wrap: anywhere; } .box { width: 60px; }",
        |doc, on| {
            let body = doc.body();
            let wrap = doc.create_element("div");
            doc.set_attribute(wrap, "class", if on { "wrap on" } else { "wrap" });
            let p = doc.create_element("p");
            doc.set_attribute(p, "class", "box");
            let t = doc.create_text("supercalifragilistic word");
            doc.append_child(p, t);
            doc.append_child(wrap, p);
            doc.append_child(body, wrap);
            wrap
        },
        toggle_on,
    );
}

/// The same inherited change arriving by `:hover` instead of an attribute.
#[test]
fn font_size_from_a_hovered_ancestor() {
    twin(
        "font_size_from_a_hovered_ancestor",
        ".wrap:hover span, .wrap.on span { font-size: 27px; }",
        ifc_doc,
        |doc, wrap| {
            let mut changed = false;
            doc.update_hover(Some(wrap.0), &mut changed);
        },
    );
}

/// An inline `style` write on the IFC root itself, whose text is its own
/// (no inline element inside it to inherit the write).
#[test]
fn font_size_written_inline_on_the_root() {
    twin(
        "font_size_written_inline_on_the_root",
        ".on .box { font-size: 23px; }",
        |doc, on| {
            let body = doc.body();
            let wrap = doc.create_element("div");
            doc.set_attribute(wrap, "class", if on { "wrap on" } else { "wrap" });
            let p = doc.create_element("p");
            doc.set_attribute(p, "class", "box");
            let t = doc.create_text("alpha beta gamma delta epsilon zeta eta theta iota");
            doc.append_child(p, t);
            doc.append_child(wrap, p);
            doc.append_child(body, wrap);
            p
        },
        |doc, p| doc.set_style(*p, "font-size", "23px"),
    );
}

// ── Structure inside an IFC ─────────────────────────────────────────────────

/// An inline member stops generating a box (`display: none`), so its text
/// leaves the line.
#[test]
fn an_inline_member_goes_display_none() {
    twin(
        "an_inline_member_goes_display_none",
        ".on span { display: none; }",
        ifc_doc,
        toggle_on,
    );
}

/// An inline member becomes atomic.
#[test]
fn an_inline_member_goes_inline_block() {
    twin(
        "an_inline_member_goes_inline_block",
        ".on span { display: inline-block; width: 50px; }",
        ifc_doc,
        toggle_on,
    );
}

/// Text appended into a span inside the paragraph: the IFC root is the
/// paragraph, two levels above the insertion.
#[test]
fn text_appended_inside_an_inline_span() {
    twin(
        "text_appended_inside_an_inline_span",
        "",
        |doc, on| {
            let wrap = ifc_doc(doc, false);
            let p = doc.tree.get(wrap.0).unwrap().children[0];
            let span = NodeId(doc.tree.get(p).unwrap().children[1]);
            if on {
                let extra = doc.create_text(" plus a good deal more text");
                doc.append_child(span, extra);
            }
            span
        },
        |doc, span| {
            let extra = doc.create_text(" plus a good deal more text");
            doc.append_child(*span, extra);
        },
    );
}

/// Text edited to a length that wraps onto more lines (#878's shape, in a
/// block container whose height is its content's).
#[test]
fn set_text_content_that_adds_lines() {
    twin(
        "set_text_content_that_adds_lines",
        "",
        |doc, on| {
            let body = doc.body();
            let list = doc.create_element("div");
            doc.set_attribute(list, "class", "list box");
            let row = doc.create_element("div");
            doc.set_attribute(row, "class", "row");
            let t = doc.create_text(if on {
                "a much longer line of text that wraps several times inside the box"
            } else {
                "short"
            });
            doc.append_child(row, t);
            doc.append_child(list, row);
            doc.append_child(body, list);
            t
        },
        |doc, t| {
            doc.set_text_content(
                *t,
                "a much longer line of text that wraps several times inside the box",
            );
        },
    );
}

/// Issue #878: the same edit on text one element **below** the IFC root —
/// inside a `<span>`, and inside a `display: contents` wrapper (what `rsx!`
/// emits around every `{|| …}`). The invalidation dropped the row's cached
/// measure but marked only the wrapper's Taffy node, which is detached from
/// the tree the compute walks, so the compute served the row its old height:
/// 20px around four lines of text.
fn nested_text_doc(doc: &mut RinchDocument, on: bool, wrapper_style: &str) -> NodeId {
    let body = doc.body();
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    doc.set_attribute(list, "style", "width: 200px");
    let row = doc.create_element("div");
    doc.set_attribute(row, "class", "row");
    let wrap = doc.create_element("span");
    if !wrapper_style.is_empty() {
        doc.set_attribute(wrap, "style", wrapper_style);
    }
    let t = doc.create_text(if on {
        "a line of text that is long enough to wrap onto four lines at two hundred px"
    } else {
        "one line"
    });
    doc.append_child(wrap, t);
    doc.append_child(row, wrap);
    doc.append_child(list, row);
    doc.append_child(body, list);
    t
}

#[test]
fn set_text_content_below_the_root_in_a_span() {
    twin(
        "set_text_content_below_the_root_in_a_span",
        "",
        |doc, on| nested_text_doc(doc, on, ""),
        |doc, t| {
            doc.set_text_content(
                *t,
                "a line of text that is long enough to wrap onto four lines at two hundred px",
            )
        },
    );
}

#[test]
fn set_text_content_below_the_root_in_a_contents_wrapper() {
    twin(
        "set_text_content_below_the_root_in_a_contents_wrapper",
        "",
        |doc, on| nested_text_doc(doc, on, "display: contents"),
        |doc, t| {
            doc.set_text_content(
                *t,
                "a line of text that is long enough to wrap onto four lines at two hundred px",
            )
        },
    );
}

/// An atomic inline's content changes in the same frame as a structural
/// change elsewhere, so the structural pass is what re-sizes the chip — and
/// the IFC that holds the chip has to break its line against the new size.
#[test]
fn a_chip_grows_during_a_structural_pass() {
    twin(
        "a_chip_grows_during_a_structural_pass",
        "",
        |doc, on| {
            let body = doc.body();
            let p = doc.create_element("p");
            doc.set_attribute(p, "class", "box");
            let t = doc.create_text("before the chip ");
            doc.append_child(p, t);
            let chip = doc.create_element("span");
            doc.set_attribute(chip, "class", "chip");
            let ct = doc.create_text(if on { "a far wider chip label" } else { "c" });
            doc.append_child(chip, ct);
            doc.append_child(p, chip);
            let t2 = doc.create_text(" after");
            doc.append_child(p, t2);
            doc.append_child(body, p);
            let other = doc.create_element("div");
            doc.append_child(body, other);
            if on {
                let x = doc.create_element("div");
                doc.append_child(other, x);
            }
            (ct, other)
        },
        |doc, (ct, other)| {
            doc.set_text_content(*ct, "a far wider chip label");
            let x = doc.create_element("div");
            doc.append_child(*other, x);
        },
    );
}

/// `::before` content that exists only in state B.
#[test]
fn before_content_toggled_by_class() {
    twin(
        "before_content_toggled_by_class",
        ".on .box::before { content: \"PREFIX TEXT \"; }",
        ifc_doc,
        toggle_on,
    );
}

/// `::before` content that changes text between two non-empty values.
#[test]
fn before_content_changed_by_class() {
    twin(
        "before_content_changed_by_class",
        ".box::before { content: \"one \"; } .on .box::before { content: \"a longer prefix \"; }",
        ifc_doc,
        toggle_on,
    );
}

/// The same swap one level down, inside a `<span>`: the mutation verb
/// invalidates the span (the moved node's parent), which is not the IFC
/// root, so what reaches the paragraph is the moved node's own `ifc_root` —
/// or, failing that, the structural pass's signature, whose sibling index is
/// the only part of it a pure reorder moves.
#[test]
fn reorder_text_inside_a_span_inside_the_root() {
    twin(
        "reorder_text_inside_a_span_inside_the_root",
        "",
        |doc, on| {
            let body = doc.body();
            let p = doc.create_element("p");
            doc.set_attribute(p, "class", "box");
            let span = doc.create_element("span");
            let a = doc.create_element("b");
            let at = doc.create_text("a rather long bold opening phrase ");
            doc.append_child(a, at);
            let b = doc.create_text("then plain");
            if on {
                doc.append_child(span, b);
                doc.append_child(span, a);
            } else {
                doc.append_child(span, a);
                doc.append_child(span, b);
            }
            doc.append_child(p, span);
            doc.append_child(body, p);
            (span, a)
        },
        |doc, (span, a)| doc.append_child(*span, *a),
    );
}
// ── Rows in a list: append, remove, reorder ─────────────────────────────────

fn list_doc(doc: &mut RinchDocument, labels: &[&str]) -> (NodeId, Vec<NodeId>) {
    let body = doc.body();
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list box");
    let mut rows = Vec::new();
    for l in labels {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let span = doc.create_element("span");
        let t = doc.create_text(l);
        doc.append_child(span, t);
        doc.append_child(row, span);
        let chip = doc.create_element("span");
        doc.set_attribute(chip, "class", "chip");
        let ct = doc.create_text("chip");
        doc.append_child(chip, ct);
        doc.append_child(row, chip);
        doc.append_child(list, row);
        rows.push(row);
    }
    doc.append_child(body, list);
    (list, rows)
}

const LABELS: [&str; 4] = [
    "first row, which is long enough to wrap in the box",
    "second",
    "third row with a moderate amount of text",
    "fourth",
];

#[test]
fn append_a_row() {
    twin(
        "append_a_row",
        "",
        |doc, on| {
            let mut l = LABELS.to_vec();
            if on {
                l.push("a new row that also wraps onto a second line");
            }
            list_doc(doc, &l)
        },
        |doc, (list, _)| {
            let row = doc.create_element("div");
            doc.set_attribute(row, "class", "row");
            let span = doc.create_element("span");
            let t = doc.create_text("a new row that also wraps onto a second line");
            doc.append_child(span, t);
            doc.append_child(row, span);
            let chip = doc.create_element("span");
            doc.set_attribute(chip, "class", "chip");
            let ct = doc.create_text("chip");
            doc.append_child(chip, ct);
            doc.append_child(row, chip);
            doc.append_child(*list, row);
        },
    );
}

#[test]
fn remove_a_row() {
    twin(
        "remove_a_row",
        "",
        |doc, on| {
            let l: Vec<&str> = if on {
                vec![LABELS[0], LABELS[2], LABELS[3]]
            } else {
                LABELS.to_vec()
            };
            list_doc(doc, &l)
        },
        |doc, (list, rows)| doc.remove_child(*list, rows[1]),
    );
}

#[test]
fn reorder_rows() {
    twin(
        "reorder_rows",
        "",
        |doc, on| {
            let l: Vec<&str> = if on {
                vec![LABELS[2], LABELS[0], LABELS[1], LABELS[3]]
            } else {
                LABELS.to_vec()
            };
            list_doc(doc, &l)
        },
        |doc, (list, rows)| doc.insert_before(*list, rows[2], rows[0]),
    );
}

/// Two text nodes swap places inside one IFC: the same members, the same
/// text, a different order.
#[test]
fn reorder_text_inside_one_ifc() {
    twin(
        "reorder_text_inside_one_ifc",
        "",
        |doc, on| {
            let body = doc.body();
            let p = doc.create_element("p");
            doc.set_attribute(p, "class", "box");
            let a = doc.create_element("b");
            let at = doc.create_text("a rather long bold opening phrase ");
            doc.append_child(a, at);
            let b = doc.create_text("then plain");
            if on {
                doc.append_child(p, b);
                doc.append_child(p, a);
            } else {
                doc.append_child(p, a);
                doc.append_child(p, b);
            }
            doc.append_child(body, p);
            (p, a, b)
        },
        |doc, (p, a, _)| {
            doc.append_child(*p, *a);
        },
    );
}
