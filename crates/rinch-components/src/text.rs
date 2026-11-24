//! Text component.
//!
//! A typography component for displaying text with various styles.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Text size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl TextSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            TextSize::Xs => "rinch-text--xs",
            TextSize::Sm => "rinch-text--sm",
            TextSize::Md => "rinch-text--md",
            TextSize::Lg => "rinch-text--lg",
            TextSize::Xl => "rinch-text--xl",
        }
    }
}

impl std::str::FromStr for TextSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(TextSize::Xs),
            "sm" => Ok(TextSize::Sm),
            "md" => Ok(TextSize::Md),
            "lg" => Ok(TextSize::Lg),
            "xl" => Ok(TextSize::Xl),
            _ => Err(()),
        }
    }
}

/// Text weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextWeight {
    Thin,
    ExtraLight,
    Light,
    Normal,
    Medium,
    SemiBold,
    Bold,
    ExtraBold,
    Black,
}

impl TextWeight {
    /// Get the CSS class name for this weight.
    pub fn class_name(&self) -> &'static str {
        match self {
            TextWeight::Thin => "rinch-text--thin",
            TextWeight::ExtraLight => "rinch-text--extralight",
            TextWeight::Light => "rinch-text--light",
            TextWeight::Normal => "rinch-text--normal",
            TextWeight::Medium => "rinch-text--medium",
            TextWeight::SemiBold => "rinch-text--semibold",
            TextWeight::Bold => "rinch-text--bold",
            TextWeight::ExtraBold => "rinch-text--extrabold",
            TextWeight::Black => "rinch-text--black",
        }
    }
}

impl std::str::FromStr for TextWeight {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "thin" | "100" => Ok(TextWeight::Thin),
            "extralight" | "200" => Ok(TextWeight::ExtraLight),
            "light" | "300" => Ok(TextWeight::Light),
            "normal" | "400" => Ok(TextWeight::Normal),
            "medium" | "500" => Ok(TextWeight::Medium),
            "semibold" | "600" => Ok(TextWeight::SemiBold),
            "bold" | "700" => Ok(TextWeight::Bold),
            "extrabold" | "800" => Ok(TextWeight::ExtraBold),
            "black" | "900" => Ok(TextWeight::Black),
            _ => Err(()),
        }
    }
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl TextAlign {
    /// Get the CSS class name for this alignment.
    pub fn class_name(&self) -> &'static str {
        match self {
            TextAlign::Left => "rinch-text--left",
            TextAlign::Center => "rinch-text--center",
            TextAlign::Right => "rinch-text--right",
            TextAlign::Justify => "rinch-text--justify",
        }
    }
}

impl std::str::FromStr for TextAlign {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "left" => Ok(TextAlign::Left),
            "center" => Ok(TextAlign::Center),
            "right" => Ok(TextAlign::Right),
            "justify" => Ok(TextAlign::Justify),
            _ => Err(()),
        }
    }
}

/// A typography component for displaying text.
#[derive(Debug, Default)]
pub struct Text {
    /// Text size (xs, sm, md, lg, xl).
    pub size: String,
    /// Font weight (thin, light, normal, medium, semibold, bold, etc.).
    pub weight: String,
    /// Text color. A named value (`primary`, `dimmed`, `inherit`, or one of the
    /// theme palettes: `red`, `blue`, `gray`, ...) is a class, as before.
    /// Anything else is written to `--rinch-text-color` and applied by
    /// `.rinch-text--custom-color`: a palette shade as `name.N` (`"blue.3"` →
    /// `var(--rinch-color-blue-3)`), another bare name as that palette's shade 6
    /// with the name itself as the fallback (`"brand"` →
    /// `var(--rinch-color-brand-6, brand)`, so a custom theme palette and a CSS
    /// colour keyword such as `crimson` both work), and any other value (`#hex`,
    /// `rgb(…)`, `var(…)`) as written.
    pub color: String,
    /// Text alignment (left, center, right, justify).
    pub align: String,
    /// Whether to render as a span instead of p (inline).
    pub inline: bool,
    /// Cut the text to one line ending in "…" (`.rinch-text--truncate`:
    /// `display: block`, `white-space: nowrap`, `overflow: hidden`,
    /// `text-overflow: ellipsis`).
    pub truncate: bool,
    /// Clamp the text to this many lines, the last ending in "…"
    /// (`.rinch-text--line-clamp`, a `-webkit-line-clamp` box reading
    /// `--rinch-text-line-clamp`). `None` (the default) does not clamp.
    pub line_clamp: Option<u32>,
}

/// The CSS value a non-named `Text::color` stands for (see its doc).
fn custom_color_value(color: &str) -> String {
    let c = color.trim();
    let is_word = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    };
    if let Some((name, shade)) = c.split_once('.') {
        if is_word(name) && !shade.is_empty() && shade.chars().all(|ch| ch.is_ascii_digit()) {
            return format!("var(--rinch-color-{name}-{shade})");
        }
    }
    if is_word(c) && c.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        return format!("var(--rinch-color-{c}-6, {c})");
    }
    c.to_string()
}

/// The class a named `Text::color` maps to, if it is one.
fn named_color_class(color: &str) -> Option<&'static str> {
    Some(match color {
        "primary" => "rinch-text--primary",
        "dimmed" => "rinch-text--dimmed",
        "inherit" => "rinch-text--inherit",
        "red" => "rinch-text--red",
        "pink" => "rinch-text--pink",
        "grape" => "rinch-text--grape",
        "violet" => "rinch-text--violet",
        "indigo" => "rinch-text--indigo",
        "blue" => "rinch-text--blue",
        "cyan" => "rinch-text--cyan",
        "teal" => "rinch-text--teal",
        "green" => "rinch-text--green",
        "lime" => "rinch-text--lime",
        "yellow" => "rinch-text--yellow",
        "orange" => "rinch-text--orange",
        "gray" => "rinch-text--gray",
        "dark" => "rinch-text--dark",
        _ => return None,
    })
}

impl Text {
    /// Generate the CSS class string for this text.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-text"];

        // Size class
        if self.size.is_empty() {
            classes.push(TextSize::Md.class_name());
        } else if let Ok(s) = self.size.parse::<TextSize>() {
            classes.push(s.class_name());
        }

        // Weight class
        if !self.weight.is_empty() {
            if let Ok(w) = self.weight.parse::<TextWeight>() {
                classes.push(w.class_name());
            }
        }

        // Color class
        if !self.color.trim().is_empty() {
            classes.push(named_color_class(&self.color).unwrap_or("rinch-text--custom-color"));
        }

        // Alignment class
        if !self.align.is_empty() {
            if let Ok(a) = self.align.parse::<TextAlign>() {
                classes.push(a.class_name());
            }
        }

        if self.truncate {
            classes.push("rinch-text--truncate");
        }
        if self.line_clamp.is_some() {
            classes.push("rinch-text--line-clamp");
        }

        classes.join(" ")
    }
}

impl Component for Text {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();
        let container = if self.inline {
            rinch_macros::rsx! { span { class: "rinch-text" } }
        } else {
            rinch_macros::rsx! { p { class: "rinch-text" } }
        };
        container.set_attribute("class", &class);
        if !self.color.trim().is_empty() && named_color_class(&self.color).is_none() {
            container.set_style("--rinch-text-color", &custom_color_value(&self.color));
        }
        if let Some(lines) = self.line_clamp {
            container.set_style("--rinch-text-line-clamp", &lines.to_string());
        }
        for child in children {
            container.append_child(child);
        }
        container
    }
}
