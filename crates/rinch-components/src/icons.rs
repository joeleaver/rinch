//! SVG icons as structured Element nodes.
//!
//! Using structured nodes instead of HTML strings enables differential updates.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_tabler_icons::{TablerIcon, TablerIconOptions, TablerIconStyle};

use crate::SectionRenderer;

// =============================================================================
// The app's chrome glyphs (#2208)
// =============================================================================

/// The app-wide replacements for the components' built-in chrome glyphs:
/// the close X, the chevrons and the check (issue #2208).
///
/// Provide it once, near the root, with `create_context`:
///
/// ```ignore
/// create_context(
///     IconSet::default()
///         .close(IconSet::tabler(TablerIcon::SquareX))
///         .chevron_down(IconSet::tabler(TablerIcon::CaretDown)),
/// );
/// ```
///
/// Each slot left `None` keeps rinch's built-in glyph, so the default set
/// draws exactly what the components drew before. A glyph is drawn where the
/// built-in `<svg>` was and is sized the same way: the component's CSS sizes
/// an `svg` there, and where the built-in glyph carried `width`/`height`
/// attributes (CloseButton, Alert's close) the replacement root is given them
/// too. Draw an `<svg>` with `stroke="currentColor"` (as
/// [`IconSet::tabler`] does) and it takes the component's colour and size.
///
/// | slot | drawn by |
/// |---|---|
/// | `close` | CloseButton (with no `icon` of its own), Alert's close button, Modal, Drawer and Notification close buttons |
/// | `chevron_down` | Accordion's chevron |
/// | `chevron_left`, `chevron_right` | Pagination's previous/next, Tree's expander (`chevron_right`) |
/// | `chevrons_left`, `chevrons_right` | Pagination's first/last |
/// | `check` | Stepper's completed step (with no `completed_icon` of its own) |
///
/// A component's own icon prop (CloseButton's `icon`, a step's
/// `completed_icon`) still wins over the set.
#[derive(Clone, Default)]
pub struct IconSet {
    /// The close X.
    pub close: Option<SectionRenderer>,
    /// The downward chevron (Accordion).
    pub chevron_down: Option<SectionRenderer>,
    /// The left chevron (Pagination's previous).
    pub chevron_left: Option<SectionRenderer>,
    /// The right chevron (Pagination's next, Tree's expander).
    pub chevron_right: Option<SectionRenderer>,
    /// The double left chevron (Pagination's first).
    pub chevrons_left: Option<SectionRenderer>,
    /// The double right chevron (Pagination's last).
    pub chevrons_right: Option<SectionRenderer>,
    /// The check (Stepper's completed step).
    pub check: Option<SectionRenderer>,
}

impl std::fmt::Debug for IconSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let slot = |s: &Option<SectionRenderer>| s.as_ref().map(|_| "<glyph>");
        f.debug_struct("IconSet")
            .field("close", &slot(&self.close))
            .field("chevron_down", &slot(&self.chevron_down))
            .field("chevron_left", &slot(&self.chevron_left))
            .field("chevron_right", &slot(&self.chevron_right))
            .field("chevrons_left", &slot(&self.chevrons_left))
            .field("chevrons_right", &slot(&self.chevrons_right))
            .field("check", &slot(&self.check))
            .finish()
    }
}

impl IconSet {
    /// A glyph that draws a Tabler icon (outline) the component's CSS sizes:
    /// the icon's own inline `1em` size is left off.
    pub fn tabler(icon: TablerIcon) -> SectionRenderer {
        std::rc::Rc::new(move |scope: &mut RenderScope| {
            let svg = rinch_tabler_icons::render_tabler_icon_with_options(
                scope,
                icon,
                TablerIconOptions {
                    style: TablerIconStyle::Outline,
                    ..Default::default()
                },
            );
            svg.remove_attribute("style");
            svg
        })
    }

