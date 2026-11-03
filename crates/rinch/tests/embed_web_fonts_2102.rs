//! Fonts in an embedded `RinchContext` in the browser (#2101, #2102), where
//! there are no platform fonts: every generic slot starts empty, and the
//! app's fonts are all there is.
//!
//! Before #2102 each registered face was prepended to `sans-serif`, so the
//! last one registered — a script face as easily as a text face — was what
//! every `sans-serif` element drew in. Now the first face of the sans class
//! fills the empty slot, whatever came before or after it.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch --no-default-features --features embed \
//!     --target wasm32-unknown-unknown --test embed_web_fonts_2102
//! ```
#![cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "embed"))]

#[path = "support/font_variants.rs"]
mod font_variants;

use font_variants::{Declared, variant};
use rinch::embed::{RinchContext, RinchContextConfig};
use rinch_core::dom::{NodeHandle, RenderScope};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn leaked(name: &str, class: Declared) -> &'static [u8] {
    Box::leak(variant(name, class).into_boxed_slice())
}

fn component(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("style", "font-family: sans-serif; font-size: 16px");
    root.append_child(&scope.create_text("Interface text"));
    root
}

/// The address of the bytes each glyph run of "Mm" in `stack` came from.
fn resolved(ctx: &RinchContext, stack: &str) -> Vec<usize> {
    let doc = ctx.app().doc().expect("mounted");
    let mut d = doc.borrow_mut();
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    let text = "Mm";
    let mut builder = layout_cx.ranged_builder(&mut d.font_cx, text, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(16.0));
    builder.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::Source(stack.to_owned().into()),
    ));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    let mut out = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                out.push(run.run().font().data.as_ref().as_ptr() as usize);
            }
        }
    }
    out
}

#[wasm_bindgen_test]
fn sans_serif_is_the_first_sans_face_and_owned_bytes_are_released() {
    let sans = leaked("Sansa", Declared::SansSerif);
    let script = leaked("Scrpt", Declared::Script);
    let serif = leaked("Serfa", Declared::Serif);
    for order in [[sans, script, serif], [script, serif, sans]] {
        let mut ctx = RinchContext::new(
            RinchContextConfig {
                width: 400,
                height: 300,
                scale_factor: 1.0,
                theme: None,
                fonts: order.iter().map(|f| (*f).into()).collect(),
            },
            component,
        );
        ctx.update(&[]);
        let _ = ctx.scene();
        assert_eq!(
            resolved(&ctx, "sans-serif"),
            vec![sans.as_ptr() as usize],
            "`sans-serif` must be the sans face, not the last registered"
        );
        assert_eq!(resolved(&ctx, "serif"), vec![serif.as_ptr() as usize]);
        assert_eq!(resolved(&ctx, "Scrpt"), vec![script.as_ptr() as usize]);
        ctx.register_font(leaked("Sansb", Declared::SansSerif));
        assert_eq!(
            resolved(&ctx, "sans-serif"),
            vec![sans.as_ptr() as usize],
            "a later sans face does not displace the first"
        );
    }

    // #2101 on wasm: owned bytes are dropped with the context.
    let bytes: std::sync::Arc<[u8]> = variant("Ownad", Declared::SansSerif).into();
    {
        let mut ctx = RinchContext::new(
            RinchContextConfig {
                width: 400,
                height: 300,
                scale_factor: 1.0,
                theme: None,
                fonts: vec![bytes.clone().into()],
            },
            component,
        );
        ctx.update(&[]);
        let _ = ctx.scene();
        assert_eq!(
            resolved(&ctx, "sans-serif"),
            vec![bytes.as_ptr() as usize],
            "positive control: the owned face renders `sans-serif`"
        );
    }
    assert_eq!(std::sync::Arc::strong_count(&bytes), 1);
}
