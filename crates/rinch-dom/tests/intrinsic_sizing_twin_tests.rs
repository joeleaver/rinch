//! #691 — the intrinsic sizing keywords on `width`, `height` and `flex-basis`,
//! twinned against **Chrome 153** (headless, standards mode, `file://`, an
//! 800x513 viewport — headless Chrome's inner size at `--window-size=800,600`,
//! which only the `fixed` height rows read).
//!
//! Every row is `<cb {cb}><t {t}; {prop}: {keyword}>{content}</t></cb>`, with
//! `*{box-sizing: border-box}` in Chrome (so `t` is border-box here too), and
//! the number is `t`'s border-box size on that axis. Two contents:
//!
//! - `block`: one block child declared `300x20`. No line box, so nothing is
//!   font-derived; but `min-content` and `max-content` coincide.
//! - `two`: two `inline-block`s declared `100x20` inside a `font-size: 0;
//!   line-height: 0` wrapper, so the soft wrap opportunity between them has no
//!   width: `min-content` 100, `max-content` 200. This is what tells the two
//!   keywords apart and what makes `fit-content` clamp (150 in a 150px block).
//!
//! The containing block is **800 or 150 wide**, never the content's own width,
//! so a keyword answering `auto` (fill) and one answering the content cannot
//! agree by accident — the fixed-point trap the #626 table was built to avoid.
//!
//! `fit-content(<length-percentage>)` is not here: **Chrome 153 does not parse
//! it on `width`** (the declaration is dropped, measured), so it computes to
//! `auto` there and rinch's `Auto` for it is the browser's answer.
//!
//! The probe that produced the table is reproduced in the PR for #691.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
/// Chrome's headless inner height at `--window-size=800,600` (measured: a
/// `position: fixed; top: 10px; bottom: 20px` box is 483 tall).
const VH: f32 = 513.0;

const KEYWORDS: [&str; 5] = [
    "auto",
    "max-content",
    "min-content",
    "fit-content",
    "stretch",
];

fn content(doc: &mut RinchDocument, parent: NodeId, kind: &str) {
    match kind {
        "block" => {
            let c = doc.create_element("div");
            doc.set_attribute(c, "style", "width: 300px; height: 20px");
            doc.append_child(parent, c);
        }
        "two" => {
            let w = doc.create_element("div");
            doc.set_attribute(w, "style", "font-size: 0; line-height: 0");
            doc.append_child(parent, w);
            for _ in 0..2 {
                let ib = doc.create_element("span");
                doc.set_attribute(
                    ib,
                    "style",
                    "display: inline-block; width: 100px; height: 20px; vertical-align: top",
                );
                doc.append_child(w, ib);
            }
        }
        _ => unreachable!(),
    }
}

/// The size of `t` on `prop`'s axis.
fn measure(kind: &str, cb_style: &str, t_style: &str, prop: &str, keyword: &str) -> f32 {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let cb = doc.create_element("div");
    doc.set_attribute(cb, "style", cb_style);
    doc.append_child(body, cb);
    let t = doc.create_element("div");
    doc.set_attribute(
        t,
        "style",
        &format!("box-sizing: border-box; {t_style}; {prop}: {keyword}"),
    );
    doc.append_child(cb, t);
    content(&mut doc, t, kind);
    doc.resolve_layout(VW, VH);
    let l = &doc.tree.get(t.0).unwrap().layout;
    if prop == "height" { l.height } else { l.width }
}

