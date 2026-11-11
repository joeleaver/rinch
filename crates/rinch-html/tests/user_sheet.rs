//! `Policy::user_css`: the app's own style sheet, matched against the
//! document as written and marked for the view. Synthetic documents only.

use rinch_html::{Cleaned, Policy, USER_MARK, clean};

fn with_sheet(doc: &str, css: &str) -> Cleaned {
    clean(
        doc,
        &Policy {
            user_css: css.into(),
            ..Policy::default()
        },
    )
}

/// The text of each element carrying `mark` (a rule index) in `html`, by the
/// `>text<` after the opening tag.
fn marked(html: &str, rule: usize) -> Vec<String> {
    let needle = format!("{USER_MARK}=\"");
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = html[at..].find(&needle) {
        let start = at + i + needle.len();
        let end = start + html[start..].find('"').unwrap();
        at = end;
        if html[start..end]
            .split_ascii_whitespace()
            .any(|r| r == rule.to_string())
        {
            let rest = &html[end..];
            let open = rest.find('>').unwrap() + 1;
            let close = rest[open..].find('<').unwrap() + open;
            out.push(rest[open..close].trim().to_owned());
        }
    }
    out
}

const REPLY: &str = r#"<html><body>
<p>Sounds good, see you then.</p>
<div class="gmail_quote">On Monday, Ann wrote:
  <blockquote type="cite">Shall we meet at noon?</blockquote>
</div>
<div name="messageReplySection">Reply section</div>
<p>Bring the slides.</p>
</body></html>"#;

#[test]
fn selectors_match_classes_and_attributes_cleaning_strips() {
    let plain = clean(REPLY, &Policy::default()).html;
    assert!(
        !plain.contains("gmail_quote") && !plain.contains("type=") && !plain.contains("name="),
        "the allow-list strips them: {plain}"
    );
    let out = with_sheet(
        REPLY,
        ".gmail_quote { color: grey }
         blockquote[type=cite] { display: none }
         div[name=messageReplySection] { display: none }
         blockquote[type=CITE] { margin: 0 }",
    );
    assert_eq!(
        out.user_selectors,
        [
            ".gmail_quote",
            "blockquote[type=cite]",
            "div[name=messageReplySection]",
            "blockquote[type=CITE]"
        ]
    );
    assert_eq!(out.user_matches, [1, 1, 1, 1], "{}", out.html);
    assert_eq!(marked(&out.html, 0), ["On Monday, Ann wrote:"]);
    assert_eq!(marked(&out.html, 1), ["Shall we meet at noon?"]);
    assert_eq!(marked(&out.html, 2), ["Reply section"]);
    // `type` matches case-insensitively in HTML, as in a browser; the
    // blockquote carries both rules.
    assert!(
        out.html.contains(&format!("{USER_MARK}=\"1 3\"")),
        "{}",
        out.html
    );
}

#[test]
fn a_later_sibling_selector_marks_what_follows_the_marker() {
    let doc = r#"<div>
        <p>My answer is yes.</p>
        <div id="divRplyFwdMsg"><b>From:</b> Ann</div>
        <div>Earlier message <span>inner</span></div>
        tail text
        <p>Older</p>
    </div><p>After the wrapper</p>"#;
    let out = with_sheet(
        doc,
        "#divRplyFwdMsg ~ * { display: none } #divRplyFwdMsg { display: none }",
    );
    assert_eq!(out.user_matches, [2, 1], "{}", out.html);
    assert_eq!(marked(&out.html, 0), ["Earlier message", "Older"]);
    assert_eq!(
        marked(&out.html, 1),
        [""],
        "the marker itself (its first child is <b>)"
    );
    // A `~` whose right side has a combinator goes through the crate.
    let out = with_sheet(doc, "#divRplyFwdMsg ~ div span { color: red }");
    assert_eq!(out.user_matches, [1]);
    assert_eq!(marked(&out.html, 0), ["inner"]);
    // `+` is the next element only.
    let out = with_sheet(doc, "#divRplyFwdMsg + * { color: red }");
    assert_eq!(out.user_matches, [1]);
    assert_eq!(marked(&out.html, 0), ["Earlier message"]);
}

#[test]
fn counts_are_per_rule_and_count_every_matching_element() {
    let doc = "<blockquote>a<blockquote>b<blockquote>c</blockquote></blockquote></blockquote>\
               <blockquote>d</blockquote><p>e</p>";
    let out = with_sheet(
        doc,
        "blockquote { color: grey } blockquote blockquote { color: red } table { x: y }",
    );
    assert_eq!(out.user_matches, [4, 2, 0]);
}

#[test]
fn selectors_that_do_not_parse_match_nothing_and_at_rules_are_skipped() {
    let out = with_sheet(
        REPLY,
        "div! { color: red } p::before { color: red } a:hover { color: red }
         @media (max-width: 600px) { p { color: red } }
         @import url(x.css);
         p { color: blue }",
    );
    assert_eq!(out.user_selectors, ["div!", "p::before", "a:hover", "p"]);
    assert_eq!(out.user_matches, [0, 0, 0, 2], "{}", out.html);
}

#[test]
fn the_documents_own_marks_are_dropped() {
    let doc = format!(r#"<p {USER_MARK}="0">forged</p><blockquote>real</blockquote>"#);
    assert!(!clean(&doc, &Policy::default()).html.contains(USER_MARK));
    let out = with_sheet(&doc, "blockquote { display: none }");
    assert_eq!(marked(&out.html, 0), ["real"], "{}", out.html);
    assert_eq!(out.user_matches, [1]);
}

#[test]
fn the_document_itself_is_never_marked_and_no_sheet_changes_nothing() {
    let doc = r#"<html class="x"><body class="x"><p class="x">kept</p></body></html>"#;
    let out = with_sheet(doc, ".x { color: red } body { color: red }");
    assert_eq!(out.user_matches, [1, 0]);
    let none = with_sheet(doc, "");
    assert_eq!(none, clean(doc, &Policy::default()));
    assert!(none.user_matches.is_empty());
}
