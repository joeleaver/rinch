//! Hidden Textarea Spike
//!
//! Tests the "hidden textarea + custom DOM rendering" approach for a rich-text editor.
//!
//! Core concept: Use a `<textarea>` to capture all text input (typing, IME, clipboard)
//! via blitz's native handling, then read the value and render it ourselves in a
//! custom visible div with formatting and cursor.
//!
//! ## What this proves:
//! 1. Textarea input events flow to our handler
//! 2. We can reactively render the same content in a separate DOM tree
//! 3. The two stay in sync
//! 4. Custom cursor rendering works alongside textarea input capture

use rinch::prelude::*;
use rinch::{WindowProps, run_with_window_props};

#[component]
fn app() -> NodeHandle {
    let content = use_signal(String::new);
    let cursor_pos = use_signal(|| 0_usize);
    let event_log = use_signal(Vec::<String>::new);
    let input_count = use_signal(|| 0_usize);

    let root = __scope.create_element("div");
    root.set_attribute("style", "padding: 20px; max-width: 900px; margin: 0 auto; font-family: sans-serif; background: #f8f9fa; min-height: 100vh;");

    // Title
    let title = __scope.create_element("h1");
    title.set_attribute("style", "margin: 0 0 4px 0;");
    title.set_text("Hidden Textarea Spike");
    root.append_child(&title);

    let subtitle = __scope.create_element("p");
    subtitle.set_attribute("style", "color: #666; margin-bottom: 20px;");
    subtitle.set_text("Testing: textarea captures input -> custom div renders content with cursor");
    root.append_child(&subtitle);

    // === SECTION 0: Button Click Test (sanity check) ===
    {
        let panel = create_panel(
            __scope,
            "0. Button Click Test (Sanity Check)",
            "Click the button to verify reactive text updates work at all.",
        );

        let click_count = use_signal(|| 0_usize);

        let display = __scope.create_element("div");
        display.set_attribute("style", "font-size: 18px; padding: 8px; margin-bottom: 8px; background: #e3fafc; border-radius: 4px;");

        let display_clone = display.clone();
        __scope.create_effect({
            move || {
                let count = click_count.get();
                display_clone.set_text(&format!("Button clicked {} times", count));
            }
        });

        let btn = __scope.create_element("button");
        btn.set_attribute("style", "padding: 10px 20px; background: #4c6ef5; color: white; border: none; border-radius: 4px; cursor: pointer; font-size: 16px;");
        btn.set_text("Click Me!");

        let handler_id = __scope.register_handler({
            move || {
                tracing::info!("BUTTON CLICKED");
                click_count.update(|c| *c += 1);
            }
        });
        btn.set_attribute("data-rid", &handler_id.0.to_string());

        panel.append_child(&display);
        panel.append_child(&btn);
        root.append_child(&panel);
    }

    // === SECTION 1: Input Capture (Textarea) ===
    {
        let panel = create_panel(
            __scope,
            "1. Input Capture (Textarea)",
            "Type here. The textarea handles input/IME/clipboard natively via blitz.",
        );

        let textarea = __scope.create_element("textarea");
        textarea.set_attribute("style", "width: 100%; min-height: 100px; padding: 12px; font-family: monospace; font-size: 16px; border: 2px solid #4c6ef5; border-radius: 4px; box-sizing: border-box;");
        textarea.set_attribute("placeholder", "Type here...");

        let handler_id = __scope.register_input_handler({
            move |value: String| {
                let len = value.len();
                let lines = value.lines().count();
                tracing::info!(
                    "SPIKE INPUT HANDLER: len={}, lines={}, value='{}'",
                    len,
                    lines,
                    &value[..value.len().min(50)]
                );
                event_log.update(|l| {
                    l.push(format!("INPUT: len={}, lines={}", len, lines));
                    if l.len() > 20 {
                        l.remove(0);
                    }
                });
                input_count.update(|c| *c += 1);
                // Move cursor to end of content on each input
                cursor_pos.set(len);
                tracing::info!("SPIKE: About to set content signal");
                content.set(value);
                tracing::info!("SPIKE: Content signal set, effects should have run");
            }
        });
        textarea.set_attribute("data-oninput", &handler_id.to_string());

        panel.append_child(&textarea);
        root.append_child(&panel);
    }

    // === SECTION 1b: HIDDEN Textarea Test ===
    // This is the critical test: can we hide the textarea offscreen and still capture input?
    {
        let panel = create_panel(
            __scope,
            "1b. HIDDEN Textarea Test",
            "Click the green box below, then type. The textarea is offscreen but should still capture input.",
        );

        let hidden_content = use_signal(String::new);
        let hidden_count = use_signal(|| 0_usize);

        // The hidden textarea — positioned offscreen
        let hidden_textarea = __scope.create_element("textarea");
        hidden_textarea.set_attribute(
            "style",
            "\
            position: absolute; \
            left: -9999px; \
            top: -9999px; \
            width: 1px; \
            height: 1px; \
            opacity: 0; \
        ",
        );

        let hidden_handler_id = __scope.register_input_handler({
            move |value: String| {
                tracing::info!("HIDDEN TEXTAREA INPUT: len={}", value.len());
                hidden_count.update(|c| *c += 1);
                hidden_content.set(value);
            }
        });
        hidden_textarea.set_attribute("data-oninput", &hidden_handler_id.to_string());

        // Visible display area — clicking this should ideally focus the hidden textarea
        let display_area = __scope.create_element("div");
        display_area.set_attribute(
            "style",
            "\
            border: 2px solid #40c057; \
            border-radius: 4px; \
            padding: 16px; \
            min-height: 80px; \
            background: #ebfbee; \
            font-family: monospace; \
            font-size: 16px; \
            cursor: text; \
        ",
        );

        let display_clone = display_area.clone();
        __scope.create_effect({
            move || {
                let text = hidden_content.get();
                let count = hidden_count.get();
                if text.is_empty() {
                    display_clone.set_text(&format!("Click here, then type (events: {})", count));
                } else {
                    display_clone.set_text(&format!("[{}] {}", count, text));
                }
            }
        });

        // Test: textarea with opacity:0 overlaying the display area via absolute positioning
        let wrapper = __scope.create_element("div");
        wrapper.set_attribute("style", "position: relative; margin-top: 8px;");

        // Visible label behind
        let label = __scope.create_element("div");
        label.set_attribute("style", "padding: 12px; background: #fff3bf; border: 2px solid #fab005; border-radius: 4px; font-size: 14px;");
        label.set_text("Click HERE and type — transparent textarea is on top of this yellow box");

        // Transparent textarea on top
        let overlay_textarea = __scope.create_element("textarea");
        overlay_textarea.set_attribute(
            "style",
            "\
            position: absolute; \
            top: 0; \
            left: 0; \
            width: 100%; \
            height: 100%; \
            opacity: 0; \
            font-family: monospace; \
            font-size: 16px; \
            border: none; \
            background: transparent; \
            resize: none; \
            cursor: text; \
        ",
        );

        let overlay_content = use_signal(String::new);

        let overlay_handler_id = __scope.register_input_handler({
            move |value: String| {
                tracing::info!("OVERLAY TEXTAREA INPUT: len={}", value.len());
                hidden_count.update(|c| *c += 1);
                overlay_content.set(value);
            }
        });
        overlay_textarea.set_attribute("data-oninput", &overlay_handler_id.to_string());

        let overlay_display = __scope.create_element("div");
        overlay_display.set_attribute("style", "font-size: 13px; color: #495057; margin-top: 4px;");

        let overlay_display_clone = overlay_display.clone();
        __scope.create_effect({
            move || {
                let text = overlay_content.get();
                if text.is_empty() {
                    overlay_display_clone.set_text("Overlay textarea: (no input yet)");
                } else {
                    overlay_display_clone.set_text(&format!("Overlay textarea: \"{}\"", text));
                }
            }
        });

        panel.append_child(&hidden_textarea);
        panel.append_child(&display_area);
        wrapper.append_child(&label);
        wrapper.append_child(&overlay_textarea);
        panel.append_child(&wrapper);
        panel.append_child(&overlay_display);
        root.append_child(&panel);
    }

    // === SECTION 2: Custom Rendered View ===
    {
        let panel = create_panel(
            __scope,
            "2. Custom Rendered View",
            "This div renders the SAME content with custom formatting and line numbers. Proves textarea -> custom DOM sync.",
        );

        let render_container = __scope.create_element("div");
        render_container.set_attribute("style", "border: 2px solid #40c057; border-radius: 4px; padding: 16px; min-height: 120px; background: white; font-family: monospace; font-size: 16px; line-height: 1.6; white-space: pre-wrap;");

        let render_clone = render_container.clone();
        __scope.create_effect({
            move || {
                let text = content.get();
                let display = if text.is_empty() {
                    String::from("(empty - type in the textarea above)")
                } else {
                    text.lines()
                        .enumerate()
                        .map(|(i, line)| format!("{}: {}", i + 1, line))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                render_clone.set_text(&display);
            }
        });

        panel.append_child(&render_container);
        root.append_child(&panel);
    }

    // === SECTION 3: Cursor Position Tracking ===
    {
        let panel = create_panel(
            __scope,
            "3. Cursor Position Tracking",
            "Demonstrates custom cursor rendering. Shows text split at cursor position.",
        );

        let cursor_display = __scope.create_element("div");
        cursor_display.set_attribute("style", "font-family: monospace; font-size: 14px; padding: 12px; background: #f8f9fa; border-radius: 4px; white-space: pre-wrap;");

        let cursor_display_clone = cursor_display.clone();
        __scope.create_effect({
            move || {
                let pos = cursor_pos.get();
                let text = content.get();
                let total = text.len();
                let clamped = pos.min(total);

                let before = &text[..clamped];
                let after = &text[clamped..];

                let before_display = if before.len() > 40 {
                    format!("...{}", &before[before.len() - 40..])
                } else {
                    before.to_string()
                };
                let after_display = if after.len() > 40 {
                    format!("{}...", &after[..40])
                } else {
                    after.to_string()
                };

                cursor_display_clone.set_text(&format!(
                    "Cursor at: {}/{}\nBefore cursor: \"{}\"\nAfter cursor: \"{}\"",
                    clamped, total, before_display, after_display
                ));
            }
        });

        panel.append_child(&cursor_display);
        root.append_child(&panel);
    }

    // === SECTION 4: Visual Cursor Demo ===
    {
        let panel = create_panel(
            __scope,
            "4. Visual Cursor Demo",
            "Renders text with a blinking cursor at the current position (end of input).",
        );

        let cursor_view = __scope.create_element("div");
        cursor_view.set_attribute("style", "border: 2px solid #7950f2; border-radius: 4px; padding: 16px; min-height: 60px; background: white; font-family: monospace; font-size: 16px; line-height: 1.6;");

        // We build three inline spans: before-cursor text, cursor element, after-cursor text
        let before_span = __scope.create_element("span");
        let cursor_span = __scope.create_element("span");
        // CSS blinking cursor: a zero-width span with a left border
        cursor_span.set_attribute(
            "style",
            "\
            display: inline-block; \
            width: 0; \
            height: 1.2em; \
            border-left: 2px solid #7950f2; \
            vertical-align: text-bottom; \
            animation: blink 1s step-end infinite;\
        ",
        );
        let after_span = __scope.create_element("span");

        // We also need the @keyframes rule. Inject it via a <style> element.
        let style_el = __scope.create_element("style");
        style_el.set_text("@keyframes blink { 0%, 100% { opacity: 1; } 50% { opacity: 0; } }");
        cursor_view.append_child(&style_el);
        cursor_view.append_child(&before_span);
        cursor_view.append_child(&cursor_span);
        cursor_view.append_child(&after_span);

        let before_clone = before_span.clone();
        let after_clone = after_span.clone();
        __scope.create_effect({
            move || {
                let text = content.get();
                let pos = cursor_pos.get().min(text.len());
                let before_text = &text[..pos];
                let after_text = &text[pos..];

                if text.is_empty() {
                    before_clone.set_text("");
                    after_clone.set_text("");
                } else {
                    before_clone.set_text(before_text);
                    after_clone.set_text(after_text);
                }
            }
        });

        panel.append_child(&cursor_view);
        root.append_child(&panel);
    }

    // === STATUS BADGE ===
    {
        let panel = create_panel(__scope, "Status", "");

        let badge = __scope.create_element("span");
        badge.set_attribute("style", "background: #4c6ef5; color: white; padding: 4px 12px; border-radius: 12px; font-size: 14px;");

        let badge_clone = badge.clone();
        __scope.create_effect({
            move || {
                badge_clone.set_text(&format!("Input Events: {}", input_count.get()));
            }
        });

        panel.append_child(&badge);
        root.append_child(&panel);
    }

    // === EVENT LOG ===
    {
        let panel = create_panel(__scope, "Event Log", "");

        let log_box = __scope.create_element("pre");
        log_box.set_attribute("style", "background: #212529; color: #f8f9fa; padding: 12px; border-radius: 4px; font-size: 11px; max-height: 200px; overflow-y: auto; white-space: pre-wrap;");

        let log_clone = log_box.clone();
        __scope.create_effect({
            move || {
                let log = event_log.get();
                let joined = log.join("\n");
                log_clone.set_text(if log.is_empty() {
                    "No events yet..."
                } else {
                    &joined
                });
            }
        });

        panel.append_child(&log_box);
        root.append_child(&panel);
    }

    // === ARCHITECTURE NOTES ===
    {
        let panel = create_panel(__scope, "Architecture Notes", "");

        let notes = __scope.create_element("div");
        notes.set_attribute(
            "style",
            "font-size: 13px; line-height: 1.6; white-space: pre-wrap;",
        );
        notes.set_text(
            "KEY CONCEPT:\n\
             1. Textarea handles: typing, IME composition, Ctrl+C/V clipboard, undo (native)\n\
             2. Custom view handles: rich formatting, block structure, custom cursor\n\
             3. Keyboard interceptor handles: arrow navigation, selection extension, shortcuts\n\
             \n\
             WHAT THIS SPIKE PROVES:\n\
             - Textarea input events flow to our handler\n\
             - We can reactively render the same content differently\n\
             - The two stay in sync\n\
             - Visual cursor rendering works\n\
             \n\
             WHAT THE FULL IMPL WOULD ADD:\n\
             - Hidden textarea (offscreen, not visible)\n\
             - Click on content area -> programmatic focus of textarea\n\
             - Rich DOM tree (paragraphs, inline formatting) instead of plain text\n\
             - Custom cursor overlay positioned via text layout metrics\n\
             - Selection highlighting via computed ranges",
        );

        panel.append_child(&notes);
        root.append_child(&panel);
    }

    root
}

/// Helper to create a styled panel with title and optional description.
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
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();

    let window_props = WindowProps {
        title: "Hidden Textarea Spike".into(),
        width: 900,
        height: 900,
        ..Default::default()
    };

    run_with_window_props(app, window_props, None);
}
