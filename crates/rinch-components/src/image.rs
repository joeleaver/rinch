//! Image component.
//!
//! A responsive image component with fallback support.

use std::cell::Cell;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};

use crate::borderless_window::SectionRenderer;

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
///
/// A picture that fails to load fires `on_error`, then shows `fallback_src`
/// in its place; if that fails too (or there is none), the `fallback`
/// content replaces the `<img>`. With neither, the `<img>` shows its `alt`
/// text, as a browser does. An empty `src` loads nothing and fires nothing:
/// the fallback (picture, else content) shows from the start, as in Mantine.
#[derive(Default)]
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
    /// The picture shown when `src` fails to load (or is empty). Tried
    /// once: a failing fallback does not retry itself.
    pub fallback_src: String,
    /// Caption text.
    pub caption: String,
    /// Runs each time a picture fails to load (the browser's `error` event):
    /// `src`, and `fallback_src` if that fails as well.
    pub on_error: Option<Callback>,
    /// Content that replaces the `<img>` once no picture can be shown: `src`
    /// failed (or is empty) and so did `fallback_src`, or there is none.
    pub fallback: Option<SectionRenderer>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("src", &self.src)
            .field("alt", &self.alt)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fit", &self.fit)
            .field("radius", &self.radius)
            .field("fallback_src", &self.fallback_src)
            .field("caption", &self.caption)
            .field("on_error", &self.on_error.as_ref().map(|_| "<callback>"))
            .field("fallback", &self.fallback.as_ref().map(|_| "<section>"))
            .finish()
    }
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
        let src = self.src.as_str();
        let alt = if self.alt.is_empty() { "" } else { &self.alt };

        // Build inline styles for the <img> element directly.
        // Setting dimensions on the <img> (not the wrapper) ensures Taffy's
        // image measure function can resolve intrinsic sizing correctly.
        let mut img_styles = Vec::new();
        if !self.width.is_empty() {
            let w = &self.width;
            let w = if w.chars().all(|c| c.is_ascii_digit()) {
                format!("{}px", w)
            } else {
                w.clone()
            };
            img_styles.push(format!("width: {}", w));
        }
        if !self.height.is_empty() {
            let h = &self.height;
            let h = if h.chars().all(|c| c.is_ascii_digit()) {
                format!("{}px", h)
            } else {
                h.clone()
            };
            img_styles.push(format!("height: {}", h));
        }

        // Apply object-fit as inline style so the paint code picks it up
        // from computed styles (CSS class selectors may not cascade reliably).
        let fit: ImageFit = if self.fit.is_empty() {
            ImageFit::default()
        } else {
            self.fit.parse().unwrap_or_default()
        };
        match fit {
            ImageFit::Cover => img_styles.push("object-fit: cover".into()),
            ImageFit::Contain => img_styles.push("object-fit: contain".into()),
            ImageFit::None => img_styles.push("object-fit: none".into()),
            ImageFit::ScaleDown => img_styles.push("object-fit: scale-down".into()),
            ImageFit::Fill => {} // default, no need to set
        }

        let figure = rinch_macros::rsx! { figure { class: "rinch-image__wrapper" } };

        let img = rinch_macros::rsx! { img { class: "rinch-image" } };
        img.set_attribute("class", &self.class_string());
        if !img_styles.is_empty() {
            img.set_attribute("style", &img_styles.join("; "));
        }
        img.set_attribute("alt", alt);

        // The fallback content is built now, while there is a scope to build
        // it in, and goes in only once no picture can be shown.
        let fallback = self.fallback.as_ref().map(|render| render(__scope));
        // An empty `src` loads nothing (a browser would fire `error` for
        // `src=""`, the desktop has nothing to fail), so the fallback is
        // taken straight away.
        let fallback_tried = src.is_empty() && !self.fallback_src.is_empty();
        if fallback_tried {
            img.set_attribute("src", &self.fallback_src);
        } else if !src.is_empty() {
            img.set_attribute("src", src);
        }

        // A load failure (#2203): `onerror` on the `<img>`, which the desktop
        // fires after the layout that learnt of it and the web from the
        // browser's `error` event. It used to be a JavaScript
        // `onerror="this.src=…"` string, which nothing on the desktop reads.
        if self.on_error.is_some() || !self.fallback_src.is_empty() || fallback.is_some() {
            let on_error = self.on_error.clone();
            let fallback_src = self.fallback_src.clone();
            let fallback = fallback.clone();
            let img_handle = img.clone();
            let tried = Cell::new(fallback_tried);
            let replaced = Cell::new(false);
            let handler = __scope.register_handler(move || {
                if let Some(cb) = &on_error {
                    cb.invoke();
                }
                if !tried.get() && !fallback_src.is_empty() {
                    tried.set(true);
                    img_handle.set_attribute("src", &fallback_src);
                } else if let Some(node) = &fallback
                    && !replaced.get()
                {
                    replaced.set(true);
                    img_handle.replace_with(node);
                }
            });
            img.set_attribute("data-onerror", &handler.0.to_string());
        }

        match &fallback {
            // Nothing to load at all: the content from the start.
            Some(node) if src.is_empty() && self.fallback_src.is_empty() => {
                figure.append_child(node);
            }
            _ => figure.append_child(&img),
        }

        if !self.caption.is_empty() {
            let caption = &self.caption;
            let figcaption =
                rinch_macros::rsx! { figcaption { class: "rinch-image__caption", {caption} } };
            figure.append_child(&figcaption);
        }

        figure
    }
}