/// (label, containing-block style, `t` style, property). Same order and
/// spelling as the Chrome probe.
const CONTEXTS: &[(&str, &str, &str, &str)] = &[
    ("block cb800", "width: 800px", "", "width"),
    ("block cb150", "width: 150px", "", "width"),
    (
        "inline-block cb800",
        "width: 800px",
        "display: inline-block",
        "width",
    ),
    (
        "inline-block cb150",
        "width: 150px",
        "display: inline-block",
        "width",
    ),
    (
        "inline-flex cb800",
        "width: 800px",
        "display: inline-flex",
        "width",
    ),
    ("flex-row cb800", "display: flex; width: 800px", "", "width"),
    ("flex-row cb150", "display: flex; width: 150px", "", "width"),
    (
        "flex-row basis cb800",
        "display: flex; width: 800px",
        "",
        "flex-basis",
    ),
    (
        "flex-col cb800",
        "display: flex; flex-direction: column; width: 800px; height: 300px",
        "",
        "width",
    ),
    (
        "flex-col cb150",
        "display: flex; flex-direction: column; width: 150px; height: 300px",
        "",
        "width",
    ),
    (
        "grid auto cb800",
        "display: grid; grid-template-columns: auto; width: 800px",
        "",
        "width",
    ),
    (
        "grid 1fr cb150",
        "display: grid; grid-template-columns: 1fr; width: 150px",
        "",
        "width",
    ),
    (
        "abs cb800",
        "position: relative; width: 800px; height: 300px",
        "position: absolute; left: 0; top: 0",
        "width",
    ),
    (
        "abs lr cb800",
        "position: relative; width: 800px; height: 300px",
        "position: absolute; left: 10px; right: 10px; top: 0",
        "width",
    ),
    (
        "abs cb150",
        "position: relative; width: 150px; height: 300px",
        "position: absolute; left: 0; top: 0",
        "width",
    ),
    (
        "fixed lr",
        "width: 800px",
        "position: fixed; left: 10px; right: 10px; top: 0",
        "width",
    ),
    (
        "fixed l",
        "width: 300px",
        "position: fixed; left: 10px; top: 0",
        "width",
    ),
    (
        "abs icb l",
        "width: 300px",
        "position: absolute; left: 10px; top: 0",
        "width",
    ),
    (
        "fixed h tb",
        "width: 800px",
        "position: fixed; left: 0; top: 10px; bottom: 20px",
        "height",
    ),
    (
        "fixed h t",
        "width: 800px",
        "position: fixed; left: 0; top: 10px",
        "height",
    ),
    ("block h cb300", "width: 800px; height: 300px", "", "height"),
    (
        "inline-block h cb300",
        "width: 800px; height: 300px",
        "display: inline-block",
        "height",
    ),
    (
        "flex-row h cb300 (cross)",
        "display: flex; width: 800px; height: 300px; align-items: flex-start",
        "",
        "height",
    ),
    (
        "flex-col h cb300 (main)",
        "display: flex; flex-direction: column; width: 800px; height: 300px",
        "",
        "height",
    ),
    (
        "grid h cb300",
        "display: grid; width: 800px; height: 300px; align-items: start",
        "",
        "height",
    ),
    (
        "abs h cb300",
        "position: relative; width: 800px; height: 300px",
        "position: absolute; left: 0; top: 10px",
        "height",
    ),
    (
        "block margin cb800",
        "width: 800px",
        "margin: 0 30px; padding: 0 5px; border: 0 solid; border-width: 0 2px",
        "width",
    ),
    (
        "inline-block margin cb800",
        "width: 800px",
        "display: inline-block; margin: 0 30px; padding: 0 5px; border: 0 solid; \
         border-width: 0 2px",
        "width",
    ),
];
/// Chrome 153's answer per (content, context) for `KEYWORDS` in order.
const CHROME: &[(&str, &str, [f32; 5])] = &[
    ("block", "block cb800", [800.0, 300.0, 300.0, 300.0, 800.0]),
    ("block", "block cb150", [150.0, 300.0, 300.0, 300.0, 150.0]),
    (
        "block",
        "inline-block cb800",
        [300.0, 300.0, 300.0, 300.0, 800.0],
    ),
    (
        "block",
        "inline-block cb150",
        [300.0, 300.0, 300.0, 300.0, 150.0],
    ),
    (
        "block",
        "inline-flex cb800",
        [300.0, 300.0, 300.0, 300.0, 800.0],
    ),
    (
        "block",
        "flex-row cb800",
        [300.0, 300.0, 300.0, 300.0, 800.0],
    ),
    (
        "block",
        "flex-row cb150",
        [300.0, 300.0, 300.0, 300.0, 150.0],
    ),
    (
        "block",
        "flex-row basis cb800",
        [300.0, 300.0, 300.0, 300.0, 800.0],
    ),
    (
        "block",
        "flex-col cb800",
        [800.0, 300.0, 300.0, 300.0, 800.0],
    ),
    (
        "block",
        "flex-col cb150",
        [150.0, 300.0, 300.0, 300.0, 150.0],
    ),
    (
        "block",
        "grid auto cb800",
        [800.0, 300.0, 300.0, 300.0, 800.0],
    ),
    (
        "block",
        "grid 1fr cb150",
        [300.0, 300.0, 300.0, 300.0, 300.0],
    ),
    ("block", "abs cb800", [300.0, 300.0, 300.0, 300.0, 800.0]),
    ("block", "abs lr cb800", [780.0, 300.0, 300.0, 300.0, 780.0]),
    ("block", "abs cb150", [300.0, 300.0, 300.0, 300.0, 150.0]),
    ("block", "fixed lr", [780.0, 300.0, 300.0, 300.0, 780.0]),
    ("block", "fixed l", [300.0, 300.0, 300.0, 300.0, 790.0]),
    ("block", "abs icb l", [300.0, 300.0, 300.0, 300.0, 790.0]),
    ("block", "fixed h tb", [483.0, 20.0, 20.0, 20.0, 483.0]),
    ("block", "fixed h t", [20.0, 20.0, 20.0, 20.0, 503.0]),
    ("block", "block h cb300", [20.0, 20.0, 20.0, 20.0, 300.0]),
    (
        "block",
        "inline-block h cb300",
        [20.0, 20.0, 20.0, 20.0, 300.0],
    ),
    (
        "block",
        "flex-row h cb300 (cross)",
        [20.0, 20.0, 20.0, 20.0, 300.0],
    ),
    (
        "block",
        "flex-col h cb300 (main)",
        [20.0, 20.0, 20.0, 20.0, 300.0],
    ),
    ("block", "grid h cb300", [20.0, 20.0, 20.0, 20.0, 300.0]),
    ("block", "abs h cb300", [20.0, 20.0, 20.0, 20.0, 290.0]),
    (
        "block",
        "block margin cb800",
        [740.0, 314.0, 314.0, 314.0, 740.0],
    ),
    (
        "block",
        "inline-block margin cb800",
        [314.0, 314.0, 314.0, 314.0, 740.0],
    ),
    ("two", "block cb800", [800.0, 200.0, 100.0, 200.0, 800.0]),
    ("two", "block cb150", [150.0, 200.0, 100.0, 150.0, 150.0]),
    (
        "two",
        "inline-block cb800",
        [200.0, 200.0, 100.0, 200.0, 800.0],
    ),
    (
        "two",
        "inline-block cb150",
        [150.0, 200.0, 100.0, 150.0, 150.0],
    ),
    (
        "two",
        "inline-flex cb800",
        [200.0, 200.0, 100.0, 200.0, 800.0],
    ),
    ("two", "flex-row cb800", [200.0, 200.0, 100.0, 200.0, 800.0]),
    ("two", "flex-row cb150", [150.0, 150.0, 100.0, 150.0, 150.0]),
    (
        "two",
        "flex-row basis cb800",
        [200.0, 200.0, 100.0, 200.0, 800.0],
    ),
    ("two", "flex-col cb800", [800.0, 200.0, 100.0, 200.0, 800.0]),
    ("two", "flex-col cb150", [150.0, 200.0, 100.0, 150.0, 150.0]),
    (
        "two",
        "grid auto cb800",
        [800.0, 200.0, 100.0, 200.0, 800.0],
    ),
    ("two", "grid 1fr cb150", [150.0, 200.0, 100.0, 150.0, 150.0]),
    ("two", "abs cb800", [200.0, 200.0, 100.0, 200.0, 800.0]),
    ("two", "abs lr cb800", [780.0, 200.0, 100.0, 200.0, 780.0]),
    ("two", "abs cb150", [150.0, 200.0, 100.0, 150.0, 150.0]),
    ("two", "fixed lr", [780.0, 200.0, 100.0, 200.0, 780.0]),
    ("two", "fixed l", [200.0, 200.0, 100.0, 200.0, 790.0]),
    ("two", "abs icb l", [200.0, 200.0, 100.0, 200.0, 790.0]),
    ("two", "fixed h tb", [483.0, 20.0, 20.0, 20.0, 483.0]),
    ("two", "fixed h t", [20.0, 20.0, 20.0, 20.0, 503.0]),
    ("two", "block h cb300", [20.0, 20.0, 20.0, 20.0, 300.0]),
    (
        "two",
        "inline-block h cb300",
        [20.0, 20.0, 20.0, 20.0, 300.0],
    ),
    (
        "two",
        "flex-row h cb300 (cross)",
        [20.0, 20.0, 20.0, 20.0, 300.0],
    ),
    (
        "two",
        "flex-col h cb300 (main)",
        [20.0, 20.0, 20.0, 20.0, 300.0],
    ),
    ("two", "grid h cb300", [20.0, 20.0, 20.0, 20.0, 300.0]),
    ("two", "abs h cb300", [20.0, 20.0, 20.0, 20.0, 290.0]),
    (
        "two",
        "block margin cb800",
        [740.0, 214.0, 114.0, 214.0, 740.0],
    ),
    (
        "two",
        "inline-block margin cb800",
        [214.0, 214.0, 114.0, 214.0, 740.0],
    ),
];

