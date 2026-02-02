//! Fine-grained rendering with actual window.
//!
//! This example demonstrates fine-grained reactive rendering in a real window.
//! Run with: cargo run -p fine_grained_window

use rinch::prelude::*;

fn main() {
    // Initialize tracing for debug output
    let _ = tracing_subscriber::fmt::try_init();

    println!("Starting fine-grained window example...");

    run("Fine-Grained Counter", 400, 300, counter_component);
}

fn counter_component(__scope: &mut RenderScope) -> NodeHandle {
    // Create a signal for the counter
    let count = Signal::new(42);
    let scope = __scope;
    // Build the DOM structure
    let container = scope.create_element("div");
    container.set_attribute(
        "style",
        "padding: 40px; font-family: system-ui, sans-serif; text-align: center; background: #f5f5f5; min-height: 100vh;",
    );

    // Title
    let title = scope.create_element("h1");
    title.set_attribute("style", "color: #333; margin-bottom: 20px;");
    let title_text = scope.create_text("Fine-Grained Rendering");
    title.append_child(&title_text);
    container.append_child(&title);

    // Counter display
    let counter_div = scope.create_element("div");
    counter_div.set_attribute(
        "style",
        "font-size: 48px; font-weight: bold; color: #007bff; margin: 20px 0;",
    );

    // Create text node for count value - this will be updated reactively
    let count_text = scope.create_text("0");
    counter_div.append_child(&count_text);
    container.append_child(&counter_div);

    // Set up effect to update the text when count changes
    let count_handle = count_text.clone();
    scope.create_effect(move || {
        let current = count.get();
        count_handle.set_text(&current.to_string());
    });

    // Description
    let desc = scope.create_element("p");
    desc.set_attribute("style", "color: #666; margin-top: 30px;");
    let desc_text = scope.create_text("This window is rendered using fine-grained reactivity!");
    desc.append_child(&desc_text);
    container.append_child(&desc);

    // Status
    let status = scope.create_element("p");
    status.set_attribute("style", "color: #888; font-size: 14px;");
    let status_text = scope.create_text("Initial count value: 42 (set via Signal)");
    status.append_child(&status_text);
    container.append_child(&status);

    // The effect ran during initialization, so the count should show 42
    // In the future, event handlers will allow interactive updates

    // Store the signal somewhere accessible if we want to update it later
    // For now, we demonstrate that the effect correctly updates the text node
    let _ = count; // Keep signal alive

    container
}
