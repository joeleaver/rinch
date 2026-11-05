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

fn walk(doc: &RinchDocument, id: u64, depth: usize, out: &mut Vec<String>) {
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
            "{:indent$}  ifc {:?} {}x{} lines={} painted={:?}",
            "",
            tl.text_content,
            r(tl.layout.width()),
            r(tl.layout.height()),
            tl.layout.len(),
            // What a `text-overflow: ellipsis` cut draws (#1100).
            tl.painted_text(),
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
    // A text leaf — a flex or grid item's text, in a block-level container or
    // inside an `inline-flex` / `inline-grid` (#904) — is painted from the
    // layout its measure built, cached on the node. Compared here, run by run,
    // like an IFC root's — except the brush: paint draws a leaf in its parent's
    // *current* colour, not the one it was shaped with (#904), so that colour
    // is what is compared. An IFC member is compared too: it has none in a
    // fresh layout, and one left from when it was a leaf is the #970 leak
    // (the caret and glyph queries read it).
    if let Some(cl) = node.cached_text_parley.as_ref() {
        out.push(format!(
            "{:indent$}  leaf {}x{} lines={}",
            "",
            r(cl.width()),
            r(cl.height()),
            cl.len(),
            indent = depth * 2
        ));
        let leaf_paint = node
            .parent
            .and_then(|p| doc.tree.get(p))
            .and_then(|p| p.computed_style.color);
        for (li, line) in cl.lines().enumerate() {
            for item in line.items() {
                if let parley::layout::PositionedLayoutItem::GlyphRun(gr) = item {
                    out.push(format!(
                        "{:indent$}  leaf line{li} run x={} base={} adv={} glyphs={} size={} paint={:?}",
                        "",
                        r(gr.offset()),
                        r(gr.baseline()),
                        r(gr.advance()),
                        gr.glyphs().count(),
                        r(gr.run().font_size()),
                        leaf_paint,
                        indent = depth * 2
                    ));
                }
            }
        }
    }
    for &c in &node.children {
        walk(doc, c, depth + 1, out);
    }
    // An anonymous block box is in no node's `children` (#566) — it hangs off
    // its container's `run_boxes` — and it is the IFC root of the run it
    // stands for, so its box and shaped text are compared here or not at all.
    for &b in &node.run_boxes {
        out.push(format!(
            "{:indent$}(anonymous)",
            "",
            indent = (depth + 1) * 2
        ));
        walk(doc, b, depth + 1, out);
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

// ── An atomic inline is an IFC root *and* a member of another ────────────────
//
// An `inline-block` / `inline-flex` / `inline-grid` holding text is the IFC
// root of that text and, at the same time, a member of the IFC it sits in. A
// restyle of it has to drop **both** layouts: the outer one (its box moved)
// and its own (its glyphs changed). The member branch of
// `invalidate_ifc_for_node` used to reach only the outer one, which was
// invisible while every restyle also dropped every layout under the restyled
// node — the atomic's own included, through its text child.

/// `p.box > ["alpha beta gamma ", span.chip > "chip label", " theta …"]`.
fn chip_doc(doc: &mut RinchDocument, on: bool) -> NodeId {
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", if on { "wrap on" } else { "wrap" });
    let p = doc.create_element("p");
    doc.set_attribute(p, "class", "box");
    let t1 = doc.create_text("alpha beta gamma ");
    doc.append_child(p, t1);
    let c = doc.create_element("span");
    doc.set_attribute(c, "class", "chip");
    let ct = doc.create_text("chip label");
    doc.append_child(c, ct);
    doc.append_child(p, c);
    let t3 = doc.create_text(" theta iota kappa lambda mu");
    doc.append_child(p, t3);
    doc.append_child(wrap, p);
    doc.append_child(body, wrap);
    wrap
}

fn chip_of(doc: &RinchDocument, wrap: NodeId) -> NodeId {
    let p = doc.tree.get(wrap.0).unwrap().children[0];
    NodeId(doc.tree.get(p).unwrap().children[1])
}

/// The chip's font size, inherited from an ancestor rule.
#[test]
fn an_atomic_members_own_text_takes_an_ancestor_font_size() {
    twin(
        "an_atomic_members_own_text_takes_an_ancestor_font_size",
        ".on .chip { font-size: 23px; }",
        chip_doc,
        toggle_on,
    );
}

/// The chip's colour, from a class written on the chip itself.
#[test]
fn an_atomic_members_own_text_takes_its_class_colour() {
    twin(
        "an_atomic_members_own_text_takes_its_class_colour",
        ".chip.hot { color: rgb(200, 10, 10); }",
        |doc, on| {
            let w = chip_doc(doc, false);
            let c = chip_of(doc, w);
            if on {
                doc.set_attribute(c, "class", "chip hot");
            }
            c
        },
        |doc, c| doc.set_attribute(*c, "class", "chip hot"),
    );
}

/// The shape users meet first: two plain `<button>`s (UA `inline-block`) side
/// by side in a `div`, one gaining a class that changes its colour and weight.
#[test]
fn a_button_in_a_row_of_buttons_takes_its_class_colour_and_weight() {
    twin(
        "a_button_in_a_row_of_buttons_takes_its_class_colour_and_weight",
        "button.active { color: rgb(200, 10, 10); font-weight: 700; }",
        |doc, on| {
            let body = doc.body();
            let d = doc.create_element("div");
            let a = doc.create_element("button");
            let at = doc.create_text("Save");
            doc.append_child(a, at);
            let b = doc.create_element("button");
            let bt = doc.create_text("Cancel");
            doc.append_child(b, bt);
            doc.append_child(d, a);
            doc.append_child(d, b);
            doc.append_child(body, d);
            if on {
                doc.set_attribute(a, "class", "active");
            }
            a
        },
        |doc, a| doc.set_attribute(*a, "class", "active"),
    );
}

fn paint_pixels(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    let mut painter =
        rinch_dom::paint::skia_painter::TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// The chip-colour case read off the pixels: red ink, counted on both sides.
/// (A local pixel oracle — the incremental frame against a fresh one — rather
/// than a glyph-run description, so it also covers what paint does with it.)
#[test]
fn an_atomic_members_class_colour_reaches_the_pixels() {
    let css = ".chip.hot { color: rgb(200, 10, 10); }";
    let make = |on: bool| {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(css);
        let w = chip_doc(&mut d, false);
        let c = chip_of(&d, w);
        if on {
            d.set_attribute(c, "class", "chip hot");
        }
        settle(&mut d);
        (d, c)
    };
    let red = |px: &[[u8; 4]]| {
        px.iter()
            .filter(|p| p[3] > 0 && p[0] as i32 > p[1] as i32 + 80)
            .count()
    };
    let (mut d, c) = make(false);
    assert_eq!(
        red(&paint_pixels(&mut d)),
        0,
        "counter-oracle: no red ink yet"
    );
    d.set_attribute(c, "class", "chip hot");
    d.resolve_layout(VP.0, VP.1);
    let incremental = red(&paint_pixels(&mut d));
    let (mut f, _) = make(true);
    let fresh = red(&paint_pixels(&mut f));
    assert!(fresh > 0, "counter-oracle: the fresh chip draws red ink");
    assert_eq!(incremental, fresh, "red ink, incremental vs fresh");
}

/// Text appended two levels below a chip (inside a `<b>` in it): the verb
/// invalidates the `<b>`, which is neither the chip nor the paragraph. The
/// structural pass's signature finds the chip changed — and has to size the
/// chip again, since `compute_inline_block_layouts` measured it earlier in the
/// same pass against the cached sizes the signature is only now dropping.
#[test]
fn text_appended_inside_an_element_inside_a_chip() {
    twin(
        "text_appended_inside_an_element_inside_a_chip",
        "",
        |doc, on| {
            let body = doc.body();
            let p = doc.create_element("p");
            doc.set_attribute(p, "class", "box");
            let t = doc.create_text("before the chip ");
            doc.append_child(p, t);
            let chip = doc.create_element("span");
            doc.set_attribute(chip, "class", "chip");
            let b = doc.create_element("b");
            let bt = doc.create_text("c");
            doc.append_child(b, bt);
            doc.append_child(chip, b);
            doc.append_child(p, chip);
            let t2 = doc.create_text(" after");
            doc.append_child(p, t2);
            doc.append_child(body, p);
            if on {
                let x = doc.create_text(" and much more chip text");
                doc.append_child(b, x);
            }
            b
        },
        |doc, b| {
            let x = doc.create_text(" and much more chip text");
            doc.append_child(*b, x);
        },
    );
}

// ── Content the verbs do not report to the root ──────────────────────────────

/// `set_inner_html` on a span inside a paragraph frees the span's text node
/// and mints a new one — which the slab hands the **same** id, under the same
/// parent at the same index — and invalidates only the span. So the
/// paragraph's content signature differs from before **only in the text**:
/// this is the witness for hashing text content into the signature.
#[test]
fn set_inner_html_on_a_span_inside_the_root() {
    const NEW: &str = "a far longer replacement that wraps onto more lines than before";
    twin(
        "set_inner_html_on_a_span_inside_the_root",
        "",
        |doc, on| {
            let w = ifc_doc(doc, false);
            let p = doc.tree.get(w.0).unwrap().children[0];
            let s = NodeId(doc.tree.get(p).unwrap().children[1]);
            if on {
                doc.set_inner_html(s, NEW);
            }
            s
        },
        |doc, s| doc.set_inner_html(*s, NEW),
    );
}

// ── A node that stops being an IFC root loses its layout ─────────────────────
//
// Several readers take `text_layout.is_some()` to mean "this node is an IFC
// root" (caret and selection rects, layer bounds, the ancestor walk in
// `invalidate_ifc_for_node`). A node that was a root and no longer is must not
// keep the layout it built then.

/// A span goes `inline → inline-block → inline`: while atomic it is a root and
/// builds a layout of its own; back inline, it is a member again.
#[test]
fn a_span_that_was_briefly_atomic_keeps_no_layout_of_its_own() {
    let css = ".on span { display: inline-block; width: 50px; }";
    let make = || {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(css);
        let w = ifc_doc(&mut d, false);
        settle(&mut d);
        (d, w)
    };
    let (mut d, w) = make();
    let a = snapshot(&d);
    d.set_attribute(w, "class", "wrap on");
    d.resolve_layout(VP.0, VP.1);
    assert_ne!(
        a,
        snapshot(&d),
        "counter-oracle: the atomic span lays out differently"
    );
    d.set_attribute(w, "class", "wrap");
    d.resolve_layout(VP.0, VP.1);
    assert_eq!(
        snapshot(&d),
        a,
        "back to the first state, and nothing left over"
    );
}

/// The paragraph goes `block → flex → block`: while a flex container it is no
/// root, its text runs are flex items, and one of them is edited meanwhile.
#[test]
fn a_root_that_was_briefly_flex_keeps_no_stale_layout() {
    let css = ".on .box { display: flex; }";
    let make = || {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(css);
        let w = ifc_doc(&mut d, false);
        settle(&mut d);
        (d, w)
    };
    let first_text = |d: &RinchDocument, w: NodeId| {
        let p = d.tree.get(w.0).unwrap().children[0];
        NodeId(d.tree.get(p).unwrap().children[0])
    };
    let (mut d, w) = make();
    let a = snapshot(&d);
    d.set_attribute(w, "class", "wrap on");
    d.resolve_layout(VP.0, VP.1);
    assert_ne!(
        a,
        snapshot(&d),
        "counter-oracle: flex lays the paragraph out differently"
    );
    let t = first_text(&d, w);
    d.set_text_content(t, "short ");
    d.resolve_layout(VP.0, VP.1);
    d.set_attribute(w, "class", "wrap");
    d.resolve_layout(VP.0, VP.1);
    let incremental = snapshot(&d);
    let (mut f, w2) = make();
    let t2 = first_text(&f, w2);
    f.set_text_content(t2, "short ");
    settle(&mut f);
    assert_eq!(incremental, snapshot(&f));
}

/// #970: a text node that was a flex item's leaf and becomes an IFC member
/// keeps no leaf layout, so the caret and glyph queries on it answer as they
/// do in a fresh layout (the IFC root answers for its text; the member itself
/// has no layout of its own). The last text run is used: as a flex item it is
/// narrow and wraps, so a stale leaf layout is several lines tall.
#[test]
fn a_text_node_that_stops_being_a_leaf_answers_no_stale_caret_970() {
    use rinch_dom::text_query::{caret_position_for_offset, glyph_bounds_for_offset};
    let css = ".on .box { display: flex; }";
    let make = |on: bool| {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(css);
        let w = ifc_doc(&mut d, on);
        settle(&mut d);
        (d, w)
    };
    let last_text = |d: &RinchDocument, w: NodeId| {
        let p = d.tree.get(w.0).unwrap().children[0];
        *d.tree.get(p).unwrap().children.last().unwrap()
    };
    let (mut d, w) = make(true);
    let t = last_text(&d, w);
    // Positive control: as a flex item's text the node is a leaf and answers.
    assert!(
        caret_position_for_offset(&d, t, 8).is_some(),
        "counter-oracle: a flex item's text has a leaf layout"
    );
    d.set_attribute(w, "class", "wrap");
    d.resolve_layout(VP.0, VP.1);
    let (f, w2) = make(false);
    let t2 = last_text(&f, w2);
    assert_eq!(
        caret_position_for_offset(&d, t, 8),
        caret_position_for_offset(&f, t2, 8),
        "caret on an IFC member that used to be a leaf"
    );
    assert_eq!(
        glyph_bounds_for_offset(&d, t, 8).map(|g| (g.x, g.y, g.width, g.height)),
        glyph_bounds_for_offset(&f, t2, 8).map(|g| (g.x, g.y, g.width, g.height)),
        "glyph bounds on an IFC member that used to be a leaf"
    );
    assert!(d.tree.get(t).unwrap().cached_text_parley.is_none());
}

// ── Width alone ──────────────────────────────────────────────────────────────

/// A paragraph whose width changes because a flex sibling's text grew, while
/// the sibling's IFC is the one in the dirty set: the paragraph's own text is
/// not dirty, but it has to be re-broken at its new width.
#[test]
fn a_width_change_from_a_siblings_text_rebreaks_the_root() {
    twin(
        "a_width_change_from_a_siblings_text_rebreaks_the_root",
        ".row2 { display: flex; width: 300px; } .row2 > p { flex: 1; margin: 0; } \
         .row2 > div { flex: none; }",
        |doc, on| {
            let body = doc.body();
            let r = doc.create_element("div");
            doc.set_attribute(r, "class", "row2");
            let p = doc.create_element("p");
            let t =
                doc.create_text("paragraph text that wraps across several lines in the flex item");
            doc.append_child(p, t);
            let side = doc.create_element("div");
            let st = doc.create_text(if on { "wide side label" } else { "s" });
            doc.append_child(side, st);
            doc.append_child(r, p);
            doc.append_child(r, side);
            doc.append_child(body, r);
            st
        },
        |doc, st| doc.set_text_content(*st, "wide side label"),
    );
}

// ── A text run laid out by a box the element tree does not hold ─────────────

/// `div.outer > div.a > ["t", li.b > ""]`. `div.a` holds a block child, so its
/// text run is laid out by an **anonymous block box** (CSS 2.1 §9.2.1.1) that
/// is not in `div.a`'s `children` — the IFC root of `"t"` is that box, not
/// `div.a`. State B makes `div.a`'s font-size 16px → 20px through an ancestor
/// class; the `line-height` is a declared multiple of it, so the line box
/// grows 20px → 25px on every font set.
///
/// The restyle has to reach the anonymous box's measure. It used to reach only
/// `div.a` itself (which holds no IFC) and its text child's `NodeContext`, so
/// the box kept its 20px line and `div.a` came out 5px short.
#[test]
fn a_font_change_reaches_an_anonymous_block_box() {
    twin(
        "a_font_change_reaches_an_anonymous_block_box",
        ".c .a { font-size: 20px; } .a { line-height: 1.25; } .b { width: 2em; }",
        |doc, on| {
            let body = doc.body();
            let outer = doc.create_element("div");
            doc.set_attribute(outer, "class", if on { "x c" } else { "x" });
            doc.append_child(body, outer);
            let a = doc.create_element("div");
            doc.set_attribute(a, "class", "a");
            doc.append_child(outer, a);
            let t = doc.create_text("t");
            doc.append_child(a, t);
            let li = doc.create_element("li");
            doc.set_attribute(li, "class", "b");
            doc.append_child(a, li);
            let t2 = doc.create_text("");
            doc.append_child(li, t2);
            outer
        },
        |doc, outer| doc.set_attribute(*outer, "class", "x c"),
    );
}

/// The block-in-inline shape (#513): `fieldset.c > div > span.b.d > ["t",
/// fieldset]`. The span holds a block-level child, so it is **split** and its
/// text run is laid out by an anonymous box around the inline fragment. State
/// B adds `a` to the span itself, taking its font-size 16px → 20px; the text's
/// IFC is not the span's (it has none) and not the div's.
#[test]
fn a_font_change_on_a_split_inline_reaches_its_anonymous_box() {
    twin(
        "a_font_change_on_a_split_inline_reaches_its_anonymous_box",
        ".c .a { font-size: 20px; } .b { line-height: 1.25; }",
        |doc, on| {
            let body = doc.body();
            let fs = doc.create_element("fieldset");
            doc.set_attribute(fs, "class", "c");
            doc.append_child(body, fs);
            let div = doc.create_element("div");
            doc.append_child(fs, div);
            let span = doc.create_element("span");
            doc.set_attribute(span, "class", if on { "b d a" } else { "b d" });
            doc.append_child(div, span);
            let t = doc.create_text("t");
            doc.append_child(span, t);
            let inner = doc.create_element("fieldset");
            doc.append_child(span, inner);
            span
        },
        |doc, span| doc.set_attribute(*span, "class", "b d a"),
    );
}

/// The two fixtures above with the line box left to the font's own metrics —
/// the shape the defect was found in. Twin comparison only, so no literal
/// here depends on the local font set.
#[test]
fn a_font_change_reaches_an_anonymous_block_box_with_a_normal_line_height() {
    twin(
        "a_font_change_reaches_an_anonymous_block_box_with_a_normal_line_height",
        "body { line-height: normal; } .c .a { font-size: 20px; } .b { width: 2em; }",
        |doc, on| {
            let body = doc.body();
            let outer = doc.create_element("div");
            doc.set_attribute(outer, "class", if on { "x c" } else { "x" });
            doc.append_child(body, outer);
            let a = doc.create_element("div");
            doc.set_attribute(a, "class", "a");
            doc.append_child(outer, a);
            let t = doc.create_text("t");
            doc.append_child(a, t);
            let li = doc.create_element("li");
            doc.set_attribute(li, "class", "b");
            doc.append_child(a, li);
            let t2 = doc.create_text("");
            doc.append_child(li, t2);
            outer
        },
        |doc, outer| doc.set_attribute(*outer, "class", "x c"),
    );
}

#[test]
fn a_font_change_on_a_split_inline_with_a_normal_line_height() {
    twin(
        "a_font_change_on_a_split_inline_with_a_normal_line_height",
        "body { line-height: normal; } .c .a { font-size: 20px; }",
        |doc, on| {
            let body = doc.body();
            let fs = doc.create_element("fieldset");
            doc.set_attribute(fs, "class", "c");
            doc.append_child(body, fs);
            let div = doc.create_element("div");
            doc.append_child(fs, div);
            let span = doc.create_element("span");
            doc.set_attribute(span, "class", if on { "b d a" } else { "b d" });
            doc.append_child(div, span);
            let t = doc.create_text("t");
            doc.append_child(span, t);
            let inner = doc.create_element("fieldset");
            doc.append_child(span, inner);
            span
        },
        |doc, span| doc.set_attribute(*span, "class", "b d a"),
    );
}

// ── A connected move keeps the moved root's own layout (#914) ───────────────
//
// A keyed `for` reorder repositions a live row with `insert_before`, and the
// move verbs no longer drop the moved node's **own** IFC layout nor re-cascade
// it when it stays under the same parent: its content is unchanged, and what
// its position can change is left to the cascade's typography comparison and
// to the selector flags (`note_child_list_changed`). Each fixture below is a
// move where the moved row's text *does* have to change, through one of those
// two routes — so a move path that kept too much fails here.

/// Two lists, the second one's rows inherit a different colour, size and
/// letter-spacing. A row moved **between** them is re-cascaded (the parent
/// changed), and its glyphs must follow the new inherited typography.
#[test]
fn a_row_moved_to_another_parent_takes_its_inherited_typography() {
    twin(
        "a_row_moved_to_another_parent_takes_its_inherited_typography",
        ".big { color: rgb(0, 120, 200); font-size: 20px; line-height: 26px; \
                letter-spacing: 3px; }",
        |doc, on| {
            let body = doc.body();
            let mk_row = |doc: &mut RinchDocument, label: &str| {
                let row = doc.create_element("div");
                doc.set_attribute(row, "class", "row");
                let t = doc.create_text(label);
                doc.append_child(row, t);
                row
            };
            let a = doc.create_element("div");
            doc.set_attribute(a, "class", "list box");
            let b = doc.create_element("div");
            doc.set_attribute(b, "class", "list box big");
            let r0 = mk_row(doc, LABELS[0]);
            let r1 = mk_row(doc, LABELS[1]);
            let r2 = mk_row(doc, LABELS[2]);
            let r3 = mk_row(doc, LABELS[3]);
            doc.append_child(a, r0);
            if !on {
                doc.append_child(a, r1);
            }
            doc.append_child(a, r2);
            doc.append_child(b, r3);
            if on {
                doc.append_child(b, r1);
            }
            doc.append_child(body, a);
            doc.append_child(body, b);
            (b, r1)
        },
        |doc, (b, r1)| doc.append_child(*b, *r1),
    );
}

/// Same parent, and a structural selector that sets typography: the row that
/// lands at `:nth-child(2)` must be re-cascaded and re-shaped although its
/// parent did not change, and the row it displaced must lose that style.
#[test]
fn a_row_moved_into_an_nth_child_slot_takes_its_typography() {
    twin(
        "a_row_moved_into_an_nth_child_slot_takes_its_typography",
        ".row:nth-child(2) { color: rgb(200, 30, 30); font-size: 22px; line-height: 28px; }",
        |doc, on| {
            let l: Vec<&str> = if on {
                vec![LABELS[0], LABELS[3], LABELS[1], LABELS[2]]
            } else {
                LABELS.to_vec()
            };
            list_doc(doc, &l)
        },
        |doc, (list, rows)| doc.insert_before(*list, rows[3], rows[1]),
    );
}

/// `:first-child` and `+`: moving the last row to the front changes which row
/// is first and which follows which, in one parent.
#[test]
fn a_row_moved_to_the_front_takes_first_child_and_sibling_typography() {
    twin(
        "a_row_moved_to_the_front_takes_first_child_and_sibling_typography",
        ".row:first-child { font-size: 22px; line-height: 28px; } \
         .row:first-child + .row { color: rgb(0, 150, 0); letter-spacing: 3px; }",
        |doc, on| {
            let l: Vec<&str> = if on {
                vec![LABELS[3], LABELS[0], LABELS[1], LABELS[2]]
            } else {
                LABELS.to_vec()
            };
            list_doc(doc, &l)
        },
        |doc, (list, rows)| doc.insert_before(*list, rows[3], rows[0]),
    );
}

/// An atomic inline (`inline-block` chip, an IFC root of its own text) moved
/// from one paragraph into another whose inherited colour and size differ:
/// both paragraphs' layouts change, and so do the chip's own glyphs.
#[test]
fn a_chip_moved_between_paragraphs_takes_the_new_typography() {
    twin(
        "a_chip_moved_between_paragraphs_takes_the_new_typography",
        ".p2 { color: rgb(0, 120, 200); font-size: 20px; line-height: 26px; }",
        |doc, on| {
            let body = doc.body();
            let p1 = doc.create_element("p");
            doc.set_attribute(p1, "class", "box");
            let t1 = doc.create_text("first paragraph text that wraps in the box ");
            doc.append_child(p1, t1);
            let p2 = doc.create_element("p");
            doc.set_attribute(p2, "class", "box p2");
            let t2 = doc.create_text("second paragraph ");
            doc.append_child(p2, t2);
            let chip = doc.create_element("span");
            doc.set_attribute(chip, "class", "chip");
            let ct = doc.create_text("chip label");
            doc.append_child(chip, ct);
            doc.append_child(if on { p2 } else { p1 }, chip);
            doc.append_child(body, p1);
            doc.append_child(body, p2);
            (p2, chip)
        },
        |doc, (p2, chip)| doc.append_child(*p2, *chip),
    );
}

/// Width alone: a row moved from a 180px list into a 320px one, with no
/// typography between them to change. The cascade has nothing to drop, so the
/// re-break at the new width is the measure cache's and `build_ifc_layouts`'
/// width key, not an invalidation.
#[test]
fn a_row_moved_to_a_wider_parent_rebreaks_at_its_width() {
    twin(
        "a_row_moved_to_a_wider_parent_rebreaks_at_its_width",
        ".wide { width: 320px; }",
        |doc, on| {
            let body = doc.body();
            let mk_row = |doc: &mut RinchDocument, label: &str| {
                let row = doc.create_element("div");
                doc.set_attribute(row, "class", "row");
                let t = doc.create_text(label);
                doc.append_child(row, t);
                row
            };
            let a = doc.create_element("div");
            doc.set_attribute(a, "class", "list box");
            let b = doc.create_element("div");
            doc.set_attribute(b, "class", "list wide");
            let r0 = mk_row(doc, LABELS[0]);
            let r1 = mk_row(doc, LABELS[2]);
            doc.append_child(if on { b } else { a }, r0);
            doc.append_child(a, r1);
            doc.append_child(body, a);
            doc.append_child(body, b);
            (b, r0)
        },
        |doc, (b, r0)| doc.append_child(*b, *r0),
    );
}

// ── A pure edge-child sheet: the moved row's only restyle (review of #949) ──
//
// With only `:first-child` / `:last-child` in the sheet the list carries
// `HAS_EDGE_CHILD_SELECTOR` and nothing else, so the moved row — which keeps
// its style across a move within its parent — is restyled by one mark alone:
// the `after` child at its new index in `note_child_list_changed`. (The
// `:first-child` fixture above also has `+`, whose later-siblings flag masks
// that mark.)

fn edge_list(doc: &mut RinchDocument, labels: &[&str]) -> (NodeId, Vec<NodeId>) {
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
        doc.append_child(list, row);
        rows.push(row);
    }
    doc.append_child(body, list);
    (list, rows)
}

#[test]
fn a_row_moved_to_the_front_under_only_first_child() {
    twin(
        "a_row_moved_to_the_front_under_only_first_child",
        ".row:first-child { font-size: 22px; line-height: 28px; }",
        |doc, on| {
            let l: Vec<&str> = if on {
                vec![LABELS[3], LABELS[0], LABELS[1], LABELS[2]]
            } else {
                LABELS.to_vec()
            };
            edge_list(doc, &l)
        },
        |doc, (list, rows)| doc.insert_before(*list, rows[3], rows[0]),
    );
}

#[test]
fn a_row_moved_to_the_end_under_only_last_child() {
    twin(
        "a_row_moved_to_the_end_under_only_last_child",
        ".row:last-child { font-size: 22px; line-height: 28px; }",
        |doc, on| {
            let l: Vec<&str> = if on {
                vec![LABELS[1], LABELS[2], LABELS[3], LABELS[0]]
            } else {
                LABELS.to_vec()
            };
            edge_list(doc, &l)
        },
        |doc, (list, rows)| doc.append_child(*list, rows[0]),
    );
}

// ── A whole-document restyle keeps what its typography did not move (#913) ──
//
// A theme change replaces the theme sheet and re-cascades every element
// (`recompute_all_styles_full`). It used to drop every IFC's paint layout on
// the way in, whatever the new sheet changed; it now leaves that to the same
// per-node comparison a targeted restyle uses
// (`ComputedStyle::same_text_layout_inputs`). Each fixture below is a theme
// toggle whose variable **does** reach text, through a different route — the
// root's own style, an inline span, an anonymous box, an atomic inline, a
// generated box — so a full restyle that kept too much fails here. State A
// sets `:root { --t… }` to one value, state B to another; the change is
// exactly what `RinchApp` does on a theme change.

const THEME_A: &str = ":root { --tc: rgb(10, 20, 30); --tfs: 16px; --tls: 1px; \
     --tff: sans-serif; --tbg: rgb(250, 240, 10); --tw: 400; \
     --tcontent: \"one \"; }";
const THEME_B: &str = ":root { --tc: rgb(200, 100, 50); --tfs: 23px; --tls: 3px; \
     --tff: monospace; --tbg: rgb(10, 200, 240); --tw: 700; \
     --tcontent: \"a longer prefix \"; }";

fn toggle_theme(doc: &mut RinchDocument) {
    doc.update_theme_variables(THEME_B);
    doc.recompute_all_styles_full();
}

macro_rules! theme_typography {
    ($name:ident, $css:expr, $build:expr) => {
        #[test]
        fn $name() {
            twin(
                stringify!($name),
                $css,
                |doc, on| {
                    doc.set_theme_css(if on { THEME_B } else { THEME_A });
                    #[allow(clippy::redundant_closure_call)]
                    ($build)(doc, on)
                },
                |doc, _| toggle_theme(doc),
            );
        }
    };
}

fn ifc_doc_off(doc: &mut RinchDocument, _on: bool) -> NodeId {
    ifc_doc(doc, false)
}

theme_typography!(
    a_theme_colour_on_the_root,
    ".box { color: var(--tc); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_colour_inherited_from_the_body,
    "body { color: var(--tc); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_font_size_on_the_root,
    ".box { font-size: var(--tfs); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_letter_spacing_on_the_root,
    ".box { letter-spacing: var(--tls); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_font_family_on_the_root,
    ".box { font-family: var(--tff); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_font_weight_on_the_span_only,
    "span { font-weight: var(--tw); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_colour_on_the_span_only,
    "span { color: var(--tc); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_inline_background_on_the_span_only,
    "span { background-color: var(--tbg); padding-left: 9px; }",
    ifc_doc_off
);
theme_typography!(
    a_theme_string_in_generated_content,
    ".box::before { content: var(--tcontent); }",
    ifc_doc_off
);
theme_typography!(
    a_theme_colour_on_an_atomic_members_own_text,
    ".chip { color: var(--tc); }",
    |doc: &mut RinchDocument, _on: bool| chip_doc(doc, false)
);
theme_typography!(
    a_theme_font_size_on_an_atomic_members_own_text,
    ".chip { font-size: var(--tfs); }",
    |doc: &mut RinchDocument, _on: bool| chip_doc(doc, false)
);
theme_typography!(
    a_theme_colour_reaches_an_anonymous_block_box,
    ".a { color: var(--tc); line-height: 1.25; } .b { width: 2em; }",
    |doc: &mut RinchDocument, _on: bool| {
        let body = doc.body();
        let a = doc.create_element("div");
        doc.set_attribute(a, "class", "a");
        doc.append_child(body, a);
        let t = doc.create_text("some text beside a block");
        doc.append_child(a, t);
        let li = doc.create_element("li");
        doc.set_attribute(li, "class", "b");
        doc.append_child(a, li);
        a
    }
);
/// `body > div.a > ["first run of text ", li.b, " second run of text"]`: a
/// container holding **two** anonymous block boxes, so a whole-document pass
/// that re-mints them hands their slab ids out in another order (#964).
fn two_anon_boxes(doc: &mut RinchDocument, _on: bool) -> NodeId {
    let body = doc.body();
    let a = doc.create_element("div");
    doc.set_attribute(a, "class", "a");
    doc.append_child(body, a);
    let t = doc.create_text("first run of text beside a block ");
    doc.append_child(a, t);
    let li = doc.create_element("li");
    doc.set_attribute(li, "class", "b");
    doc.append_child(a, li);
    let t2 = doc.create_text(" second run, longer, of text after it");
    doc.append_child(a, t2);
    a
}

/// [`two_anon_boxes`] whose final state (`on`) has a third run and a block
/// after the second run, so a structural pass over the container re-mints both
/// boxes and must keep the first run's layout while laying the new run out.
fn two_anon_boxes_grown(doc: &mut RinchDocument, on: bool) -> NodeId {
    let a = two_anon_boxes(doc, false);
    if on {
        grow_anon_boxes(doc, &a);
    }
    a
}

fn grow_anon_boxes(doc: &mut RinchDocument, a: &NodeId) {
    let li = doc.create_element("li");
    doc.set_attribute(li, "class", "b");
    doc.append_child(*a, li);
    let t = doc.create_text(" a third run after the second block");
    doc.append_child(*a, t);
}

/// A scoped pass over a container whose two anonymous boxes keep their runs
/// (#964): incremental against fresh.
#[test]
fn anonymous_boxes_kept_through_a_scoped_pass_lay_out_as_fresh() {
    twin(
        "anonymous_boxes_kept_through_a_scoped_pass_lay_out_as_fresh",
        ".a { line-height: 1.25; } .b { width: 2em; }",
        two_anon_boxes_grown,
        grow_anon_boxes,
    );
}

/// The same structural pass in the same frame as the container's own font
/// change, which every box inherits: no box may keep a layout shaped at the
/// old size.
#[test]
fn anonymous_boxes_kept_through_a_pass_that_also_restyles_them_lay_out_as_fresh() {
    twin(
        "anonymous_boxes_kept_through_a_pass_that_also_restyles_them_lay_out_as_fresh",
        ".a { line-height: 1.25; } .a.on { font-size: 23px; letter-spacing: 2px; } .b { width: 2em; }",
        |doc, on| {
            let a = two_anon_boxes_grown(doc, on);
            if on {
                doc.set_attribute(a, "class", "a on");
            }
            a
        },
        |doc, a| {
            grow_anon_boxes(doc, a);
            doc.set_attribute(*a, "class", "a on");
        },
    );
}

/// A text edit in the first run, then a whole-document pass in the same
/// frame: the kept box must not keep the layout of its old text.
#[test]
fn an_edited_run_kept_through_a_whole_document_pass_lays_out_as_fresh() {
    twin(
        "an_edited_run_kept_through_a_whole_document_pass_lays_out_as_fresh",
        ".a { line-height: 1.25; } .b { width: 2em; }",
        |doc, on| {
            let a = two_anon_boxes(doc, false);
            if on {
                let t = doc.tree.get(a.0).unwrap().children[0];
                doc.set_text_content(NodeId(t), "a much longer first run than before, wrapping");
            }
            a
        },
        |doc, a| {
            let t = doc.tree.get(a.0).unwrap().children[0];
            doc.set_text_content(NodeId(t), "a much longer first run than before, wrapping");
            doc.recompute_all_styles_full();
        },
    );
}

// The theme reaching the anonymous boxes' **inherited** typography from their
// container: the whole-document pass keeps each box (#964), so the restyle's
// own invalidation of the container's boxes is what must re-shape them.
theme_typography!(
    a_theme_font_size_reaches_two_anonymous_block_boxes,
    ".a { font-size: var(--tfs); line-height: 1.25; } .b { width: 2em; }",
    two_anon_boxes
);
theme_typography!(
    a_theme_letter_spacing_reaches_two_anonymous_block_boxes,
    ".a { letter-spacing: var(--tls); line-height: 1.25; } .b { width: 2em; }",
    two_anon_boxes
);
theme_typography!(
    a_theme_font_family_reaches_two_anonymous_block_boxes,
    ".a { font-family: var(--tff); line-height: 1.25; } .b { width: 2em; }",
    two_anon_boxes
);
theme_typography!(
    a_theme_colour_reaches_two_anonymous_block_boxes,
    ".a { color: var(--tc); line-height: 1.25; } .b { width: 2em; }",
    two_anon_boxes
);
theme_typography!(
    a_theme_colour_on_a_split_inline_reaches_its_anonymous_box,
    ".b { color: var(--tc); line-height: 1.25; }",
    |doc: &mut RinchDocument, _on: bool| {
        let body = doc.body();
        let div = doc.create_element("div");
        doc.append_child(body, div);
        let span = doc.create_element("span");
        doc.set_attribute(span, "class", "b");
        doc.append_child(div, span);
        let t = doc.create_text("split text");
        doc.append_child(span, t);
        let inner = doc.create_element("fieldset");
        doc.append_child(span, inner);
        span
    }
);

/// The #913 shape itself: the theme changes a variable that **no text** reads
/// (a wrapper's background). The layout must equal a fresh one — there is no
/// counter-oracle, since nothing is meant to move — and no IFC may be
/// re-shaped: every paragraph keeps the layout it had.
#[test]
fn a_theme_change_that_reaches_no_text_reshapes_nothing() {
    let css =
        ".wrap { background-color: var(--tbg); } .box { color: var(--tc-unused, rgb(1, 2, 3)); }";
    let make = |theme: &str| {
        let mut doc = RinchDocument::new();
        doc.load_css(BASE_CSS);
        doc.load_css(css);
        doc.set_theme_css(theme);
        let _ = ifc_doc(&mut doc, false);
        // A second paragraph, and an atomic inline with an IFC of its own.
        let _ = chip_doc(&mut doc, false);
        // Two anonymous block boxes: the whole-document IFC setup pass a theme
        // change runs re-mints them, and re-shaped their text whatever the
        // restyle did until #964.
        let _ = two_anon_boxes(&mut doc, false);
        settle(&mut doc);
        doc
    };
    let mut doc = make(THEME_A);
    let _ = doc.tree.perf.end_frame();
    toggle_theme(&mut doc);
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    assert_eq!(
        s.get(rinch_dom::perf::Counter::FullRestyleTheme),
        1,
        "positive control: the full restyle ran: {s:?}"
    );
    assert!(
        s.get(rinch_dom::perf::Counter::ElementsCascaded) > 5,
        "positive control: every element was re-cascaded: {s:?}"
    );
    assert_eq!(
        s.get(rinch_dom::perf::Counter::ShapeIfcBuild),
        0,
        "a theme change that reaches no text rebuilt a paint layout: {s:?}"
    );
    assert_eq!(
        s.get(rinch_dom::perf::Counter::ShapeMeasureIfc),
        0,
        "a theme change that reaches no text re-measured an IFC: {s:?}"
    );
    let fresh = make(THEME_B);
    assert_eq!(snapshot(&doc), snapshot(&fresh));
}

/// The pixels of a theme colour change on an IFC root, incremental against
/// fresh: the brush is baked into the Parley layout, so a kept layout would
/// paint the old colour.
#[test]
fn a_theme_colour_change_reaches_the_pixels() {
    let css = ".box { color: var(--tc); }";
    let make = |theme: &str| {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(css);
        d.set_theme_css(theme);
        let _ = ifc_doc(&mut d, false);
        settle(&mut d);
        d
    };
    let mut before = make(THEME_A);
    let before_px = paint_pixels(&mut before);
    let mut doc = make(THEME_A);
    toggle_theme(&mut doc);
    doc.resolve_layout(VP.0, VP.1);
    let incremental = paint_pixels(&mut doc);
    let mut fresh = make(THEME_B);
    let fresh_px = paint_pixels(&mut fresh);
    let orange = |px: &Vec<[u8; 4]>| {
        px.iter()
            .filter(|p| p[0] > 120 && p[2] < 90 && p[0] > p[2] + 60)
            .count()
    };
    assert_eq!(
        orange(&before_px),
        0,
        "counter-oracle: state A has no orange ink"
    );
    assert!(
        orange(&fresh_px) > 20,
        "counter-oracle: state B has orange ink"
    );
    assert_eq!(orange(&incremental), orange(&fresh_px));
    assert!(
        incremental == fresh_px,
        "incremental pixels differ from fresh"
    );
}

// ── A text leaf: a flex item's text (#904) ───────────────────────────────────
//
// Text that is a direct child of a flex container is a Taffy leaf of its own
// (`NodeContext::Text`), not an IFC member, and is painted from the layout its
// measure built (`cached_text_parley`). In a block-level flex container that
// measure is the root compute's; inside an `inline-flex` it is the detached
// atomic-inline compute's (`measure_inline_blocks`), which kept nothing until
// #904, so paint shaped the label again on every frame. Every text input has
// to reach that cached layout on both routes.

const LEAF_CSS: &str = "
    .ichip { display: inline-flex; padding: 2px; }
    .frow { display: flex; width: 200px; }
    .ichip.hot, .frow.hot { color: rgb(200, 10, 10); }
    .ichip.big, .frow.big { font-size: 23px; line-height: 29px; }
    .ichip.spaced, .frow.spaced { letter-spacing: 3px; }
    .ichip.narrow { max-width: 60px; }
    .frow.narrow { width: 60px; }
";

/// `p > ["before ", span.ichip > "chip label text"]`, the `inline-flex` shape
/// `Button` / `Badge` render. Returns the chip and its text.
fn inline_flex_doc(doc: &mut RinchDocument, class: &str, label: &str) -> (NodeId, NodeId) {
    let body = doc.body();
    let p = doc.create_element("p");
    let t1 = doc.create_text("before ");
    doc.append_child(p, t1);
    let c = doc.create_element("span");
    doc.set_attribute(c, "class", class);
    let ct = doc.create_text(label);
    doc.append_child(c, ct);
    doc.append_child(p, c);
    doc.append_child(body, p);
    (c, ct)
}

/// `div.frow > "chip label text"`: the block-level control.
fn flex_row_doc(doc: &mut RinchDocument, class: &str, label: &str) -> (NodeId, NodeId) {
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "class", class);
    let t = doc.create_text(label);
    doc.append_child(d, t);
    doc.append_child(body, d);
    (d, t)
}

macro_rules! leaf_class_case {
    ($name:ident, $builder:ident, $base:expr, $on:expr) => {
        #[test]
        fn $name() {
            twin(
                stringify!($name),
                LEAF_CSS,
                |doc, on| $builder(doc, if on { $on } else { $base }, "chip label text").0,
                |doc, c| doc.set_attribute(*c, "class", $on),
            );
        }
    };
}

leaf_class_case!(
    an_inline_flex_label_takes_a_class_colour,
    inline_flex_doc,
    "ichip",
    "ichip hot"
);
leaf_class_case!(
    an_inline_flex_label_takes_a_class_font_size,
    inline_flex_doc,
    "ichip",
    "ichip big"
);
leaf_class_case!(
    an_inline_flex_label_takes_a_class_letter_spacing,
    inline_flex_doc,
    "ichip",
    "ichip spaced"
);
leaf_class_case!(
    an_inline_flex_label_rewraps_when_the_chip_narrows,
    inline_flex_doc,
    "ichip",
    "ichip narrow"
);
leaf_class_case!(
    a_flex_row_label_takes_a_class_colour,
    flex_row_doc,
    "frow",
    "frow hot"
);
leaf_class_case!(
    a_flex_row_label_takes_a_class_font_size,
    flex_row_doc,
    "frow",
    "frow big"
);
leaf_class_case!(
    a_flex_row_label_takes_a_class_letter_spacing,
    flex_row_doc,
    "frow",
    "frow spaced"
);
leaf_class_case!(
    a_flex_row_label_rewraps_when_the_row_narrows,
    flex_row_doc,
    "frow",
    "frow narrow"
);

/// The colour reaching an `inline-flex` label through inheritance from an
/// ancestor rather than a class on the chip.
#[test]
fn an_inline_flex_label_takes_an_ancestor_colour() {
    twin(
        "an_inline_flex_label_takes_an_ancestor_colour",
        ".on .ichip { color: rgb(0, 150, 60); }",
        |doc, on| {
            let (c, _) = inline_flex_doc(doc, "ichip", "chip label text");
            let p = doc.tree.get(c.0).unwrap().parent.unwrap();
            let p = NodeId(p);
            if on {
                doc.set_attribute(p, "class", "on");
            }
            p
        },
        |doc, p| doc.set_attribute(*p, "class", "on"),
    );
}

macro_rules! leaf_text_case {
    ($name:ident, $builder:ident, $class:expr) => {
        #[test]
        fn $name() {
            twin(
                stringify!($name),
                LEAF_CSS,
                |doc, on| {
                    $builder(
                        doc,
                        $class,
                        if on {
                            "a much longer chip label"
                        } else {
                            "chip label text"
                        },
                    )
                    .1
                },
                |doc, t| doc.set_text_content(*t, "a much longer chip label"),
            );
        }
    };
}

leaf_text_case!(
    an_inline_flex_label_takes_new_text,
    inline_flex_doc,
    "ichip"
);
leaf_text_case!(a_flex_row_label_takes_new_text, flex_row_doc, "frow");

/// The same `inline-flex` label painted from its cached layout and from
/// paint's on-demand fallback (the cache taken away): the pixels must match.
/// The cache is a cost saving, never a change in what is drawn. The positive
/// control — the leaf does hold a cached layout — is what fails before #904,
/// when paint only ever took the fallback.
#[test]
fn an_inline_flex_label_paints_the_same_from_its_cached_layout() {
    let mut doc = RinchDocument::new();
    doc.load_css(BASE_CSS);
    doc.load_css(LEAF_CSS);
    let (_, t) = inline_flex_doc(&mut doc, "ichip hot", "chip label text");
    settle(&mut doc);
    assert!(
        doc.tree.get(t.0).unwrap().cached_text_parley.is_some(),
        "the inline-flex label keeps the layout its measure built"
    );
    let cached = paint_pixels(&mut doc);
    let red = cached
        .iter()
        .filter(|p| p[3] > 0 && p[0] as i32 > p[1] as i32 + 80)
        .count();
    assert!(red > 0, "counter-oracle: the label draws red ink");
    doc.tree.nodes[t.0].cached_text_parley = None;
    let fallback = paint_pixels(&mut doc);
    assert!(
        cached == fallback,
        "cached-layout pixels differ from the fallback's"
    );
}

/// A finished `transition: color` on the element holding a text leaf, read off
/// the pixels against a document built red. A transition tick writes
/// `computed_style` without a cascade, so it reaches none of the cascade's
/// text invalidation, and a leaf's cached layout still holds the brush it was
/// measured with. Paint colours the leaf from the live style instead (#904).
/// Before #904 an `inline-flex` label had no cached layout — paint shaped it
/// every frame from the live style — so caching it must not freeze its colour
/// (the flex-row twin was frozen already, #679).
fn a_finished_colour_transition_reaches_the_label(kind: &str) {
    let css = ".x { transition: color 150ms linear; } .x.hot { color: rgb(220, 0, 0); }
        .flex { display: flex; } .iflex { display: inline-flex; }";
    let make = |hot: bool| {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(css);
        let body = d.body();
        let e = d.create_element("div");
        d.set_attribute(
            e,
            "class",
            &format!("x {kind}{}", if hot { " hot" } else { "" }),
        );
        let t = d.create_text("some label text");
        d.append_child(e, t);
        d.append_child(body, e);
        settle(&mut d);
        (d, e)
    };
    let red = |px: &[[u8; 4]]| {
        px.iter()
            .filter(|p| p[3] > 0 && p[0] as i32 > p[1] as i32 + 80)
            .count()
    };
    let (mut d, e) = make(false);
    assert_eq!(
        red(&paint_pixels(&mut d)),
        0,
        "counter-oracle: no red ink yet"
    );
    d.set_attribute(e, "class", &format!("x {kind} hot"));
    d.resolve_layout(VP.0, VP.1);
    let _ = paint_pixels(&mut d);
    // Back-date the transition past its duration so the next tick completes
    // it — `tick_transitions` reads the wall clock itself.
    for t in d
        .tree
        .active_transitions
        .get_mut(&e.0)
        .expect("the class change starts a colour transition")
        .values_mut()
    {
        t.start_time_ms -= 10_000.0;
    }
    d.tick_transitions();
    d.resolve_layout(VP.0, VP.1);
    let incremental = red(&paint_pixels(&mut d));
    let (mut f, _) = make(true);
    let fresh = red(&paint_pixels(&mut f));
    assert!(fresh > 0, "counter-oracle: the fresh label draws red ink");
    assert_eq!(
        incremental, fresh,
        "red ink after the transition, incremental vs fresh"
    );
}

#[test]
fn a_finished_colour_transition_reaches_an_inline_flex_label() {
    a_finished_colour_transition_reaches_the_label("iflex");
}

#[test]
fn a_finished_colour_transition_reaches_a_flex_row_label() {
    a_finished_colour_transition_reaches_the_label("flex");
}

/// The animation twin: a `forwards` colour animation run to its end on an
/// `inline-flex`, against a label built red. An animation tick writes
/// `computed_style` without a cascade too (#904).
#[test]
fn a_finished_colour_animation_reaches_an_inline_flex_label() {
    let css =
        "@keyframes recolour { from { color: rgb(10, 20, 30); } to { color: rgb(220, 0, 0); } }
        .iflex { display: inline-flex; } .iflex.anim { animation: recolour 150ms linear forwards; }
        .iflex.hot { color: rgb(220, 0, 0); }";
    let make = |class: &str| {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(css);
        let body = d.body();
        let e = d.create_element("div");
        d.set_attribute(e, "class", class);
        let t = d.create_text("some label text");
        d.append_child(e, t);
        d.append_child(body, e);
        settle(&mut d);
        (d, e)
    };
    let red = |px: &[[u8; 4]]| {
        px.iter()
            .filter(|p| p[3] > 0 && p[0] as i32 > p[1] as i32 + 80)
            .count()
    };
    let (mut d, e) = make("iflex");
    assert_eq!(
        red(&paint_pixels(&mut d)),
        0,
        "counter-oracle: no red ink yet"
    );
    d.set_attribute(e, "class", "iflex anim");
    d.resolve_layout(VP.0, VP.1);
    assert_eq!(
        red(&paint_pixels(&mut d)),
        0,
        "the animation starts from its first keyframe"
    );
    for a in d
        .tree
        .active_animations
        .get_mut(&e.0)
        .expect("the class change starts the animation")
    {
        a.start_time_ms -= 10_000.0;
    }
    d.tick_animations();
    d.resolve_layout(VP.0, VP.1);
    let incremental = red(&paint_pixels(&mut d));
    let (mut f, _) = make("iflex hot");
    let fresh = red(&paint_pixels(&mut f));
    assert!(fresh > 0, "counter-oracle: the fresh label draws red ink");
    assert_eq!(
        incremental, fresh,
        "red ink after the animation, incremental vs fresh"
    );
}

/// A class colour on a text leaf's container, read off the pixels against a
/// document built in that colour: paint draws the leaf in its parent's current
/// colour (#904), with no compute and no re-shape.
fn a_class_colour_reaches_the_leaf_pixels(
    builder: fn(&mut RinchDocument, &str, &str) -> (NodeId, NodeId),
    base: &str,
    hot: &str,
) {
    let make = |class: &str| {
        let mut d = RinchDocument::new();
        d.load_css(BASE_CSS);
        d.load_css(LEAF_CSS);
        let (c, _) = builder(&mut d, class, "chip label text");
        settle(&mut d);
        (d, c)
    };
    let red = |px: &[[u8; 4]]| {
        px.iter()
            .filter(|p| p[3] > 0 && p[0] as i32 > p[1] as i32 + 80)
            .count()
    };
    let (mut d, c) = make(base);
    assert_eq!(
        red(&paint_pixels(&mut d)),
        0,
        "counter-oracle: no red ink yet"
    );
    d.set_attribute(c, "class", hot);
    d.resolve_layout(VP.0, VP.1);
    let incremental = paint_pixels(&mut d);
    let (mut f, _) = make(hot);
    let fresh = paint_pixels(&mut f);
    assert!(
        red(&fresh) > 0,
        "counter-oracle: the fresh label draws red ink"
    );
    assert!(
        incremental == fresh,
        "incremental pixels differ from a fresh document"
    );
}

#[test]
fn a_class_colour_reaches_an_inline_flex_labels_pixels() {
    a_class_colour_reaches_the_leaf_pixels(inline_flex_doc, "ichip", "ichip hot");
}

#[test]
fn a_class_colour_reaches_a_flex_row_labels_pixels() {
    a_class_colour_reaches_the_leaf_pixels(flex_row_doc, "frow", "frow hot");
}

/// `text-align` on a flex row whose text leaf wraps: applied when the
/// compute's layouts are copied, not at paint, so unlike a colour it does need
/// a compute (#904).
#[test]
fn a_flex_row_label_takes_a_class_text_align() {
    twin(
        "a_flex_row_label_takes_a_class_text_align",
        ".frow.narrow { width: 60px; } .frow.right { text-align: right; }",
        |doc, on| {
            flex_row_doc(
                doc,
                if on {
                    "frow narrow right"
                } else {
                    "frow narrow"
                },
                "chip label text",
            )
            .0
        },
        |doc, c| doc.set_attribute(*c, "class", "frow narrow right"),
    );
}

// ── review-1627 adversarial fixtures (#964) ───────────────────────────────────

fn rv_anon_invariants(doc: &RinchDocument, name: &str) {
    let t = &doc.tree;
    let mut anon = Vec::new();
    for (id, n) in t.nodes.iter() {
        if n.is_anonymous_block_box {
            anon.push(id);
            let p = n.parent.unwrap_or_else(|| {
                panic!("{name}: anon box {id} has no parent (retired box left in slab)")
            });
            assert!(
                t.nodes[p].run_boxes.contains(&id),
                "{name}: anon box {id} not listed by its container {p}"
            );
            assert!(
                t.anonymous_block_boxes.contains(&id),
                "{name}: anon box {id} not tracked"
            );
            assert!(
                !n.run_members.is_empty(),
                "{name}: anon box {id} has no members"
            );
            for &m in &n.run_members {
                assert_eq!(
                    t.nodes.get(m).and_then(|mm| mm.run_box),
                    Some(id),
                    "{name}: member {m} of {id}"
                );
            }
        }
    }
    for &b in &t.anonymous_block_boxes {
        assert!(
            t.nodes.get(b).is_some_and(|n| n.is_anonymous_block_box),
            "{name}: tracked box {b} is not a live anon box"
        );
    }
    for k in t.ifc_measure_cache.keys() {
        assert!(
            t.nodes.get(*k).is_some(),
            "{name}: measure cache entry for freed id {k}"
        );
    }
}

#[track_caller]
fn rv_twin<H>(
    name: &str,
    css: &str,
    build: impl Fn(&mut RinchDocument, bool) -> H,
    change: impl FnOnce(&mut RinchDocument, &H),
) {
    // Re-run the oracle, then the invariants on the incremental document.
    let make = |state: bool| {
        let mut doc = RinchDocument::new();
        doc.load_css(BASE_CSS);
        doc.load_css(css);
        let h = build(&mut doc, state);
        settle(&mut doc);
        (doc, h)
    };
    let (mut doc, h) = make(false);
    rv_anon_invariants(&doc, name);
    let before = snapshot(&doc);
    change(&mut doc, &h);
    doc.resolve_layout(VP.0, VP.1);
    rv_anon_invariants(&doc, name);
    let inc = snapshot(&doc);
    let (fresh_doc, _) = make(true);
    let fresh = snapshot(&fresh_doc);
    assert_ne!(before, fresh, "{name}: A and B lay out identically");
    if inc != fresh {
        let mut diff = String::new();
        for (i, (a, b)) in inc.iter().zip(fresh.iter()).enumerate() {
            if a != b {
                diff.push_str(&format!("  #{i}\n    inc:   {a}\n    fresh: {b}\n"));
            }
        }
        panic!(
            "{name}: incremental != fresh (len {} vs {})\n{diff}",
            inc.len(),
            fresh.len()
        );
    }
    // A second layout must also move nothing.
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    rv_anon_invariants(&doc, name);
}

const RV_CSS: &str = ".a { width: 200px; line-height: 20px; } .b { width: 2em; } \
    .s.on { color: rgb(200, 0, 0); font-size: 22px; } .blk { display: block; } \
    .rel { position: relative; } .abs { position: absolute; width: 5px; height: 5px; } \
    .ell { overflow: hidden; white-space: nowrap; } .ell.on { text-overflow: ellipsis; }";

/// `div.a > [t0, span.s > "span text", t1, li.b, t2]`
fn rv_doc(doc: &mut RinchDocument, cls: &str) -> (NodeId, NodeId, NodeId) {
    let body = doc.body();
    let a = doc.create_element("div");
    doc.set_attribute(a, "class", cls);
    doc.append_child(body, a);
    let t0 = doc.create_text("first run of text ");
    doc.append_child(a, t0);
    let s = doc.create_element("span");
    doc.set_attribute(s, "class", "s");
    doc.append_child(a, s);
    let st = doc.create_text("span text that is long enough to wrap");
    doc.append_child(s, st);
    let t1 = doc.create_text(" tail of run");
    doc.append_child(a, t1);
    let li = doc.create_element("li");
    doc.set_attribute(li, "class", "b");
    doc.append_child(a, li);
    let t2 = doc.create_text(" second run of text after the block");
    doc.append_child(a, t2);
    (a, s, li)
}

fn rv_grow(doc: &mut RinchDocument, a: NodeId) {
    let li = doc.create_element("li");
    doc.set_attribute(li, "class", "b");
    doc.append_child(a, li);
}

#[test]
fn rv_member_restyled_in_a_scoped_pass() {
    rv_twin(
        "rv_member_restyled_in_a_scoped_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                doc.set_attribute(h.1, "class", "s on");
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            doc.set_attribute(h.1, "class", "s on");
            rv_grow(doc, h.0);
        },
    );
}

