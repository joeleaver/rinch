//! ContentEditable Spike
//!
//! Tests contenteditable support for the rich-text editor project.
//!
//! ## Tests:
//! 1. Basic contenteditable div - can we type?
//! 2. Input event emission - does the runtime emit events for contenteditable changes?
//! 3. Cursor/caret rendering - does the cursor display correctly?
//! 4. Text selection - can we select text with mouse/keyboard?
//! 5. Keyboard shortcuts - can we intercept Ctrl+B, etc.?
//! 6. Programmatic DOM manipulation - can we modify content from code?

use rinch::prelude::*;
use rinch::{WindowProps, run_with_window_props};

#[component]
fn app() -> NodeHandle {
    // State for tracking what happens
    let event_log = use_signal(Vec::<String>::new);
    let content_value = use_signal(|| String::from("Initial content"));
    let input_count = use_signal(|| 0_usize);

    // Build the UI using raw DOM API for more control
    let root = __scope.create_element("div");
    root.set_attribute(
        "style",
        "padding: 20px; max-width: 1200px; margin: 0 auto; font-family: sans-serif;",
    );

    // Title
    let title = __scope.create_element("h1");
    title.set_text("ContentEditable Spike Test");
    root.append_child(&title);

    let subtitle = __scope.create_element("p");
    subtitle.set_attribute("style", "color: #666; margin-bottom: 20px;");
    subtitle
        .set_text("Testing contenteditable support for the rich-text editor project");
    root.append_child(&subtitle);

    // Grid container
    let grid = __scope.create_element("div");
    grid.set_attribute(
        "style",
        "display: grid; grid-template-columns: 1fr 1fr; gap: 20px;",
    );

    // === LEFT COLUMN: Tests ===
    let left_col = __scope.create_element("div");

    // Test 1: Basic contenteditable
    {
        let panel = create_panel(
            __scope,
            "Test 1: Basic ContentEditable",
            "A simple div with contenteditable. Try typing, selecting, and formatting.",
        );

        let editable = __scope.create_element("div");
        editable.set_attribute("contenteditable", "true");
        editable.set_attribute("style", "border: 2px solid #4c6ef5; padding: 12px; min-height: 100px; border-radius: 4px; background: white; outline: none;");
        editable.set_text(
            "Initial content - try editing me! Type here to see if contenteditable works.",
        );
        panel.append_child(&editable);

        left_col.append_child(&panel);
    }

    // Test 2: ContentEditable with input handler
    {
        let panel = create_panel(
            __scope,
            "Test 2: ContentEditable with Input Handler",
            "Testing if the runtime emits input events for contenteditable divs.",
        );

        let editable = __scope.create_element("div");
        editable.set_attribute("contenteditable", "true");
        editable.set_attribute("style", "border: 2px solid #40c057; padding: 12px; min-height: 100px; border-radius: 4px; background: white; outline: none;");

        // Register input handler
        let handler_id = __scope.register_input_handler(move |value: String| {
            event_log.update(|l| {
                l.push(format!(
                    "[CONTENTEDITABLE] value.len()={}: \"{}\"",
                    value.len(),
                    if value.len() > 60 {
                        format!("{}...", &value[..60])
                    } else {
                        value.clone()
                    }
                ));
                if l.len() > 30 {
                    l.remove(0);
                }
            });
            input_count.update(|c| *c += 1);
            content_value.set(value);
        });
        editable.set_attribute("data-oninput", &handler_id.to_string());
        editable.set_text("Type here to test input events...");

        panel.append_child(&editable);
        left_col.append_child(&panel);
    }

    // Test 3: ContentEditable with HTML content
    {
        let panel = create_panel(
            __scope,
            "Test 3: Pre-formatted Content",
            "Testing contenteditable with existing HTML formatting.",
        );

        let editable = __scope.create_element("div");
        editable.set_attribute("contenteditable", "true");
        editable.set_attribute("style", "border: 2px solid #fab005; padding: 12px; min-height: 100px; border-radius: 4px; background: white; outline: none;");

        // Create formatted content
        let p1 = __scope.create_element("p");
        let strong = __scope.create_element("strong");
        strong.set_text("Bold text");
        p1.append_child(&strong);
        let t1 = __scope.create_text(" and ");
        p1.append_child(&t1);
        let em = __scope.create_element("em");
        em.set_text("italic text");
        p1.append_child(&em);
        let t2 = __scope.create_text(" in a paragraph.");
        p1.append_child(&t2);

        let p2 = __scope.create_element("p");
        p2.set_text("Second paragraph for testing cursor movement.");

        editable.append_child(&p1);
        editable.append_child(&p2);

        panel.append_child(&editable);
        left_col.append_child(&panel);
    }

    // Test 4: Regular input for comparison
    {
        let panel = create_panel(
            __scope,
            "Test 4: Regular Input (Comparison)",
            "A regular input element - we know the runtime handles these correctly.",
        );

        let input = __scope.create_element("input");
        input.set_attribute("type", "text");
        input.set_attribute("placeholder", "Type here for comparison...");
        input.set_attribute("style", "width: 100%; padding: 12px; border: 2px solid #868e96; border-radius: 4px; font-size: 16px;");

        let handler_id = __scope.register_input_handler(move |value: String| {
            event_log.update(|l| {
                l.push(format!("[REGULAR INPUT] \"{}\"", value));
                if l.len() > 30 {
                    l.remove(0);
                }
            });
        });
        input.set_attribute("data-oninput", &handler_id.to_string());

        panel.append_child(&input);
        left_col.append_child(&panel);
    }

    // Test 5: Textarea for comparison
    {
        let panel = create_panel(
            __scope,
            "Test 5: Textarea (Comparison)",
            "A textarea element - multi-line input that the runtime handles.",
        );

        let textarea = __scope.create_element("textarea");
        textarea.set_attribute("placeholder", "Type multiple lines here...");
        textarea.set_attribute("style", "width: 100%; min-height: 80px; padding: 12px; border: 2px solid #868e96; border-radius: 4px; font-size: 16px; resize: vertical;");

        let handler_id = __scope.register_input_handler(move |value: String| {
            event_log.update(|l| {
                l.push(format!(
                    "[TEXTAREA] lines={}, len={}",
                    value.lines().count(),
                    value.len()
                ));
                if l.len() > 30 {
                    l.remove(0);
                }
            });
        });
        textarea.set_attribute("data-oninput", &handler_id.to_string());

        panel.append_child(&textarea);
        left_col.append_child(&panel);
    }

    grid.append_child(&left_col);

    // === RIGHT COLUMN: Status and Log ===
    let right_col = __scope.create_element("div");

    // Status panel
    {
        let panel = create_panel(__scope, "Status", "Current state from event handlers");

        let badge_container = __scope.create_element("div");
        badge_container.set_attribute("style", "margin-bottom: 12px;");

        let badge = __scope.create_element("span");
        badge.set_attribute("style", "background: #4c6ef5; color: white; padding: 4px 12px; border-radius: 12px; font-size: 14px;");

        // Reactive badge text
        let badge_clone = badge.clone();
        __scope.create_effect(move || {
            badge_clone.set_text(&format!("Input Events: {}", input_count.get()));
        });

        badge_container.append_child(&badge);
        panel.append_child(&badge_container);

        let content_label = __scope.create_element("div");
        content_label.set_attribute("style", "font-weight: 600; margin-bottom: 4px;");
        content_label.set_text("Last Content Value:");
        panel.append_child(&content_label);

        let content_box = __scope.create_element("pre");
        content_box.set_attribute("style", "background: #f1f3f5; padding: 8px; border-radius: 4px; font-size: 12px; max-height: 100px; overflow-y: auto; white-space: pre-wrap; word-break: break-all;");

        let content_box_clone = content_box.clone();
        __scope.create_effect(move || {
            let content = content_value.get();
            let display = if content.len() > 300 {
                format!("{}... ({} chars total)", &content[..300], content.len())
            } else {
                content
            };
            content_box_clone.set_text(&display);
        });

        panel.append_child(&content_box);
        right_col.append_child(&panel);
    }

    // Clear button
    {
        let panel = __scope.create_element("div");
        panel.set_attribute("style", "margin-bottom: 16px;");

        let btn = __scope.create_element("button");
        btn.set_attribute("style", "padding: 10px 20px; background: #868e96; color: white; border: none; border-radius: 4px; cursor: pointer; font-size: 14px;");
        btn.set_text("Clear Event Log");

        let handler_id = __scope.register_handler(move || {
            event_log.set(Vec::new());
        });
        btn.set_attribute("data-rid", &handler_id.0.to_string());

        panel.append_child(&btn);
        right_col.append_child(&panel);
    }

    // Event log
    {
        let panel = create_panel(__scope, "Event Log", "Events received from input handlers");

        let log_box = __scope.create_element("pre");
        log_box.set_attribute("style", "background: #212529; color: #f8f9fa; padding: 12px; border-radius: 4px; font-size: 11px; max-height: 400px; overflow-y: auto; white-space: pre-wrap;");

        let log_box_clone = log_box.clone();
        __scope.create_effect(move || {
            let log = event_log.get();
            let display = if log.is_empty() {
                String::from("No events yet.\n\nTry:\n1. Typing in the contenteditable areas (Tests 1-3)\n2. Typing in the regular input (Test 4)\n3. Compare which ones emit events")
            } else {
                log.join("\n")
            };
            log_box_clone.set_text(&display);
        });

        panel.append_child(&log_box);
        right_col.append_child(&panel);
    }

    // Instructions
    {
        let panel = create_panel(__scope, "Testing Instructions", "");

        let instructions = [
            (
                "1. Basic Editing:",
                "Click in Test 1 area and type. Does text appear? Does cursor work?",
            ),
            (
                "2. Event Emission:",
                "Type in Test 2 area. Watch the event log - do [CONTENTEDITABLE] events appear?",
            ),
            (
                "3. Selection:",
                "Try selecting text with mouse drag and Shift+Arrow keys.",
            ),
            (
                "4. Compare:",
                "Type in Tests 4 & 5 - these SHOULD show events.",
            ),
            (
                "5. Keyboard:",
                "Try Ctrl+A (select all), Ctrl+B, Ctrl+I in contenteditable.",
            ),
        ];

        for (title, desc) in instructions {
            let item = __scope.create_element("div");
            item.set_attribute("style", "margin-bottom: 8px; font-size: 13px;");

            let strong = __scope.create_element("strong");
            strong.set_text(title);
            item.append_child(&strong);

            let text = __scope.create_text(&format!(" {}", desc));
            item.append_child(&text);

            panel.append_child(&item);
        }

        right_col.append_child(&panel);
    }

    // Findings section
    {
        let panel = create_panel(__scope, "Expected vs Actual", "Document your findings here");

        let findings = __scope.create_element("div");
        findings.set_attribute("style", "font-size: 13px;");
        findings.set_text("After testing, document:\n- Does contenteditable allow typing?\n- Does cursor/caret render?\n- Do input events fire?\n- Does selection work?\n- What keyboard shortcuts work?");

        panel.append_child(&findings);
        right_col.append_child(&panel);
    }

    grid.append_child(&right_col);
    root.append_child(&grid);

    root
}

/// Helper to create a styled panel
fn create_panel(scope: &mut RenderScope, title: &str, description: &str) -> NodeHandle {
    let panel = scope.create_element("div");
    panel.set_attribute("style", "background: white; border: 1px solid #dee2e6; border-radius: 8px; padding: 16px; margin-bottom: 16px; box-shadow: 0 1px 3px rgba(0,0,0,0.1);");

    let h3 = scope.create_element("h3");
    h3.set_attribute("style", "margin: 0 0 4px 0; font-size: 16px;");
    h3.set_text(title);
    panel.append_child(&h3);

    if !description.is_empty() {
        let desc = scope.create_element("p");
        desc.set_attribute(
            "style",
            "margin: 0 0 12px 0; color: #868e96; font-size: 13px;",
        );
        desc.set_text(description);
        panel.append_child(&desc);
    }

    panel
}

fn main() {
    // Set up tracing for debug output
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();

    let window_props = WindowProps {
        title: "ContentEditable Spike".into(),
        width: 1400,
        height: 900,
        ..Default::default()
    };

    run_with_window_props(app, window_props, None);
}
