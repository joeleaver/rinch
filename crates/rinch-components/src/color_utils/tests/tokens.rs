use super::*;

/// #3020: `contrast_for` (Button, and every `accent_vars` component) and
/// `ColorTokens::contrast` (Alert, Badge, Progress) share one rule. A raw
/// colour gets the same computed `white` / `black` from both.
#[test]
fn both_contrast_callers_compute_a_raw_colour_alike() {
    for (color, want) in [
        ("#ffe066", "black"),
        ("#1c7ed6", "white"),
        ("rgb(255, 255, 255)", "black"),
        ("rgba(0, 0, 0, 0.5)", "white"),
        // `contrast_text` cannot read hsl(); both fall back to white.
        ("hsl(50, 100%, 70%)", "white"),
    ] {
        assert_eq!(contrast_for(color), want, "contrast_for({color})");
        assert_eq!(color_tokens(color).contrast, want, "color_tokens({color})");
    }
}

/// A name reads its family's contrast token: `contrast_for` bare, and
/// `ColorTokens::contrast` with `white` for a family that has none.
#[test]
fn a_name_reads_its_familys_contrast_token() {
    assert_eq!(contrast_for("yellow"), "var(--rinch-color-yellow-contrast)");
    assert_eq!(
        color_tokens("yellow").contrast,
        "var(--rinch-color-yellow-contrast, white)"
    );
    assert_eq!(
        color_tokens("brand").contrast,
        "var(--rinch-color-brand-contrast, white)"
    );
}

/// `"primary"` reads the `--rinch-primary-color` family, its contrast token
/// included (the branch #3020 dropped wrote the same string by hand).
#[test]
fn primary_tokens_read_the_primary_family() {
    assert_eq!(
        color_tokens("primary"),
        ColorTokens {
            base: "var(--rinch-primary-color)".to_string(),
            light: "var(--rinch-primary-color-light)".to_string(),
            light_color: "var(--rinch-primary-color-light-color)".to_string(),
            light_strong: "var(--rinch-primary-color-light-color-strong)".to_string(),
            light_text: "var(--rinch-primary-color-light-text)".to_string(),
            contrast: "var(--rinch-primary-color-contrast, white)".to_string(),
        }
    );
}