#[test]
fn rv_member_restyled_in_a_whole_document_pass() {
    rv_twin(
        "rv_member_restyled_in_a_whole_document_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                doc.set_attribute(h.1, "class", "s on");
            }
            h
        },
        |doc, h| {
            doc.set_attribute(h.1, "class", "s on");
            doc.recompute_all_styles_full();
        },
    );
}

#[test]
fn rv_member_text_edited_in_a_scoped_pass() {
    rv_twin(
        "rv_member_text_edited_in_a_scoped_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                let st = doc.tree.get(h.1.0).unwrap().children[0];
                doc.set_text_content(NodeId(st), "x");
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            let st = doc.tree.get(h.1.0).unwrap().children[0];
            doc.set_text_content(NodeId(st), "x");
            rv_grow(doc, h.0);
        },
    );
}

#[test]
fn rv_member_moved_out_and_back() {
    rv_twin(
        "rv_member_moved_out_and_back",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            let other = doc.create_element("div");
            let body = doc.body();
            doc.append_child(body, other);
            doc.append_child(other, h.1);
            let t1 = doc.tree.get(h.0.0).unwrap().children[1];
            doc.insert_before(h.0, h.1, NodeId(t1));
            doc.remove_node(other);
            rv_grow(doc, h.0);
        },
    );
}

