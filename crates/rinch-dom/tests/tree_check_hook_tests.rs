//! #584 — the post-layout sweep **fails** now, and this file is what proves it
//! fires.
//!
//! `RINCH_TREE_CHECK=1` has run the validators after every `resolve_layout`
//! since #476, and reported what it found with `eprintln!`. Two things
//! compounded: an `eprintln!` fails nothing, and `cargo test` captures stderr
//! for every test that *passes* — which was all of them, precisely because
//! nothing failed. Measured while wiring #578: a deliberately injected violation
//! printed **149** times under `-- --nocapture` and **0** times without it. So
//! the flag ran the sweep, found whatever was there, and showed it to nobody,
//! for its whole life — including the #476/#477 work it was built for, and
//! including the sixteen lines #597's defect was hiding inside.
//!
//! This is an issue *about a silent instrument*, so the fixtures here are held
//! to the rule that applies to one: **a zero needs a positive control.** Four of
//! them re-run a deliberately-broken fixture as a child process and read its
//! exit status, because that is the only way to observe what the harness does to
//! a violation rather than what the validator says about one:
//!
//! | fixture | child | asserts |
//! |---|---|---|
//! | `a_fatal_violation_fails_the_test_without_nocapture` | `=1`, capture on | child **fails**, and the reason survives capture |
//! | `warn_mode_keeps_the_old_print_only_behaviour` | `=warn` | child passes |
//! | `an_unset_flag_leaves_the_sweep_off` | unset | child passes |
//! | `the_broken_fixture_is_only_broken_because_of_the_corruption` | `=1`, uncorrupted | child passes |
//!
//! The last is the one that keeps the others honest: without it, a child that
//! failed for some unrelated reason would read exactly like a sweep that caught
//! something.
//!
//! The remaining fixtures pin that **every class is fatal** — see
//! [`rinch_dom::RinchDocument::tree_check_verdict`]. The verdict used to carry a
//! *waived* arm for one open defect's `D detached` lines (#513's, then #591's),
//! and this file carried its retirement witness — a fixture asserting the waiver
//! still waived something, built to fail the moment it did not. It fired twice,
//! on #513's fix (a rename) and on #591's (the deletion), and is gone with the
//! arm; `the_shape_the_waiver_retired_on_is_clean` is what stands where it
//! stood. If a waived arm ever comes back, bring its witness back with it.
//!
//! # Mutation-tested, and the two survivors are named
//!
//! Eleven mutants were run against this file (`report-584.md`). Nine are killed.
//! The two survivors are recorded rather than left to be rediscovered:
//!
//! * **waiving `C` as well as `D` survived**, and was *equivalent* — the waiver
//!   listed a node only when its Taffy chain terminated at a different node,
//!   i.e. when `C` did not report it. Moot since the arm retired with #591;
//!   recorded because it is the shape of survivor a future waiver would have too.
//! * **`true ||`-ing out `run_child`'s "the child ran a test" guard survives**,
//!   because that guard catches a *bad filter*, and blanking the guard is not a
//!   bad filter. Breaking `CHILD` instead — the thing it exists for — is caught,
//!   by the guard, in four fixtures at once.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

/// The name of the child fixture the four subprocess tests re-run.
///
/// It is an integration test at the crate root, so `--exact` needs no module
/// prefix. That matters: a filter missing its prefix matches nothing, runs no
/// test, and prints `0 passed` — which reads as "the sweep found nothing" and
/// means "the sweep did not run". It cost a whole attribution pass during this
/// issue's own triage, so `run_child` asserts a test actually ran.
const CHILD: &str = "a_document_whose_taffy_tree_is_corrupt_on_purpose";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn taffy_id(doc: &RinchDocument, id: NodeId) -> taffy::NodeId {
    doc.tree
        .get(id.0)
        .and_then(|n| n.taffy_id)
        .unwrap_or_else(|| panic!("node {id:?} has no Taffy node"))
}

