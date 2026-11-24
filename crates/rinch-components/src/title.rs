//! Title component.
//!
//! Heading component with semantic h1-h6 levels.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A heading element (h1-h6).
#[derive(Debug, Default)]
pub struct Title {
    /// Heading level (1-6). Default is 1.
    pub order: Option<u8>,
    /// Text alignment ("left", "center", "right").
    pub align: String,
    /// Size override, independent of `order` (which still picks the tag, the
    /// weight and the line height). `"1"`–`"6"` or `"h1"`–`"h6"` take that
    /// level's font size; `"xs"`–`"xl"` the theme's text sizes
    /// (`--rinch-font-size-*`); any other value (`"40px"`, `"2.5rem"`,
    /// `"clamp(…)"`) is used as the font size as written. A bare number outside
    /// 1–6 is ignored (the size stays `order`'s). The last two are
    /// written to `--rinch-title-size` and applied by
    /// `.rinch-title--custom-size`.
    pub size: String,
}

/// What a `Title::size` asks for.
enum TitleSize {
    /// A heading level, 1–6.
    Level(u8),
    /// A CSS font size, for `--rinch-title-size`.
    Custom(String),
}

fn parse_size(size: &str) -> Option<TitleSize> {
    let s = size.trim();
    if s.is_empty() {
        return None;
    }
    let digits = s.strip_prefix(['h', 'H']).unwrap_or(s);
    if let Ok(n) = digits.parse::<u32>() {
        // A bare number outside 1–6 is no font size; it keeps `order`'s.
        return (1..=6).contains(&n).then_some(TitleSize::Level(n as u8));
    }
    Some(TitleSize::Custom(match s {
        "xs" | "sm" | "md" | "lg" | "xl" => format!("var(--rinch-font-size-{s})"),
        _ => s.to_string(),
    }))
}

impl Title {
    /// Generate the CSS class string for this title.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-title"];

        // Order/size class
        let order = self.order.unwrap_or(1).clamp(1, 6);
        let parsed = parse_size(&self.size);
        let size = match parsed {
            Some(TitleSize::Level(n)) => n,
            _ => order,
        };
        classes.push(match size {
            1 => "rinch-title--1",
            2 => "rinch-title--2",
            3 => "rinch-title--3",
            4 => "rinch-title--4",
            5 => "rinch-title--5",
            _ => "rinch-title--6",
        });

        if let Some(TitleSize::Custom(_)) = parsed {
            classes.push("rinch-title--custom-size");
        }

        // Alignment
        if !self.align.is_empty() {
            let align = &self.align;
            match align.as_str() {
                "left" => classes.push("rinch-title--left"),
                "center" => classes.push("rinch-title--center"),
                "right" => classes.push("rinch-title--right"),
                _ => {}
            }
        }

        classes.join(" ")
    }
}

impl Component for Title {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let order = self.order.unwrap_or(1).clamp(1, 6);
        let class = self.class_string();
        let container = match order {
            1 => rinch_macros::rsx! { h1 { class: "rinch-title" } },
            2 => rinch_macros::rsx! { h2 { class: "rinch-title" } },
            3 => rinch_macros::rsx! { h3 { class: "rinch-title" } },
            4 => rinch_macros::rsx! { h4 { class: "rinch-title" } },
            5 => rinch_macros::rsx! { h5 { class: "rinch-title" } },
            _ => rinch_macros::rsx! { h6 { class: "rinch-title" } },
        };
        container.set_attribute("class", &class);
        if let Some(TitleSize::Custom(value)) = parse_size(&self.size) {
            container.set_style("--rinch-title-size", &value);
        }
        for child in children {
            container.append_child(child);
        }
        container
    }
}
