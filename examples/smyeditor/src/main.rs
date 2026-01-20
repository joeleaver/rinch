//! smyeditor - A rich-text editor built with rinch.
//!
//! This example demonstrates rinch's reactive system with:
//! - Signals and event handlers
//! - Menu item callbacks (onclick)
//! - use_context for shared state
//! - use_derived for computed state

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

            Window { title: "smyeditor - Reactive Demo", width: 1024, height: 768,
                html {
                    head {
                        style {
                            "
                            * {
                                box-sizing: border-box;
                            }
                            body {
                                font-family: system-ui, -apple-system, sans-serif;
                                margin: 0;
                                padding: 20px;
                                background: " {theme.background.clone()} ";
                                color: #d4d4d4;
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
                                border-radius: 8px;
                                padding: 20px;
                                margin-bottom: 20px;
                            }
                            .about-dialog {
                                background: #2d2d2d;
                                border: 2px solid #569cd6;
                                border-radius: 8px;
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
                                padding: 12px 24px;
                                border-radius: 4px;
                                font-size: 16px;
                                cursor: pointer;
                                transition: background 0.2s;
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
                                background: #1e1e1e;
                                border-radius: 4px;
                                margin: 15px 0;
                            }
                            .info {
                                color: #808080;
                                font-size: 14px;
                                margin-top: 10px;
                            }
                            .feature-badge {
                                display: inline-block;
                                background: #4ec9b0;
                                color: #1e1e1e;
                                padding: 2px 8px;
                                border-radius: 4px;
                                font-size: 11px;
                                margin-left: 8px;
                            }
                            .status-bar {
                                margin-top: 20px;
                                padding: 8px 12px;
                                background: #007acc;
                                color: white;
                                font-size: 12px;
                                border-radius: 4px;
                            }
                            .keyboard-hint {
                                color: #808080;
                                font-size: 12px;
                                margin-top: 10px;
                            }
                            kbd {
                                background: #3c3c3c;
                                padding: 2px 6px;
                                border-radius: 3px;
                                font-family: monospace;
                            }
                            "
                        }
                    }
                    body {
                        h1 { "smyeditor" }
                        p { "A demonstration of rinch's reactive system" }

                        // About dialog using menu callback
                        div { class: "about-dialog", style: if show_about.get() { "display: block" } else { "display: none" },
                            h2 { "About smyeditor" }
                            p { "Built with " strong { "rinch" } " - a reactive GUI framework for Rust" }
                            p { "Features demonstrated:" }
                            ul { style: "text-align: left; display: inline-block;",
                                li { "Menu item callbacks (onclick)" }
                                li { "use_context for shared state" }
                                li { "use_derived for computed values" }
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

                        div { class: "status-bar",
                            "Ready | Reactive UI powered by Signals | Count: " {count.get()} " | Doubled: " {doubled.get()}
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