/// `<div><a>text<div style="position:absolute">…</div></a></div>` — the shape
/// the waived arm retired on (#591), returned with the absolute's id.
///
/// The `<a>` is inline, so the IFC detaches its Taffy node and Parley lays its
/// content out; the out-of-flow child used to stay in the `<a>`'s Taffy child list
/// and go with it — every edge above it intact and nothing computing any of them,
/// which is what `D` reports (#589) and what the waiver waived. The IFC root loop
/// now collects that box into the root's own Taffy list, so the tree is clean.
///
/// Before #591 it was `<div><a>text<div>block</div></a></div>` — #513's shape,
/// the waiver's subject for its whole life until #513's fix split the inline
/// around the block. The predicate never changed between the two: it was
/// structural (a `D detached` node whose Taffy chain terminates at one of its own
/// DOM ancestors that the IFC detached on purpose), which is why #513's fix
/// retargeted it as a rename and #591's deleted it.
fn out_of_flow_in_inline() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = el(
        &mut doc,
        body,
        "div",
        "position: relative; width: 400px; line-height: 20px",
    );
    let inline = el(&mut doc, container, "a", "");
    txt(&mut doc, inline, "before");
    let abs = el(
        &mut doc,
        inline,
        "div",
        "position: absolute; top: 0; left: 0; width: 30px; height: 30px",
    );
    txt(&mut doc, abs, "out of flow");
    doc.resolve_layout(VW, VH);
    (doc, abs)
}

// ---------------------------------------------------------------------------
// The waiver
// ---------------------------------------------------------------------------

/// **Where the waiver's retirement witness stood.** That fixture asserted the
/// waived arm still waived something, so that a fixed defect's exemption could
/// not stay on the books and pass the next `D` of the same shape in for free.
/// It fired on #513's fix and again on #591's, and is gone with the arm.
///
/// What stands here is the positive form: the shape it waived is clean — the
/// absolute is held by the IFC root's Taffy list, reachable, and no class reports
/// anything — and there is no `waived` field left for a blanket exemption to hide
/// in. `a_detachment_under_a_contents_node_is_fatal` and
/// `a_taffy_edge_moved_under_an_unrelated_inline_is_fatal` below keep pinning that
/// a `D` of any other shape fails, as they did when the arm existed.
#[test]
fn the_shape_the_waiver_retired_on_is_clean() {
    let (doc, abs) = out_of_flow_in_inline();
    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.is_empty(),
        "an out-of-flow child of an inline is laid out by its IFC root now (#591):\n  {}",
        verdict.fatal.join("\n  ")
    );
    let t = taffy_id(&doc, abs);
    let parent = doc
        .tree
        .taffy
        .parent(t)
        .expect("the absolute has a Taffy parent");
    assert_eq!(
        doc.tree.taffy_map.get(&parent).copied(),
        Some(NodeId(3).0),
        "…and it is the IFC root — the container — not the detached <a>"
    );
}

/// A DOM-tree violation is fatal, with no waiver anywhere near it.
///
/// `tree_check_verdict` folds [`rinch_dom::RinchDocument::dom_tree_violations`]
/// straight into `fatal`, which is one line of code and therefore one line
/// nothing was testing: dropping it left every fixture in this file green
/// (measured as mutant M8). That check is the one the sweep most needs — the
/// Taffy validator compares the Taffy tree against itself, so it is structurally
/// blind to DOM corruption, and it reported all-clear through both of #566's
/// reconciler failures.
///
/// The corruption is #566's own, from
/// `anon_box_out_of_tree_tests::the_dom_tree_invariant_catches_a_one_way_link`:
/// a child stops pointing at its parent while the parent still lists it.
#[test]
fn a_dom_tree_violation_is_fatal() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "width: 400px");
    let child = el(&mut doc, col, "div", "height: 10px");
    doc.resolve_layout(VW, VH);
    assert!(doc.tree_check_verdict().fatal.is_empty(), "precondition");

    doc.tree.nodes[child.0].parent = None;

    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.iter().any(|l| l.starts_with("A one-way")),
        "a one-way parent link must be fatal:\n  {}",
        verdict.fatal.join("\n  ")
    );
}

