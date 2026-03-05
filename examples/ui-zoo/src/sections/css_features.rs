//! CSS Features section - showcasing advanced CSS capabilities.

use rinch::prelude::*;

#[component]
pub fn css_features_section() -> NodeHandle {
    rsx! {
        Fragment {
            // Title
            Title { order: 1, "CSS Features" }
            Text { size: "lg", color: "dimmed",
                "Showcasing transforms, gradients, borders, filters, and more"
            }
            Space { h: "xl" }

            // Group 1: Borders & Outline
            Title { order: 3, "Borders & Outline" }
            Space { h: "md" }
            Group { gap: "md",
                // Solid border
                div { style: "border: 2px solid var(--rinch-color-blue-6); width: 80px; height: 80px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", "Solid" }
                }
                // Dashed border
                div { style: "border: 2px dashed var(--rinch-color-red-6); width: 80px; height: 80px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", "Dashed" }
                }
                // Dotted border
                div { style: "border: 2px dotted var(--rinch-color-green-6); width: 80px; height: 80px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", "Dotted" }
                }
                // Per-side colors
                div { style: "border-style: solid; border-width: 3px; border-top-color: red; border-right-color: green; border-bottom-color: blue; border-left-color: orange; width: 80px; height: 80px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", "Per-side" }
                }
                // Outline
                div { style: "outline: 3px solid var(--rinch-color-violet-6); outline-offset: 4px; width: 80px; height: 80px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", "Outline" }
                }
            }

            // Group 2: Backgrounds
            Space { h: "xl" }
            Title { order: 3, "Backgrounds" }
            Space { h: "md" }
            Group { gap: "md",
                // Solid color
                div { style: "background-color: var(--rinch-color-blue-6); width: 80px; height: 80px; border-radius: 8px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "Solid" }
                }
                // Linear gradient horizontal
                div { style: "background: linear-gradient(to right, red, blue); width: 80px; height: 80px; border-radius: 8px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "Linear H" }
                }
                // Linear gradient diagonal
                div { style: "background: linear-gradient(135deg, orange, purple); width: 80px; height: 80px; border-radius: 8px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "Linear D" }
                }
                // Radial gradient
                div { style: "background: radial-gradient(red, blue); width: 80px; height: 80px; border-radius: 8px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "Radial" }
                }
            }

            // Group 3: Transforms
            Space { h: "xl" }
            Title { order: 3, "Transforms" }
            Space { h: "md" }
            Group { gap: "xl",
                // Rotated
                div { style: "transform: rotate(45deg); background-color: var(--rinch-color-blue-6); width: 60px; height: 60px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "45deg" }
                }
                // Scaled
                div { style: "transform: scale(1.3); background-color: var(--rinch-color-teal-6); width: 60px; height: 60px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "1.3x" }
                }
                // Skewed
                div { style: "transform: skewX(15deg); background-color: var(--rinch-color-orange-6); width: 60px; height: 60px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "Skew" }
                }
            }

            // Group 4: Visual Effects
            Space { h: "xl" }
            Title { order: 3, "Visual Effects" }
            Space { h: "md" }
            Group { gap: "md",
                // Opacity levels
                div { style: "opacity: 0.25; background-color: var(--rinch-color-blue-6); width: 60px; height: 60px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "25%" }
                }
                div { style: "opacity: 0.5; background-color: var(--rinch-color-blue-6); width: 60px; height: 60px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "50%" }
                }
                div { style: "opacity: 0.75; background-color: var(--rinch-color-blue-6); width: 60px; height: 60px; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "75%" }
                }
                // Text shadow
                div { style: "width: 120px; height: 60px; display: flex; align-items: center; justify-content: center",
                    Text { size: "lg", style: "text-shadow: 2px 2px 4px rgba(0,0,0,0.5)", "Shadow" }
                }
            }

            // Group 5: Stacking (z-index)
            Space { h: "xl" }
            Title { order: 3, "Z-Index Stacking" }
            Space { h: "md" }
            div { style: "position: relative; width: 200px; height: 120px",
                div { style: "position: absolute; z-index: 1; top: 0px; left: 0px; background-color: red; width: 80px; height: 80px; opacity: 0.8; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "z:1" }
                }
                div { style: "position: absolute; z-index: 3; top: 20px; left: 20px; background-color: green; width: 80px; height: 80px; opacity: 0.8; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "z:3" }
                }
                div { style: "position: absolute; z-index: 2; top: 40px; left: 40px; background-color: blue; width: 80px; height: 80px; opacity: 0.8; display: flex; align-items: center; justify-content: center",
                    Text { size: "xs", style: "color: white", "z:2" }
                }
            }

            // Group 6: Interaction Properties
            Space { h: "xl" }
            Title { order: 3, "Interaction" }
            Space { h: "md" }
            Group { gap: "md",
                div { style: "cursor: pointer; background-color: var(--rinch-color-blue-1); border: 1px solid var(--rinch-color-blue-6); padding: 12px; border-radius: 8px; display: flex; align-items: center; justify-content: center",
                    Text { size: "sm", "cursor: pointer" }
                }
                div { style: "pointer-events: none; background-color: var(--rinch-color-gray-1); border: 1px solid var(--rinch-color-gray-4); padding: 12px; border-radius: 8px; display: flex; align-items: center; justify-content: center",
                    Text { size: "sm", color: "dimmed", "pointer-events: none" }
                }
                div { style: "visibility: hidden; background-color: var(--rinch-color-red-1); border: 1px solid var(--rinch-color-red-6); padding: 12px; width: 140px; height: 44px; border-radius: 8px",
                }
                Text { size: "xs", color: "dimmed", "(hidden box preserves space above)" }
            }

            // Group 7: Transitions
            Space { h: "xl" }
            Title { order: 3, "Transitions" }
            Text { size: "sm", color: "dimmed", "Hover over the elements below to see smooth CSS transitions" }
            Space { h: "md" }

            // CSS for transition demos - uses :hover which Stylo handles
            style {
                r#"
                .transition-bg {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-blue-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    transition: background-color 0.3s ease;
                    cursor: pointer;
                }
                .transition-bg:hover {
                    background-color: var(--rinch-color-red-6);
                }

                .transition-opacity {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-green-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    opacity: 1.0;
                    transition: opacity 0.5s ease;
                    cursor: pointer;
                }
                .transition-opacity:hover {
                    opacity: 0.3;
                }

                .transition-transform {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-violet-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    transition: transform 0.3s ease;
                    cursor: pointer;
                }
                .transition-transform:hover {
                    transform: scale(1.2);
                }

                .transition-multi {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-orange-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    transition: all 0.4s ease-in-out;
                    cursor: pointer;
                }
                .transition-multi:hover {
                    background-color: var(--rinch-color-cyan-6);
                    border-radius: 50px;
                    opacity: 0.8;
                }
                "#
            }

            Group { gap: "md",
                // Background color transition
                div { class: "transition-bg",
                    Text { size: "xs", style: "color: white", "BG Color" }
                }
                // Opacity transition
                div { class: "transition-opacity",
                    Text { size: "xs", style: "color: white", "Opacity" }
                }
                // Transform transition
                div { class: "transition-transform",
                    Text { size: "xs", style: "color: white", "Scale" }
                }
                // Multiple properties transition
                div { class: "transition-multi",
                    Text { size: "xs", style: "color: white", "All" }
                }
            }

            // Group 8: Text Overflow
            Space { h: "xl" }
            Title { order: 3, "Text Overflow" }
            Text { size: "sm", color: "dimmed", "text-overflow: ellipsis with overflow: hidden and white-space: nowrap" }
            Space { h: "md" }

            Stack { gap: "md",
                // Fixed width container — text should truncate with ellipsis
                div { style: "width: 200px; border: 1px solid var(--rinch-color-gray-5); border-radius: 4px; padding: 8px;",
                    div { style: "overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
                        "This is a long sentence that should be truncated with an ellipsis character"
                    }
                }

                // Wider container — same text should show more before truncating
                div { style: "width: 350px; border: 1px solid var(--rinch-color-gray-5); border-radius: 4px; padding: 8px;",
                    div { style: "overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
                        "This is a long sentence that should be truncated with an ellipsis character"
                    }
                }

                // Full width — text fits so no ellipsis should appear
                div { style: "border: 1px solid var(--rinch-color-gray-5); border-radius: 4px; padding: 8px;",
                    div { style: "overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
                        "Short text — no truncation needed"
                    }
                }

                // Flex layout test: flex: 1 child should truncate
                Text { size: "sm", weight: "bold", "Flex layout:" }
                div { style: "display: flex; gap: 8px; border: 1px solid var(--rinch-color-gray-5); border-radius: 4px; padding: 8px;",
                    div { style: "width: 80px; flex-shrink: 0; color: var(--rinch-color-blue-6); font-weight: bold;",
                        "Label"
                    }
                    div { style: "flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
                        "This is the value text in a flex child that should truncate with ellipsis when it overflows the available space in the flex container"
                    }
                }

                // Without text-overflow (just clip) — for comparison
                Text { size: "sm", weight: "bold", "Without ellipsis (just clip):" }
                div { style: "width: 200px; border: 1px solid var(--rinch-color-gray-5); border-radius: 4px; padding: 8px;",
                    div { style: "overflow: hidden; white-space: nowrap;",
                        "This text is clipped without any ellipsis indicator at the end"
                    }
                }

                // Without overflow hidden — should not truncate (just overflow)
                Text { size: "sm", weight: "bold", "Without overflow: hidden (wraps normally):" }
                div { style: "width: 200px; border: 1px solid var(--rinch-color-gray-5); border-radius: 4px; padding: 8px;",
                    div { style: "text-overflow: ellipsis;",
                        "This text has text-overflow: ellipsis but no overflow: hidden so it wraps normally"
                    }
                }
            }

            Space { h: "xl" }
            Title { order: 2, "Click Transitions" }
            Text { size: "sm", color: "dimmed", "Click the elements below to toggle CSS transitions" }
            Space { h: "md" }

            // CSS for click-toggled transitions
            style {
                r#"
                .click-bg {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-blue-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    transition: background-color 0.3s ease;
                    cursor: pointer;
                }
                .click-bg.active {
                    background-color: var(--rinch-color-red-6);
                }

                .click-opacity {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-green-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    opacity: 1.0;
                    transition: opacity 0.5s ease;
                    cursor: pointer;
                }
                .click-opacity.active {
                    opacity: 0.3;
                }

                .click-scale {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-violet-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    transition: transform 0.3s ease;
                    cursor: pointer;
                }
                .click-scale.active {
                    transform: scale(1.3);
                }

                .click-multi {
                    width: 100px;
                    height: 100px;
                    background-color: var(--rinch-color-orange-6);
                    border-radius: 8px;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    transition: all 0.4s ease-in-out;
                    cursor: pointer;
                }
                .click-multi.active {
                    background-color: var(--rinch-color-cyan-6);
                    border-radius: 50px;
                    transform: scale(1.1);
                }
                "#
            }

            {click_transition_demos(__scope)}
        }
    }
}

#[component]
fn click_transition_demos() -> NodeHandle {
    let bg_active = Signal::new(false);
    let opacity_active = Signal::new(false);
    let scale_active = Signal::new(false);
    let multi_active = Signal::new(false);

    rsx! {
        Group { gap: "md",
            div {
                class: {move || if bg_active.get() { "click-bg active" } else { "click-bg" }},
                onclick: move || bg_active.update(|v| *v = !*v),
                Text { size: "xs", style: "color: white", "BG Color" }
            }
            div {
                class: {move || if opacity_active.get() { "click-opacity active" } else { "click-opacity" }},
                onclick: move || opacity_active.update(|v| *v = !*v),
                Text { size: "xs", style: "color: white", "Opacity" }
            }
            div {
                class: {move || if scale_active.get() { "click-scale active" } else { "click-scale" }},
                onclick: move || scale_active.update(|v| *v = !*v),
                Text { size: "xs", style: "color: white", "Scale" }
            }
            div {
                class: {move || if multi_active.get() { "click-multi active" } else { "click-multi" }},
                onclick: move || multi_active.update(|v| *v = !*v),
                Text { size: "xs", style: "color: white", "All" }
            }
        }
    }
}
