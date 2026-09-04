use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);
    let count_inc = count;

    rsx! {
        div { style: "display: flex; flex-direction: column; padding: 20px; gap: 10px; width: 600px",
            div { style: "background-color: #2196F3; padding: 16px; color: white; font-size: 24px",
                "Hello, rinch-dom!"
            }

            // Test inline layout: multiple text nodes and inline elements
            div { style: "background-color: #f5f5f5; padding: 12px; font-size: 16px",
                "This is "
                span { style: "color: #e53935; font-weight: bold",
                    "red bold text"
                }
                " mixed with "
                span { style: "color: #1e88e5; font-style: italic",
                    "blue italic text"
                }
                " in a single paragraph."
            }

            // Test inline with different font sizes
            div { style: "background-color: #e8f5e9; padding: 12px; font-size: 14px",
                "Normal text, then "
                span { style: "font-size: 20px; color: #2e7d32",
                    "BIGGER"
                }
                " text, then "
                span { style: "font-size: 10px; color: #666",
                    "smaller"
                }
                " text."
            }

            // Test text wrapping with inline elements
            div { style: "background-color: #fff3e0; padding: 12px; font-size: 14px; width: 300px",
                "This is a longer paragraph that should wrap. It contains "
                span { style: "font-weight: bold",
                    "bold words"
                }
                " and "
                span { style: "text-decoration: underline",
                    "underlined words"
                }
                " that flow naturally across multiple lines."
            }

            // Test inline-block: button flowing within text
            div { style: "background-color: #e3f2fd; padding: 12px; font-size: 14px; width: 400px",
                "Click the "
                button { style: "width: 60px; height: 24px; background-color: #1976D2; color: white",
                    "OK"
                }
                " button or the "
                button { style: "width: 80px; height: 24px; background-color: #388E3C; color: white",
                    "Cancel"
                }
                " button to proceed."
            }

            // Issue #61: inline content behind an `if` (emits a display:contents
            // wrapper) inside a BLOCK parent. The surrounding text must not be
            // dropped, and inline-block buttons must flow inline (not overlap).
            div { style: "background-color: #ede7f6; padding: 12px; font-size: 16px; width: 500px",
                if count.get() >= 0 {
                    "Behind an if: Hello "
                    span { style: "color: #6a1b9a; font-weight: bold", "bold" }
                    " world, then a "
                    button { style: "width: 60px; height: 24px; background-color: #6a1b9a; color: white", "BTN" }
                    " and "
                    button { style: "width: 70px; height: 24px; background-color: #8e24aa; color: white", "BTN2" }
                    " inline."
                }
            }

            // Control: the same content placed directly (no display:contents).
            div { style: "background-color: #f3e5f5; padding: 12px; font-size: 16px; width: 500px",
                "Directly placed: Hello "
                span { style: "color: #6a1b9a; font-weight: bold", "bold" }
                " world, then a "
                button { style: "width: 60px; height: 24px; background-color: #6a1b9a; color: white", "BTN" }
                " and "
                button { style: "width: 70px; height: 24px; background-color: #8e24aa; color: white", "BTN2" }
                " inline."
            }

            div { style: "display: flex; flex-direction: row; gap: 8px; align-items: center",
                button {
                    style: "background-color: #4CAF50; padding: 8px; color: white",
                    onclick: move || count_inc.update(|n| *n += 1),
                    "Click me!"
                }
                div { style: "font-size: 18px",
                    "Count: " {|| count.get().to_string()}
                }
            }
        }
    }
}

fn main() {
    App::new(app).title("Hello rinch-dom").size(800, 600).run();
}