/// A `D detached` under a `display: contents` wrapper is fatal.
///
/// While the verdict had a waived arm this was its narrowness pin — the shape a
/// blanket "any `D`" exemption would have swallowed. The arm is gone (#591) and
/// the fixture stays, because `D` is the rule #589 added to catch a whole branch
/// laid out by nobody, and it is the class most likely to carry the next real
/// defect; this is what says it still fires on the shape that motivated it.
///
/// The shape is `taffy_reachability_tests`' — a chain hanging off a
/// `display: contents` wrapper, whose Taffy node is detached by
/// `sync_display_contents` and whose children are hoisted. Put a child back
/// under it and every edge is valid, the wrapper is exempt from `C`, and nothing
/// computes the branch.
#[test]
fn a_detachment_under_a_contents_node_is_fatal() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "width: 400px");
    let wrap = el(&mut doc, col, "div", "display: contents");
    let blk = el(&mut doc, wrap, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);
    let clean = doc.tree_check_verdict();
    assert!(
        clean.fatal.is_empty(),
        "precondition: a flattened contents wrapper is clean:\n  {}",
        clean.fatal.join("\n  ")
    );

    let blk_t = taffy_id(&doc, blk);
    let wrap_t = taffy_id(&doc, wrap);
    if let Some(p) = doc.tree.taffy.parent(blk_t) {
        doc.tree.taffy.remove_child(p, blk_t).unwrap();
    }
    doc.tree.taffy.add_child(wrap_t, blk_t).unwrap();

    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.iter().any(|l| l.starts_with("D detached")),
        "it must be fatal:\n  {}",
        verdict.fatal.join("\n  ")
    );
}

/// A Taffy edge *moved* under an unrelated IFC-detached inline is fatal.
///
/// This was the waived arm's second-condition pin: the waiver asked of the
/// chain's terminus that it be IFC-detached inline content *and* the reported
/// node's own DOM ancestor, and this shape satisfies the first and breaks the
/// second. The arm is gone (#591); the fixture stays as the `D` rule's own pin
/// on the `reparent_taffy` shape #589 added it for — a chain that ends at a
/// detached inline which is nobody's ancestor is laid out by nobody.
#[test]
fn a_taffy_edge_moved_under_an_unrelated_inline_is_fatal() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // An IFC with a detached inline, to be the terminus.
    let para = el(&mut doc, body, "div", "width: 400px; line-height: 20px");
    txt(&mut doc, para, "text ");
    let inline = el(&mut doc, para, "a", "");
    txt(&mut doc, inline, "link");
    // A block-level box elsewhere in the document, no relation to that inline.
    let col = el(&mut doc, body, "div", "width: 400px");
    let stray = el(&mut doc, col, "div", "height: 30px");
    doc.resolve_layout(VW, VH);
    let clean = doc.tree_check_verdict();
    assert!(
        clean.fatal.is_empty(),
        "precondition: the document is clean:\n  {}",
        clean.fatal.join("\n  ")
    );

    let stray_t = taffy_id(&doc, stray);
    let inline_t = taffy_id(&doc, inline);
    assert!(
        doc.tree.taffy.parent(inline_t).is_none(),
        "precondition: the inline's Taffy node is detached into the IFC"
    );
    if let Some(p) = doc.tree.taffy.parent(stray_t) {
        doc.tree.taffy.remove_child(p, stray_t).unwrap();
    }
    doc.tree.taffy.add_child(inline_t, stray_t).unwrap();

    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.iter().any(|l| l.starts_with("D detached")),
        "it must be fatal:\n  {}",
        verdict.fatal.join("\n  ")
    );
}

/// A `C orphan` — the trivial detachment — is fatal, and `C` is the class #584's
/// own sixteen lines were (fixed in #597, so the suite reports none today).
#[test]
fn an_orphan_is_fatal() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "width: 400px");
    let blk = el(&mut doc, col, "div", "height: 30px");
    doc.resolve_layout(VW, VH);
    assert!(doc.tree_check_verdict().fatal.is_empty(), "precondition");

    let t = taffy_id(&doc, blk);
    let p = doc.tree.taffy.parent(t).expect("attached");
    doc.tree.taffy.remove_child(p, t).unwrap();

    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.iter().any(|l| l.starts_with("C orphan")),
        "it must be fatal:\n  {}",
        verdict.fatal.join("\n  ")
    );
}

