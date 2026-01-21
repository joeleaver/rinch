//! smyeditor - A rich-text editor built with rinch.
//!
//! This example demonstrates rinch's reactive system with:
//! - Signals and event handlers
//! - Menu item callbacks (onclick)
//! - use_context for shared state
//! - use_derived for computed state
//! - Frameless window with custom chrome

use rinch::prelude::*;

/// Theme context shared across the application.
#[derive(Clone)]
struct ThemeContext {
    primary_color: String,
    background: String,
}

fn app() -> Element {
    // Create a theme context accessible from anywhere
    let theme = create_context(ThemeContext {
        primary_color: "#569cd6".into(),
        background: "#1e1e1e".into(),
    });

    // Persistent reactive state using hooks
    let count = use_signal(|| 0);
    let text = use_signal(|| String::from("Hello, Rinch!"));
    let show_about = use_signal(|| false);

    // Use derived to compute values automatically
    let doubled = use_derived({
        let count = count.clone();
        move || count.get() * 2
    });

    let is_positive = use_derived({
        let count = count.clone();
        move || count.get() > 0
    });

    // Clone signals for use in event handlers
    let count_inc = count.clone();
    let count_dec = count.clone();
    let count_reset = count.clone();
    let text_change = text.clone();

    // Clones for menu callbacks
    let menu_count_reset = count.clone();
    let menu_show_about = show_about.clone();

    rsx! {
        Fragment {
            AppMenu { native: true,
                Menu { label: "File",
                    MenuItem { label: "New", shortcut: "Cmd+N", onclick: || {
                        println!("File > New clicked!");
                    }}
                    MenuItem { label: "Open...", shortcut: "Cmd+O", onclick: || {
                        println!("File > Open clicked!");
                    }}
                    MenuSeparator {}
                    MenuItem { label: "Save", shortcut: "Cmd+S", onclick: || {
                        println!("File > Save clicked!");
                    }}
                    MenuItem { label: "Save As...", shortcut: "Cmd+Shift+S" }
                    MenuSeparator {}
                    MenuItem { label: "Exit", shortcut: "Alt+F4" }
                }
                Menu { label: "Edit",
                    MenuItem { label: "Undo", shortcut: "Cmd+Z" }
                    MenuItem { label: "Redo", shortcut: "Cmd+Shift+Z" }
                    MenuSeparator {}
                    MenuItem { label: "Cut", shortcut: "Cmd+X" }
                    MenuItem { label: "Copy", shortcut: "Cmd+C" }
                    MenuItem { label: "Paste", shortcut: "Cmd+V" }
                    MenuSeparator {}
                    MenuItem { label: "Reset Counter", onclick: move || {
                        menu_count_reset.set(0);
                        println!("Counter reset from menu!");
                    }}
                }
                Menu { label: "View",
                    MenuItem { label: "Zoom In", shortcut: "Cmd+=" }
                    MenuItem { label: "Zoom Out", shortcut: "Cmd+-" }
                    MenuItem { label: "Reset Zoom", shortcut: "Cmd+0" }
                }
                Menu { label: "Help",
                    MenuItem { label: "About smyeditor", onclick: move || {
                        menu_show_about.update(|v| *v = !*v);
                    }}
                }
            }

            // Try transparent window. On Windows with DX12/DirectComposition, this enables
            // true transparency. Falls back to opaque rendering if not supported.
            Window { title: "smyeditor", width: 1024, height: 768, borderless: true, transparent: true,
                html {
                    head {
                        style {
                            "
                            * {
                                box-sizing: border-box;
                                margin: 0;
                                padding: 0;
                            }
                            html, body {
                                background: transparent;
                                height: 100%;
                            }
                            body {
                                font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                                color: #cccccc;
                                display: flex;
                                flex-direction: column;
                                padding: 8px;
                            }
                            /* Window frame with rounded corners */
                            .window-frame {
                                background: #1e1e1e;
                                height: 100%;
                                display: flex;
                                flex-direction: column;
                                overflow: hidden;
                                border-radius: 8px;
                            }
                            /* VS Code Style Title Bar */
                            .titlebar {
                                height: 32px;
                                background: #323233;
                                display: flex;
                                align-items: center;
                                flex-shrink: 0;
                                user-select: none;
                                border-radius: 8px 8px 0 0;
                            }
                            .titlebar-drag {
                                flex: 1;
                                height: 100%;
                                display: flex;
                                align-items: center;
                                padding-left: 12px;
                                gap: 8px;
                            }
                            .titlebar-icon {
                                width: 16px;
                                height: 16px;
                                background: linear-gradient(135deg, #0098ff 0%, #00d4aa 100%);
                                border-radius: 3px;
                            }
                            .titlebar-text {
                                font-size: 13px;
                                color: #cccccc;
                            }
                            .window-controls {
                                display: flex;
                                height: 100%;
                            }
                            .window-control {
                                width: 46px;
                                height: 32px;
                                border: none;
                                background: transparent;
                                color: #cccccc;
                                cursor: pointer;
                                display: flex;
                                align-items: center;
                                justify-content: center;
                            }
                            .window-control:hover {
                                background: rgba(255, 255, 255, 0.1);
                            }
                            .window-control.close:hover {
                                background: #e81123;
                            }
                            .window-control.close:hover .icon-close-1,
                            .window-control.close:hover .icon-close-2 {
                                background: white;
                            }
                            /* Window control icons using CSS */
                            .icon-minimize {
                                width: 10px;
                                height: 1px;
                                background: #cccccc;
                            }
                            .icon-maximize {
                                width: 9px;
                                height: 9px;
                                border: 1px solid #cccccc;
                                background: transparent;
                            }
                            /* X icon using two rotated lines */
                            .icon-close {
                                width: 10px;
                                height: 10px;
                                position: relative;
                            }
                            .icon-close-1, .icon-close-2 {
                                position: absolute;
                                width: 12px;
                                height: 1px;
                                background: #cccccc;
                                top: 50%;
                                left: 50%;
                            }
                            .icon-close-1 {
                                transform: translate(-50%, -50%) rotate(45deg);
                            }
                            .icon-close-2 {
                                transform: translate(-50%, -50%) rotate(-45deg);
                            }
                            /* Main Content Area */
                            .main-content {
                                flex: 1;
                                overflow: auto;
                                padding: 20px;
                                background: " {theme.background.clone()} ";
                            }
                            h1 {
                                color: " {theme.primary_color.clone()} ";
                                margin-bottom: 10px;
                            }
                            h2 {
                                color: #4ec9b0;
                                margin-top: 30px;
                                margin-bottom: 15px;
                            }
                            .section {
                                background: #252526;
                                border: 1px solid #3c3c3c;
                                border-radius: 4px;
                                padding: 20px;
                                margin-bottom: 20px;
                            }
                            .about-dialog {
                                background: #2d2d2d;
                                border: 1px solid #569cd6;
                                border-radius: 4px;
                                padding: 20px;
                                margin-bottom: 20px;
                                text-align: center;
                            }
                            .counter-display {
                                font-size: 48px;
                                font-weight: bold;
                                color: #ce9178;
                                text-align: center;
                                margin: 20px 0;
                            }
                            .derived-values {
                                display: flex;
                                justify-content: center;
                                gap: 30px;
                                margin: 15px 0;
                                color: #9cdcfe;
                            }
                            .derived-value {
                                text-align: center;
                            }
                            .derived-label {
                                font-size: 12px;
                                color: #808080;
                            }
                            .derived-number {
                                font-size: 24px;
                                font-weight: bold;
                            }
                            .button-row {
                                display: flex;
                                gap: 10px;
                                justify-content: center;
                                margin-top: 15px;
                            }
                            button {
                                background: #0e639c;
                                color: white;
                                border: none;
                                padding: 8px 16px;
                                border-radius: 2px;
                                font-size: 13px;
                                cursor: pointer;
                            }
                            button:hover {
                                background: #1177bb;
                            }
                            button.danger {
                                background: #c72e2e;
                            }
                            button.danger:hover {
                                background: #e03e3e;
                            }
                            .text-display {
                                font-size: 24px;
                                color: #9cdcfe;
                                text-align: center;
                                padding: 20px;
                                background: #252526;
                                border-radius: 4px;
                                margin: 15px 0;
                            }
                            .info {
                                color: #808080;
                                font-size: 13px;
                                margin-top: 10px;
                            }
                            p {
                                margin: 8px 0;
                            }
                            ul {
                                margin: 8px 0;
                                padding-left: 20px;
                            }
                            .feature-badge {
                                display: inline-block;
                                background: #4ec9b0;
                                color: #1e1e1e;
                                padding: 2px 8px;
                                border-radius: 2px;
                                font-size: 11px;
                                margin-left: 8px;
                            }
                            /* VS Code Style Status Bar */
                            .status-bar {
                                height: 22px;
                                padding: 0 10px;
                                background: #007acc;
                                color: white;
                                font-size: 12px;
                                display: flex;
                                align-items: center;
                                flex-shrink: 0;
                                border-radius: 0 0 8px 8px;
                            }
                            .keyboard-hint {
                                color: #808080;
                                font-size: 12px;
                                margin-top: 10px;
                            }
                            kbd {
                                background: #3c3c3c;
                                padding: 2px 6px;
                                border-radius: 2px;
                                font-family: monospace;
                                font-size: 11px;
                            }
                            "
                        }
                    }
                    body {
                        // Window frame for rounded corners
                        div { class: "window-frame",
                            // VS Code Style Title Bar
                            div { class: "titlebar",
                                div { class: "titlebar-drag", draggable: "true",
                                    div { class: "titlebar-icon" }
                                    span { class: "titlebar-text", "smyeditor" }
                                }
                                div { class: "window-controls",
                                    button { class: "window-control minimize", title: "Minimize",
                                        onclick: || minimize_current_window(),
                                        span { class: "icon-minimize" }
                                    }
                                    button { class: "window-control maximize", title: "Maximize",
                                        onclick: || toggle_maximize_current_window(),
                                        span { class: "icon-maximize" }
                                    }
                                    button { class: "window-control close", title: "Close",
                                        onclick: || close_current_window(),
                                        div { class: "icon-close",
                                            div { class: "icon-close-1" }
                                            div { class: "icon-close-2" }
                                        }
                                    }
                                }
                            }

                            // Main Content Area
                            div { class: "main-content",
                            h1 { "smyeditor" }
                            p { "A demonstration of rinch's reactive system with custom window chrome" }

                            // About dialog using menu callback
                            div { class: "about-dialog", style: if show_about.get() { "display: block" } else { "display: none" },
                                h2 { "About smyeditor" }
                                p { "Built with " strong { "rinch" } " - a reactive GUI framework for Rust" }
                                p { "Features demonstrated:" }
                                ul { style: "text-align: left; display: inline-block;",
                                    li { "Menu item callbacks (onclick)" }
                                    li { "use_context for shared state" }
                                    li { "use_derived for computed values" }
                                    li { "Frameless window with custom chrome" }
                                }
                                p { style: "color: #808080;", "Click Help > About again to close" }
                            }

                            div { class: "section",
                            h2 {
                                "Counter Demo"
                                span { class: "feature-badge", "use_derived" }
                            }
                            p { "Click the buttons to update the counter. Derived values update automatically!" }

                            div { class: "counter-display",
                                {count.get()}
                            }

                            div { class: "derived-values",
                                div { class: "derived-value",
                                    div { class: "derived-label", "Doubled (use_derived)" }
                                    div { class: "derived-number", {doubled.get()} }
                                }
                                div { class: "derived-value",
                                    div { class: "derived-label", "Is Positive?" }
                                    div { class: "derived-number",
                                        {if is_positive.get() { "Yes" } else { "No" }}
                                    }
                                }
                            }

                            div { class: "button-row",
                                button { onclick: move || count_dec.update(|n| *n -= 1),
                                    "- Decrement"
                                }
                                button { onclick: move || count_inc.update(|n| *n += 1),
                                    "+ Increment"
                                }
                                button { class: "danger", onclick: move || count_reset.set(0),
                                    "Reset"
                                }
                            }

                            p { class: "info",
                                "use_derived automatically tracks signal dependencies and recomputes when they change."
                            }
                            p { class: "info",
                                "Try Edit > Reset Counter from the menu to see menu callbacks in action!"
                            }
                        }

                        div { class: "section",
                            h2 {
                                "Dynamic Text Demo"
                                span { class: "feature-badge", "use_context" }
                            }
                            p { "The theme colors come from a shared ThemeContext:" }

                            div { class: "text-display",
                                {text.get()}
                            }

                            div { class: "button-row",
                                button { onclick: move || {
                                    let messages = [
                                        "Hello, Rinch!",
                                        "Fine-grained reactivity!",
                                        "Built with Rust!",
                                        "GPU-accelerated rendering!",
                                    ];
                                    text_change.update(|t| {
                                        let current_idx = messages.iter().position(|&m| m == t.as_str()).unwrap_or(0);
                                        let next_idx = (current_idx + 1) % messages.len();
                                        *t = messages[next_idx].to_string();
                                    });
                                },
                                    "Change Message"
                                }
                            }
                        }

                            div { class: "keyboard-hint",
                                "Developer Tools: "
                                kbd { "F12" } " Toggle DevTools | "
                                kbd { "Alt+D" } " Layout Debug | "
                                kbd { "Alt+I" } " Inspect Mode | "
                                kbd { "Alt+T" } " Print Taffy Tree"
                            }
                        }

                            // VS Code Style Status Bar
                            div { class: "status-bar",
                                "smyeditor | Count: " {count.get()} " | Doubled: " {doubled.get()}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn main() {
    // Use hot reload for development - UI updates when files change
    rinch::run_with_hot_reload(app);
}
