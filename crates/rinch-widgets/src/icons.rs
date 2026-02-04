//! SVG icons as structured Element nodes.
//!
//! Using structured nodes instead of HTML strings enables differential updates.

use rinch_core::Icon;
use rinch_core::dom::{NodeHandle, RenderScope};

// =============================================================================
// DOM Rendering Variants (for fine-grained rendering)
// =============================================================================

/// Create a chevron up icon as a NodeHandle (for DOM rendering).
pub fn chevron_up_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("width", "10");
    svg.set_attribute("height", "10");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let path = scope.create_element("path");
    path.set_attribute("d", "M18 15l-6-6-6 6");
    svg.append_child(&path);

    svg
}

/// Create a chevron down small icon as a NodeHandle (for DOM rendering).
pub fn chevron_down_small_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("width", "10");
    svg.set_attribute("height", "10");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let path = scope.create_element("path");
    path.set_attribute("d", "M6 9l6 6 6-6");
    svg.append_child(&path);

    svg
}

/// Create an eye icon as a NodeHandle (for DOM rendering).
pub fn eye_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("width", "16");
    svg.set_attribute("height", "16");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");
    svg.set_attribute("stroke-linecap", "round");
    svg.set_attribute("stroke-linejoin", "round");

    let path = scope.create_element("path");
    path.set_attribute("d", "M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z");
    svg.append_child(&path);

    let circle = scope.create_element("circle");
    circle.set_attribute("cx", "12");
    circle.set_attribute("cy", "12");
    circle.set_attribute("r", "3");
    svg.append_child(&circle);

    svg
}

/// Create an eye-off icon as a NodeHandle (for DOM rendering).
pub fn eye_off_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("width", "16");
    svg.set_attribute("height", "16");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");
    svg.set_attribute("stroke-linecap", "round");
    svg.set_attribute("stroke-linejoin", "round");

    let path = scope.create_element("path");
    path.set_attribute("d", "M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24");
    svg.append_child(&path);

    let line = scope.create_element("line");
    line.set_attribute("x1", "1");
    line.set_attribute("y1", "1");
    line.set_attribute("x2", "23");
    line.set_attribute("y2", "23");
    svg.append_child(&line);

    svg
}

/// Create a checkmark icon as a NodeHandle (for DOM rendering).
pub fn checkmark_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 10 7");
    svg.set_attribute("fill", "none");
    svg.set_attribute("xmlns", "http://www.w3.org/2000/svg");

    let path = scope.create_element("path");
    path.set_attribute("d", "M4 4.586L1.707 2.293A1 1 0 1 0 .293 3.707l3 3a1 1 0 0 0 1.414 0l5-5A1 1 0 0 0 8.293.293L4 4.586z");
    path.set_attribute("fill", "white");
    svg.append_child(&path);

    svg
}

/// Create an indeterminate icon as a NodeHandle (for DOM rendering).
pub fn indeterminate_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 12 12");
    svg.set_attribute("fill", "none");
    svg.set_attribute("xmlns", "http://www.w3.org/2000/svg");

    let rect = scope.create_element("rect");
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
pub fn close_icon_lines_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let line1 = scope.create_element("line");
    line1.set_attribute("x1", "18");
    line1.set_attribute("y1", "6");
    line1.set_attribute("x2", "6");
    line1.set_attribute("y2", "18");
    svg.append_child(&line1);

    let line2 = scope.create_element("line");
    line2.set_attribute("x1", "6");
    line2.set_attribute("y1", "6");
    line2.set_attribute("x2", "18");
    line2.set_attribute("y2", "18");
    svg.append_child(&line2);

    svg
}

/// Create a close (X) icon as a NodeHandle (for DOM rendering).
pub fn close_icon_dom(scope: &mut RenderScope, size: &str) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("width", size);
    svg.set_attribute("height", size);
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");
    svg.set_attribute("stroke-linecap", "round");
    svg.set_attribute("stroke-linejoin", "round");

    let path = scope.create_element("path");
    path.set_attribute("d", "M18 6L6 18M6 6l12 12");
    svg.append_child(&path);

    svg
}

