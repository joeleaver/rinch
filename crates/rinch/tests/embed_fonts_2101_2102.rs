//! Fonts in an embedded `RinchContext` (desktop host).
//!
//! #2102: a generic family drew in the font registered **last** through
//! `RinchContextConfig::fonts`, whatever its style: every face was prepended
//! to `sans-serif`. Registered data now changes no generic slot the platform
//! fills, and fills an empty one only with a face of its class.
//!
//! #2101: the embed took only `&'static [u8]`, so an app's run-time fonts had
//! to be leaked. It now takes owned bytes, and drops them with the context.
//!
//! ```text
//! cargo test -p rinch --features embed --test embed_fonts_2101_2102
//! ```
//!
//! The wasm half (no platform fonts, every slot empty) is
//! `embed_web_fonts_2102.rs`.
#![cfg(any(feature = "gpu", feature = "embed"))]

#[path = "support/font_variants.rs"]
mod font_variants;

use font_variants::{Declared, variant};
use rinch::embed::{RinchContext, RinchContextConfig};
use rinch_core::dom::{NodeHandle, RenderScope};

fn leaked(name: &str, class: Declared) -> &'static [u8] {
    Box::leak(variant(name, class).into_boxed_slice())
}

/// The issue's document: the root in `sans-serif`, one element in the script
/// face by name.
fn component(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("style", "font-family: sans-serif; font-size: 16px");
    root.append_child(&scope.create_text("Interface text"));
    let fancy = scope.create_element("p");
    fancy.set_attribute("style", "font-family: Scrpt");
    fancy.append_child(&scope.create_text("Decorative"));
    root.append_child(&fancy);
    root
}

/// The address of the bytes each glyph run of `text` in `stack` came from, in
/// the context's document font context: the one layout and paint use.
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

fn blob_lens(ctx: &RinchContext, stack: &str) -> Vec<usize> {
    let doc = ctx.app().doc().expect("mounted");
    let mut d = doc.borrow_mut();
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    let text = "Mm";
    let mut builder = layout_cx.ranged_builder(&mut d.font_cx, text, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::Source(stack.to_owned().into()),
    ));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    let mut out = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                out.push(run.run().font().data.as_ref().len());
            }
        }
    }
    out
}

/// The issue as reported: a sans face and then a script face in the config.
/// `sans-serif` resolves to what it resolves to in a context with no app
/// fonts at all, the script face only where it is named. Before #2102 the
/// script face, registered last, answered `sans-serif`.
#[test]
fn config_fonts_do_not_change_what_sans_serif_means() {
    let sans = leaked("Sansa", Declared::SansSerif);
    let script = leaked("Scrpt", Declared::Script);
    let config = |fonts| RinchContextConfig {
        width: 400,
        height: 300,
        scale_factor: 1.0,
        theme: None,
        fonts,
    };
    let plain = RinchContext::new(config(Vec::new()), component);
    let platform = blob_lens(&plain, "sans-serif");
    assert!(
        !platform.is_empty(),
        "precondition: this host maps `sans-serif` to a face"
    );
    drop(plain);

    let mut ctx = RinchContext::new(config(vec![sans.into(), script.into()]), component);
    ctx.update(&[]);
    let _ = ctx.scene();
    let answered = resolved(&ctx, "sans-serif");
    assert!(
        !answered.contains(&(script.as_ptr() as usize)),
        "the script face, registered last, must not answer `sans-serif`"
    );
    assert!(
        !answered.contains(&(sans.as_ptr() as usize)),
        "nor the sans face: the platform's mapping stands"
    );
    assert_eq!(blob_lens(&ctx, "sans-serif"), platform);
    assert_eq!(
        resolved(&ctx, "Scrpt"),
        vec![script.as_ptr() as usize],
        "the script face answers its own name"
    );

    // A late registration changes nothing either.
    ctx.register_font(leaked("Scrpb", Declared::Script));
    assert_eq!(blob_lens(&ctx, "sans-serif"), platform);
}

/// Owned bytes, given in the config, through `register_font` and as a
/// claimed face, are held while the context lives and released when it is
/// dropped, after an update and a scene painted text in that face.
#[test]
fn owned_font_bytes_are_released_with_the_context() {
    use rinch::font::AppFont;
    use std::sync::Arc;

    let bytes: Arc<[u8]> = variant("Ownad", Declared::SansSerif).into();
    {
        let mut ctx = RinchContext::new(
            RinchContextConfig {
                width: 400,
                height: 300,
                scale_factor: 1.0,
                theme: None,
                fonts: vec![bytes.clone().into()],
            },
            |scope: &mut RenderScope| {
                let root = scope.create_element("p");
                root.set_attribute("style", "font-family: Ownad; font-size: 16px");
                root.append_child(&scope.create_text("Owned bytes"));
                root
            },
        );
        ctx.register_font(bytes.clone());
        ctx.register_app_font(AppFont::from_data(bytes.clone()).with_generics(AppFont::SERIF));
        ctx.update(&[]);
        let _ = ctx.scene();
        assert!(Arc::strong_count(&bytes) > 1, "positive control");
        assert_eq!(
            resolved(&ctx, "Ownad"),
            vec![bytes.as_ptr() as usize],
            "positive control: the text is in the owned bytes"
        );
        assert_eq!(
            resolved(&ctx, "serif"),
            vec![bytes.as_ptr() as usize],
            "the explicit claim takes the slot the platform fills"
        );
    }
    assert_eq!(
        Arc::strong_count(&bytes),
        1,
        "every reference rinch took must be gone with the context"
    );
}