    /// Sets the close X.
    pub fn close(mut self, glyph: SectionRenderer) -> Self {
        self.close = Some(glyph);
        self
    }
    /// Sets the downward chevron.
    pub fn chevron_down(mut self, glyph: SectionRenderer) -> Self {
        self.chevron_down = Some(glyph);
        self
    }
    /// Sets the left chevron.
    pub fn chevron_left(mut self, glyph: SectionRenderer) -> Self {
        self.chevron_left = Some(glyph);
        self
    }
    /// Sets the right chevron.
    pub fn chevron_right(mut self, glyph: SectionRenderer) -> Self {
        self.chevron_right = Some(glyph);
        self
    }
    /// Sets the double left chevron.
    pub fn chevrons_left(mut self, glyph: SectionRenderer) -> Self {
        self.chevrons_left = Some(glyph);
        self
    }
    /// Sets the double right chevron.
    pub fn chevrons_right(mut self, glyph: SectionRenderer) -> Self {
        self.chevrons_right = Some(glyph);
        self
    }
    /// Sets the check.
    pub fn check(mut self, glyph: SectionRenderer) -> Self {
        self.check = Some(glyph);
        self
    }
}

/// The app's replacement for one glyph slot, drawn, if it set one.
fn replaced(
    scope: &mut RenderScope,
    slot: fn(&IconSet) -> &Option<SectionRenderer>,
) -> Option<NodeHandle> {
    let set = rinch_core::try_use_context::<IconSet>()?;
    let glyph = slot(&set).clone()?;
    // The app's closure, run untracked like every other app callback: a
    // signal it reads subscribes no effect of the component drawing it.
    Some(rinch_core::untracked_handler(|| glyph(scope)))
}

// =============================================================================
// DOM Rendering Variants (for fine-grained rendering)
// =============================================================================

/// Create a chevron up icon as a NodeHandle (for DOM rendering).
pub fn chevron_up_dom(__scope: &mut RenderScope) -> NodeHandle {
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("width", "10");
    svg.set_attribute("height", "10");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let path = rinch_macros::rsx! { path {} };
    path.set_attribute("d", "M18 15l-6-6-6 6");
    svg.append_child(&path);

    svg
}

/// Create a chevron down small icon as a NodeHandle (for DOM rendering).
pub fn chevron_down_small_dom(__scope: &mut RenderScope) -> NodeHandle {
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("width", "10");
    svg.set_attribute("height", "10");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let path = rinch_macros::rsx! { path {} };
    path.set_attribute("d", "M6 9l6 6 6-6");
    svg.append_child(&path);

    svg
}

/// Create an eye icon as a NodeHandle (for DOM rendering).
pub fn eye_dom(__scope: &mut RenderScope) -> NodeHandle {
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("width", "16");
    svg.set_attribute("height", "16");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");
    svg.set_attribute("stroke-linecap", "round");
    svg.set_attribute("stroke-linejoin", "round");

    let path = rinch_macros::rsx! { path {} };
    path.set_attribute("d", "M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z");
    svg.append_child(&path);

    let circle = rinch_macros::rsx! { circle {} };
    circle.set_attribute("cx", "12");
    circle.set_attribute("cy", "12");
    circle.set_attribute("r", "3");
    svg.append_child(&circle);

    svg
}

/// Create an eye-off icon as a NodeHandle (for DOM rendering).
pub fn eye_off_dom(__scope: &mut RenderScope) -> NodeHandle {
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("width", "16");
    svg.set_attribute("height", "16");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");
    svg.set_attribute("stroke-linecap", "round");
    svg.set_attribute("stroke-linejoin", "round");

    let path = rinch_macros::rsx! { path {} };
    path.set_attribute("d", "M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24");
    svg.append_child(&path);

    let line = rinch_macros::rsx! { line {} };
    line.set_attribute("x1", "1");
    line.set_attribute("y1", "1");
    line.set_attribute("x2", "23");
    line.set_attribute("y2", "23");
    svg.append_child(&line);

    svg
}

/// Create a checkmark icon as a NodeHandle (for DOM rendering).
pub fn checkmark_dom(__scope: &mut RenderScope) -> NodeHandle {
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 10 7");
    svg.set_attribute("fill", "none");
    svg.set_attribute("xmlns", "http://www.w3.org/2000/svg");

    let path = rinch_macros::rsx! { path {} };
    path.set_attribute("d", "M4 4.586L1.707 2.293A1 1 0 1 0 .293 3.707l3 3a1 1 0 0 0 1.414 0l5-5A1 1 0 0 0 8.293.293L4 4.586z");
    path.set_attribute("fill", "white");
    svg.append_child(&path);

    svg
}

