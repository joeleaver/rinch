//! `Policy::allow`: an app's narrower allow-list, for a format that fixes
//! its own (a protocol's message bodies). Synthetic documents only.

use std::sync::Arc;

use rinch_html::{AllowList, CARRIED, Cleaned, Policy, USER_MARK, clean};

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

fn list(tags: &[&str], attributes: &[(&str, &str)], removed: &[&str]) -> AllowList {
    AllowList {
        tags: strings(tags),
        attributes: attributes
            .iter()
            .map(|(t, a)| ((*t).to_owned(), (*a).to_owned()))
            .collect(),
        removed_with_content: strings(removed),
        carried: Vec::new(),
    }
}

fn narrowed(doc: &str, allow: AllowList) -> Cleaned {
    clean(
        doc,
        &Policy {
            allow: Some(allow),
            ..Policy::default()
        },
    )
}

#[test]
fn only_the_listed_elements_and_attributes_stay() {
    let out = narrowed(
        r#"<div class="x" style="color: red"><p id="p" style="color: blue">a <b>b</b>
           <font color="red">c</font></p><a href="https://e.example/" title="t">d</a>
           <img src="data:image/png;base64,AA" alt="pic" width="9"></div>"#,
        list(
            &["p", "a", "img"],
            &[("a", "href"), ("img", "alt"), ("img", "src")],
            &[],
        ),
    );
    let html = out.html;
    for gone in [
        "<div", "<b>", "<font", "class=", "style=", "id=", "width=", "title=",
    ] {
        assert!(!html.contains(gone), "{gone} survived:\n{html}");
    }
    // What was inside a removed element stays.
    for kept in [
        "<p>a b",
        "c</p>",
        r#"href="https://e.example/""#,
        r#"alt="pic""#,
        r#"src="data:image/png;base64,AA""#,
    ] {
        assert!(html.contains(kept), "{kept} missing:\n{html}");
    }
    // Links still get rinch's rel.
    assert!(html.contains("noopener"), "{html}");
}

#[test]
fn naming_what_rinch_leaves_out_lets_nothing_through() {
    let out = narrowed(
        r#"<p onclick="steal()" data-rid="7" data-oninput="x">a</p>
           <script>alert(1)</script><form action="https://e.example/"><input name="q"></form>
           <iframe src="https://e.example/"></iframe><a href="javascript:alert(1)">j</a>"#,
        list(
            &["p", "a", "script", "form", "input", "iframe"],
            &[
                ("p", "onclick"),
                ("p", "data-rid"),
                ("*", "data-oninput"),
                ("*", "onclick"),
                ("a", "href"),
                ("iframe", "src"),
            ],
            &[],
        ),
    );
    let lower = out.html.to_ascii_lowercase();
    for gone in [
        "onclick",
        "data-rid",
        "data-oninput",
        "<script",
        "alert(1)",
        "<form",
        "<input",
        "<iframe",
        "javascript:",
    ] {
        assert!(!lower.contains(gone), "{gone} survived:\n{}", out.html);
    }
    assert!(lower.contains("<p>a</p>"), "{}", out.html);
}

#[test]
fn removed_with_content_takes_what_is_inside() {
    let out = narrowed(
        r#"<p>keep</p><noscript><p>fallback</p></noscript><object><p>object fallback</p></object>
           <template><p>template</p></template><iframe>frame text</iframe><embed src="x">
           <style>p { color: red }</style><p>also</p>"#,
        list(
            &["p", "noscript"],
            &[],
            &["noscript", "object", "template", "iframe", "embed"],
        ),
    );
    let html = out.html;
    assert!(
        html.contains("<p>keep</p>") && html.contains("<p>also</p>"),
        "{html}"
    );
    for gone in [
        "fallback",
        "template",
        "frame text",
        "<noscript",
        "color: red",
    ] {
        assert!(!html.contains(gone), "{gone} survived:\n{html}");
    }
}

#[test]
fn the_body_style_is_left_out() {
    let doc = r#"<html><body bgcolor="black" style="color: white"><p>a</p></body></html>"#;
    assert!(!clean(doc, &Policy::default()).body_style.is_empty());
    assert_eq!(narrowed(doc, list(&["p"], &[], &[])).body_style, "");
}