/// Render an Icon to a NodeHandle (for DOM rendering).
///
/// This is the central function for converting the type-safe `Icon` enum
/// to actual SVG elements. All icons use consistent styling:
/// - 24x24 viewBox
/// - Stroke-based rendering
/// - `currentColor` for theming
pub fn render_icon(scope: &mut RenderScope, icon: Icon) -> NodeHandle {
    // Helper to create base SVG element
    fn svg_base_dom(scope: &mut RenderScope) -> NodeHandle {
        let svg = scope.create_element("svg");
        svg.set_attribute("viewBox", "0 0 24 24");
        svg.set_attribute("fill", "none");
        svg.set_attribute("stroke", "currentColor");
        svg.set_attribute("stroke-width", "2");
        svg.set_attribute("stroke-linecap", "round");
        svg.set_attribute("stroke-linejoin", "round");
        svg
    }

    match icon {
        Icon::Close => close_icon_lines_dom(scope),
        Icon::Check => {
            let svg = svg_base_dom(scope);
            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "20 6 9 17 4 12");
            svg.append_child(&polyline);
            svg
        }
        Icon::InfoCircle => {
            let svg = svg_base_dom(scope);
            let circle = scope.create_element("circle");
            circle.set_attribute("cx", "12");
            circle.set_attribute("cy", "12");
            circle.set_attribute("r", "10");
            svg.append_child(&circle);

            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "12");
            line1.set_attribute("y1", "16");
            line1.set_attribute("x2", "12");
            line1.set_attribute("y2", "12");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "12");
            line2.set_attribute("y1", "8");
            line2.set_attribute("x2", "12.01");
            line2.set_attribute("y2", "8");
            svg.append_child(&line2);
            svg
        }
        Icon::CheckCircle => {
            let svg = svg_base_dom(scope);
            let path = scope.create_element("path");
            path.set_attribute("d", "M22 11.08V12a10 10 0 1 1-5.93-9.14");
            svg.append_child(&path);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "22 4 12 14.01 9 11.01");
            svg.append_child(&polyline);
            svg
        }
        Icon::AlertCircle => {
            let svg = svg_base_dom(scope);
            let circle = scope.create_element("circle");
            circle.set_attribute("cx", "12");
            circle.set_attribute("cy", "12");
            circle.set_attribute("r", "10");
            svg.append_child(&circle);

            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "12");
            line1.set_attribute("y1", "8");
            line1.set_attribute("x2", "12");
            line1.set_attribute("y2", "12");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "12");
            line2.set_attribute("y1", "16");
            line2.set_attribute("x2", "12.01");
            line2.set_attribute("y2", "16");
            svg.append_child(&line2);
            svg
        }
        Icon::AlertTriangle => {
            let svg = svg_base_dom(scope);
            let path = scope.create_element("path");
            path.set_attribute("d", "M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z");
            svg.append_child(&path);

            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "12");
            line1.set_attribute("y1", "9");
            line1.set_attribute("x2", "12");
            line1.set_attribute("y2", "13");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "12");
            line2.set_attribute("y1", "17");
            line2.set_attribute("x2", "12.01");
            line2.set_attribute("y2", "17");
            svg.append_child(&line2);
            svg
        }
        Icon::XCircle => {
            let svg = svg_base_dom(scope);
            let circle = scope.create_element("circle");
            circle.set_attribute("cx", "12");
            circle.set_attribute("cy", "12");
            circle.set_attribute("r", "10");
            svg.append_child(&circle);

            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "15");
            line1.set_attribute("y1", "9");
            line1.set_attribute("x2", "9");
            line1.set_attribute("y2", "15");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "9");
            line2.set_attribute("y1", "9");
            line2.set_attribute("x2", "15");
            line2.set_attribute("y2", "15");
            svg.append_child(&line2);
            svg
        }
        Icon::ChevronUp => chevron_up_dom(scope),
        Icon::ChevronDown => {
            let svg = svg_base_dom(scope);
            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "6 9 12 15 18 9");
            svg.append_child(&polyline);
            svg
        }
        Icon::ChevronLeft => chevron_left_dom(scope),
        Icon::ChevronRight => chevron_right_dom(scope),
        Icon::ChevronsLeft => chevrons_left_dom(scope),
        Icon::ChevronsRight => chevrons_right_dom(scope),
        Icon::ArrowUp => {
            let svg = svg_base_dom(scope);
            let line = scope.create_element("line");
            line.set_attribute("x1", "12");
            line.set_attribute("y1", "19");
            line.set_attribute("x2", "12");
            line.set_attribute("y2", "5");
            svg.append_child(&line);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "5 12 12 5 19 12");
            svg.append_child(&polyline);
            svg
        }
        Icon::ArrowDown => {
            let svg = svg_base_dom(scope);
            let line = scope.create_element("line");
            line.set_attribute("x1", "12");
            line.set_attribute("y1", "5");
            line.set_attribute("x2", "12");
            line.set_attribute("y2", "19");
            svg.append_child(&line);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "19 12 12 19 5 12");
            svg.append_child(&polyline);
            svg
        }
        Icon::ArrowLeft => {
            let svg = svg_base_dom(scope);
            let line = scope.create_element("line");
            line.set_attribute("x1", "19");
            line.set_attribute("y1", "12");
            line.set_attribute("x2", "5");
            line.set_attribute("y2", "12");
            svg.append_child(&line);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "12 19 5 12 12 5");
            svg.append_child(&polyline);
            svg
        }
        Icon::ArrowRight => {
            let svg = svg_base_dom(scope);
            let line = scope.create_element("line");
            line.set_attribute("x1", "5");
            line.set_attribute("y1", "12");
            line.set_attribute("x2", "19");
            line.set_attribute("y2", "12");
            svg.append_child(&line);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "12 5 19 12 12 19");
            svg.append_child(&polyline);
            svg
        }
        Icon::Plus => {
            let svg = svg_base_dom(scope);
            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "12");
            line1.set_attribute("y1", "5");
            line1.set_attribute("x2", "12");
            line1.set_attribute("y2", "19");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "5");
            line2.set_attribute("y1", "12");
            line2.set_attribute("x2", "19");
            line2.set_attribute("y2", "12");
            svg.append_child(&line2);
            svg
        }
        Icon::Minus => {
            let svg = svg_base_dom(scope);
            let line = scope.create_element("line");
            line.set_attribute("x1", "5");
            line.set_attribute("y1", "12");
            line.set_attribute("x2", "19");
            line.set_attribute("y2", "12");
            svg.append_child(&line);
            svg
        }
        Icon::Search => {
            let svg = svg_base_dom(scope);
            let circle = scope.create_element("circle");
            circle.set_attribute("cx", "11");
            circle.set_attribute("cy", "11");
            circle.set_attribute("r", "8");
            svg.append_child(&circle);

            let line = scope.create_element("line");
            line.set_attribute("x1", "21");
            line.set_attribute("y1", "21");
            line.set_attribute("x2", "16.65");
            line.set_attribute("y2", "16.65");
            svg.append_child(&line);
            svg
        }
        Icon::Settings => {
            let svg = svg_base_dom(scope);
            let circle = scope.create_element("circle");
            circle.set_attribute("cx", "12");
            circle.set_attribute("cy", "12");
            circle.set_attribute("r", "3");
            svg.append_child(&circle);

            let path = scope.create_element("path");
            path.set_attribute("d", "M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z");
            svg.append_child(&path);
            svg
        }
        Icon::Edit => {
            let svg = svg_base_dom(scope);
            let path1 = scope.create_element("path");
            path1.set_attribute(
                "d",
                "M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7",
            );
            svg.append_child(&path1);

            let path2 = scope.create_element("path");
            path2.set_attribute(
                "d",
                "M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z",
            );
            svg.append_child(&path2);
            svg
        }
        Icon::Trash => {
            let svg = svg_base_dom(scope);
            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "3 6 5 6 21 6");
            svg.append_child(&polyline);

            let path = scope.create_element("path");
            path.set_attribute(
                "d",
                "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2",
            );
            svg.append_child(&path);

            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "10");
            line1.set_attribute("y1", "11");
            line1.set_attribute("x2", "10");
            line1.set_attribute("y2", "17");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "14");
            line2.set_attribute("y1", "11");
            line2.set_attribute("x2", "14");
            line2.set_attribute("y2", "17");
            svg.append_child(&line2);
            svg
        }
        Icon::User => {
            let svg = scope.create_element("svg");
            svg.set_attribute("class", "rinch-avatar__placeholder");
            svg.set_attribute("viewBox", "0 0 24 24");
            svg.set_attribute("fill", "currentColor");

            let path = scope.create_element("path");
            path.set_attribute("d", "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z");
            svg.append_child(&path);
            svg
        }
        Icon::Mail => {
            let svg = svg_base_dom(scope);
            let path = scope.create_element("path");
            path.set_attribute(
                "d",
                "M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z",
            );
            svg.append_child(&path);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "22,6 12,13 2,6");
            svg.append_child(&polyline);
            svg
        }
        Icon::Phone => {
            let svg = svg_base_dom(scope);
            let path = scope.create_element("path");
            path.set_attribute("d", "M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z");
            svg.append_child(&path);
            svg
        }
        Icon::Calendar => {
            let svg = svg_base_dom(scope);
            let rect = scope.create_element("rect");
            rect.set_attribute("x", "3");
            rect.set_attribute("y", "4");
            rect.set_attribute("width", "18");
            rect.set_attribute("height", "18");
            rect.set_attribute("rx", "2");
            rect.set_attribute("ry", "2");
            svg.append_child(&rect);

            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "16");
            line1.set_attribute("y1", "2");
            line1.set_attribute("x2", "16");
            line1.set_attribute("y2", "6");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "8");
            line2.set_attribute("y1", "2");
            line2.set_attribute("x2", "8");
            line2.set_attribute("y2", "6");
            svg.append_child(&line2);

            let line3 = scope.create_element("line");
            line3.set_attribute("x1", "3");
            line3.set_attribute("y1", "10");
            line3.set_attribute("x2", "21");
            line3.set_attribute("y2", "10");
            svg.append_child(&line3);
            svg
        }
        Icon::Clock => {
            let svg = svg_base_dom(scope);
            let circle = scope.create_element("circle");
            circle.set_attribute("cx", "12");
            circle.set_attribute("cy", "12");
            circle.set_attribute("r", "10");
            svg.append_child(&circle);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "12 6 12 12 16 14");
            svg.append_child(&polyline);
            svg
        }
        Icon::File => {
            let svg = svg_base_dom(scope);
            let path = scope.create_element("path");
            path.set_attribute(
                "d",
                "M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z",
            );
            svg.append_child(&path);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "13 2 13 9 20 9");
            svg.append_child(&polyline);
            svg
        }
        Icon::Folder => {
            let svg = svg_base_dom(scope);
            let path = scope.create_element("path");
            path.set_attribute(
                "d",
                "M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z",
            );
            svg.append_child(&path);
            svg
        }
        Icon::Image => {
            let svg = svg_base_dom(scope);
            let rect = scope.create_element("rect");
            rect.set_attribute("x", "3");
            rect.set_attribute("y", "3");
            rect.set_attribute("width", "18");
            rect.set_attribute("height", "18");
            rect.set_attribute("rx", "2");
            rect.set_attribute("ry", "2");
            svg.append_child(&rect);

            let circle = scope.create_element("circle");
            circle.set_attribute("cx", "8.5");
            circle.set_attribute("cy", "8.5");
            circle.set_attribute("r", "1.5");
            svg.append_child(&circle);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "21 15 16 10 5 21");
            svg.append_child(&polyline);
            svg
        }
        Icon::Link => {
            let svg = svg_base_dom(scope);
            let path1 = scope.create_element("path");
            path1.set_attribute(
                "d",
                "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71",
            );
            svg.append_child(&path1);

            let path2 = scope.create_element("path");
            path2.set_attribute(
                "d",
                "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71",
            );
            svg.append_child(&path2);
            svg
        }
        Icon::ExternalLink => {
            let svg = svg_base_dom(scope);
            let path = scope.create_element("path");
            path.set_attribute(
                "d",
                "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6",
            );
            svg.append_child(&path);

            let polyline = scope.create_element("polyline");
            polyline.set_attribute("points", "15 3 21 3 21 9");
            svg.append_child(&polyline);

            let line = scope.create_element("line");
            line.set_attribute("x1", "10");
            line.set_attribute("y1", "14");
            line.set_attribute("x2", "21");
            line.set_attribute("y2", "3");
            svg.append_child(&line);
            svg
        }
        Icon::Eye => eye_dom(scope),
        Icon::EyeOff => eye_off_dom(scope),
        Icon::Menu => {
            let svg = svg_base_dom(scope);
            let line1 = scope.create_element("line");
            line1.set_attribute("x1", "3");
            line1.set_attribute("y1", "12");
            line1.set_attribute("x2", "21");
            line1.set_attribute("y2", "12");
            svg.append_child(&line1);

            let line2 = scope.create_element("line");
            line2.set_attribute("x1", "3");
            line2.set_attribute("y1", "6");
            line2.set_attribute("x2", "21");
            line2.set_attribute("y2", "6");
            svg.append_child(&line2);

            let line3 = scope.create_element("line");
            line3.set_attribute("x1", "3");
            line3.set_attribute("y1", "18");
            line3.set_attribute("x2", "21");
            line3.set_attribute("y2", "18");
            svg.append_child(&line3);
            svg
        }
        Icon::MoreHorizontal => {
            let svg = svg_base_dom(scope);
            let circle1 = scope.create_element("circle");
            circle1.set_attribute("cx", "12");
            circle1.set_attribute("cy", "12");
            circle1.set_attribute("r", "1");
            svg.append_child(&circle1);

            let circle2 = scope.create_element("circle");
            circle2.set_attribute("cx", "19");
            circle2.set_attribute("cy", "12");
            circle2.set_attribute("r", "1");
            svg.append_child(&circle2);

            let circle3 = scope.create_element("circle");
            circle3.set_attribute("cx", "5");
            circle3.set_attribute("cy", "12");
            circle3.set_attribute("r", "1");
            svg.append_child(&circle3);
            svg
        }
        Icon::MoreVertical => {
            let svg = svg_base_dom(scope);
            let circle1 = scope.create_element("circle");
            circle1.set_attribute("cx", "12");
            circle1.set_attribute("cy", "12");
            circle1.set_attribute("r", "1");
            svg.append_child(&circle1);

            let circle2 = scope.create_element("circle");
            circle2.set_attribute("cx", "12");
            circle2.set_attribute("cy", "5");
            circle2.set_attribute("r", "1");
            svg.append_child(&circle2);

            let circle3 = scope.create_element("circle");
            circle3.set_attribute("cx", "12");
            circle3.set_attribute("cy", "19");
            circle3.set_attribute("r", "1");
            svg.append_child(&circle3);
            svg
        }
        Icon::Loader => {
            // Oval loader SVG
            let svg = scope.create_element("svg");
            svg.set_attribute("viewBox", "0 0 38 38");
            svg.set_attribute("xmlns", "http://www.w3.org/2000/svg");

            let outer_group = scope.create_element("g");
            outer_group.set_attribute("fill", "none");
            outer_group.set_attribute("fill-rule", "evenodd");

            let inner_group = scope.create_element("g");
            inner_group.set_attribute("transform", "translate(1 1)");
            inner_group.set_attribute("stroke-width", "2");
            inner_group.set_attribute("stroke", "currentColor");

            let circle = scope.create_element("circle");
            circle.set_attribute("stroke-opacity", ".25");
            circle.set_attribute("cx", "18");
            circle.set_attribute("cy", "18");
            circle.set_attribute("r", "18");
            inner_group.append_child(&circle);

            let animated_path = scope.create_element("path");
            animated_path.set_attribute("d", "M36 18c0-9.94-8.06-18-18-18");

            let animate_transform = scope.create_element("animateTransform");
            animate_transform.set_attribute("attributeName", "transform");
            animate_transform.set_attribute("type", "rotate");
            animate_transform.set_attribute("from", "0 18 18");
            animate_transform.set_attribute("to", "360 18 18");
            animate_transform.set_attribute("dur", "0.8s");
            animate_transform.set_attribute("repeatCount", "indefinite");
            animated_path.append_child(&animate_transform);

            inner_group.append_child(&animated_path);
            outer_group.append_child(&inner_group);
            svg.append_child(&outer_group);
            svg
        }
        Icon::Quote => {
            let svg = svg_base_dom(scope);
            let path1 = scope.create_element("path");
            path1.set_attribute("d", "M3 21c3 0 7-1 7-8V5c0-1.25-.756-2.017-2-2H4c-1.25 0-2 .75-2 1.972V11c0 1.25.75 2 2 2 1 0 1 0 1 1v1c0 1-1 2-2 2s-1 .008-1 1.031V21z");
            svg.append_child(&path1);

            let path2 = scope.create_element("path");
            path2.set_attribute("d", "M15 21c3 0 7-1 7-8V5c0-1.25-.757-2.017-2-2h-4c-1.25 0-2 .75-2 1.972V11c0 1.25.75 2 2 2h.75c0 2.25.25 4-2.75 4v3c0 1 0 1 1 1z");
            svg.append_child(&path2);
            svg
        }
    }
}