/// Where rinch and Chrome 153 still disagree, as (content, context, keyword,
/// rinch's answer). Every entry is a known deviation with its reason; the test
/// fails if one of these is fixed (so the list must shrink with it) or if any
/// other cell disagrees.
const KNOWN: &[(&str, &str, &str, f32)] = &[];

fn rinch_table() -> Vec<(&'static str, &'static str, &'static str, f32, f32)> {
    let mut cells = Vec::new();
    for &(kind, label, chrome) in CHROME {
        let &(_, cb, ts, prop) = CONTEXTS
            .iter()
            .find(|c| c.0 == label)
            .unwrap_or_else(|| panic!("no context {label}"));
        for (i, k) in KEYWORDS.iter().enumerate() {
            let got = measure(kind, cb, ts, prop, k);
            cells.push((kind, label, *k, got, chrome[i]));
        }
    }
    cells
}

/// The twin: every cell agrees with Chrome 153, except the ones `KNOWN` names
/// with rinch's own answer.
#[test]
fn intrinsic_keywords_match_chrome_153() {
    let cells = rinch_table();
    assert_eq!(cells.len(), CHROME.len() * KEYWORDS.len());
    let mut wrong = Vec::new();
    for &(kind, label, k, got, want) in &cells {
        let known = KNOWN
            .iter()
            .find(|e| e.0 == kind && e.1 == label && e.2 == k);
        let agrees = (got - want).abs() < 0.5;
        match known {
            None if !agrees => wrong.push(format!(
                "{kind} | {label} | {k}: rinch {got}, Chrome 153 {want}"
            )),
            Some(e) if agrees => wrong.push(format!(
                "{kind} | {label} | {k}: now agrees with Chrome ({got}) — remove it from KNOWN"
            )),
            Some(e) if (got - e.3).abs() >= 0.5 => wrong.push(format!(
                "{kind} | {label} | {k}: KNOWN says rinch gives {}, it gives {got}",
                e.3
            )),
            _ => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "{} cells:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
