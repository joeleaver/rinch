//! An embed `RinchContext` dropped without a discard walk leaves no scope
//! ancestry behind (issue #732).
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_drop_minting_732

#![cfg(any(feature = "gpu", feature = "embed"))]

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 200,
        height: 200,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

#[test]
fn dropping_contexts_leaves_no_minting_entries_or_scope_parents() {
    let mb0 = rinch_core::dom::__minted_by_len();
    let sp0 = rinch_core::dom::__scope_parents_len();
    let mut pairs = Vec::new();
    for round in 0..20u32 {
        let open = Signal::new(true);
        let items = Signal::new(vec![1u32]);
        let mut ctx = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    if open.get() {
                        section {
                            for n in items.get() {
                                p { key: n, {n.to_string()} }
                            }
                        }
                    }
                }
            }
        });
        let _ = ctx.update(&[]);
        items.set(vec![1, 2, 3 + round]);
        let _ = ctx.update(&[]);
        open.set(false);
        open.set(true);
        let _ = ctx.update(&[]);
        let live = rinch_core::dom::__minted_by_len();
        drop(ctx);
        pairs.push((
            live,
            rinch_core::dom::__minted_by_len(),
            rinch_core::dom::__scope_parents_len(),
        ));
    }
    eprintln!("(live-before-drop, minted-after-drop, scope_parents-after-drop) = {pairs:?}");
    assert_eq!(
        rinch_core::dom::__minted_by_len(),
        mb0,
        "minting entries survive context drop"
    );
    assert_eq!(
        rinch_core::dom::__scope_parents_len(),
        sp0,
        "scope parents survive context drop"
    );
}
