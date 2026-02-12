//! ContentEditable Test Harness
//!
//! Comprehensive test scenarios for the contentEditable implementation.
//! Launch this app and use MCP tools to verify behavior at each phase.

use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let cursor_info = use_signal(|| String::from("No focus"));
    let content_mirror = use_signal(|| String::new());

    rsx! {
        div { style: "display: flex; flex-direction: column; height: 100vh; padding: 16px; gap: 12px; font-family: sans-serif; overflow: auto;",

            h2 { style: "margin: 0;", "ContentEditable Test Harness" }
            p { style: "color: #666; margin: 0 0 8px 0; font-size: 14px;",
                "Testing contenteditable implementation. Use MCP tools to interact."
            }

            // Test 1: plaintext-only (primary test target for phases 1-4)
            div { style: "border: 1px solid #ddd; border-radius: 8px; padding: 12px;",
                h3 { style: "margin: 0 0 8px 0; font-size: 14px; color: #4a9eff;",
                    "Test 1: plaintext-only"
                }
                div {
                    contenteditable: "plaintext-only",
                    style: "border: 2px solid #4a9eff; padding: 12px; min-height: 120px; font-size: 16px; line-height: 1.6; outline: none; border-radius: 4px; background: #fafcff; overflow-wrap: break-word;",
                    oninput: move |value: String| content_mirror.set(value),
                    "Hello, world!" br {} "This is a multi-line" br {} "plaintext-only contenteditable element." br {} br {} "Try typing, selecting, and using keyboard shortcuts."
                }
            }

            // Test 2: contenteditable="true" with rich inline content (Phase 5 target)
            div { style: "border: 1px solid #ddd; border-radius: 8px; padding: 12px;",
                h3 { style: "margin: 0 0 8px 0; font-size: 14px; color: #4aff7e;",
                    "Test 2: Rich inline contenteditable"
                }
                div {
                    contenteditable: "true",
                    style: "border: 2px solid #4aff7e; padding: 12px; min-height: 80px; font-size: 16px; line-height: 1.6; outline: none; border-radius: 4px; background: #fafffc; overflow-wrap: break-word;",
                    "Hello "
                    span { style: "font-weight: bold;", "bold" }
                    " and "
                    span { style: "font-style: italic;", "italic" }
                    " text with "
                    span { style: "color: blue;", "colored" }
                    " spans."
                }
            }

            // Test 3: Empty contenteditable (edge case)
            div { style: "border: 1px solid #ddd; border-radius: 8px; padding: 12px;",
                h3 { style: "margin: 0 0 8px 0; font-size: 14px; color: #ff4a4a;",
                    "Test 3: Empty (edge case)"
                }
                div {
                    contenteditable: "plaintext-only",
                    style: "border: 2px solid #ff4a4a; padding: 12px; min-height: 60px; font-size: 16px; line-height: 1.6; outline: none; border-radius: 4px; background: #fffafa; overflow-wrap: break-word;",
                }
            }

            // Test 4: Scrollable contenteditable (scroll-into-view test)
            div { style: "border: 1px solid #ddd; border-radius: 8px; padding: 12px;",
                h3 { style: "margin: 0 0 8px 0; font-size: 14px; color: #ff9f4a;",
                    "Test 4: Scrollable (overflow test)"
                }
                div {
                    contenteditable: "plaintext-only",
                    style: "border: 2px solid #ff9f4a; padding: 12px; height: 80px; overflow: auto; font-size: 16px; line-height: 1.6; outline: none; border-radius: 4px; background: #fffcfa; overflow-wrap: break-word;",
                    "Line 1" br {} "Line 2" br {} "Line 3" br {} "Line 4" br {} "Line 5" br {} "Line 6" br {} "Line 7" br {} "Line 8" br {} "Line 9" br {} "Line 10"
                }
            }

            // Test 5: Block-level elements contenteditable
            div { style: "border: 1px solid #ddd; border-radius: 8px; padding: 12px;",
                h3 { style: "margin: 0 0 8px 0; font-size: 14px; color: #9f4aff;",
                    "Test 5: Block elements (Phase 5 target)"
                }
                div {
                    contenteditable: "true",
                    style: "border: 2px solid #9f4aff; padding: 12px; min-height: 120px; font-size: 16px; line-height: 1.6; outline: none; border-radius: 4px; background: #fcfaff; overflow-wrap: break-word;",
                    h2 { style: "margin: 0 0 8px 0; font-size: 20px;", "Heading" }
                    p { style: "margin: 0 0 8px 0;", "First paragraph with some text." }
                    p { style: "margin: 0 0 8px 0;", "Second paragraph." }
                    ul { style: "margin: 0; padding-left: 24px;",
                        li { "List item 1" }
                        li { "List item 2" }
                    }
                }
            }

            // Test 6: Nested list (indent/outdent test)
            div { style: "border: 1px solid #ddd; border-radius: 8px; padding: 12px;",
                h3 { style: "margin: 0 0 8px 0; font-size: 14px; color: #ff4aff;",
                    "Test 6: Nested List (Tab/Shift+Tab)"
                }
                div {
                    contenteditable: "true",
                    style: "border: 2px solid #ff4aff; padding: 12px; min-height: 120px; font-size: 16px; line-height: 1.6; outline: none; border-radius: 4px; background: #fffaff; overflow-wrap: break-word;",
                    ul { style: "margin: 0; padding-left: 24px;",
                        li { "Item 1" }
                        li { "Item 2" }
                        li { "Item 3" }
                    }
                    p { style: "margin: 8px 0 0 0; color: #888; font-size: 13px;",
                        "Tab to indent, Shift+Tab to outdent. Enter in empty li exits list."
                    }
                }
            }

            // Comparison: native textarea
            div { style: "border: 1px solid #ddd; border-radius: 8px; padding: 12px;",
                h3 { style: "margin: 0 0 8px 0; font-size: 14px; color: #999;",
                    "Comparison: Native Textarea"
                }
                textarea {
                    style: "border: 2px solid #999; padding: 12px; min-height: 80px; font-size: 16px; line-height: 1.6; resize: vertical; width: 100%; box-sizing: border-box; border-radius: 4px;",
                    oninput: move |value: String| cursor_info.set(format!("Textarea value: {} chars", value.len())),
                    "Same text in a native textarea for comparison.\nLine 2 of textarea."
                }
            }

            // Status bar
            div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                div { style: "flex: 1; padding: 8px; background: #f0f0f0; border-radius: 4px; font-family: monospace; font-size: 12px; border: 1px solid #ddd; min-width: 200px;",
                    strong { "Status: " }
                    span { {|| cursor_info.get()} }
                }
                div { style: "flex: 1; padding: 8px; background: #f0f0f0; border-radius: 4px; font-family: monospace; font-size: 12px; border: 1px solid #ddd; min-width: 200px; max-height: 60px; overflow: auto;",
                    strong { "Content: " }
                    span { {|| if content_mirror.get().is_empty() { "(empty)".to_string() } else { content_mirror.get() }} }
                }
            }
        }
    }
}

fn main() {
    run("ContentEditable Test", 900, 800, app);
}
