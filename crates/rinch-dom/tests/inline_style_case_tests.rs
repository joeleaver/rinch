//! #711 — an inline-style property name is ASCII case-insensitive, through
//! the real document.
//!
//! `rinch_core::dom::inline_style`'s own unit tests pin the parser. These pin
//! the thing a user sees: the value Stylo computes after the attribute has been
//! rewritten by a `set_style`, which is where a name compared the wrong way
//! stops being a spelling question and becomes a wrong colour on screen.
//!
//! Every answer asserted here was measured in Chrome 150 first, on the same
//! markup, via `data:`/`file:` + `getComputedStyle`.
//!
//! **Two of these fixtures sit deliberately off a fixed point**, because the
//! obvious spelling of each is one. A `set_style` of a property the attribute
//! already declares *appends* when the names fail to match, and an appended
//! declaration wins by position — so `style="COLOR: red"` +
//! `set_style("color", "green")` computes green whether or not the names were
//! ever compared, and says nothing. `!important` is what breaks the tie: it
//! wins from wherever it sits, so a stray second declaration of the same
//! property is visible in the computed value rather than hidden by it.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

const RED: &str = "#ff0000";
const GREEN: &str = "#00ff00";
const BLUE: &str = "#0000ff";
/// Not a colour anybody declares — the value a rejected `color` falls back to.
const BLACK: &str = "#000000";

fn hex(doc: &RinchDocument, n: NodeId) -> String {
    let c = doc.tree.get(n.0).unwrap().computed_style.color;
    let c = c.expect("color should resolve");
    let rgba = c.to_rgba8();
    format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b)
}

fn div(doc: &mut RinchDocument, style: &str) -> NodeId {
    let body = doc.body();
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(body, n);
    n
}

fn style_of(doc: &RinchDocument, n: NodeId) -> String {
    doc.get_attribute(n, "style").unwrap_or_default()
}

/// The issue's own case, end to end.
///
/// Chrome 150: `style="color: blue; COLOR: red"` computes red (one property,
/// last declaration wins), and `style.setProperty("color", "green")` then
/// leaves `cssText === "color: green;"` and computes green.
///
/// rinch computed **red**: `COLOR` was a second key, so the `set_style`
/// rewrote `color` in place and left `COLOR: red` sitting after it.
///
/// Kills the mutant `let name = part[..colon].trim();` — i.e. dropping the
/// `normalize_property_name` call from `split_declarations`, which is the
/// state this file was written against.
#[test]
fn two_spellings_of_one_property_are_one_property() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, &format!("color: {BLUE}; COLOR: {RED}"));
    doc.set_style(n, "color", GREEN);
    doc.resolve_layout(VW, VH);

    assert_eq!(hex(&doc, n), GREEN, "attribute = {:?}", style_of(&doc, n));
    assert_eq!(style_of(&doc, n), format!("color: {GREEN}"));
}

/// A `set_style` finds the property whichever end the difference is at. These
/// two are a pair, and it takes both: each pins one end of the comparison and
/// neither notices the other end going wrong.
///
/// **Off the fixed point**, and the `!important` is what puts them there. A
/// `set_style` of a property whose name fails to match *appends*, and an
/// appended declaration wins by position — so without the `!important` both
/// the working and the broken code compute green and the fixture says nothing
/// about names at all. With it, a second declaration left beside the first is
/// visible: Chrome's `setProperty` replaces an important declaration outright
/// (measured in Chrome 150, the block's `cssText` becomes `color: green;` with
/// the priority cleared), where a failed match leaves `red !important` in the
/// attribute for Stylo to cascade over the new value.
///
/// Lowercase caller, uppercase declaration: this is the **parse** end, and it
/// kills the mutant that drops `normalize_property_name` from
/// `split_declarations`.
#[test]
fn a_lowercase_caller_name_meets_an_uppercase_declaration() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, &format!("COLOR: {RED} !important"));
    doc.set_style(n, "color", GREEN);
    doc.resolve_layout(VW, VH);

    assert_eq!(hex(&doc, n), GREEN, "attribute = {:?}", style_of(&doc, n));
    assert_eq!(style_of(&doc, n), format!("color: {GREEN}"));
}

