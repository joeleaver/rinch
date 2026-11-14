//! A window sized by its content ([`WindowProps::size_to_content`]).
//!
//! [`WindowProps::size_to_content`]: crate::element::WindowProps::size_to_content

/// How a window sized by its content is sized ([`WindowProps::size_to_content`]):
/// the window takes the size its content lays out to, up to a maximum, and
/// follows it as the content changes. Content that wants more than the
/// maximum scrolls inside the window.
///
/// The maxima are logical pixels. Without one, an axis's maximum is the size
/// of the output the window is on, less the margins of a layer surface
/// anchored on that axis.
///
/// Built with [`ContentSizing::new`] (or `Default`) and the `with_` methods,
/// so later rules can be added without breaking an app:
///
/// ```
/// use rinch_core::element::ContentSizing;
/// let sizing = ContentSizing::new().with_max_width(360);
/// assert_eq!(sizing.max_width, Some(360));
/// assert_eq!(sizing.max_height, None);
/// ```
///
/// [`WindowProps::size_to_content`]: crate::element::WindowProps::size_to_content
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ContentSizing {
    /// The widest the window grows, logical pixels; `None`: the output's
    /// width (less a layer surface's left and right margins).
    pub max_width: Option<u32>,
    /// The tallest the window grows, logical pixels; `None`: the output's
    /// height (less a layer surface's top and bottom margins).
    pub max_height: Option<u32>,
}

impl ContentSizing {
    /// Sized by the content, with the output's size as the maximum.
    pub fn new() -> Self {
        Self::default()
    }

    /// At most `width` logical pixels wide.
    pub fn with_max_width(mut self, width: u32) -> Self {
        self.max_width = Some(width);
        self
    }

    /// At most `height` logical pixels tall.
    pub fn with_max_height(mut self, height: u32) -> Self {
        self.max_height = Some(height);
        self
    }
}