/// An ordinary document produces neither. The rules' quietness is what makes a
/// failure mean something, and this is the only fixture here that asserts it
/// without corrupting anything first.
#[test]
fn an_ordinary_document_is_clean_on_both_counts() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "display: flex; width: 400px");
    for _ in 0..3 {
        let cell = el(&mut doc, col, "div", "flex: 1; padding: 4px");
        txt(&mut doc, cell, "cell");
    }
    let hidden = el(&mut doc, body, "div", "display: none");
    txt(&mut doc, hidden, "invisible");
    let contents = el(&mut doc, body, "div", "display: contents");
    let inner = el(&mut doc, contents, "p", "");
    txt(&mut doc, inner, "flattened");
    doc.resolve_layout(VW, VH);

    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.is_empty(),
        "flex, `display: none` and `display: contents` are all ordinary:\n  {}",
        verdict.fatal.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// The sweep itself, observed from outside the process
// ---------------------------------------------------------------------------

/// The child fixture the four subprocess tests re-run. **It is supposed to
/// panic** when `RINCH_TREE_CHECK` asks for the failing mode, so it is
/// `#[ignore]`d and never contributes to an ordinary run.
///
/// Two things about it are load-bearing and neither is obvious.
///
/// **The second pass must be dirty.** `resolve_layout` returns early on
/// `!layout_dirty`, before the sweep, so a no-op pass sweeps nothing — measured
/// while writing this file, and it is why the second call changes the viewport
/// rather than repeating the first. A frame that computes no layout also leaves
/// no new violation, so the early return costs the sweep nothing; a fixture that
/// did not know about it would have a child that silently never ran the check,
/// which is this issue's own failure mode in miniature.
///
/// **The corruption has to survive that pass.** Removing a Taffy child edge by
/// hand does: only a structural DOM mutation rebuilds a parent's Taffy child
/// list, and nothing here mutates the DOM after the removal. Measured, because
/// it is not true of every corruption — re-parenting a box under a
/// `display: contents` wrapper's Taffy node *is* repaired by a dirty pass, so
/// that shape would make a useless child. And it is *asserted*, not assumed:
/// under `warn` and with the flag unset the sweep does not stop the process, so
/// the child checks the violation is still there and fails if it is not. Without
/// that, a future repair would turn the child green and be indistinguishable
/// from a sweep that stopped firing.
///
/// `RINCH_TREE_CHECK_FIXTURE=clean` skips the corruption, which is how
/// `the_broken_fixture_is_only_broken_because_of_the_corruption` shows the child
/// has no other way to fail.
#[test]
#[ignore = "child process fixture: panics on purpose under RINCH_TREE_CHECK"]
fn a_document_whose_taffy_tree_is_corrupt_on_purpose() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "width: 400px");
    let blk = el(&mut doc, col, "div", "height: 30px");
    txt(&mut doc, blk, "content");
    doc.resolve_layout(VW, VH);

    let mode = std::env::var("RINCH_TREE_CHECK_FIXTURE").unwrap_or_default();
    let corrupt = mode != "clean";
    if corrupt {
        let t = taffy_id(&doc, blk);
        let p = doc.tree.taffy.parent(t).expect("attached");
        doc.tree.taffy.remove_child(p, t).unwrap();
        assert!(
            doc.tree_check_verdict()
                .fatal
                .iter()
                .any(|l| l.starts_with("C orphan")),
            "the corruption did not take"
        );
    }

    // A viewport change forces `layout_dirty`, so this pass reaches the sweep.
    // Under the failing mode it panics inside this call; under `warn` or with
    // the flag unset it returns.
    doc.resolve_layout(VW + 100.0, VH);

    if corrupt {
        assert!(
            doc.tree_check_verdict()
                .fatal
                .iter()
                .any(|l| l.starts_with("C orphan")),
            "the corruption no longer survives a dirty layout pass, so this \
             fixture can no longer exercise the sweep. Pick another violation \
             the engine does not repair — and note that three of the four \
             subprocess tests above are failing for this reason, not because the \
             sweep broke."
        );
    }
}