/// The other end — uppercase caller, lowercase declaration — is the **writer**,
/// and it is the only fixture here that kills the mutant dropping
/// `normalize_property_name` from `RinchDocument::merged_inline_style`. Its
/// twin above passes against that one, because the parse has already put the
/// attribute's name in the case the caller happened to use.
#[test]
fn an_uppercase_caller_name_meets_a_lowercase_declaration() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, &format!("color: {RED} !important"));
    doc.set_style(n, "COLOR", GREEN);
    doc.resolve_layout(VW, VH);

    assert_eq!(hex(&doc, n), GREEN, "attribute = {:?}", style_of(&doc, n));
    assert_eq!(style_of(&doc, n), format!("color: {GREEN}"));
}

/// Custom properties are case-**sensitive** and must stay two properties.
/// Chrome 150: `style="--Foo: 1px; --foo: 2px"` has `length === 2`.
///
/// **Off the fixed point**, and this is the whole reason for the declaration
/// order: `--foo` is declared *first*. Lowercasing a custom property collapses
/// the pair to the **last** one, `--Foo`'s red — so with the values the other
/// way round the mutant would compute green too and the fixture would be blind
/// to it.
///
/// Kills the mutant `name.to_ascii_lowercase()` unconditionally — i.e. dropping
/// the `starts_with("--")` arm of `normalize_property_name`.
#[test]
fn two_custom_properties_differing_only_in_case_stay_two() {
    let mut doc = RinchDocument::new();
    let n = div(
        &mut doc,
        &format!("--foo: {GREEN}; --Foo: {RED}; color: var(--foo)"),
    );
    // Force the attribute through the parser; nothing else here rewrites it.
    doc.set_style(n, "gap", "1px");
    doc.resolve_layout(VW, VH);

    assert_eq!(hex(&doc, n), GREEN, "attribute = {:?}", style_of(&doc, n));
    assert_eq!(
        style_of(&doc, n),
        format!("--foo: {GREEN}; --Foo: {RED}; color: var(--foo); gap: 1px")
    );
}

/// `set_style` on a custom property is case-sensitive at the caller's end too.
/// Chrome 150: `setProperty("--Foo", "3px")` on a block holding `--foo: 2px`
/// gives `cssText === "--foo: 2px; --Foo: 3px;"` and `length === 2`.
#[test]
fn set_style_does_not_fold_a_custom_property_onto_a_differently_cased_one() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, &format!("--foo: {GREEN}; color: var(--foo)"));
    doc.set_style(n, "--Foo", RED);
    doc.resolve_layout(VW, VH);

    assert_eq!(hex(&doc, n), GREEN, "attribute = {:?}", style_of(&doc, n));
    assert_eq!(
        style_of(&doc, n),
        format!("--foo: {GREEN}; color: var(--foo); --Foo: {RED}")
    );
}

/// An `!important` declaration is not replaced by a later plain one, whichever
/// case each is spelled in. Chrome 150 serialises
/// `COLOR: red !important; color: blue` as `color: red !important;` and
/// computes red.
///
/// This is what makes the case fix safe rather than a trade. Before it, the two
/// spellings were two keys, so *both* declarations reached Stylo and its own
/// cascade picked the important one — the right answer by accident. Folding
/// them into one key without carrying the priority would have turned this
/// green fixture red.
///
/// Kills the mutant `out.remove(at);` — the unconditional last-wins collapse,
/// i.e. dropping the `is_important` arm.
#[test]
fn an_important_declaration_survives_the_collapse_across_cases() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, &format!("COLOR: {RED} !important; color: {BLUE}"));
    doc.set_style(n, "gap", "1px");
    doc.resolve_layout(VW, VH);

    assert_eq!(hex(&doc, n), RED, "attribute = {:?}", style_of(&doc, n));
}

/// The same shape spelled in one case, which is the half that was **red at
/// HEAD**: two identical keys already collapsed, and the collapse threw the
/// `!important` away. `color: red !important; color: blue` plus any unrelated
/// `set_style` computed blue in rinch and red in Chrome.
///
/// A pre-existing #670 residue rather than a #711 symptom, repaired here
/// because #711's fix cannot be correct without it.
#[test]
fn an_important_declaration_survives_the_collapse_in_one_case() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, &format!("color: {RED} !important; color: {BLUE}"));
    doc.set_style(n, "gap", "1px");
    doc.resolve_layout(VW, VH);

    assert_eq!(hex(&doc, n), RED, "attribute = {:?}", style_of(&doc, n));
}