#[test]
fn rv_member_moved_out_and_back_across_a_layout() {
    // Out, lay out (the box re-mints for [t0, t1]), back in, lay out.
    let name = "rv_member_moved_out_and_back_across_a_layout";
    rv_twin(
        name,
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            let other = doc.create_element("div");
            let body = doc.body();
            doc.append_child(body, other);
            doc.append_child(other, h.1);
            doc.resolve_layout(VP.0, VP.1);
            rv_anon_invariants(doc, name);
            let t1 = doc.tree.get(h.0.0).unwrap().children[1];
            doc.insert_before(h.0, h.1, NodeId(t1));
            doc.remove_node(other);
            rv_grow(doc, h.0);
        },
    );
}

#[test]
fn rv_member_becomes_block() {
    rv_twin(
        "rv_member_becomes_block",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                doc.set_attribute(h.1, "class", "s blk");
            }
            h
        },
        |doc, h| doc.set_attribute(h.1, "class", "s blk"),
    );
}

#[test]
fn rv_member_becomes_block_and_back_across_layouts() {
    rv_twin(
        "rv_member_becomes_block_and_back_across_layouts",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            doc.set_attribute(h.1, "class", "s blk");
            doc.resolve_layout(VP.0, VP.1);
            doc.set_attribute(h.1, "class", "s");
            rv_grow(doc, h.0);
        },
    );
}