/// Create an indeterminate icon as a NodeHandle (for DOM rendering).
pub fn indeterminate_dom(__scope: &mut RenderScope) -> NodeHandle {
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 12 12");
    svg.set_attribute("fill", "none");
    svg.set_attribute("xmlns", "http://www.w3.org/2000/svg");

    let rect = rinch_macros::rsx! { rect {} };
    rect.set_attribute("x", "2");
    rect.set_attribute("y", "5");
    rect.set_attribute("width", "8");
    rect.set_attribute("height", "2");
    rect.set_attribute("rx", "1");
    rect.set_attribute("fill", "white");
    svg.append_child(&rect);

    svg
}

/// Create a close icon with lines as a NodeHandle (for DOM rendering).
/// Used in drawer, modal, notification.
pub fn close_icon_lines_dom(__scope: &mut RenderScope) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.close) {
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let line1 = rinch_macros::rsx! { line {} };
    line1.set_attribute("x1", "18");
    line1.set_attribute("y1", "6");
    line1.set_attribute("x2", "6");
    line1.set_attribute("y2", "18");
    svg.append_child(&line1);

    let line2 = rinch_macros::rsx! { line {} };
    line2.set_attribute("x1", "6");
    line2.set_attribute("y1", "6");
    line2.set_attribute("x2", "18");
    line2.set_attribute("y2", "18");
    svg.append_child(&line2);

    svg
}

/// Create a close (X) icon as a NodeHandle (for DOM rendering).
pub fn close_icon_dom(__scope: &mut RenderScope, size: &str) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.close) {
        glyph.set_attribute("width", size);
        glyph.set_attribute("height", size);
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("width", size);
    svg.set_attribute("height", size);
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");
    svg.set_attribute("stroke-linecap", "round");
    svg.set_attribute("stroke-linejoin", "round");

    let path = rinch_macros::rsx! { path {} };
    path.set_attribute("d", "M18 6L6 18M6 6l12 12");
    svg.append_child(&path);

    svg
}

/// Create a chevron left icon as a NodeHandle (for DOM rendering).
pub fn chevron_left_dom(__scope: &mut RenderScope) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.chevron_left) {
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = rinch_macros::rsx! { polyline {} };
    polyline.set_attribute("points", "15 18 9 12 15 6");
    svg.append_child(&polyline);

    svg
}

/// Create a chevron right icon as a NodeHandle (for DOM rendering).
pub fn chevron_right_dom(__scope: &mut RenderScope) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.chevron_right) {
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = rinch_macros::rsx! { polyline {} };
    polyline.set_attribute("points", "9 18 15 12 9 6");
    svg.append_child(&polyline);

    svg
}

/// Create a double chevron left icon as a NodeHandle (for DOM rendering / pagination first).
pub fn chevrons_left_dom(__scope: &mut RenderScope) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.chevrons_left) {
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline1 = rinch_macros::rsx! { polyline {} };
    polyline1.set_attribute("points", "11 17 6 12 11 7");
    svg.append_child(&polyline1);

    let polyline2 = rinch_macros::rsx! { polyline {} };
    polyline2.set_attribute("points", "18 17 13 12 18 7");
    svg.append_child(&polyline2);

    svg
}

/// Create a double chevron right icon as a NodeHandle (for DOM rendering / pagination last).
pub fn chevrons_right_dom(__scope: &mut RenderScope) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.chevrons_right) {
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline1 = rinch_macros::rsx! { polyline {} };
    polyline1.set_attribute("points", "13 17 18 12 13 7");
    svg.append_child(&polyline1);

    let polyline2 = rinch_macros::rsx! { polyline {} };
    polyline2.set_attribute("points", "6 17 11 12 6 7");
    svg.append_child(&polyline2);

    svg
}

/// Create a check/complete icon as a NodeHandle (for DOM rendering / stepper completed step).
pub fn check_dom(__scope: &mut RenderScope) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.check) {
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = rinch_macros::rsx! { polyline {} };
    polyline.set_attribute("points", "20 6 9 17 4 12");
    svg.append_child(&polyline);

    svg
}

/// Create a chevron down icon with custom class as a NodeHandle (for DOM rendering / accordion).
pub fn chevron_down_dom(class: &str, __scope: &mut RenderScope) -> NodeHandle {
    if let Some(glyph) = replaced(__scope, |s| &s.chevron_down) {
        glyph.add_class(class);
        return glyph;
    }
    let svg = rinch_macros::rsx! { svg {} };
    svg.set_attribute("class", class);
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = rinch_macros::rsx! { polyline {} };
    polyline.set_attribute("points", "6 9 12 15 18 9");
    svg.append_child(&polyline);

    svg
}