/// Run [`CHILD`] in a fresh process. `env` is applied on top of this process's
/// environment; a `None` value removes the variable, which matters because the
/// parent may itself be running under `RINCH_TREE_CHECK=1`.
///
/// The child keeps its harness's stderr capture. That is the whole claim under
/// test: a violation fails with the capture in place, so nobody has to remember
/// `--nocapture` — and libtest prints a *failing* test's captured output in its
/// summary, which is how the reason survives too. (A `--nocapture` variant
/// existed for the waived arm, whose child *passed* and whose printed line the
/// parent had to read; it went with the arm, #591.)
fn run_child(env: &[(&str, Option<&str>)]) -> std::process::Output {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = std::process::Command::new(exe);
    cmd.args([CHILD, "--exact", "--ignored", "--test-threads=1"]);
    for (k, v) in env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    let out = cmd.output().expect("spawn the child test binary");
    let all =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    // A filter that matches nothing prints `0 passed` and exits 0 — which reads
    // exactly like a clean sweep. Assert the child ran a test at all.
    assert!(
        all.contains("1 passed") || all.contains("1 failed"),
        "the child ran no test — check the `--exact` filter against `CHILD`:\n{all}"
    );
    out
}

/// **The positive control, and the whole of #584's tooling half.** A violation
/// fails the test it happened in, with stderr captured.
#[test]
fn a_fatal_violation_fails_the_test_without_nocapture() {
    let out = run_child(&[
        ("RINCH_TREE_CHECK", Some("1")),
        ("RINCH_TREE_CHECK_FIXTURE", None),
    ]);
    let all =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "the child must fail — this is the assertion that would have caught \
         #597's sixteen lines, and #476's before them:\n{all}"
    );
    // The reason has to survive capture too. A failure whose cause is only in
    // swallowed stderr is the same problem one step along.
    assert!(
        all.contains("RINCH_TREE_CHECK") && all.contains("C orphan"),
        "the failure must name the violation, not merely fail:\n{all}"
    );
}

/// `RINCH_TREE_CHECK=warn` keeps the pre-#584 behaviour: print and carry on.
///
/// That mode exists for driving a real app under the flag, where a panic inside
/// a frame is less use than a window that keeps running and complains. It is
/// asserted here so the escape hatch cannot rot into the default by accident.
#[test]
fn warn_mode_keeps_the_old_print_only_behaviour() {
    let out = run_child(&[
        ("RINCH_TREE_CHECK", Some("warn")),
        ("RINCH_TREE_CHECK_FIXTURE", None),
    ]);
    let all =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "warn must not fail:\n{all}");
}

/// With the flag unset the sweep does not run, so the same corrupt document is
/// silently fine. This is the gate, asserted rather than assumed — and it is
/// also what makes the previous test's failure attributable to the flag.
#[test]
fn an_unset_flag_leaves_the_sweep_off() {
    let out = run_child(&[
        ("RINCH_TREE_CHECK", None),
        ("RINCH_TREE_CHECK_FIXTURE", None),
    ]);
    let all =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "an unset flag must change nothing:\n{all}"
    );
}

/// The child has no way to fail other than the corruption.
///
/// Without this, a child that failed for an unrelated reason — a panic in the
/// fixture, a missing font, a changed API — would read exactly like a sweep that
/// caught something, and `a_fatal_violation_fails_the_test_without_nocapture`
/// would go green for the wrong reason and stay there.
#[test]
fn the_broken_fixture_is_only_broken_because_of_the_corruption() {
    let out = run_child(&[
        ("RINCH_TREE_CHECK", Some("1")),
        ("RINCH_TREE_CHECK_FIXTURE", Some("clean")),
    ]);
    let all =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the same fixture, same flag, no corruption — it must pass:\n{all}"
    );
}