#[test]
fn rv_container_loses_its_block() {
    rv_twin(
        "rv_container_loses_its_block",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                doc.remove_node(h.2);
            }
            h
        },
        |doc, h| doc.remove_node(h.2),
    );
}

#[test]
fn rv_container_loses_its_block_then_gets_one_back() {
    rv_twin(
        "rv_container_loses_its_block_then_gets_one_back",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            if on {
                doc.remove_node(h.2);
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            doc.remove_node(h.2);
            doc.resolve_layout(VP.0, VP.1);
            rv_grow(doc, h.0);
        },
    );
}

#[test]
fn rv_split_inline_in_a_run_through_a_whole_document_pass() {
    // span.s holds a block: a split inline (#513) inside a container with anon boxes.
    rv_twin(
        "rv_split_inline_in_a_run_through_a_whole_document_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            let d = doc.create_element("div");
            doc.append_child(h.1, d);
            let dt = doc.create_text("block in span");
            doc.append_child(d, dt);
            let after = doc.create_text(" after the block inside the span");
            doc.append_child(h.1, after);
            if on {
                doc.set_attribute(h.1, "class", "s on");
            }
            h
        },
        |doc, h| {
            doc.set_attribute(h.1, "class", "s on");
            doc.recompute_all_styles_full();
        },
    );
}

