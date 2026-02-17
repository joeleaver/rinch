//! Image component.
//!
//! A responsive image component with fallback support.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Image fit mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    #[default]
    Cover,
    Contain,
    Fill,
    None,
    ScaleDown,
}

impl ImageFit {
    pub fn class_name(&self) -> &'static str {
        match self {
            ImageFit::Cover => "rinch-image--fit-cover",
            ImageFit::Contain => "rinch-image--fit-contain",
            ImageFit::Fill => "rinch-image--fit-fill",
            ImageFit::None => "rinch-image--fit-none",
            ImageFit::ScaleDown => "rinch-image--fit-scale-down",
        }
    }
}

impl std::str::FromStr for ImageFit {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cover" => Ok(ImageFit::Cover),
            "contain" => Ok(ImageFit::Contain),
            "fill" => Ok(ImageFit::Fill),
            "none" => Ok(ImageFit::None),
            "scale-down" | "scaledown" => Ok(ImageFit::ScaleDown),
            _ => Err(()),
        }
    }
}

/// A responsive image component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Image { src: "/photo.jpg", alt: "A photo", radius: "md" }
///     Image { src: "/photo.jpg", width: "200", height: "150", fit: "cover" }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Image {
    /// Image source URL.
    pub src: String,
    /// Alt text.
    pub alt: String,
    /// Width (CSS value or number for pixels).
    pub width: String,
    /// Height (CSS value or number for pixels).
    pub height: String,
    /// Object fit mode.
    pub fit: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Fallback image source.
    pub fallback_src: String,
    /// Caption text.
    pub caption: String,
}

impl Image {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-image"];

        let fit: ImageFit = if self.fit.is_empty() {
            ImageFit::default()
        } else {
            self.fit.parse().unwrap_or_default()
        };
        classes.push(fit.class_name());

        if !self.radius.is_empty() {
            let r = &self.radius;
            classes.push(match r.as_str() {
                "xs" => "rinch-image--radius-xs",
                "sm" => "rinch-image--radius-sm",
                "md" => "rinch-image--radius-md",
                "lg" => "rinch-image--radius-lg",
                "xl" => "rinch-image--radius-xl",
                _ => "",
            });
        }

        classes.join(" ")
    }
}

impl Component for Image {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let src = if self.src.is_empty() { "" } else { &self.src };
        let alt = if self.alt.is_empty() { "" } else { &self.alt };

        let mut styles = Vec::new();
        if !self.width.is_empty() {
            let w = &self.width;
            let w = if w.chars().all(|c| c.is_ascii_digit()) {
                format!("{}px", w)
            } else {
                w.clone()
            };
            styles.push(format!("width: {}", w));
        }
        if !self.height.is_empty() {
            let h = &self.height;
            let h = if h.chars().all(|c| c.is_ascii_digit()) {
                format!("{}px", h)
            } else {
                h.clone()
            };
            styles.push(format!("height: {}", h));
        }

        let figure = rinch_macros::rsx! { figure { class: "rinch-image__wrapper" } };

        if !styles.is_empty() {
            figure.set_attribute("style", &styles.join("; "));
        }

        let img = rinch_macros::rsx! { img { class: "rinch-image" } };
        img.set_attribute("class", &self.class_string());
        img.set_attribute("src", src);
        img.set_attribute("alt", alt);

        if !self.fallback_src.is_empty() {
            let fallback = &self.fallback_src;
            img.set_attribute("onerror", &format!("this.src='{}'", fallback));
        }

        figure.append_child(&img);

        if !self.caption.is_empty() {
            let caption = &self.caption;
            let figcaption = rinch_macros::rsx! { figcaption { class: "rinch-image__caption" } };
            let text_node = __scope.create_text(caption);
            figcaption.append_child(&text_node);
            figure.append_child(&figcaption);
        }

        figure
    }
}
