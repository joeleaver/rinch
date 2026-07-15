//! Regression: app `:root` custom properties must keep winning over the theme
//! sheet's `:root` after a runtime theme update (rinch/plotweb native bug).

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

// `--rinch-color-accent` is theme-only: the app never overrides it, so it must
// track theme updates.
const THEME_V1: &str = ":root { --rinch-color-text: #c9c9c9; --rinch-color-accent: #ff0000; }";
// A later theme regeneration (e.g. ThemeProvider re-run with different props).
const THEME_V2: &str = ":root { --rinch-color-text: #c9c9c9; --rinch-color-accent: #0000ff; }";

const APP_CSS: &str = r#"
:root {
    --rinch-color-text: #e7e0d8;
    --pw-color-deep: #1a1714;
}
.text { color: var(--rinch-color-text); }
.deep { color: var(--pw-color-deep); }
.accent { color: var(--rinch-color-accent); }
"#;

fn hex(c: Option<peniko::Color>) -> String {
    let c = c.expect("color should resolve");
    let rgba = c.to_rgba8();
    format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b)
}

struct Doc {
    doc: RinchDocument,
    text: rinch_core::dom::NodeId,
    deep: rinch_core::dom::NodeId,
    accent: rinch_core::dom::NodeId,
}

/// Build a doc the way the native shell does: theme sheet first (via the theme
/// slot), then the app's `<style>` element.
fn setup() -> Doc {
    let mut doc = RinchDocument::new();
    doc.set_theme_css(THEME_V1);

    let body = doc.body();

    let style_el = doc.create_element("style");
    let style_text = doc.create_text(APP_CSS);
    doc.append_child(style_el, style_text);
    doc.append_child(body, style_el);

    let text = doc.create_element("div");
    doc.set_attribute(text, "class", "text");
    doc.append_child(body, text);

    let deep = doc.create_element("div");
    doc.set_attribute(deep, "class", "deep");
    doc.append_child(body, deep);

    let accent = doc.create_element("div");
    doc.set_attribute(accent, "class", "accent");
    doc.append_child(body, accent);

    doc.resolve_layout(800.0, 600.0);
    Doc {
        doc,
        text,
        deep,
        accent,
    }
}

impl Doc {
    fn color_of(&self, id: rinch_core::dom::NodeId) -> String {
        hex(self.doc.tree.get(id.0).unwrap().computed_style.color)
    }
}

#[test]
fn app_root_var_overrides_theme_var_initially() {
    let d = setup();
    assert_eq!(
        d.color_of(d.text),
        "#e7e0d8",
        "app :root should win over the theme sheet (later source order)"
    );
}

#[test]
fn app_root_var_still_overrides_theme_var_after_theme_update() {
    let mut d = setup();

    // Runtime theme change (ThemeProvider props resolved during render).
    d.doc.update_theme_variables(THEME_V2);
    d.doc.recompute_all_styles_full();

    // Non-colliding app var is unaffected — matches the observed symptom.
    assert_eq!(
        d.color_of(d.deep),
        "#1a1714",
        "app-only var should survive a theme update"
    );

    assert_eq!(
        d.color_of(d.text),
        "#e7e0d8",
        "app :root must STILL win after a theme update; re-appending the theme \
         sheet must not jump it ahead of app CSS in the cascade"
    );
}

/// The theme slot must still be *live*: variables the app does not override have
/// to follow theme updates (e.g. a dark-mode toggle).
#[test]
fn theme_only_var_follows_theme_updates() {
    let mut d = setup();
    assert_eq!(d.color_of(d.accent), "#ff0000", "initial theme accent");

    d.doc.update_theme_variables(THEME_V2);
    d.doc.recompute_all_styles_full();

    assert_eq!(
        d.color_of(d.accent),
        "#0000ff",
        "theme-only var must track the updated theme sheet"
    );
}

/// Repeated theme updates must replace the sheet, not stack copies of it.
#[test]
fn repeated_theme_updates_do_not_stack_sheets() {
    let mut d = setup();

    for _ in 0..5 {
        d.doc.update_theme_variables(THEME_V1);
        d.doc.recompute_all_styles_full();
        d.doc.update_theme_variables(THEME_V2);
        d.doc.recompute_all_styles_full();
    }

    assert_eq!(
        d.color_of(d.text),
        "#e7e0d8",
        "app override must survive repeated theme updates"
    );
    assert_eq!(
        d.color_of(d.accent),
        "#0000ff",
        "latest theme value must win after repeated updates"
    );
}