#[test]
fn rv_split_inline_in_a_run_through_a_scoped_pass() {
    rv_twin(
        "rv_split_inline_in_a_run_through_a_scoped_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_doc(doc, "a");
            let d = doc.create_element("div");
            doc.append_child(h.1, d);
            let dt = doc.create_text("block in span");
            doc.append_child(d, dt);
            let after = doc.create_text(" after the block inside the span");
            doc.append_child(h.1, after);
            if on {
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            rv_grow(doc, h.0);
        },
    );
}

fn rv_abs_doc(doc: &mut RinchDocument, hoisted: bool) -> (NodeId, NodeId, NodeId) {
    let h = rv_doc(doc, "a rel");
    let abs = doc.create_element("i");
    doc.set_attribute(abs, "class", "abs");
    if hoisted {
        doc.append_child(h.1, abs);
        let more = doc.create_text(" more span text after the abs");
        doc.append_child(h.1, more);
    } else {
        let t1 = doc.tree.get(h.0.0).unwrap().children[2];
        doc.insert_before(h.0, abs, NodeId(t1));
    }
    (h.0, h.1, abs)
}

#[test]
fn rv_hoisted_abs_through_a_scoped_pass() {
    rv_twin(
        "rv_hoisted_abs_through_a_scoped_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_abs_doc(doc, true);
            if on {
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| rv_grow(doc, h.0),
    );
}

#[test]
fn rv_hoisted_abs_through_a_whole_document_pass_with_a_restyle() {
    rv_twin(
        "rv_hoisted_abs_through_a_whole_document_pass_with_a_restyle",
        RV_CSS,
        |doc, on| {
            let h = rv_abs_doc(doc, true);
            if on {
                doc.set_attribute(h.1, "class", "s on");
            }
            h
        },
        |doc, h| {
            doc.set_attribute(h.1, "class", "s on");
            doc.recompute_all_styles_full();
        },
    );
}

#[test]
fn rv_run_abs_through_a_scoped_pass() {
    rv_twin(
        "rv_run_abs_through_a_scoped_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_abs_doc(doc, false);
            if on {
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| rv_grow(doc, h.0),
    );
}

#[test]
fn rv_run_abs_turns_block_level_before_blockify() {
    rv_twin(
        "rv_run_abs_turns_block_level_before_blockify",
        RV_CSS,
        |doc, on| {
            let h = rv_abs_doc(doc, false);
            if on {
                doc.set_attribute(h.2, "class", "abs blk");
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            doc.set_attribute(h.2, "class", "abs blk");
            rv_grow(doc, h.0);
        },
    );
}

#[test]
fn rv_run_abs_removed_through_a_whole_document_pass() {
    rv_twin(
        "rv_run_abs_removed_through_a_whole_document_pass",
        RV_CSS,
        |doc, on| {
            let h = rv_abs_doc(doc, false);
            if on {
                doc.remove_node(h.2);
            }
            h
        },
        |doc, h| {
            doc.remove_node(h.2);
            doc.recompute_all_styles_full();
        },
    );
}

/// Runs of spans only, so only the container's `run_boxes` reach the boxes
/// on a non-inherited change (text-overflow), with no structural change.
fn rv_span_runs(doc: &mut RinchDocument, cls: &str) -> NodeId {
    let body = doc.body();
    let a = doc.create_element("div");
    doc.set_attribute(a, "class", cls);
    doc.append_child(body, a);
    let s = doc.create_element("span");
    doc.append_child(a, s);
    let st = doc.create_text("a long span run that will not fit in the width");
    doc.append_child(s, st);
    let li = doc.create_element("li");
    doc.set_attribute(li, "class", "b");
    doc.append_child(a, li);
    let s2 = doc.create_element("span");
    doc.append_child(a, s2);
    let st2 = doc.create_text("another long span run that will not fit either");
    doc.append_child(s2, st2);
    a
}

#[test]
fn rv_container_ellipsis_toggled_spans_only() {
    rv_twin(
        "rv_container_ellipsis_toggled_spans_only",
        RV_CSS,
        |doc, on| rv_span_runs(doc, if on { "a ell on" } else { "a ell" }),
        |doc, a| doc.set_attribute(*a, "class", "a ell on"),
    );
}

#[test]
fn rv_container_ellipsis_toggled_spans_only_with_a_scoped_pass() {
    rv_twin(
        "rv_container_ellipsis_toggled_spans_only_with_a_scoped_pass",
        RV_CSS,
        |doc, on| {
            let a = rv_span_runs(doc, if on { "a ell on" } else { "a ell" });
            if on {
                rv_grow(doc, a);
            }
            a
        },
        |doc, a| {
            doc.set_attribute(*a, "class", "a ell on");
            rv_grow(doc, *a);
        },
    );
}

#[test]
fn rv_container_moved_under_a_bigger_font() {
    rv_twin(
        "rv_container_moved_under_a_bigger_font",
        ".big { font-size: 25px; } .a { width: 200px; } .b { width: 2em; }",
        |doc, on| {
            let body = doc.body();
            let big = doc.create_element("div");
            doc.set_attribute(big, "class", "big");
            let h = rv_doc(doc, "a");
            doc.append_child(body, big);
            if on {
                doc.append_child(big, h.0);
            }
            (h.0, big)
        },
        |doc, h| doc.append_child(h.1, h.0),
    );
}

#[test]
fn rv_container_width_and_structure_in_one_frame() {
    rv_twin(
        "rv_container_width_and_structure_in_one_frame",
        ".a { width: 200px; } .a.w { width: 120px; } .b { width: 2em; }",
        |doc, on| {
            let h = rv_doc(doc, if on { "a w" } else { "a" });
            if on {
                rv_grow(doc, h.0);
            }
            h
        },
        |doc, h| {
            doc.set_attribute(h.0, "class", "a w");
            rv_grow(doc, h.0);
        },
    );
}

// seed-1733 shape: an anonymous box whose only member is an inline-block that
// shrinks in the frame of a scoped pass over the box's container.
const RV_CHIP_CSS: &str = ".a { width: 300px; line-height: 20px; } .b { width: 2em; } \
    .chip { display: inline-block; padding: 1px; } .gone { display: none; }";

fn rv_chip_doc(doc: &mut RinchDocument, inner_block: bool) -> (NodeId, NodeId, NodeId) {
    let body = doc.body();
    let a = doc.create_element("div");
    doc.set_attribute(a, "class", "a");
    doc.append_child(body, a);
    let li = doc.create_element("li");
    doc.set_attribute(li, "class", "b");
    doc.append_child(a, li);
    let chip = doc.create_element("span");
    doc.set_attribute(chip, "class", "chip");
    doc.append_child(a, chip);
    let ct = doc.create_text("plain text");
    doc.append_child(chip, ct);
    let x = doc.create_element(if inner_block { "div" } else { "span" });
    doc.append_child(chip, x);
    let xt = doc.create_text(" x y z and more words to wrap the line twice over");
    doc.append_child(x, xt);
    (a, chip, x)
}

fn rv_chip_twin(name: &str, inner_block: bool, whole: bool, how: u8) {
    let apply = move |doc: &mut RinchDocument, h: &(NodeId, NodeId, NodeId)| {
        match how {
            0 => doc.set_attribute(h.2, "class", "gone"),
            _ => {
                let t = doc.tree.get(h.2.0).unwrap().children[0];
                doc.set_text_content(NodeId(t), " x");
            }
        }
        if whole {
            doc.recompute_all_styles_full();
        } else {
            let p = doc.create_element("p");
            doc.set_attribute(p, "class", "chip");
            doc.append_child(h.0, p);
            let pt = doc.create_text("new");
            doc.append_child(p, pt);
        }
    };
    rv_twin(
        name,
        RV_CHIP_CSS,
        move |doc, on| {
            let h = rv_chip_doc(doc, inner_block);
            if on {
                apply(doc, &h);
            }
            h
        },
        move |doc, h| apply(doc, h),
    );
}

#[test]
fn rv_chip_block_hidden_scoped() {
    rv_chip_twin("rv_chip_block_hidden_scoped", true, false, 0);
}
#[test]
fn rv_chip_block_hidden_whole() {
    rv_chip_twin("rv_chip_block_hidden_whole", true, true, 0);
}
#[test]
fn rv_chip_block_text_scoped() {
    rv_chip_twin("rv_chip_block_text_scoped", true, false, 1);
}
#[test]
fn rv_chip_block_text_whole() {
    rv_chip_twin("rv_chip_block_text_whole", true, true, 1);
}
#[test]
fn rv_chip_inline_hidden_scoped() {
    rv_chip_twin("rv_chip_inline_hidden_scoped", false, false, 0);
}
#[test]
fn rv_chip_inline_text_scoped() {
    rv_chip_twin("rv_chip_inline_text_scoped", false, false, 1);
}
#[test]
fn rv_chip_inline_text_whole() {
    rv_chip_twin("rv_chip_inline_text_whole", false, true, 1);
}

fn rv_s(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}
fn rv_t(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// Seed 1733 of `scoped_pass_matches_the_whole_document_pass_long`, by hand.
fn rv_seed1733(
    doc: &mut RinchDocument,
    hidden: bool,
    appended: bool,
    contents: bool,
) -> (NodeId, NodeId) {
    let body = doc.body();
    let d = rv_s(doc, body, "div", "width: 360px; line-height: 20px");
    let sec = rv_s(doc, d, "section", "display: flex");
    rv_t(doc, sec, "a");
    let w = if contents {
        rv_s(doc, d, "ul", "display: contents")
    } else {
        d
    };
    rv_s(doc, w, "div", "display: flex; flex-direction: column");
    let chip = rv_s(doc, w, "ul", "display: inline-block; padding: 1px");
    rv_t(doc, chip, "plain text");
    let inner = rv_s(doc, chip, "ul", "display: block");
    rv_t(doc, inner, " ");
    let x = rv_s(doc, inner, "div", if hidden { "display: none" } else { "" });
    rv_t(doc, x, "x y z");
    let after = rv_s(doc, w, "ul", "");
    rv_t(
        doc,
        after,
        "a rather longer run of text that wraps somewhere",
    );
    rv_s(doc, d, "span", "display: flex");
    if appended {
        let p = rv_s(doc, d, "p", "display: inline-flex");
        rv_t(doc, p, "x y z");
    }
    (d, x)
}

fn rv_seed1733_twin(name: &str, contents: bool, whole: bool) {
    rv_twin(
        name,
        "",
        move |doc, on| rv_seed1733(doc, on, on && !whole, contents),
        move |doc, h| {
            doc.set_attribute(h.1, "style", "display: none");
            if whole {
                doc.recompute_all_styles_full();
            } else {
                let p = rv_s(doc, h.0, "p", "display: inline-flex");
                rv_t(doc, p, "x y z");
            }
        },
    );
}

#[test]
fn rv_seed1733_scoped_contents() {
    rv_seed1733_twin("rv_seed1733_scoped_contents", true, false);
}
#[test]
fn rv_seed1733_scoped_plain() {
    rv_seed1733_twin("rv_seed1733_scoped_plain", false, false);
}
#[test]
fn rv_seed1733_whole_contents() {
    rv_seed1733_twin("rv_seed1733_whole_contents", true, true);
}

/// As [`rv_seed1733`], the chip's block inserted one frame before step 9.
fn rv_seed1733_hist(contents: bool, whole: bool, name: &str) {
    rv_twin(
        name,
        "",
        move |doc, on| {
            let (d, x) = rv_seed1733(doc, on, on && !whole, contents);
            if !on {
                // State A: the chip without its block.
                let inner = doc.tree.get(x.0).unwrap().parent.unwrap();
                doc.remove_node(NodeId(inner));
            }
            (d, x)
        },
        move |doc, h| {
            // Re-insert the block (step 8), lay out, then step 9.
            let x = h.1;
            let chip = {
                let w = if contents {
                    NodeId(doc.tree.get(h.0.0).unwrap().children[1])
                } else {
                    h.0
                };
                let kids: Vec<u64> = doc.tree.get(w.0).unwrap().children.clone();
                NodeId(
                    kids.into_iter()
                        .find(|&k| {
                            doc.tree.get(k).unwrap().children.len() == 1
                                && doc.tree.get(k).unwrap().tag() == Some("ul")
                        })
                        .unwrap(),
                )
            };
            let inner = doc.tree.get(x.0).unwrap().parent.unwrap();
            doc.append_child(chip, NodeId(inner));
            doc.resolve_layout(VP.0, VP.1);
            doc.set_attribute(x, "style", "display: none");
            if whole {
                doc.recompute_all_styles_full();
            } else {
                let p = rv_s(doc, h.0, "p", "display: inline-flex");
                rv_t(doc, p, "x y z");
            }
        },
    );
}

#[test]
fn rv_seed1733_hist_scoped_contents() {
    rv_seed1733_hist(true, false, "rv_seed1733_hist_scoped_contents");
}
#[test]
fn rv_seed1733_hist_scoped_plain() {
    rv_seed1733_hist(false, false, "rv_seed1733_hist_scoped_plain");
}
#[test]
fn rv_seed1733_hist_whole_contents() {
    rv_seed1733_hist(true, true, "rv_seed1733_hist_whole_contents");
}

/// The seed-1733 chip history in a plain IFC root (no anonymous box): is the
/// stale measure the anonymous box's alone?
fn rv_chip_hist_plain(name: &str, structural: bool, sibling_text: bool) {
    let build = move |doc: &mut RinchDocument, hidden: bool, appended: bool| {
        let body = doc.body();
        let d = rv_s(doc, body, "div", "width: 360px; line-height: 20px");
        if sibling_text {
            rv_t(doc, d, "before ");
        }
        let chip = rv_s(doc, d, "ul", "display: inline-block; padding: 1px");
        rv_t(doc, chip, "plain text");
        let inner = rv_s(doc, chip, "ul", "display: block");
        rv_t(doc, inner, " ");
        let x = rv_s(doc, inner, "div", if hidden { "display: none" } else { "" });
        rv_t(doc, x, "x y z");
        if appended {
            let p = rv_s(doc, d, "p", "display: inline-flex");
            rv_t(doc, p, "x y z");
        }
        (d, chip, x)
    };
    rv_twin(
        name,
        "",
        move |doc, on| {
            let h = build(doc, on, on && structural);
            if !on {
                let inner = doc.tree.get(h.2.0).unwrap().parent.unwrap();
                doc.remove_node(NodeId(inner));
            }
            h
        },
        move |doc, h| {
            let inner = doc.tree.get(h.2.0).unwrap().parent.unwrap();
            doc.append_child(h.1, NodeId(inner));
            doc.resolve_layout(VP.0, VP.1);
            doc.set_attribute(h.2, "style", "display: none");
            if structural {
                let p = rv_s(doc, h.0, "p", "display: inline-flex");
                rv_t(doc, p, "x y z");
            } else {
                doc.recompute_all_styles_full();
            }
        },
    );
}
#[test]
fn rv_chip_hist_plain_whole() {
    rv_chip_hist_plain("rv_chip_hist_plain_whole", false, true);
}
#[test]
fn rv_chip_hist_plain_structural() {
    rv_chip_hist_plain("rv_chip_hist_plain_structural", true, true);
}

// ── review2-1627 fixtures ─────────────────────────────────────────────────────

/// A member moved from the first run to the second in one frame: both runs'
/// first members still key a retired box, but the members differ.
#[test]
fn rv2_member_moved_between_runs_in_one_frame() {
    rv_twin(
        "rv2_member_moved_between_runs_in_one_frame",
        RV_CSS,
        |doc, on| {
            let (a, s, li) = rv_doc(doc, "a");
            let _ = li;
            if on {
                doc.append_child(a, s);
            }
            (a, s)
        },
        |doc, h| doc.append_child(h.0, h.1),
    );
}

/// The same member moved there and back in one frame, with a restyle so the
/// two states differ: the boxes are kept and must not keep the old layout.
#[test]
fn rv2_member_moved_between_runs_and_back_with_a_restyle() {
    rv_twin(
        "rv2_member_moved_between_runs_and_back_with_a_restyle",
        RV_CSS,
        |doc, on| {
            let (a, s, li) = rv_doc(doc, "a");
            if on {
                doc.set_attribute(s, "class", "s on");
            }
            (a, s, li)
        },
        |doc, h| {
            doc.append_child(h.0, h.1);
            let t1 = doc.tree.get(h.0.0).unwrap().children[1];
            doc.insert_before(h.0, h.1, NodeId(t1));
            doc.set_attribute(h.1, "class", "s on");
        },
    );
}

/// The second run's text moved to the front of the first run in one frame.
#[test]
fn rv2_second_runs_first_member_moved_to_the_front() {
    rv_twin(
        "rv2_second_runs_first_member_moved_to_the_front",
        RV_CSS,
        |doc, on| {
            let (a, _, _) = rv_doc(doc, "a");
            if on {
                let kids = doc.tree.get(a.0).unwrap().children.clone();
                doc.insert_before(a, NodeId(kids[4]), NodeId(kids[0]));
            }
            a
        },
        |doc, a| {
            let kids = doc.tree.get(a.0).unwrap().children.clone();
            doc.insert_before(*a, NodeId(kids[4]), NodeId(kids[0]));
        },
    );
}

/// `div.a(width: W) > ["text ", span(inline-block, width: 50%) > "chip", " more", li, " after"]`
fn rv2_pct_doc(doc: &mut RinchDocument, w: &str, chip_style: &str) -> (NodeId, NodeId) {
    let body = doc.body();
    let a = rv_s(doc, body, "div", &format!("width: {w}; line-height: 20px"));
    rv_t(doc, a, "text before the chip ");
    let c = rv_s(doc, a, "span", chip_style);
    rv_t(doc, c, "chip text that can wrap inside");
    rv_t(doc, a, " more text after the chip in the same run");
    rv_s(doc, a, "li", "width: 2em");
    rv_t(doc, a, " after");
    (a, c)
}

fn rv2_resize_and_back(name: &str, chip_style: &'static str, whole: bool, by_chip: bool) {
    rv_twin(
        name,
        "",
        move |doc, on| {
            let h = rv2_pct_doc(doc, "200px", chip_style);
            if on && !whole {
                rv_s(doc, h.0, "li", "width: 2em");
            }
            if on && whole {
                doc.set_style(h.0, "color", "rgb(200, 0, 0)");
            }
            h
        },
        move |doc, h| {
            if by_chip {
                doc.set_style(h.1, "width", "150px");
            } else {
                doc.set_style(h.0, "width", "320px");
            }
            doc.resolve_layout(VP.0, VP.1);
            if by_chip {
                doc.set_style(h.1, "width", "60px");
            } else {
                doc.set_style(h.0, "width", "200px");
            }
            if whole {
                doc.set_style(h.0, "color", "rgb(200, 0, 0)");
                doc.recompute_all_styles_full();
            } else {
                rv_s(doc, h.0, "li", "width: 2em");
            }
        },
    );
}

#[test]
fn rv2_pct_chip_container_resized_and_back_scoped() {
    rv2_resize_and_back(
        "rv2_pct_chip_container_resized_and_back_scoped",
        "display: inline-block; width: 50%",
        false,
        false,
    );
}
#[test]
fn rv2_pct_chip_container_resized_and_back_whole() {
    rv2_resize_and_back(
        "rv2_pct_chip_container_resized_and_back_whole",
        "display: inline-block; width: 50%",
        true,
        false,
    );
}
#[test]
fn rv2_auto_chip_container_resized_and_back_scoped() {
    rv2_resize_and_back(
        "rv2_auto_chip_container_resized_and_back_scoped",
        "display: inline-block",
        false,
        false,
    );
}
#[test]
fn rv2_chip_width_styled_and_back_scoped() {
    rv2_resize_and_back(
        "rv2_chip_width_styled_and_back_scoped",
        "display: inline-block; width: 60px",
        false,
        true,
    );
}
#[test]
fn rv2_chip_width_styled_and_back_whole() {
    rv2_resize_and_back(
        "rv2_chip_width_styled_and_back_whole",
        "display: inline-block; width: 60px",
        true,
        true,
    );
}

/// The anonymous-box container is a flex item (content-sized), so its chip
/// is capped by its min-content width (#1476): a chip text edit that grows
/// the chip, laid out, then the text put back and a block appended.
#[test]
fn rv2_chip_in_flex_item_text_edited_and_back_scoped() {
    rv_twin(
        "rv2_chip_in_flex_item_text_edited_and_back_scoped",
        "",
        |doc, on| {
            let body = doc.body();
            let row = rv_s(
                doc,
                body,
                "div",
                "display: flex; width: 300px; line-height: 20px",
            );
            let a = rv_s(doc, row, "div", "");
            rv_t(doc, a, "lead ");
            let c = rv_s(doc, a, "span", "display: inline-block");
            let ct = doc.create_text("short");
            doc.append_child(c, ct);
            rv_t(doc, a, " tail of the run");
            rv_s(doc, a, "li", "width: 2em");
            rv_t(doc, a, " after the block");
            let lab = rv_s(doc, row, "div", "");
            rv_t(doc, lab, "a label beside it that is wide enough to squeeze");
            if on {
                rv_s(doc, a, "li", "width: 2em");
            }
            (a, ct)
        },
        |doc, h| {
            doc.set_text_content(h.1, "a much longer chip text that wraps");
            doc.resolve_layout(VP.0, VP.1);
            doc.set_text_content(h.1, "short");
            rv_s(doc, h.0, "li", "width: 2em");
        },
    );
}
