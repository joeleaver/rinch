//! Floating Panels Demo
//!
//! A canvas app with a floating toolbar and floating properties panel.
//! Demonstrates drag-to-move, resize, and close interactions.
//!
//! Run with: cargo run -p floating-panels-demo

use rinch::prelude::*;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

fn main() {
    let _ = tracing_subscriber::fmt::try_init();

    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    run_with_theme("Floating Panels", 1200, 800, app, theme);
}

#[component]
fn app() -> NodeHandle {
    // Toolbar panel state
    let tb_x = use_signal(|| 16.0f32);
    let tb_y = use_signal(|| 16.0f32);
    let tb_w = use_signal(|| 320.0f32);
    let tb_h = use_signal(|| 80.0f32);

    // Properties panel state
    let props_x = use_signal(|| 900.0f32);
    let props_y = use_signal(|| 16.0f32);
    let props_w = use_signal(|| 280.0f32);
    let props_h = use_signal(|| 580.0f32);
    let props_visible = use_signal(|| true);

    // Editor panel state
    let ed_x = use_signal(|| 16.0f32);
    let ed_y = use_signal(|| 120.0f32);
    let ed_w = use_signal(|| 420.0f32);
    let ed_h = use_signal(|| 500.0f32);
    let ed_visible = use_signal(|| true);

    // Canvas state
    let active_tool = use_signal(|| "select");
    let canvas_color = use_signal(|| "#e7f5ff");
    let shape_x = use_signal(|| 250);
    let shape_y = use_signal(|| 150);
    let shape_size = use_signal(|| 120);
    let shape_color = use_signal(|| "#228be6");
    let shape_radius = use_signal(|| 8);

    rsx! {
        // Full-window relative container
        div { style: "position: relative; width: 1200px; min-height: 800px; overflow: hidden; background-color: var(--rinch-color-gray-1)",

            // Canvas area (the "artboard")
            div {
                style: {move || format!(
                    "position: absolute; top: 8px; left: 8px; width: 1184px; height: 756px; background-color: {}; border-radius: 8px; border: 1px solid var(--rinch-color-gray-3); overflow: hidden",
                    canvas_color.get()
                )},

                // Grid pattern overlay for a design-tool feel
                div { style: "position: absolute; inset: 0; opacity: 0.15; background-image: radial-gradient(circle, var(--rinch-color-gray-5) 1px, transparent 1px); background-size: 20px 20px" }

                // A shape on the canvas
                div {
                    style: {move || format!(
                        "width: {}px; height: {}px; background-color: {}; border-radius: {}px; position: absolute; left: {}px; top: {}px; box-shadow: 0 4px 16px rgba(0,0,0,0.12), 0 1px 3px rgba(0,0,0,0.08)",
                        shape_size.get(), shape_size.get(), shape_color.get(), shape_radius.get(), shape_x.get(), shape_y.get()
                    )},
                }
            }

            // Status bar at bottom
            div { style: "position: absolute; top: 768px; left: 0; width: 1200px; height: 32px; background: var(--rinch-color-gray-0); border-top: 1px solid var(--rinch-color-gray-3); display: flex; align-items: center; padding-left: 12px; padding-right: 12px; justify-content: space-between",
                Text { size: "xs", color: "dimmed",
                    {move || format!("Tool: {}  |  Shape: {}x{} at ({}, {})", active_tool.get(), shape_size.get(), shape_size.get(), shape_x.get(), shape_y.get())}
                }
                Text { size: "xs", color: "dimmed", "Floating Panels Demo" }
            }

            // === Floating Toolbar ===
            FloatingPanel {
                title: "Tools",
                x: Some(tb_x),
                y: Some(tb_y),
                width: Some(tb_w),
                height: Some(tb_h),
                resizable: false,

                Group { gap: "xs",
                    {tool_icon(__scope, TablerIcon::Pointer, "select", active_tool)}
                    {tool_icon(__scope, TablerIcon::ArrowsMove, "move", active_tool)}
                    {tool_icon(__scope, TablerIcon::Square, "rect", active_tool)}
                    {tool_icon(__scope, TablerIcon::Circle, "circle", active_tool)}

                    // Separator
                    div { style: "width: 1px; height: 28px; background: var(--rinch-color-gray-3); margin-left: 4px; margin-right: 4px" }

                    {tool_icon(__scope, TablerIcon::Palette, "palette", active_tool)}
                    {tool_icon(__scope, TablerIcon::HandClick, "hand", active_tool)}
                }
            }

            // === Floating Editor Panel ===
            if ed_visible.get() {
                FloatingPanel {
                    title: "Editor",
                    x: Some(ed_x),
                    y: Some(ed_y),
                    width: Some(ed_w),
                    height: Some(ed_h),
                    on_close: move || ed_visible.set(false),

                    Stack { gap: "md",
                        // Plaintext contenteditable
                        Text { size: "xs", weight: "bold", color: "dimmed", "Plain Text" }
                        div {
                            contenteditable: "plaintext-only",
                            style: "border: 1px solid var(--rinch-color-gray-3); padding: 10px; min-height: 80px; font-size: 14px; line-height: 1.6; outline: none; border-radius: var(--rinch-radius-sm); background: var(--rinch-color-gray-0); overflow-wrap: break-word; cursor: text",
                            "The quick brown fox jumps over the lazy dog." br {} br {} "Try clicking to place the cursor, selecting text, and using arrow keys to navigate." br {} br {} "Drag this panel by its title bar to test that cursor and selection still work at a different position."
                        }

                        Divider {}

                        // Rich contenteditable
                        Text { size: "xs", weight: "bold", color: "dimmed", "Rich Text" }
                        div {
                            contenteditable: "true",
                            style: "border: 1px solid var(--rinch-color-gray-3); padding: 10px; min-height: 100px; font-size: 14px; line-height: 1.6; outline: none; border-radius: var(--rinch-radius-sm); background: var(--rinch-color-gray-0); overflow-wrap: break-word; cursor: text",
                            "This has "
                            span { style: "font-weight: bold;", "bold" }
                            " and "
                            span { style: "font-style: italic;", "italic" }
                            " text, plus "
                            span { style: "color: #228be6;", "colored" }
                            " and "
                            span { style: "text-decoration: underline;", "underlined" }
                            " spans."
                            br {}
                            br {}
                            "Second paragraph. "
                            span { style: "font-weight: bold; font-style: italic;", "Bold italic" }
                            " mixed with "
                            span { style: "color: #e64980; font-weight: bold;", "bold pink" }
                            " text."
                        }
                    }
                }
            }

            // === Floating Properties Panel ===
            if props_visible.get() {
                FloatingPanel {
                    title: "Properties",
                    x: Some(props_x),
                    y: Some(props_y),
                    width: Some(props_w),
                    height: Some(props_h),
                    on_close: move || props_visible.set(false),

                    Stack { gap: "sm",
                        // Position section
                        {section_header(__scope, "Position")}
                        Group { gap: "sm",
                            {labeled_input(__scope, "X", shape_x)}
                            {labeled_input(__scope, "Y", shape_y)}
                        }

                        Divider {}

                        // Size section
                        {section_header(__scope, "Size")}
                        Group { gap: "sm",
                            {labeled_input(__scope, "W", shape_size)}
                            {labeled_input(__scope, "R", shape_radius)}
                        }

                        Divider {}

                        // Appearance section
                        {section_header(__scope, "Appearance")}

                        Stack { gap: "xs",
                            Text { size: "xs", color: "dimmed", "Shape Color" }
                            Group { gap: "xs",
                                {color_swatch(__scope, "#228be6", shape_color)}
                                {color_swatch(__scope, "#40c057", shape_color)}
                                {color_swatch(__scope, "#fa5252", shape_color)}
                                {color_swatch(__scope, "#fab005", shape_color)}
                                {color_swatch(__scope, "#7950f2", shape_color)}
                                {color_swatch(__scope, "#212529", shape_color)}
                            }
                        }

                        Stack { gap: "xs",
                            Text { size: "xs", color: "dimmed", "Canvas" }
                            Group { gap: "xs",
                                {color_swatch(__scope, "#e7f5ff", canvas_color)}
                                {color_swatch(__scope, "#ebfbee", canvas_color)}
                                {color_swatch(__scope, "#fff9db", canvas_color)}
                                {color_swatch(__scope, "#f8f9fa", canvas_color)}
                                {color_swatch(__scope, "#fff5f5", canvas_color)}
                                {color_swatch(__scope, "#ffffff", canvas_color)}
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Section header label for the properties panel.
fn section_header(__scope: &mut RenderScope, label: &str) -> NodeHandle {
    rsx! {
        Text { size: "xs", weight: "bold", color: "dimmed", {label} }
    }
}

/// A labeled numeric input field.
fn labeled_input(__scope: &mut RenderScope, label: &str, signal: Signal<i32>) -> NodeHandle {
    rsx! {
        Stack { gap: "2",
            Text { size: "xs", color: "dimmed", {label} }
            TextInput {
                size: "xs",
                value_fn: move || signal.get().to_string(),
                oninput: move |v: String| { if let Ok(n) = v.parse() { signal.set(n); } },
            }
        }
    }
}

/// A toolbar button with an icon that highlights when active.
/// Uses reactive style on an HTML element to avoid the reactive component prop re-parenting bug.
fn tool_icon(
    __scope: &mut RenderScope,
    icon: TablerIcon,
    tool_id: &'static str,
    active_tool: Signal<&'static str>,
) -> NodeHandle {
    let icon_node = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
    rsx! {
        div {
            style: {move || format!(
                "display: flex; align-items: center; justify-content: center; width: 36px; height: 32px; border-radius: var(--rinch-radius-sm); cursor: pointer; user-select: none; transition: background 0.15s; {}",
                if active_tool.get() == tool_id {
                    "background: var(--rinch-primary-color); color: white"
                } else {
                    "background: transparent; color: var(--rinch-color-gray-7)"
                }
            )},
            onclick: move || active_tool.set(tool_id),
            {icon_node}
        }
    }
}

/// A color swatch button with a check overlay when selected.
fn color_swatch(
    __scope: &mut RenderScope,
    color: &'static str,
    target: Signal<&'static str>,
) -> NodeHandle {
    rsx! {
        div {
            style: {move || format!(
                "width: 28px; height: 28px; border-radius: var(--rinch-radius-sm); background-color: {}; cursor: pointer; border: 2px solid {}; box-shadow: {}; transition: border-color 0.15s, box-shadow 0.15s",
                color,
                if target.get() == color { "var(--rinch-primary-color)" } else { "var(--rinch-color-gray-3)" },
                if target.get() == color { "0 0 0 2px var(--rinch-color-blue-2)" } else { "none" }
            )},
            onclick: move || target.set(color),
        }
    }
}
