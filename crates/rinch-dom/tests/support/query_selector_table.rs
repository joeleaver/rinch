// The selector table for #1796, shared by the desktop test
// (`crates/rinch-dom/tests/query_selector_1796_tests.rs`) and the browser
// twin (`crates/rinch-web/tests/query_selector_1796.rs`), which runs the same
// rows through Chrome's own `Element.querySelectorAll` and through rinch-web.
// The browser is the oracle: a row that Chrome answers differently is a wrong
// row, not a desktop bug.

/// Set as the `innerHTML` of a host `<div id="host">`.
pub const MARKUP: &str = concat!(
    r#"<div id="root" class="panel panel--right">"#,
    r#"<header id="h" class="panel__head"><h2 id="t" class="title">T</h2></header>"#,
    r#"<section id="body" class="panel__body" data-kind="main">"#,
    r#"<button id="b1" class="primary big">A</button>"#,
    r#"<button id="b2" class="secondary" disabled>B</button>"#,
    r#"<p id="p1" lang="en-US">x <span id="s1" class="a b">y</span></p>"#,
    r#"<p id="p2" data-kind="note x"><span id="s2" class="a">z</span></p>"#,
    r#"<input id="i1" type="checkbox" checked>"#,
    r#"<ul id="u"><li id="l1">1</li><li id="l2">2</li><li id="l3">3</li></ul>"#,
    r#"</section></div>"#,
    r#"<div id="other" class="panel__body"><span id="s3" class="a b">w</span></div>"#,
);

/// `(scope id, selector, ids of the matches in document order)`.
/// `query_selector_all` on the scope must answer exactly these; `query_selector`
/// the first (or none).
pub const CASES: &[(&str, &str, &[&str])] = &[
    // The four shapes the old desktop matcher took, still answered.
    ("host", "span", &["s1", "s2", "s3"]),
    ("host", ".panel__body", &["body", "other"]),
    ("host", "#b2", &["b2"]),
    ("host", "[data-kind]", &["body", "p2"]),
    ("host", "[data-kind=main]", &["body"]),
    ("host", "[data-kind='note x']", &["p2"]),
    // Combinators (#1796's own example first).
    ("host", ".panel--right .panel__body", &["body"]),
    ("host", "section > p > span", &["s1", "s2"]),
    ("host", "header + section", &["body"]),
    ("host", "#b1 ~ p", &["p1", "p2"]),
    ("host", "#b2 ~ button", &[]),
    // Compounds and lists (a list answers in document order, not list order).
    ("host", "button.primary", &["b1"]),
    ("host", ".a.b", &["s1", "s3"]),
    ("host", "#t, #b1", &["t", "b1"]),
    ("host", "#b1, #t", &["t", "b1"]),
    ("host", "SPAN.a", &["s1", "s2", "s3"]),
    // Pseudo-classes.
    ("host", "button:not(.primary)", &["b2"]),
    ("host", "button:disabled", &["b2"]),
    ("host", "button:enabled", &["b1"]),
    ("host", "input:checked", &["i1"]),
    ("host", "li:first-child", &["l1"]),
    ("host", "li:nth-child(2)", &["l2"]),
    ("host", "li:last-child", &["l3"]),
    ("host", "h2:only-child", &["t"]),
    ("host", "p:lang(en)", &["p1"]),
    ("host", "section > :not(p):not(ul)", &["b1", "b2", "i1"]),
    // Attribute operators and the case flag.
    ("host", "[data-kind~=x]", &["p2"]),
    ("host", "[data-kind^=no]", &["p2"]),
    ("host", "[data-kind$=n]", &["body"]),
    ("host", "[class*='--right']", &["root"]),
    ("host", "[data-kind=MAIN i]", &["body"]),
    ("host", "[data-kind=MAIN]", &[]),
    // Scoped: the scope's descendants only, the scope itself never; the
    // selector still sees ancestors outside the scope; `:scope` is the scope.
    ("body", "span", &["s1", "s2"]),
    ("body", ".panel--right span", &["s1", "s2"]),
    ("body", "section", &[]),
    ("body", "section span", &["s1", "s2"]),
    ("body", ":scope > p", &["p1", "p2"]),
    ("body", ":scope > span", &[]),
    ("p1", "span", &["s1"]),
    ("other", ".a", &["s3"]),
    ("l1", "li", &[]),
];

/// Selectors the browser throws a SyntaxError for: `None` and empty.
pub const INVALID: &[&str] = &["", "div {", "[", ".", "#", "a >", "p::", "button:not("];
