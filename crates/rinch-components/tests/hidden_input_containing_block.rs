//! Every wrapper of a visually hidden, absolutely positioned native input must
//! itself be positioned (issue #388).
//!
//! `Checkbox`, `Radio` and `Switch` share one construction: a `<label>` wrapper
//! holding an `opacity: 0; position: absolute` native `<input>` that keeps the
//! control keyboard- and click-accessible, plus the painted fake control beside
//! it. The wrapper has to declare `position: relative`, because that is what
//! makes it the input's containing block.
//!
//! Switch shipped without it. Nothing was visibly wrong, because all four of
//! that input's insets are `auto` and it carries an explicit size, so it keeps
//! its static position and its containing block never enters into its geometry.
//! The hazard is what happens the moment anyone gives it an inset: since #204 an
//! absolute box with **no** positioned ancestor resolves against the *viewport*,
//! so an `inset: 0` meant to fill the switch would fill the window instead. The
//! mechanics of that correction are pinned in
//! `rinch-dom/tests/layout_tests.rs::absolute_containing_block`; this test pins
//! the component-side precondition that keeps these three out of its way.
//!
//! Asserted against the sheet the app actually ships
//! (`generate_all_component_styles`), not the per-component fragments.

/// The wrapper selector and the hidden-input selector for each control.
const HIDDEN_INPUT_CONTROLS: &[(&str, &str)] = &[
    (".rinch-checkbox", ".rinch-checkbox__input"),
    (".rinch-radio", ".rinch-radio__input"),
    (".rinch-switch", ".rinch-switch__input"),
];

/// Strip `/* ... */` comments.
///
/// Load-bearing, and verified so: with this reduced to the identity the test
/// fails rather than passing, because a comment sitting immediately above a rule
/// is swept into that rule's prelude and stops the selector matching. These
/// three rules are all commented, so nothing here is matched without it.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every declaration in every rule whose selector list contains exactly
/// `selector`, lowercased and whitespace-normalized.
///
/// Selectors are matched as whole comma-separated entries so `.rinch-switch`
/// does not match `.rinch-switch--disabled` or `.rinch-switch__track`.
fn declarations_for(css: &str, selector: &str) -> Vec<String> {
    let css = strip_comments(css);
    let mut found = Vec::new();
    let mut rest = css.as_str();
    while let Some(open) = rest.find('{') {
        let prelude = rest[..open].rsplit('}').next().unwrap_or("").trim();
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let body = &rest[open + 1..open + close];
        let matches = prelude
            .split(',')
            .any(|s| s.split_whitespace().collect::<Vec<_>>().join(" ") == selector);
        if matches {
            found.extend(
                body.split(';')
                    .map(|d| {
                        d.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_lowercase()
                    })
                    .filter(|d| !d.is_empty()),
            );
        }
        rest = &rest[open + close + 1..];
    }
    found
}

#[test]
fn a_wrapper_of_a_hidden_absolute_input_is_itself_positioned() {
    let css = rinch_components::styles::generate_all_component_styles();

    for (wrapper, input) in HIDDEN_INPUT_CONTROLS {
        let input_decls = declarations_for(&css, input);
        assert!(
            !input_decls.is_empty(),
            "{input} has no rule at all — this test is asserting against a selector \
             that no longer exists, so it would pass vacuously"
        );
        assert!(
            input_decls.iter().any(|d| d == "position: absolute"),
            "{input} is expected to be the absolutely positioned hidden input; \
             got {input_decls:?}. If the construction changed, this whole \
             invariant needs rethinking rather than the assertion relaxing."
        );

        let wrapper_decls = declarations_for(&css, wrapper);
        assert!(!wrapper_decls.is_empty(), "{wrapper} has no rule at all");
        assert!(
            wrapper_decls.iter().any(|d| d == "position: relative"),
            "{wrapper} must declare `position: relative` so it is the containing \
             block for {input}; without it an inset on that input would resolve \
             against the viewport (#204/#388). Declarations found: {wrapper_decls:?}"
        );
    }
}