#[test]
fn a_kept_attribute_still_goes_through_its_rule() {
    let out = clean(
        r#"<img src="https://e.example/a.png" alt="a"><p style="position: fixed; color: red">p</p>
           <a href="ftp://e.example/">f</a>"#,
        &Policy {
            picture: Arc::new(|_| None),
            allow: Some(list(
                &["img", "p", "a"],
                &[
                    ("img", "src"),
                    ("img", "alt"),
                    ("p", "style"),
                    ("a", "href"),
                ],
                &[],
            )),
            ..Policy::default()
        },
    );
    let html = out.html;
    assert!(!html.contains("e.example/a.png"), "{html}");
    assert!(!html.contains("position"), "{html}");
    assert!(html.contains("color: red"), "{html}");
    assert!(!html.contains("ftp:"), "{html}");
}

#[test]
fn the_user_sheet_marks_what_the_list_strips_from_the_document() {
    // A data attribute the list cannot keep is still matched as written:
    // the mark carries it through.
    let out = clean(
        r#"<p><img data-attachment="att_1" alt="one"><img data-attachment="att_2" alt="two"></p>"#,
        &Policy {
            user_css: r#"img[data-attachment="att_2"] {}"#.into(),
            allow: Some(list(&["p", "img"], &[("img", "alt")], &[])),
            ..Policy::default()
        },
    );
    assert_eq!(out.user_matches, vec![1]);
    assert!(!out.html.contains("data-attachment"), "{}", out.html);
    assert!(
        out.html
            .contains(&format!(r#"<img alt="two" {USER_MARK}="0">"#)),
        "{}",
        out.html
    );
}

#[test]
fn a_deep_document_is_still_cut() {
    let doc = "<blockquote>".repeat(20_000) + "deep" + &"</blockquote>".repeat(20_000);
    let out = narrowed(&doc, list(&["blockquote"], &[], &[]));
    let depth = out.html.matches("<blockquote>").count();
    assert!(depth <= 512, "{depth} levels kept");
}

#[test]
fn a_carried_attribute_comes_out_renamed_and_only_carried_values_bear_the_name() {
    let mut allow = list(&["p", "img"], &[("img", "alt")], &[]);
    allow.carried = vec![("img".into(), "data-attachment".into())];
    let name = format!("{CARRIED}data-attachment");
    let out = narrowed(
        &format!(
            r#"<p {name}="forged"><img data-attachment="a&quot;b" alt="one">
               <img {name}="forged" alt="two"></p><div data-attachment="not an img">x</div>"#
        ),
        allow,
    );
    let html = out.html;
    assert_eq!(html.matches(&name).count(), 1, "{html}");
    assert!(
        html.contains(&format!(r#"{name}="a&quot;b""#)) && html.contains(r#"alt="one""#),
        "{html}"
    );
    assert!(
        !html.contains("forged") && !html.contains(" data-attachment"),
        "{html}"
    );
}

#[test]
fn a_document_that_parses_differently_twice_cannot_forge_a_carried_value() {
    // A `<form>` in MathML: what one parse reads as text, a second parse of
    // the written-out document reads as elements.
    let mut allow = list(&["p", "a", "img"], &[("a", "href"), ("img", "alt")], &[]);
    allow.carried = vec![
        ("img".into(), "data-attachment".into()),
        // Named twice, and once under the carried prefix itself.
        ("*".into(), "data-attachment".into()),
        ("img".into(), format!("{CARRIED}data-attachment")),
    ];
    let name = format!("{CARRIED}data-attachment");
    let out = narrowed(
        &format!(
            "<img data-attachment=real alt=real>\
             <form><math><mtext></form><form><mglyph><style></math>\
             <p {name}=x>t</p><a href=http://e.example {name}=x>l</a>\
             <img {name}=x alt=forged>"
        ),
        allow,
    );
    let html = out.html;
    assert_eq!(html.matches(&name).count(), 1, "{html}");
    assert!(html.contains(&format!(r#"{name}="real""#)), "{html}");
}
