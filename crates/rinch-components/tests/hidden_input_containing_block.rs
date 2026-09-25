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

mod common;
use common::{declarations_for, strip_comments};

/// The wrapper selector and the hidden-input selector for each control.
const HIDDEN_INPUT_CONTROLS: &[(&str, &str)] = &[
    (".rinch-checkbox", ".rinch-checkbox__input"),
    (".rinch-radio", ".rinch-radio__input"),
    (".rinch-switch", ".rinch-switch__input"),
];

#[test]
fn a_wrapper_of_a_hidden_absolute_input_is_itself_positioned() {
    // Comments stripped once, up front: every one of these three rules is
    // commented, so none of them matches without it.
    let css = strip_comments(&rinch_components::styles::generate_all_component_styles());

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
