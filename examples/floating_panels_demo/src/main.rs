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
    let tb_x = Signal::new(16.0f32);
    let tb_y = Signal::new(16.0f32);
    let tb_w = Signal::new(320.0f32);
    let tb_h = Signal::new(80.0f32);

    // Properties panel state
    let props_x = Signal::new(900.0f32);
    let props_y = Signal::new(16.0f32);
    let props_w = Signal::new(280.0f32);
    let props_h = Signal::new(580.0f32);
    let props_visible = Signal::new(true);

    // Editor panel state
    let ed_x = Signal::new(16.0f32);
    let ed_y = Signal::new(120.0f32);
    let ed_w = Signal::new(420.0f32);
    let ed_h = Signal::new(500.0f32);
    let ed_visible = Signal::new(true);

    // Stress test panel state
    let stress_x = Signal::new(350.0f32);
    let stress_y = Signal::new(50.0f32);
    let stress_w = Signal::new(340.0f32);
    let stress_h = Signal::new(650.0f32);
    let stress_visible = Signal::new(true);

    // Slider signals for stress test
    let sliders: Vec<Signal<f64>> = (0..12).map(|i| Signal::new(i as f64 * 8.0)).collect();
    let checks: Vec<Signal<bool>> = (0..8).map(|i| Signal::new(i % 2 == 0)).collect();

    // Canvas state
    let active_tool = Signal::new("select");
    let canvas_color = Signal::new("#e7f5ff");
    let shape_x = Signal::new(250);
    let shape_y = Signal::new(150);
    let shape_size = Signal::new(120);
    let shape_color = Signal::new("#228be6");
    let shape_radius = Signal::new(8);

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
                        // The rich-text editor, embedded in a draggable panel — the
                        // caret and selection track correctly even after the panel
                        // moves. (`Editor {}` self-creates an `EditorHandle`.)
                        Text { size: "xs", weight: "bold", color: "dimmed", "Plain Text" }
                        Editor {
                            content: "<p>The quick brown fox jumps over the lazy dog.</p>\
                                <p>Click to place the cursor, select text, and use the arrow keys to navigate.</p>\
                                <p>Drag this panel by its title bar to test that the cursor and selection still work at a different position.</p>",
                        }

                        Divider {}

                        Text { size: "xs", weight: "bold", color: "dimmed", "Rich Text" }
                        Editor {
                            content: "<p>This has <strong>bold</strong> and <em>italic</em> text, plus <span style=\"color: #228be6\">colored</span> and <u>underlined</u> spans.</p>\
                                <p>Second paragraph. <strong><em>Bold italic</em></strong> mixed with <strong><span style=\"color: #e64980\">bold pink</span></strong> text.</p>",
                        }
                    }
                }
            }

            // === Stress Test Panel (heavy!) ===
            if stress_visible.get() {
                FloatingPanel {
                    title: "Stress Test (heavy panel)",
                    x: Some(stress_x),
                    y: Some(stress_y),
                    width: Some(stress_w),
                    height: Some(stress_h),
                    on_close: move || stress_visible.set(false),

                    Stack { gap: "xs", p: "xs",
                        Text { size: "xs", color: "dimmed", weight: "bold", "12 Sliders" }
                        {stress_sliders(__scope, &sliders)}

                        Divider {}
                        Text { size: "xs", color: "dimmed", weight: "bold", "8 Checkboxes" }
                        {stress_checks(__scope, &checks)}

                        Divider {}
                        Text { size: "xs", color: "dimmed", weight: "bold", "Nested Components" }

                        Paper { p: "sm", radius: "sm", with_border: true,
                            Stack { gap: "xs",
                                Alert { color: "blue", "This panel has 12 sliders, 8 checkboxes, badges, alerts, and nested papers. Drag it around — it should be smooth." }
                                Group { gap: "xs",
                                    Badge { color: "green", variant: "filled", "Fast" }
                                    Badge { color: "blue", variant: "light", "Optimized" }
                                    Badge { color: "grape", variant: "outline", "No Stylo" }
                                    Badge { color: "orange", variant: "dot", "No IFC" }
                                }
                                Progress { value: 73.0, color: "teal" }
                                TextInput { size: "xs", placeholder: "Type something..." }
                                TextInput { size: "xs", placeholder: "Another input..." }
                            }
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

/// Render 12 sliders with colored badges for the stress test panel.
fn stress_sliders(__scope: &mut RenderScope, sliders: &[Signal<f64>]) -> NodeHandle {
    let colors = ["blue", "teal", "violet", "orange", "pink"];
    let container = __scope.create_element("div");
    for (i, &sig) in sliders.iter().enumerate() {
        let color = colors[i % colors.len()];
        let row = rsx! {
            Stack { gap: "2",
                Group { gap: "xs", justify: "between",
                    Text { size: "xs", {format!("Param {}", i + 1)} }
                    Badge { size: "xs", color: {color},
                        {move || format!("{:.0}", sig.get())}
                    }
                }
                Slider {
                    color: {color},
                    value_signal: Some(sig),
                    onchange: move |v| sig.set(v),
                }
            }
        };
        container.append_child(&row);
    }
    container
}

/// Render 8 checkboxes for the stress test panel.
fn stress_checks(__scope: &mut RenderScope, checks: &[Signal<bool>]) -> NodeHandle {
    let labels = [
        "Enable feature",
        "Show preview",
        "Auto-save",
        "Notify on change",
        "Debug mode",
        "Cache results",
        "Lazy loading",
        "Animations",
    ];
    let container = __scope.create_element("div");
    for (i, &sig) in checks.iter().enumerate() {
        let label = labels[i % labels.len()];
        let row = rsx! {
            Checkbox {
                label: {format!("Option {} — {}", i + 1, label)},
                size: "sm",
                checked_fn: move || sig.get(),
                onchange: move || sig.update(|v| *v = !*v),
            }
        };
        container.append_child(&row);
    }
    container
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
