//! Assertions about `RinchActivity.java`'s *source text*.
//!
//! Unusual, and deliberately so. `RinchActivity.java` is compiled only when an
//! APK is built, and **nothing in this repository builds an APK in CI** — a
//! deliberate syntax error in Android-only code passes `cargo check` for the
//! Android target, `javac` is never invoked, and all host tests still pass
//! (issue #516). Every Rust caller of these methods is behind
//! `#[cfg(target_os = "android")]`, so there is no host-reachable seam to test
//! through either.
//!
//! That leaves the source text as the only thing an automated check can reach.
//! These tests are therefore a narrow guard, not a behaviour test: they cannot
//! tell you the code *works*, only that a specific, load-bearing, easy-to-
//! "simplify" detail has not been silently dropped. Each one exists because
//! losing that detail is silent at every other layer — no compile error, no
//! test failure, no crash on device, just wrong data.
//!
//! Keep them few. A source-text assertion that merely restates the code is
//! noise; one that pins a decision the code cannot express is worth its weight.

/// The Java source, read at compile time so a moved or renamed file is a build
/// error rather than a skipped test.
const ACTIVITY_JAVA: &str = include_str!("../java/com/rinch/RinchActivity.java");

/// Extract the body of a method by name, from its signature to the first line
/// that closes it at method indentation.
fn method_body(name: &str) -> String {
    let sig = format!(" {name}(");
    let start = ACTIVITY_JAVA
        .find(&sig)
        .unwrap_or_else(|| panic!("no method named `{name}` in RinchActivity.java"));
    let rest = &ACTIVITY_JAVA[start..];
    let end = rest
        .find("\n    }\n")
        .unwrap_or_else(|| panic!("could not find the end of `{name}`"));
    rest[..end].to_string()
}

/// `writeContentUri` must open the document in a **truncating** mode.
///
/// The whole defect this guards is one character. `openOutputStream(uri)` —
/// the one-argument form — uses mode `"w"`, and truncation under `"w"` is
/// explicitly undefined: the platform javadoc says "`w` may or may not
/// truncate", and the framework's own `FileUtils.translateModeStringToPosix`
/// maps a `"w"`-leading mode to `O_WRONLY | O_CREAT`, adding `O_TRUNC` only
/// when the string contains `'t'`. Saving 6KB over an existing 10KB document
/// therefore leaves the last 4KB of the old file in place — and returns
/// success.
///
/// Nothing else can catch that. Both forms compile, both run, both report
/// success, and the corruption only appears in a file the user opens later.
#[test]
fn write_content_uri_opens_a_truncating_stream() {
    let body = method_body("writeContentUri");

    let open = body
        .find("openOutputStream(")
        .map(|i| &body[i..])
        .and_then(|s| s.find(')').map(|j| &s[..=j]))
        .expect("writeContentUri must call openOutputStream");

    let mode_start = open.find(',').unwrap_or_else(|| {
        panic!(
            "writeContentUri calls the one-argument openOutputStream: `{open}`. \
             That is mode \"w\", whose truncation is provider-defined — pass an \
             explicit truncating mode instead"
        )
    });
    let mode = open[mode_start + 1..open.len() - 1]
        .trim()
        .trim_matches('"');

    assert!(
        mode.contains('t'),
        "writeContentUri opens with mode {mode:?}, which does not request \
         truncation (`t` = O_TRUNC). A shorter write will leave the tail of the \
         previous contents behind and still report success."
    );
    assert!(
        mode.starts_with('w') || mode.starts_with("rw"),
        "writeContentUri opens with mode {mode:?}, which is not a writing mode"
    );
}

/// Both content-URI streams must be closed on every path.
///
/// A write that throws with the stream still open leaves buffered bytes
/// unwritten and can leave the provider's file locked or half-written; the
/// reader leaks the descriptor. `try (...)` is the only construct here that
/// closes on the exception path — a bare `os.close()` as the last statement of
/// a `try` block does not run when `write` throws, which is exactly how both of
/// these were first written.
#[test]
fn content_uri_streams_are_closed_on_every_path() {
    for name in ["writeContentUri", "readContentUri"] {
        let body = method_body(name);
        assert!(
            body.contains("try (") || body.contains("try("),
            "`{name}` does not use try-with-resources, so its stream stays open \
             when the read/write throws"
        );
    }
}
