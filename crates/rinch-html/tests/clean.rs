//! Cleaning documents someone else wrote: hostile ones, legacy layout,
//! style sheets, pictures and links.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_html::{Cleaned, Policy, clean};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn hostile_documents_lose_what_runs_submits_frames_or_covers() {
    let out = clean(&fixture("11_hostile.html"), &Policy::default()).html;
    let lower = out.to_ascii_lowercase();
    for gone in [
        "<script",
        "<iframe",
        "<form",
        "<input",
        "onload=",
        "onclick=",
        "onerror=",
        "javascript:",
        "<style",
        "position: fixed",
        "position:fixed",
        "position: absolute",
    ] {
        assert!(!lower.contains(gone), "{gone} survived:\n{out}");
    }
}

#[test]
fn legacy_layout_attributes_stay_for_rinch_to_map() {
    let out = clean(&fixture("03_receipt_attrs.html"), &Policy::default());
    assert!(out.html.contains("bgcolor="), "{}", out.html);
    assert!(out.html.contains("cellpadding="), "{}", out.html);
    assert!(out.html.contains("<table"));
}

#[test]
fn style_sheets_apply_inline_and_are_filtered() {
    let doc = r##"<html><head><style>
        .mobile { display: none }
        .lead { color: #332f2e; font-size: 18px }
        .cover { position: fixed; top: 0; left: 0 }
        @media (max-width: 600px) { .mobile { display: block } }
        body { background: #f4f1ed }
    </style></head><body>
        <p class="lead">Ready, set, gift</p>
        <div class="mobile">phones only</div>
        <div class="cover">over the window</div>
    </body></html>"##;
    let out = clean(doc, &Policy::default());
    let html = &out.html;
    assert!(html.contains("color: #332f2e; font-size: 18px"), "{html}");
    assert!(
        html.contains(r#"<div style="display: none">phones only"#),
        "{html}"
    );
    assert!(!html.contains("position"), "{html}");
    assert!(!html.contains("<style"), "{html}");
    assert_eq!(out.body_style, "background: #f4f1ed");
}

#[test]
fn the_default_policy_fetches_no_picture() {
    let doc = r##"<html><body bgcolor="#f0f0f0" text="#333" style="margin: 0; position: fixed">
        <img src="https://track.example/open.gif" width="1" height="1">
        <img src="data:image/png;base64,iVBORw0KGgo=">
        <div style="background: url(https://track.example/bg.png); color: red">hi</div>
        </body></html>"##;
    let out = clean(doc, &Policy::default());
    assert!(!out.html.contains("track.example"), "{}", out.html);
    assert!(out.html.contains("data:image/png;base64"), "{}", out.html);
    assert!(out.html.contains("color: red"));
    assert_eq!(
        out.body_style,
        "background-color: #f0f0f0; color: #333; margin: 0"
    );
}

#[test]
fn the_picture_rule_decides_and_can_rewrite() {
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let policy = Policy {
        picture: Arc::new(move |src| {
            counted.fetch_add(1, Ordering::Relaxed);
            if let Some(id) = src.strip_prefix("cid:") {
                return Some(format!("data:image/png;base64,{id}"));
            }
            src.starts_with("https://pictures.example/")
                .then(|| src.to_owned())
        }),
        ..Policy::default()
    };
    let doc = r#"<img src="cid:AAAA"><img src="https://track.example/p.gif">
        <img src="https://pictures.example/a.png">
        <div style="background-image: url('https://pictures.example/bg.png'); color: blue">x</div>"#;
    let out = clean(doc, &policy);
    assert!(
        out.html.contains(r#"src="data:image/png;base64,AAAA""#),
        "{}",
        out.html
    );
    assert!(!out.html.contains("track.example"), "{}", out.html);
    assert!(
        out.html.contains(r#"src="https://pictures.example/a.png""#),
        "{}",
        out.html
    );
    assert!(
        out.html
            .contains(r#"url(&quot;https://pictures.example/bg.png&quot;)"#),
        "{}",
        out.html
    );
    assert_eq!(asked.load(Ordering::Relaxed), 4);
}

#[test]
fn links_keep_only_allowed_schemes() {
    let doc = r#"<a href="https://shop.example/sale" onclick="steal()">Sale</a>
        <a href="mailto:ada@example.org">Mail</a>
        <a href="file:///etc/passwd">File</a>
        <a href="steam://run/1">Game</a>"#;
    let out = clean(doc, &Policy::default()).html;
    assert!(out.contains(r#"href="https://shop.example/sale""#), "{out}");
    assert!(out.contains(r#"href="mailto:ada@example.org""#), "{out}");
    assert!(!out.contains("file:"), "{out}");
    assert!(!out.contains("steam:"), "{out}");
    assert!(out.contains("noopener"));
    assert!(!out.contains("onclick"));
}

#[test]
fn text_is_escaped_with_its_spacing_kept() {
    let out = Cleaned::from_text("Use <b> & keep\r\n  spacing");
    assert!(
        out.html.contains("Use &lt;b&gt; &amp; keep\n  spacing"),
        "{}",
        out.html
    );
    assert!(out.html.contains("pre-wrap"));
}

#[test]
fn media_rules_apply_at_the_width_drawn() {
    let doc = r#"<html><head><style>
        .mob { display: none }
        .col { width: 300px }
        @media only screen and (max-width: 600px) {
            .mob { display: block !important }
            .desk { display: none !important }
            .col { width: 100% !important }
        }
    </style></head><body>
        <div class="desk">desktop</div><div class="mob">mobile</div>
        <div class="col" style="color: red">column</div>
    </body></html>"#;
    let narrow = rinch_html::clean_at(doc, &Policy::default(), 480.0).html;
    assert!(
        narrow.contains(r#"<div style="display: none !important">desktop"#),
        "{narrow}"
    );
    assert!(
        narrow.contains(r#"<div style="display: block !important">mobile"#),
        "{narrow}"
    );
    assert!(narrow.contains("width: 100%"), "{narrow}");
    assert!(narrow.contains("color: red"), "{narrow}");
    let wide = rinch_html::clean_at(doc, &Policy::default(), 720.0).html;
    assert!(wide.contains(r#"<div>desktop"#), "{wide}");
    assert!(
        wide.contains(r#"<div style="display: none">mobile"#),
        "{wide}"
    );
    assert!(wide.contains("width: 300px"), "{wide}");
    // Without a width, as wide: every @media block left out.
    assert_eq!(clean(doc, &Policy::default()).html, wide);
    assert_eq!(rinch_html::signature(doc, 480.0), [true]);
    assert_eq!(rinch_html::signature(doc, 720.0), [false]);
}
