//! The first review of #2213: ways past the cleaner, each as it was found.

use std::sync::Arc;

use rinch_html::{Policy, clean};

/// Time limits are a release build's; a debug build is about ten times
/// slower.
const SLOW: f32 = if cfg!(debug_assertions) { 10.0 } else { 1.0 };

fn cleaned(html: &str) -> String {
    clean(html, &Policy::default()).html
}

/// A policy that allows every picture, so a `url()` that survives is the
/// rule's to see; `seen` records what it was asked.
fn allowing(seen: Arc<std::sync::Mutex<Vec<String>>>) -> Policy {
    Policy {
        picture: Arc::new(move |src| {
            seen.lock().unwrap().push(src.to_owned());
            src.starts_with("https://ok.example/")
                .then(|| src.to_owned())
        }),
        ..Policy::default()
    }
}

#[test]
fn escaped_or_commented_property_names_are_read_as_the_browser_reads_them() {
    for style in [
        r"posit\69 on: fixed; top: 0",
        "position/**/: fixed; top: 0",
        r"z-ind\65 x: 99999",
        r"position: f\69 xed",
    ] {
        let out = cleaned(&format!(r#"<div style="{style}">x</div>"#));
        let lower = out.to_ascii_lowercase();
        assert!(
            !lower.contains("fixed") && !lower.contains("99999"),
            "{style} survived: {out}"
        );
    }
}

#[test]
fn every_way_to_name_a_picture_goes_through_the_rule() {
    let seen = Arc::default();
    let policy = allowing(Arc::clone(&seen));
    for style in [
        r"background-image:\75rl(https://track.example/esc.png)",
        "background-image: image-set('https://track.example/set.png' 1x)",
        "background-image: -webkit-image-set(url(https://track.example/w.png) 1x)",
        "background: cross-fade(url(https://ok.example/a.png), url(https://track.example/b.png), 50%)",
        "background-image: URL( 'https://track.example/upper.png' )",
    ] {
        let out = clean(&format!(r#"<div style="{style}">x</div>"#), &policy).html;
        assert!(!out.contains("track.example"), "{style} survived: {out}");
    }
}

#[test]
fn values_keep_their_strings_and_data_pictures() {
    let png = "data:image/png;base64,iVBORw0KGgo=";
    let out = cleaned(&format!(
        r#"<div style="background-image: url({png}); color: red">x</div><q style="quotes: 'a;b' 'c'">y</q>"#
    ));
    assert!(out.contains(png), "the data: picture was cut: {out}");
    assert!(out.contains("color: red"), "{out}");
    assert!(out.contains("a;b"), "a string was cut at its ';': {out}");
}

#[test]
fn citations_and_tooltips_carry_nothing() {
    let out = cleaned(
        r#"<q cite="javascript:alert(1)">a</q><blockquote cite="data:text/html,x">b</blockquote>
           <span title="Your account is verified">c</span>"#,
    );
    let lower = out.to_ascii_lowercase();
    assert!(!lower.contains("javascript:"), "{out}");
    assert!(!lower.contains("data:text"), "{out}");
    assert!(
        !lower.contains("title="),
        "a misleading tooltip survived: {out}"
    );
}

#[test]
fn absurd_sizes_in_any_unit_or_calc_are_left_out() {
    // The second review: each passed the first limits.
    for style in [
        "width: calc(1px * 1e9)",
        "margin-left: calc(-1px * 1e10)",
        "font-size: 1000pt",
        "font-size: 900em",
        "line-height: 99999",
        "font-size: 10000%",
    ] {
        let out = cleaned(&format!(r#"<div style="{style}; color: red">a</div>"#));
        assert!(
            !out.contains(style.split(':').next().unwrap()),
            "{style}: {out}"
        );
        assert!(out.contains("color: red"), "{out}");
    }
    let ok = cleaned(
        r#"<div style="font-size: 12pt; line-height: 1.4; width: calc(100% - 20px)">a</div>"#,
    );
    assert!(
        ok.contains("12pt") && ok.contains("1.4") && ok.contains("calc(100% - 20px)"),
        "{ok}"
    );
}

#[test]
fn absurd_sizes_are_left_out() {
    let out = cleaned(
        r#"<div style="font-size: 999999px; color: red">a</div><div style="width: 99999999px">b</div>"#,
    );
    assert!(!out.contains("999999"), "{out}");
    assert!(out.contains("color: red"), "{out}");
}

#[test]
fn deep_nesting_is_cut_short_not_a_crash() {
    // ~16k nested <div>s overflowed css-inline's recursive serializer and
    // aborted the process; 8k took 0.16 s.
    let depth = 16_000;
    let doc = format!(
        "<style>p {{ color: red }}</style>{}<p>deep</p>{}",
        "<div>".repeat(depth),
        "</div>".repeat(depth)
    );
    let t = std::time::Instant::now();
    let out = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || clean(&doc, &Policy::default()).html)
        .unwrap()
        .join()
        .expect("cleaning did not crash");
    assert!(
        t.elapsed().as_secs_f32() < 2.0 * SLOW,
        "took {:?}",
        t.elapsed()
    );
    assert!(out.len() < 200_000, "{} bytes", out.len());
}

#[test]
fn details_stay_open_or_closed_as_written() {
    let out = cleaned("<details open><summary>More</summary>inside</details>");
    assert!(out.contains("<details open"), "{out}");
    assert!(out.contains("<summary>More</summary>"), "{out}");
}

#[test]
fn nesting_the_parser_builds_is_cut_short_too() {
    // The second review: each nests deeper than its tags say (a `<div/>`
    // opens a div; an end tag past a cell boundary is ignored; unclosed
    // formatting elements are reopened), and overflowed an 8 MB stack.
    for (unit, n) in [
        ("<div/>", 16_000),
        ("<span><table><tbody><tr><td></span>", 2_000),
        ("<div><table><tr><td></div>", 2_000),
        ("<b><i></b>", 16_000),
    ] {
        let doc = format!("<style>p {{ color: red }}</style>{}", unit.repeat(n));
        let t = std::time::Instant::now();
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(move || clean(&doc, &Policy::default()).html.len())
            .unwrap()
            .join()
            .unwrap_or_else(|_| panic!("{unit} crashed the cleaner"));
        eprintln!("{unit} x{n}: {:?}", t.elapsed());
    }
}

#[test]
fn a_document_cannot_claim_or_fake_rinchs_own_attributes() {
    let out = cleaned(
        r#"<div data-claim-links data-rid="1" data-x="y"><a href="https://a.example/">a</a></div>"#,
    );
    assert!(!out.contains("data-"), "{out}");
}

#[test]
fn deep_media_blocks_and_many_raw_text_elements_clean_in_linear_time() {
    // The second review: 32k nested @media took 10 s; 128k <title>s 3.5 s and
    // 128k <textarea>s 7.3 s (a lowercase of the rest of the document each).
    let media = format!(
        "<style>{}.a{{color:red}}{}</style><p class=a>x</p>",
        "@media screen{".repeat(32_000),
        "}".repeat(32_000)
    );
    for doc in [
        media,
        "<title>t</title>".repeat(128_000),
        "<textarea>".repeat(128_000),
    ] {
        let t = std::time::Instant::now();
        let _ = rinch_html::clean_at(&doc, &Policy::default(), 600.0);
        assert!(
            t.elapsed().as_secs_f32() < 3.0 * SLOW,
            "took {:?}",
            t.elapsed()
        );
    }
}