/// Create a chevron left icon as a NodeHandle (for DOM rendering).
pub fn chevron_left_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = scope.create_element("polyline");
    polyline.set_attribute("points", "15 18 9 12 15 6");
    svg.append_child(&polyline);

    svg
}

/// Create a chevron right icon as a NodeHandle (for DOM rendering).
pub fn chevron_right_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = scope.create_element("polyline");
    polyline.set_attribute("points", "9 18 15 12 9 6");
    svg.append_child(&polyline);

    svg
}

/// Create a double chevron left icon as a NodeHandle (for DOM rendering / pagination first).
pub fn chevrons_left_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline1 = scope.create_element("polyline");
    polyline1.set_attribute("points", "11 17 6 12 11 7");
    svg.append_child(&polyline1);

    let polyline2 = scope.create_element("polyline");
    polyline2.set_attribute("points", "18 17 13 12 18 7");
    svg.append_child(&polyline2);

    svg
}

/// Create a double chevron right icon as a NodeHandle (for DOM rendering / pagination last).
pub fn chevrons_right_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline1 = scope.create_element("polyline");
    polyline1.set_attribute("points", "13 17 18 12 13 7");
    svg.append_child(&polyline1);

    let polyline2 = scope.create_element("polyline");
    polyline2.set_attribute("points", "6 17 11 12 6 7");
    svg.append_child(&polyline2);

    svg
}

/// Create a check/complete icon as a NodeHandle (for DOM rendering / stepper completed step).
pub fn check_dom(scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = scope.create_element("polyline");
    polyline.set_attribute("points", "20 6 9 17 4 12");
    svg.append_child(&polyline);

    svg
}

/// Create a chevron down icon with custom class as a NodeHandle (for DOM rendering / accordion).
pub fn chevron_down_dom(class: &str, scope: &mut RenderScope) -> NodeHandle {
    let svg = scope.create_element("svg");
    svg.set_attribute("class", class);
    svg.set_attribute("viewBox", "0 0 24 24");
    svg.set_attribute("fill", "none");
    svg.set_attribute("stroke", "currentColor");
    svg.set_attribute("stroke-width", "2");

    let polyline = scope.create_element("polyline");
    polyline.set_attribute("points", "6 9 12 15 18 9");
    svg.append_child(&polyline);

    svg
}
