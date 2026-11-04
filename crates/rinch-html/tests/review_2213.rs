//! The first review of #2213: ways past the cleaner, each as it was found.

use std::sync::Arc;

use rinch_html::{Policy, clean};

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
    assert!(t.elapsed().as_secs_f32() < 2.0, "took {:?}", t.elapsed());
    assert!(out.len() < 200_000, "{} bytes", out.len());
}

#[test]
fn details_stay_open_or_closed_as_written() {
    let out = cleaned("<details open><summary>More</summary>inside</details>");
    assert!(out.contains("<details open"), "{out}");
    assert!(out.contains("<summary>More</summary>"), "{out}");
}
