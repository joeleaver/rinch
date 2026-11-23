//! #2276: hit testing a run that paint draws in CSS 2.1 Appendix E phases.
//! A later block's background is painted under an earlier block's text, an
//! inline-block and a float, so a press there finds the text's block, the
//! inline-block or the float, where Chrome 158's `elementFromPoint` does.
//! Elsewhere the later box is on top and answers.
//!
//! Each case is the markup the paint fixture
//! (`rinch-dom/tests/paint_phases_2276_tests.rs`) draws, set in Inter at a
//! declared `font: 16px/20px` in a `400px` host at the origin, and the
//! answers are Chrome 158 headless at 1x on the same markup. The host is a
//! plain block (its run is walked by `hit_test_node`'s in-flow branch) and,
//! in the second test, a `z-index: 0` stacking context (its run is the
//! sequence's in-flow entries).

use super::hit_test;
use rinch_core::dom::DomDocument;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// `(x, y, the id Chrome answers)`; `""` is the host.
type Probe = (f32, f32, &'static str);

/// `(markup, probes)`.
const CASES: &[(&str, &[Probe])] = &[
    (
        "<div id=a>abc</div><div id=b style='margin-top:-20px;height:20px;background:#ff0'></div>",
        &[
            (10.0, 10.0, "a"),
            (13.0, 1.0, "a"),
            (13.0, 19.0, "a"),
            (30.0, 10.0, "b"),
            (200.0, 10.0, "b"),
        ],
    ),
    (
        "<div id=a style='height:20px'>one<br>two</div><div id=b style='height:40px;background:#ff0'></div>",
        &[(5.0, 30.0, "a"), (200.0, 30.0, "b")],
    ),
    (
        "<div><div id=f style='float:left;width:30px;height:60px;background:#00f'></div>x</div><p id=p style='margin:0;height:60px;background:#ff0'>yy</p>",
        &[(10.0, 40.0, "f"), (100.0, 40.0, "p")],
    ),
    (
        "<div><span id=s style='display:inline-block;width:30px;height:20px;background:#00f'></span></div><div id=b style='margin-top:-20px;height:20px;background:#ff0'></div>",
        &[(10.0, 10.0, "s"), (100.0, 10.0, "b")],
    ),
    (
        "<div style='display:flex'><div id=a style='width:100px'>abc</div><div id=b style='margin-left:-100px;width:100px;height:20px;background:#ff0'></div></div>",
        &[(10.0, 10.0, "b")],
    ),
    (
        "<div id=a style='overflow:hidden;height:20px'>abc</div><div id=b style='margin-top:-20px;height:20px;background:#ff0'></div>",
        &[(10.0, 10.0, "a"), (200.0, 10.0, "b")],
    ),
];

fn document(host_style: &str, markup: &str) -> (rinch_dom::RinchDocument, u64) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = rinch_dom::RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc.load_css("body { margin: 0; font: 16px/20px ProbeFace; }");
    let body = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(host, "style", &format!("width: 400px; {host_style}"));
    doc.append_child(body, host);
    doc.set_inner_html(host, markup);
    doc.resolve_layout(800.0, 600.0);
    (doc, host.0)
}

fn id_of(doc: &rinch_dom::RinchDocument, host: u64, hit: Option<u64>) -> String {
    match hit {
        Some(h) if h == host => String::new(),
        Some(h) => doc
            .tree
            .get(h)
            .and_then(|n| n.attributes.get("id").cloned())
            .unwrap_or_else(|| format!("<node {h}>")),
        None => "<none>".into(),
    }
}

fn check(host_style: &str) {
    let mut wrong = Vec::new();
    for &(markup, probes) in CASES {
        let (doc, host) = document(host_style, markup);
        for &(x, y, want) in probes {
            let got = id_of(&doc, host, hit_test(&doc.tree, x, y));
            if got != want {
                wrong.push(format!(
                    "{markup}\n  ({x}, {y}): rinch {got:?}, Chrome {want:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Mutants: probing the run's children whole, in reverse tree order (the
/// pre-#2276 walk) — the text, float and inline-block points answer the later
/// box; probing a flex container's items in phases — the flex case answers
/// `a`; leaving out the float pass — `(10, 40)` answers `p`.
#[test]
fn a_phased_runs_lines_and_floats_are_hit_over_later_backgrounds() {
    check("");
}

/// The same run as a stacking context's in-flow entries.
#[test]
fn a_stacking_contexts_in_flow_entries_are_hit_in_phases_too() {
    check("position: relative; z-index: 0");
}
