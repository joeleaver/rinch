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

/// Extract the body of a method by name, from its **declaration** to the first
/// line that closes it at method indentation.
///
/// The first ` name(` in the file is not necessarily the declaration, and
/// taking it anyway is a silent way to test the wrong text: `queueIntent` is
/// called from `onCreate` some six hundred lines above the `private void` that
/// declares it, so a plain `find` returns a slice of *onCreate* and every
/// assertion about `queueIntent` then passes or fails for unrelated reasons.
/// The declaration is the first occurrence whose own line begins with an access
/// modifier.
fn method_body(name: &str) -> String {
    let sig = format!(" {name}(");
    let start = ACTIVITY_JAVA
        .match_indices(&sig)
        .map(|(at, _)| at)
        .find(|&at| {
            let line_start = ACTIVITY_JAVA[..at].rfind('\n').map_or(0, |i| i + 1);
            let before = ACTIVITY_JAVA[line_start..at].trim_start();
            before.starts_with("private")
                || before.starts_with("public")
                || before.starts_with("protected")
        })
        .unwrap_or_else(|| panic!("no method *declared* `{name}` in RinchActivity.java"));
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

/// `getImeInset` must answer `-1`, not `0`, when there are no insets yet.
///
/// The Rust side decodes `-1` as `None` ("ask again") and `0` as `Some(0)`
/// ("the keyboard is down"), and those are opposite instructions to a caller
/// that moves its layout with the keyboard: the first says leave it alone,
/// the second says put it back. A window that has not been laid out yet is an
/// ordinary moment in an activity's life — it happens on the way into every
/// screen — so this path is taken constantly and is not an error case.
///
/// Collapsing the two is the obvious "simplification" (both are falsy-looking
/// ints, and `return 0` reads tidier than a sentinel), and nothing else would
/// notice: it compiles, it runs, the keyboard still works, and the only
/// symptom is a footer that flinches to its closed position for one frame on
/// the way into a screen.
#[test]
fn ime_inset_reports_no_insets_yet_as_a_sentinel_and_not_as_zero() {
    let body = method_body("getImeInset");

    assert!(
        body.contains("return -1;"),
        "getImeInset must return -1 when getRootWindowInsets() is null, so the \
         Rust side can tell \"no insets yet\" from \"the keyboard is down\"; \
         body was:\n{body}"
    );
}

/// Both display-metrics methods must keep their `SDK_INT` guard **and keep the
/// API 30 call behind it**.
///
/// `WindowInsets.Type.ime()` and `getCurrentWindowMetrics()` are both API 30.
/// This crate's consumers build with `--min-api 28`, and that is the whole
/// trap: an unguarded call to either compiles without complaint, passes every
/// check in this repository (nothing here runs `javac`, issue #516), installs
/// on an API 28 handset, and throws `NoSuchMethodError` the first time the
/// screen it is on opens. The guard is the only thing standing between that
/// and a crash nobody can reproduce on a modern phone.
///
/// **The ordering assertion is the load-bearing half, and its absence was a
/// real hole.** An earlier version of this test asserted only that the guard's
/// text appeared somewhere in the body. Hoisting the API 30 call *above* a
/// guard that is still there — a plausible refactor, since the value is wanted
/// in both branches often enough — left that test green while restoring the
/// exact `NoSuchMethodError` this docstring is about. Checked by doing it: the
/// mutant passed. Pinning presence pins nothing; the guard has to come first.
#[test]
fn the_api_30_display_calls_stay_behind_a_version_guard() {
    const GUARD: &str = "Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R";

    for (method, api_30_call) in [
        ("getViewportSize", "getCurrentWindowMetrics("),
        ("getImeInset", "android.view.WindowInsets.Type.ime()"),
    ] {
        let body = method_body(method);

        let guard = body.find(GUARD).unwrap_or_else(|| {
            panic!(
                "{method} calls an API 30 method and must keep its SDK_INT guard — \
                 without it this throws on any API 28-29 device, and nothing in \
                 this repository would catch it; body was:\n{body}"
            )
        });

        // Every occurrence, not the first: a second, unguarded call site is the
        // same crash as a hoisted one.
        let sites: Vec<usize> = body.match_indices(api_30_call).map(|(at, _)| at).collect();
        assert!(
            !sites.is_empty(),
            "{method} is supposed to call `{api_30_call}` on API 30+, and no \
             longer does — this test is now pinning nothing, so either restore \
             the call or retire the entry; body was:\n{body}"
        );
        for at in sites {
            assert!(
                at > guard,
                "{method} reaches `{api_30_call}` at byte {at}, before its \
                 SDK_INT guard at byte {guard}. The guard is present but the \
                 API 30 call runs whatever the version is, which is a \
                 `NoSuchMethodError` on an API 28-29 handset and green \
                 everywhere in this repository; body was:\n{body}"
            );
        }
    }
}

// ── Incoming intents (#575) ──────────────────────────────────────────────
//
// Four invariants that exist today only as prose in the Java. Each is stated in
// its own comment there, each is silent at every other layer — no compile
// error, no test failure, and for two of them no crash on a device that is
// already warm — and prose is the thing that rots.

/// Every `native` method the activity declares, by name.
///
/// Derived rather than listed, so a native added later is covered without
/// anyone remembering to come back here.
fn declared_native_methods() -> Vec<String> {
    ACTIVITY_JAVA
        .lines()
        .filter(|l| {
            l.trim_start()
                .starts_with(|c: char| c.is_ascii_alphabetic())
        })
        .filter(|l| l.contains(" native "))
        .filter_map(|l| {
            let open = l.find('(')?;
            let name = l[..open].rsplit(|c: char| c.is_whitespace()).next()?;
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// Rewrite `src` with every comment and string literal blanked to spaces,
/// **preserving byte offsets** so positions found in the result are positions
/// in the original.
///
/// Needed because the assertions below are about where an identifier *is*, and
/// a javadoc that names it, or a string literal that happens to contain a
/// brace, would otherwise be indistinguishable from code.
fn code_only(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = vec![b' '; b.len()];
    let (mut i, mut state) = (0usize, 0u8); // 0 code, 1 line, 2 block, 3 string, 4 char
    while i < b.len() {
        let two = if i + 1 < b.len() {
            &b[i..i + 2]
        } else {
            &b[i..]
        };
        match state {
            0 => {
                if two == b"//" {
                    state = 1;
                } else if two == b"/*" {
                    state = 2;
                } else {
                    if b[i] == b'"' {
                        state = 3;
                    } else if b[i] == b'\'' {
                        state = 4;
                    }
                    out[i] = b[i];
                }
            }
            1 => {
                if b[i] == b'\n' {
                    state = 0;
                    out[i] = b[i];
                }
            }
            2 => {
                if two == b"*/" {
                    out[i] = b' ';
                    i += 1;
                    state = 0;
                } else if b[i] == b'\n' {
                    out[i] = b[i];
                }
            }
            3 | 4 => {
                let quote = if state == 3 { b'"' } else { b'\'' };
                if b[i] == b'\\' {
                    i += 1;
                } else if b[i] == quote {
                    state = 0;
                    out[i] = b[i];
                }
            }
            _ => unreachable!(),
        }
        i += 1;
    }
    String::from_utf8(out).expect("blanking preserves ASCII structure")
}

/// Byte ranges covered by `synchronized (monitor) { … }` blocks, brace-matched.
fn synchronized_spans(code: &str, monitor: &str) -> Vec<(usize, usize)> {
    let needle = format!("synchronized ({monitor})");
    let bytes = code.as_bytes();
    let mut spans = Vec::new();
    for (at, _) in code.match_indices(&needle) {
        let Some(rel) = code[at..].find('{') else {
            continue;
        };
        let open = at + rel;
        let (mut depth, mut j) = (0i32, open);
        while j < bytes.len() {
            match bytes[j] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        spans.push((open, j));
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
    }
    spans
}

/// `onCreate` must hand nothing to a native.
///
/// On a cold start the native thread may not have reached `RegisterNatives`
/// when `onCreate` runs on the UI thread, and a native that is not registered
/// yet is an `UnsatisfiedLinkError` thrown out of a lifecycle override — which
/// force-finishes the activity. The app dies on launch; on a cold start; and
/// only on the machines where the race goes the wrong way.
///
/// **This is the entire reason the intent path is a queue rather than a call**,
/// and "store, do not deliver" is a rule no compiler enforces. Replacing
/// `queueIntent(getIntent(), …)` with a direct `nativeOnIncomingIntent(…)` is a
/// one-line simplification that reads better, compiles, passes every check in
/// this repository, and works on every *warm* start.
#[test]
fn on_create_hands_nothing_to_a_native() {
    let natives = declared_native_methods();
    // Anti-vacuity: if the extractor stops finding natives this test goes
    // quietly green while pinning nothing, which is the failure mode this
    // module is least able to notice.
    assert!(
        natives.len() >= 4,
        "expected the activity to declare several natives; the extractor found \
         {natives:?}, so this test is no longer checking anything"
    );

    let body = code_only(&method_body("onCreate"));
    for name in &natives {
        assert!(
            !body.contains(&format!("{name}(")),
            "onCreate calls `{name}(…)`. A native invoked from a lifecycle \
             override before `RegisterNatives` has run is an \
             UnsatisfiedLinkError that force-finishes the activity on a cold \
             start — store the work and let the native side flush it instead; \
             body was:\n{body}"
        );
    }
}

/// `onNewIntent` must call `setIntent(intent)` **before** it queues.
///
/// `getIntent()` answers with the intent the activity was *started* with until
/// `setIntent` says otherwise; the framework does not do it for you. Two things
/// then read the wrong object: anything that later calls `getIntent()`,
/// including a recreation of this activity, and — worse — the consumed marker,
/// which `queueIntent` puts on the intent it was handed. Mark the new intent
/// but leave `getIntent()` pointing at the old one and the stale-redelivery
/// guard is attached to the copy nobody asks about.
///
/// **An ordering invariant, and the same species as the SDK_INT one above**,
/// whose docstring records that pinning presence and not order was a real hole:
/// both calls being present is not the property, the order is.
#[test]
fn on_new_intent_adopts_the_intent_before_queueing_it() {
    let body = code_only(&method_body("onNewIntent"));

    let set = body.find("setIntent(").unwrap_or_else(|| {
        panic!(
            "onNewIntent must call setIntent(intent), or getIntent() keeps \
             answering with the launching intent; body was:\n{body}"
        )
    });
    let queue = body
        .find("queueIntent(")
        .unwrap_or_else(|| panic!("onNewIntent no longer queues the intent; body was:\n{body}"));

    assert!(
        set < queue,
        "onNewIntent queues at byte {queue} but adopts the intent at byte \
         {set}. The consumed marker is put on whatever queueIntent is handed, \
         so queueing first attaches it to an object getIntent() will not \
         return; body was:\n{body}"
    );
}

/// The consumed marker must be **read before it is written**.
///
/// `queueIntent` both tests `EXTRA_RINCH_CONSUMED` and sets it, and the order
/// is the whole mechanism rather than a style choice: setting first makes the
/// test that follows see this method's own mark, so the early return fires for
/// every intent and nothing is ever delivered. A share target that silently
/// receives nothing, on a path that compiles and runs.
///
/// The `recreated` flag is checked in the same breath and pinned with it — it
/// is the half that survives process death, where the marker does not.
#[test]
fn the_consumed_marker_is_read_before_it_is_written() {
    let body = code_only(&method_body("queueIntent"));

    let read = body
        .find("getBooleanExtra(EXTRA_RINCH_CONSUMED")
        .unwrap_or_else(|| {
            panic!(
                "queueIntent must test EXTRA_RINCH_CONSUMED, or an intent Android \
             hands back a second time inside one process is delivered twice — \
             for a share target, a duplicate import with no undo; body \
             was:\n{body}"
            )
        });
    let write = body
        .find("putExtra(EXTRA_RINCH_CONSUMED")
        .unwrap_or_else(|| {
            panic!(
                "queueIntent tests EXTRA_RINCH_CONSUMED but never sets it, so the \
             test can never be true; body was:\n{body}"
            )
        });
    assert!(
        read < write,
        "queueIntent sets the consumed marker at byte {write}, before it tests \
         it at byte {read}. Then every intent tests as already-consumed and \
         nothing is ever delivered; body was:\n{body}"
    );

    // **Past the signature.** `recreated` is also the parameter's name, so
    // searching the whole body finds the declaration and the assertion holds
    // whether or not anything consults it — measured: with the `recreated ||`
    // guard deleted, that version of this test stayed green.
    let after_signature = body.find('\n').map_or(0, |i| i + 1);
    let recreated = body[after_signature..]
        .find("recreated")
        .map(|i| i + after_signature)
        .unwrap_or_else(|| {
            panic!(
                "queueIntent takes `recreated` and never consults it. That is \
                 the only guard that survives process death — the extra goes on \
                 this process's copy and the system re-supplies its own after a \
                 restore, so without it a share is re-imported on every \
                 rotation; body was:\n{body}"
            )
        });
    assert!(
        recreated < write,
        "queueIntent marks the intent consumed before checking `recreated`; \
         body was:\n{body}"
    );
}

/// `nativeIntentsReady` must only be touched under `synchronized (pendingIntents)`.
///
/// The Java says so — *"Guarded by pendingIntents, because onNewIntent reads it
/// on the UI thread while flushPendingIntents writes it on the native thread"*
/// — and a stated lock with nothing checking it is exactly the shape that goes
/// stale. Drop either `synchronized` and the compiler is silent, both threads
/// keep working, and the cost is a torn read on a cold start: Java publishes the
/// write with no happens-before edge, the UI thread reads the stale `false`, and
/// the launch intent sits in the queue until some *later* intent arrives to
/// flush it. The share that started the app is simply never delivered, and only
/// sometimes.
///
/// Asserted positionally against brace-matched spans rather than by asking
/// whether the method "contains a synchronized" — which a block placed beside
/// the access, rather than around it, would satisfy.
#[test]
fn the_ready_flag_is_only_touched_under_the_queue_lock() {
    let code = code_only(ACTIVITY_JAVA);
    let spans = synchronized_spans(&code, "pendingIntents");
    assert!(
        spans.len() >= 2,
        "expected at least the read in queueIntent and the write in \
         flushPendingIntents to be synchronized on pendingIntents; found \
         {} such block(s)",
        spans.len()
    );

    let mut checked = 0;
    for (at, _) in code.match_indices("nativeIntentsReady") {
        let line_start = code[..at].rfind('\n').map_or(0, |i| i + 1);
        // The field's own declaration is not an access.
        if code[line_start..at].contains("boolean") {
            continue;
        }
        checked += 1;
        assert!(
            spans.iter().any(|&(open, close)| at > open && at < close),
            "nativeIntentsReady is touched at byte {at}, outside every \
             `synchronized (pendingIntents)` block. onNewIntent reads it on the \
             UI thread and flushPendingIntents writes it on the native thread, \
             so an unsynchronised read can see a stale `false` and strand the \
             launch intent in the queue"
        );
    }
    // Anti-vacuity again: a rename would otherwise leave this loop with nothing
    // to iterate and the test green.
    assert!(
        checked >= 2,
        "expected at least two accesses to nativeIntentsReady (the read in \
         queueIntent, the write in flushPendingIntents); found {checked}"
    );
}