/// A reader that scans the attribute for one property by name sees the
/// normalised name. `user-select` is read this way in `style_resolution`
/// (Stylo's servo build does not carry the property), so `USER-SELECT: none`
/// used to resolve to the default `Auto` — nothing rewrites the attribute
/// here, so this is the raw parse, not the merge path.
#[test]
fn an_uppercase_user_select_reaches_the_resolver() {
    use rinch_dom::computed_style::UserSelectValue;
    let mut doc = RinchDocument::new();
    let upper = div(&mut doc, "USER-SELECT: none");
    let lower = div(&mut doc, "user-select: none");
    doc.resolve_layout(VW, VH);

    let of = |n: NodeId| doc.tree.get(n.0).unwrap().computed_style.user_select;
    assert!(
        matches!(of(lower), UserSelectValue::None),
        "positive control: the lowercase spelling must resolve"
    );
    assert!(matches!(of(upper), UserSelectValue::None));
}

/// What the attribute reads back as afterwards: the **CSSOM-touched** form,
/// lowercased and collapsed.
///
/// Chrome keeps the author's bytes in `getAttribute("style")` until something
/// touches the CSSOM — measured, a CSSOM *read* does not disturb it and a
/// CSSOM *write* rewrites it — where rinch re-serialises on every merge and so
/// reaches the touched form one write earlier. The divergence is in *when*,
/// not in what: both end at `margin-left: 7px; gap: 1px` for
/// `style="MARGIN-left: 7px"` plus a `gap` write, which is this fixture.
#[test]
fn the_attribute_reads_back_in_the_cssom_touched_form() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, "MARGIN-left: 7px");
    assert_eq!(
        style_of(&doc, n),
        "MARGIN-left: 7px",
        "untouched, the author's bytes are still there"
    );

    doc.set_style(n, "gap", "1px");
    doc.resolve_layout(VW, VH);
    assert_eq!(style_of(&doc, n), "margin-left: 7px; gap: 1px");
    assert_eq!(
        doc.tree
            .get(n.0)
            .unwrap()
            .computed_style
            .margin_left
            .to_px(),
        7.0,
        "and the uppercase spelling was a real margin all along — Stylo \
         parses the name case-insensitively, which is why this bug was \
         invisible until a second author wrote the same property"
    );
}

/// **A deviation, pinned rather than merely described — issue #722.** rinch collapses a
/// duplicated property **syntactically**; a browser collapses after validity,
/// because it drops an invalid declaration at parse and the duplicate then
/// never competes. So rinch's priority arm can preserve an `!important`
/// declaration Stylo will go on to reject, leaving the property with no value
/// at all, where Chrome 150 falls through to the later valid one — measured,
/// `color: notacolor !important; color: blue` and
/// `color: red !important !important; color: blue` both compute **blue** there.
///
/// The class is older than the priority arm, and the `older` node is what says
/// so: `color: blue; color: notacolor` computes black in rinch and blue in
/// Chrome, and did before this fix too, because two *plain* declarations
/// collapse exactly as they always did. What #711 changed is the reach — the
/// two important shapes were accidentally right under the old unconditional
/// collapse, which discarded the invalid declaration for the wrong reason.
///
/// Both shapes need invalid CSS to reach. This fixture exists so the caveat in
/// `split_declarations`' doc is a fact somebody can check, and so that whoever
/// closes **#722** has a fixture that flips rather than prose to re-derive.
/// When it does close, all three assertions below become `BLUE` and this doc
/// describes history — the `control` assertion is the only one that survives
/// unchanged, which is why it is here.
#[test]
fn a_duplicate_collapses_before_validity_not_after() {
    let mut doc = RinchDocument::new();
    let invalid = div(
        &mut doc,
        &format!("color: notacolor !important; color: {BLUE}"),
    );
    let doubled = div(
        &mut doc,
        &format!("color: {RED} !important !important; color: {BLUE}"),
    );
    let control = div(
        &mut doc,
        &format!("color: {GREEN} !important; color: {BLUE}"),
    );
    let older = div(&mut doc, &format!("color: {BLUE}; color: notacolor"));
    for n in [invalid, doubled, control, older] {
        doc.set_style(n, "gap", "1px");
    }
    doc.resolve_layout(VW, VH);

    assert_eq!(
        hex(&doc, control),
        GREEN,
        "positive control: a *valid* important declaration still wins, which is \
         the rule this deviation is the edge of"
    );
    assert_eq!(
        hex(&doc, invalid),
        BLACK,
        "rinch keeps the invalid important declaration, Stylo rejects it, and \
         the property falls to its initial value; Chrome computes blue"
    );
    assert_eq!(hex(&doc, doubled), BLACK, "same, for a doubled flag");
    assert_eq!(
        hex(&doc, older),
        BLACK,
        "and the plain-vs-plain shape, which this fix did not touch — the \
         deviation is the collapse being syntactic, not the priority arm"
    );
}
