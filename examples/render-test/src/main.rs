//! Headless renderer test fixture.
//!
//! Renders individual rinch components to PNG files without opening a window.
//! Compile without `gpu` feature for software rendering (tiny-skia),
//! or with `gpu` feature for GPU rendering (vello).
//!
//! Usage:
//!   cargo run -p render-test                    # software renderer
//!   cargo run -p render-test -- --output /tmp   # custom output dir
//!   cargo run -p render-test -- --list          # list available tests
//!   cargo run -p render-test -- button badge    # render specific tests

use rinch::prelude::*;
use std::path::{Path, PathBuf};

// ── Test definitions ─────────────────────────────────────────────────────────

struct TestCase {
    name: &'static str,
    width: u32,
    height: u32,
    component: fn(&mut RenderScope) -> NodeHandle,
}

fn test_cases() -> Vec<TestCase> {
    vec![
        // ── Buttons ──────────────────────────────────────────────────────
        TestCase {
            name: "button_variants",
            width: 800,
            height: 60,
            component: test_button_variants,
        },
        TestCase {
            name: "button_sizes",
            width: 800,
            height: 60,
            component: test_button_sizes,
        },
        TestCase {
            name: "button_colors",
            width: 800,
            height: 120,
            component: test_button_colors,
        },
        TestCase {
            name: "button_radius",
            width: 800,
            height: 60,
            component: test_button_radius,
        },
        TestCase {
            name: "button_states",
            width: 600,
            height: 60,
            component: test_button_states,
        },
        TestCase {
            name: "button_full_width",
            width: 400,
            height: 200,
            component: test_button_full_width,
        },
        TestCase {
            name: "action_icon_variants",
            width: 600,
            height: 60,
            component: test_action_icon_variants,
        },
        TestCase {
            name: "close_button",
            width: 400,
            height: 60,
            component: test_close_button,
        },
        // ── Typography ───────────────────────────────────────────────────
        TestCase {
            name: "title_levels",
            width: 600,
            height: 300,
            component: test_title_levels,
        },
        TestCase {
            name: "text_sizes",
            width: 800,
            height: 200,
            component: test_text_sizes,
        },
        TestCase {
            name: "text_weights",
            width: 600,
            height: 200,
            component: test_text_weights,
        },
        TestCase {
            name: "text_colors",
            width: 800,
            height: 200,
            component: test_text_colors,
        },
        TestCase {
            name: "text_align",
            width: 600,
            height: 150,
            component: test_text_align,
        },
        TestCase {
            name: "code_variants",
            width: 600,
            height: 150,
            component: test_code_variants,
        },
        TestCase {
            name: "kbd_variants",
            width: 600,
            height: 60,
            component: test_kbd_variants,
        },
        TestCase {
            name: "highlight_colors",
            width: 800,
            height: 60,
            component: test_highlight_colors,
        },
        TestCase {
            name: "blockquote",
            width: 600,
            height: 200,
            component: test_blockquote,
        },
        TestCase {
            name: "anchor",
            width: 600,
            height: 60,
            component: test_anchor,
        },
        // ── Inputs ───────────────────────────────────────────────────────
        TestCase {
            name: "text_input_states",
            width: 400,
            height: 350,
            component: test_text_input_states,
        },
        TestCase {
            name: "text_input_sizes",
            width: 400,
            height: 300,
            component: test_text_input_sizes,
        },
        TestCase {
            name: "textarea",
            width: 400,
            height: 200,
            component: test_textarea,
        },
        TestCase {
            name: "password_input",
            width: 400,
            height: 120,
            component: test_password_input,
        },
        TestCase {
            name: "number_input",
            width: 400,
            height: 200,
            component: test_number_input,
        },
        TestCase {
            name: "checkbox_states",
            width: 400,
            height: 200,
            component: test_checkbox_states,
        },
        TestCase {
            name: "checkbox_sizes",
            width: 600,
            height: 60,
            component: test_checkbox_sizes,
        },
        TestCase {
            name: "switch_states",
            width: 400,
            height: 200,
            component: test_switch_states,
        },
        TestCase {
            name: "switch_sizes",
            width: 600,
            height: 60,
            component: test_switch_sizes,
        },
        TestCase {
            name: "radio_group",
            width: 400,
            height: 200,
            component: test_radio_group,
        },
        TestCase {
            name: "slider_basic",
            width: 400,
            height: 120,
            component: test_slider_basic,
        },
        TestCase {
            name: "select_basic",
            width: 400,
            height: 120,
            component: test_select_basic,
        },
        TestCase {
            name: "color_swatch",
            width: 600,
            height: 60,
            component: test_color_swatch,
        },
        // ── Layout ───────────────────────────────────────────────────────
        TestCase {
            name: "stack_layout",
            width: 400,
            height: 250,
            component: test_stack_layout,
        },
        TestCase {
            name: "stack_align",
            width: 400,
            height: 250,
            component: test_stack_align,
        },
        TestCase {
            name: "group_layout",
            width: 800,
            height: 60,
            component: test_group_layout,
        },
        TestCase {
            name: "group_justify",
            width: 800,
            height: 250,
            component: test_group_justify,
        },
        TestCase {
            name: "center_component",
            width: 400,
            height: 100,
            component: test_center_component,
        },
        TestCase {
            name: "space_component",
            width: 400,
            height: 200,
            component: test_space_component,
        },
        TestCase {
            name: "container",
            width: 800,
            height: 100,
            component: test_container,
        },
        TestCase {
            name: "simple_grid",
            width: 600,
            height: 200,
            component: test_simple_grid,
        },
        TestCase {
            name: "paper_shadows",
            width: 800,
            height: 120,
            component: test_paper_shadows,
        },
        TestCase {
            name: "paper_radius",
            width: 800,
            height: 120,
            component: test_paper_radius,
        },
        TestCase {
            name: "paper_border",
            width: 400,
            height: 120,
            component: test_paper_border,
        },
        TestCase {
            name: "card_basic",
            width: 400,
            height: 200,
            component: test_card_basic,
        },
        TestCase {
            name: "card_sections",
            width: 400,
            height: 300,
            component: test_card_sections,
        },
        TestCase {
            name: "divider_variants",
            width: 600,
            height: 200,
            component: test_divider_variants,
        },
        TestCase {
            name: "fieldset",
            width: 400,
            height: 200,
            component: test_fieldset,
        },
        // ── Data Display ─────────────────────────────────────────────────
        TestCase {
            name: "badge_variants",
            width: 800,
            height: 60,
            component: test_badge_variants,
        },
        TestCase {
            name: "badge_sizes",
            width: 600,
            height: 60,
            component: test_badge_sizes,
        },
        TestCase {
            name: "badge_colors",
            width: 800,
            height: 120,
            component: test_badge_colors,
        },
        TestCase {
            name: "avatar_initials",
            width: 600,
            height: 80,
            component: test_avatar_initials,
        },
        TestCase {
            name: "avatar_sizes",
            width: 600,
            height: 80,
            component: test_avatar_sizes,
        },
        TestCase {
            name: "list_unordered",
            width: 400,
            height: 150,
            component: test_list_unordered,
        },
        TestCase {
            name: "list_ordered",
            width: 400,
            height: 150,
            component: test_list_ordered,
        },
        TestCase {
            name: "breadcrumbs",
            width: 600,
            height: 100,
            component: test_breadcrumbs,
        },
        // ── Navigation ───────────────────────────────────────────────────
        TestCase {
            name: "tabs_default",
            width: 600,
            height: 150,
            component: test_tabs_default,
        },
        TestCase {
            name: "tabs_pills",
            width: 600,
            height: 150,
            component: test_tabs_pills,
        },
        TestCase {
            name: "navlink_basic",
            width: 300,
            height: 250,
            component: test_navlink_basic,
        },
        TestCase {
            name: "navlink_colors",
            width: 300,
            height: 200,
            component: test_navlink_colors,
        },
        TestCase {
            name: "pagination_basic",
            width: 600,
            height: 60,
            component: test_pagination_basic,
        },
        TestCase {
            name: "stepper_basic",
            width: 600,
            height: 100,
            component: test_stepper_basic,
        },
        TestCase {
            name: "accordion_basic",
            width: 400,
            height: 250,
            component: test_accordion_basic,
        },
        // ── Feedback ─────────────────────────────────────────────────────
        TestCase {
            name: "alert_colors",
            width: 600,
            height: 350,
            component: test_alert_colors,
        },
        TestCase {
            name: "alert_variants",
            width: 600,
            height: 300,
            component: test_alert_variants,
        },
        TestCase {
            name: "loader_types",
            width: 600,
            height: 80,
            component: test_loader_types,
        },
        TestCase {
            name: "loader_sizes",
            width: 600,
            height: 80,
            component: test_loader_sizes,
        },
        TestCase {
            name: "loader_colors",
            width: 600,
            height: 60,
            component: test_loader_colors,
        },
        TestCase {
            name: "progress_basic",
            width: 600,
            height: 100,
            component: test_progress_basic,
        },
        TestCase {
            name: "progress_colors",
            width: 600,
            height: 150,
            component: test_progress_colors,
        },
        TestCase {
            name: "skeleton_patterns",
            width: 400,
            height: 300,
            component: test_skeleton_patterns,
        },
        TestCase {
            name: "notification_basic",
            width: 400,
            height: 250,
            component: test_notification_basic,
        },
        // ── Overlays (static states) ─────────────────────────────────────
        TestCase {
            name: "tooltip_positions",
            width: 600,
            height: 150,
            component: test_tooltip_positions,
        },
        TestCase {
            name: "modal_static",
            width: 600,
            height: 400,
            component: test_modal_static,
        },
        TestCase {
            name: "loading_overlay",
            width: 400,
            height: 200,
            component: test_loading_overlay,
        },
        // ── CSS Primitives ───────────────────────────────────────────────
        TestCase {
            name: "css_borders",
            width: 800,
            height: 200,
            component: test_css_borders,
        },
        TestCase {
            name: "css_border_radius",
            width: 800,
            height: 200,
            component: test_css_border_radius,
        },
        TestCase {
            name: "css_shadows",
            width: 800,
            height: 200,
            component: test_css_shadows,
        },
        TestCase {
            name: "css_gradients",
            width: 800,
            height: 200,
            component: test_css_gradients,
        },
        TestCase {
            name: "css_opacity",
            width: 800,
            height: 100,
            component: test_css_opacity,
        },
        TestCase {
            name: "css_transforms",
            width: 800,
            height: 200,
            component: test_css_transforms,
        },
        TestCase {
            name: "css_overflow",
            width: 400,
            height: 150,
            component: test_css_overflow,
        },
        TestCase {
            name: "css_flexbox",
            width: 600,
            height: 300,
            component: test_css_flexbox,
        },
        // ── Images ─────────────────────────────────────────────────────
        TestCase {
            name: "image_object_fit",
            width: 800,
            height: 400,
            component: test_image_object_fit,
        },
        TestCase {
            name: "image_sizing",
            width: 600,
            height: 400,
            component: test_image_sizing,
        },
    ]
}

// ── Buttons ──────────────────────────────────────────────────────────────────

#[component]
fn test_button_variants() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                Button { variant: "filled", "Filled" }
                Button { variant: "light", "Light" }
                Button { variant: "outline", "Outline" }
                Button { variant: "subtle", "Subtle" }
                Button { variant: "default", "Default" }
            }
        }
    }
}

#[component]
fn test_button_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm", align: "center",
                Button { size: "xs", "XS" }
                Button { size: "sm", "SM" }
                Button { size: "md", "MD" }
                Button { size: "lg", "LG" }
                Button { size: "xl", "XL" }
            }
        }
    }
}

#[component]
fn test_button_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Group { gap: "xs",
                    Button { variant: "filled", color: "blue", "Blue" }
                    Button { variant: "filled", color: "cyan", "Cyan" }
                    Button { variant: "filled", color: "teal", "Teal" }
                    Button { variant: "filled", color: "green", "Green" }
                    Button { variant: "filled", color: "lime", "Lime" }
                    Button { variant: "filled", color: "yellow", "Yellow" }
                }
                Group { gap: "xs",
                    Button { variant: "filled", color: "orange", "Orange" }
                    Button { variant: "filled", color: "red", "Red" }
                    Button { variant: "filled", color: "pink", "Pink" }
                    Button { variant: "filled", color: "grape", "Grape" }
                    Button { variant: "filled", color: "violet", "Violet" }
                    Button { variant: "filled", color: "indigo", "Indigo" }
                }
            }
        }
    }
}

#[component]
fn test_button_radius() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                Button { radius: "xs", "Sharp" }
                Button { radius: "sm", "SM" }
                Button { radius: "md", "MD" }
                Button { radius: "lg", "LG" }
                Button { radius: "xl", "Pill" }
            }
        }
    }
}

#[component]
fn test_button_states() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                Button { disabled: true, "Disabled" }
                Button { variant: "outline", disabled: true, "Disabled Outline" }
                Button { loading: true, "Loading" }
                Button { variant: "light", loading: true, "Loading Light" }
            }
        }
    }
}

#[component]
fn test_button_full_width() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Button { full_width: true, variant: "filled", "Full Width Filled" }
                Button { full_width: true, variant: "outline", "Full Width Outline" }
                Button { full_width: true, variant: "light", "Full Width Light" }
                Button { full_width: true, variant: "subtle", "Full Width Subtle" }
            }
        }
    }
}

#[component]
fn test_action_icon_variants() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                ActionIcon { variant: "filled" }
                ActionIcon { variant: "light" }
                ActionIcon { variant: "outline" }
                ActionIcon { variant: "subtle" }
                ActionIcon { variant: "default" }
                ActionIcon { variant: "filled", color: "red" }
                ActionIcon { variant: "filled", color: "green" }
                ActionIcon { variant: "filled", disabled: true }
            }
        }
    }
}

#[component]
fn test_close_button() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                CloseButton {}
                CloseButton { size: "sm" }
                CloseButton { size: "lg" }
                CloseButton { disabled: true }
            }
        }
    }
}

// ── Typography ───────────────────────────────────────────────────────────────

#[component]
fn test_title_levels() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Title { order: 1, "Heading 1" }
                Title { order: 2, "Heading 2" }
                Title { order: 3, "Heading 3" }
                Title { order: 4, "Heading 4" }
                Title { order: 5, "Heading 5" }
                Title { order: 6, "Heading 6" }
            }
        }
    }
}

#[component]
fn test_text_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Text { size: "xs", "Extra Small Text (xs)" }
                Text { size: "sm", "Small Text (sm)" }
                Text { size: "md", "Medium Text (md)" }
                Text { size: "lg", "Large Text (lg)" }
                Text { size: "xl", "Extra Large Text (xl)" }
            }
        }
    }
}

#[component]
fn test_text_weights() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Text { weight: "400", "Normal (400)" }
                Text { weight: "500", "Medium (500)" }
                Text { weight: "600", "Semibold (600)" }
                Text { weight: "700", "Bold (700)" }
                Text { weight: "900", "Black (900)" }
            }
        }
    }
}

#[component]
fn test_text_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Group { gap: "md",
                    Text { color: "blue", "Blue" }
                    Text { color: "cyan", "Cyan" }
                    Text { color: "teal", "Teal" }
                    Text { color: "green", "Green" }
                    Text { color: "red", "Red" }
                }
                Group { gap: "md",
                    Text { color: "orange", "Orange" }
                    Text { color: "yellow", "Yellow" }
                    Text { color: "violet", "Violet" }
                    Text { color: "dimmed", "Dimmed" }
                    Text { color: "grape", "Grape" }
                }
            }
        }
    }
}

#[component]
fn test_text_align() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Text { align: "left", "Left aligned text" }
                Text { align: "center", "Center aligned text" }
                Text { align: "right", "Right aligned text" }
            }
        }
    }
}

#[component]
fn test_code_variants() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Group { gap: "sm",
                    Text { "Inline: " }
                    Code { "let x = 42;" }
                }
                Code { block: true, "fn main() {\n    println!(\"Hello\");\n}" }
                Code { color: "blue", "blue code" }
                Code { color: "red", "red code" }
            }
        }
    }
}

#[component]
fn test_kbd_variants() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                Kbd { "Ctrl" }
                Kbd { "+" }
                Kbd { "C" }
                Kbd { "Shift" }
                Kbd { "Cmd" }
                Kbd { "Enter" }
            }
        }
    }
}

#[component]
fn test_highlight_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "md",
                Highlight { highlight: "default", "Default highlight" }
                Highlight { highlight: "yellow", color: "yellow", "Yellow highlight" }
                Highlight { highlight: "cyan", color: "cyan", "Cyan highlight" }
                Highlight { highlight: "pink", color: "pink", "Pink highlight" }
            }
        }
    }
}

#[component]
fn test_blockquote() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                Blockquote { cite: "— Author",
                    "Life is what happens when you're busy making other plans."
                }
                Blockquote { color: "green", cite: "— Scientist",
                    "The important thing is not to stop questioning."
                }
            }
        }
    }
}

#[component]
fn test_anchor() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "md",
                Anchor { href: "#", "Default link" }
                Anchor { href: "#", size: "sm", "Small link" }
                Anchor { href: "#", size: "lg", "Large link" }
            }
        }
    }
}

// ── Inputs ───────────────────────────────────────────────────────────────────

#[component]
fn test_text_input_states() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                TextInput { label: "Username", placeholder: "Enter username" }
                TextInput { label: "Email", placeholder: "name@example.com", description: "We'll never share your email" }
                TextInput { label: "Error field", error: "This field is required" }
                TextInput { label: "Disabled", placeholder: "Cannot edit", disabled: true }
            }
        }
    }
}

#[component]
fn test_text_input_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                TextInput { label: "XS", size: "xs", placeholder: "Extra small" }
                TextInput { label: "SM", size: "sm", placeholder: "Small" }
                TextInput { label: "MD", size: "md", placeholder: "Medium" }
                TextInput { label: "LG", size: "lg", placeholder: "Large" }
            }
        }
    }
}

#[component]
fn test_textarea() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Textarea { label: "Comment", placeholder: "Write a comment...", description: "Markdown supported" }
                Textarea { label: "Error", error: "Too short" }
            }
        }
    }
}

#[component]
fn test_password_input() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                PasswordInput { label: "Password", placeholder: "Enter password" }
            }
        }
    }
}

#[component]
fn test_number_input() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                NumberInput { label: "Quantity", placeholder: "0", min: 0.0, max: 100.0, step: 1.0 }
                NumberInput { label: "Price", placeholder: "0.00", min: 0.0, step: 0.01 }
            }
        }
    }
}

#[component]
fn test_checkbox_states() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Checkbox { label: "Unchecked" }
                Checkbox { label: "Checked", checked: true }
                Checkbox { label: "Disabled unchecked", disabled: true }
                Checkbox { label: "Disabled checked", checked: true, disabled: true }
                Checkbox { label: "Indeterminate", indeterminate: true }
            }
        }
    }
}

#[component]
fn test_checkbox_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "lg",
                Checkbox { label: "XS", size: "xs", checked: true }
                Checkbox { label: "SM", size: "sm", checked: true }
                Checkbox { label: "MD", size: "md", checked: true }
                Checkbox { label: "LG", size: "lg", checked: true }
            }
        }
    }
}

#[component]
fn test_switch_states() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Switch { label: "Off" }
                Switch { label: "On", checked: true }
                Switch { label: "Disabled off", disabled: true }
                Switch { label: "Disabled on", checked: true, disabled: true }
            }
        }
    }
}

#[component]
fn test_switch_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "lg",
                Switch { label: "XS", size: "xs", checked: true }
                Switch { label: "SM", size: "sm", checked: true }
                Switch { label: "MD", size: "md", checked: true }
                Switch { label: "LG", size: "lg", checked: true }
            }
        }
    }
}

#[component]
fn test_radio_group() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            RadioGroup { label: "Select a plan", description: "Choose your subscription",
                Stack { gap: "xs",
                    Radio { name: "plan", value: "free", label: "Free" }
                    Radio { name: "plan", value: "pro", label: "Pro", checked: true }
                    Radio { name: "plan", value: "enterprise", label: "Enterprise" }
                }
            }
        }
    }
}

#[component]
fn test_slider_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                Slider { min: 0.0, max: 100.0, value: 40.0, label: "Volume" }
                Slider { min: 0.0, max: 100.0, value: 70.0, color: "orange", label: "Brightness" }
            }
        }
    }
}

#[component]
fn test_select_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Select { label: "Framework", placeholder: "Pick one" }
                Select { label: "Disabled", placeholder: "Cannot change", disabled: true }
            }
        }
    }
}

#[component]
fn test_color_swatch() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                ColorSwatch { color: "#228be6", size: "30px" }
                ColorSwatch { color: "#40c057", size: "30px" }
                ColorSwatch { color: "#fab005", size: "30px" }
                ColorSwatch { color: "#fa5252", size: "30px" }
                ColorSwatch { color: "#7950f2", size: "30px" }
                ColorSwatch { color: "rgba(0,0,0,0.5)", size: "30px" }
                ColorSwatch { color: "#228be6", size: "30px", radius: "xl" }
            }
        }
    }
}

// ── Layout ───────────────────────────────────────────────────────────────────

#[component]
fn test_stack_layout() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: white;",
            Stack { gap: "sm",
                div { style: "background: #228be6; color: white; padding: 8px; height: 35px;", "Item 1" }
                div { style: "background: #40c057; color: white; padding: 8px; height: 35px;", "Item 2" }
                div { style: "background: #fab005; color: white; padding: 8px; height: 35px;", "Item 3" }
                div { style: "background: #fa5252; color: white; padding: 8px; height: 35px;", "Item 4" }
            }
        }
    }
}

#[component]
fn test_stack_align() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Stack { gap: "xs", align: "flex-start",
                    div { style: "background: #228be6; color: white; padding: 4px 12px;", "Start" }
                }
                Stack { gap: "xs", align: "center",
                    div { style: "background: #40c057; color: white; padding: 4px 12px;", "Center" }
                }
                Stack { gap: "xs", align: "flex-end",
                    div { style: "background: #fab005; color: white; padding: 4px 12px;", "End" }
                }
                Stack { gap: "xs", align: "stretch",
                    div { style: "background: #fa5252; color: white; padding: 4px 12px;", "Stretch" }
                }
            }
        }
    }
}

#[component]
fn test_group_layout() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: white;",
            Group { gap: "md",
                div { style: "background: #228be6; color: white; padding: 8px 16px; height: 35px;", "Left" }
                div { style: "background: #40c057; color: white; padding: 8px 16px; height: 35px;", "Center" }
                div { style: "background: #fab005; color: white; padding: 8px 16px; height: 35px;", "Right" }
            }
        }
    }
}

#[component]
fn test_group_justify() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Group { justify: "flex-start",
                    Badge { "Start" } Badge { color: "green", "Items" }
                }
                Group { justify: "center",
                    Badge { "Center" } Badge { color: "green", "Items" }
                }
                Group { justify: "flex-end",
                    Badge { "End" } Badge { color: "green", "Items" }
                }
                Group { justify: "space-between",
                    Badge { "Between" } Badge { color: "green", "Items" }
                }
                Group { justify: "space-around",
                    Badge { "Around" } Badge { color: "green", "Items" }
                }
            }
        }
    }
}

#[component]
fn test_center_component() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Center {
                Badge { size: "lg", "Centered Badge" }
            }
        }
    }
}

#[component]
fn test_space_component() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "0",
                div { style: "background: #228be6; color: white; padding: 8px;", "Above xs" }
                Space { h: "xs" }
                div { style: "background: #40c057; color: white; padding: 8px;", "Above md" }
                Space { h: "md" }
                div { style: "background: #fab005; color: white; padding: 8px;", "Above xl" }
                Space { h: "xl" }
                div { style: "background: #fa5252; color: white; padding: 8px;", "Bottom" }
            }
        }
    }
}

#[component]
fn test_container() -> NodeHandle {
    rsx! {
        div { style: "background: #f0f0f0;",
            Container { size: "sm",
                div { style: "background: #228be6; color: white; padding: 12px;", "Container sm" }
            }
        }
    }
}

#[component]
fn test_simple_grid() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            SimpleGrid { cols: 3, spacing: "sm",
                div { style: "background: #228be6; color: white; padding: 12px; text-align: center;", "1" }
                div { style: "background: #40c057; color: white; padding: 12px; text-align: center;", "2" }
                div { style: "background: #fab005; color: white; padding: 12px; text-align: center;", "3" }
                div { style: "background: #fa5252; color: white; padding: 12px; text-align: center;", "4" }
                div { style: "background: #7950f2; color: white; padding: 12px; text-align: center;", "5" }
                div { style: "background: #e64980; color: white; padding: 12px; text-align: center;", "6" }
            }
        }
    }
}

#[component]
fn test_paper_shadows() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: #f0f0f0;",
            Group { gap: "md",
                Paper { shadow: "xs", p: "md", Text { "Shadow xs" } }
                Paper { shadow: "sm", p: "md", Text { "Shadow sm" } }
                Paper { shadow: "md", p: "md", Text { "Shadow md" } }
                Paper { shadow: "lg", p: "md", Text { "Shadow lg" } }
                Paper { shadow: "xl", p: "md", Text { "Shadow xl" } }
            }
        }
    }
}

#[component]
fn test_paper_radius() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: #f0f0f0;",
            Group { gap: "md",
                Paper { shadow: "sm", p: "md", radius: "xs", Text { "xs" } }
                Paper { shadow: "sm", p: "md", radius: "sm", Text { "sm" } }
                Paper { shadow: "sm", p: "md", radius: "md", Text { "md" } }
                Paper { shadow: "sm", p: "md", radius: "lg", Text { "lg" } }
                Paper { shadow: "sm", p: "md", radius: "xl", Text { "xl" } }
            }
        }
    }
}

#[component]
fn test_paper_border() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: white;",
            Stack { gap: "md",
                Paper { p: "md", with_border: true, Text { "With border" } }
                Paper { p: "md", shadow: "sm", with_border: true, Text { "Border + shadow" } }
            }
        }
    }
}

#[component]
fn test_card_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: #f0f0f0;",
            Card { shadow: "sm", p: "lg", radius: "md",
                Text { weight: "600", "Card Title" }
                Text { size: "sm", color: "dimmed", "Card description with some content." }
            }
        }
    }
}

#[component]
fn test_card_sections() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: #f0f0f0;",
            Card { shadow: "sm", p: "lg", radius: "md", with_border: true,
                CardSection { inherit_padding: true,
                    Text { weight: "600", "Section Header" }
                }
                Text { size: "sm", color: "dimmed", "Card body content between sections." }
                CardSection { inherit_padding: true,
                    Group { justify: "flex-end",
                        Button { variant: "subtle", "Cancel" }
                        Button { variant: "filled", "Submit" }
                    }
                }
            }
        }
    }
}

#[component]
fn test_divider_variants() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                Text { "Above divider" }
                Divider {}
                Text { "Below divider" }
                Divider { label: "OR" }
                Text { "Below labeled divider" }
                Divider { size: "sm" }
                Text { "Thick divider above" }
            }
        }
    }
}

#[component]
fn test_fieldset() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Fieldset { legend: "Personal Info",
                Stack { gap: "sm",
                    TextInput { label: "Name", placeholder: "Your name" }
                    TextInput { label: "Email", placeholder: "Your email" }
                }
            }
        }
    }
}

// ── Data Display ─────────────────────────────────────────────────────────────

#[component]
fn test_badge_variants() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                Badge { variant: "filled", "Filled" }
                Badge { variant: "light", "Light" }
                Badge { variant: "outline", "Outline" }
                Badge { variant: "dot", "Dot" }
            }
        }
    }
}

#[component]
fn test_badge_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm", align: "center",
                Badge { size: "xs", "XS" }
                Badge { size: "sm", "SM" }
                Badge { size: "md", "MD" }
                Badge { size: "lg", "LG" }
                Badge { size: "xl", "XL" }
            }
        }
    }
}

#[component]
fn test_badge_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "xs",
                Group { gap: "xs",
                    Badge { color: "blue", "Blue" }
                    Badge { color: "cyan", "Cyan" }
                    Badge { color: "teal", "Teal" }
                    Badge { color: "green", "Green" }
                    Badge { color: "lime", "Lime" }
                    Badge { color: "yellow", "Yellow" }
                }
                Group { gap: "xs",
                    Badge { color: "orange", "Orange" }
                    Badge { color: "red", "Red" }
                    Badge { color: "pink", "Pink" }
                    Badge { color: "grape", "Grape" }
                    Badge { color: "violet", "Violet" }
                    Badge { color: "indigo", "Indigo" }
                }
            }
        }
    }
}

#[component]
fn test_avatar_initials() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm",
                Avatar { name: "John Doe", color: "blue" }
                Avatar { name: "Jane Smith", color: "green" }
                Avatar { name: "Bob Wilson", color: "red" }
                Avatar { name: "Alice Brown", color: "violet" }
            }
        }
    }
}

#[component]
fn test_avatar_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "sm", align: "center",
                Avatar { name: "A", size: "xs" }
                Avatar { name: "B", size: "sm" }
                Avatar { name: "C", size: "md" }
                Avatar { name: "D", size: "lg" }
                Avatar { name: "E", size: "xl" }
            }
        }
    }
}

#[component]
fn test_list_unordered() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            List { r#type: "unordered",
                ListItem { "First item in the list" }
                ListItem { "Second item with content" }
                ListItem { "Third item here" }
            }
        }
    }
}

#[component]
fn test_list_ordered() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            List { r#type: "ordered",
                ListItem { "Step one" }
                ListItem { "Step two" }
                ListItem { "Step three" }
            }
        }
    }
}

#[component]
fn test_breadcrumbs() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                Breadcrumbs {
                    BreadcrumbsItem { href: "#", "Home" }
                    BreadcrumbsItem { href: "#", "Products" }
                    BreadcrumbsItem { "Details" }
                }
                Breadcrumbs { separator: ">",
                    BreadcrumbsItem { href: "#", "Root" }
                    BreadcrumbsItem { href: "#", "Folder" }
                    BreadcrumbsItem { "File" }
                }
            }
        }
    }
}

// ── Navigation ───────────────────────────────────────────────────────────────

#[component]
fn test_tabs_default() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Tabs { default_value: "gallery",
                TabsList {
                    Tab { value: "gallery", "Gallery" }
                    Tab { value: "messages", "Messages" }
                    Tab { value: "settings", "Settings" }
                }
                TabsPanel { value: "gallery",
                    Text { size: "sm", "Gallery tab content" }
                }
            }
        }
    }
}

#[component]
fn test_tabs_pills() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Tabs { default_value: "first", variant: "pills",
                TabsList {
                    Tab { value: "first", "First" }
                    Tab { value: "second", "Second" }
                    Tab { value: "third", "Third" }
                }
                TabsPanel { value: "first",
                    Text { size: "sm", "First tab content" }
                }
            }
        }
    }
}

#[component]
fn test_navlink_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "0",
                NavLink { label: "Home", active: true }
                NavLink { label: "Dashboard" }
                NavLink { label: "Settings" }
                NavLink { label: "Profile" }
            }
        }
    }
}

#[component]
fn test_navlink_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "0",
                NavLink { label: "Blue", active: true, color: "blue" }
                NavLink { label: "Green", active: true, color: "green" }
                NavLink { label: "Red", active: true, color: "red" }
                NavLink { label: "Violet", active: true, color: "violet" }
            }
        }
    }
}

#[component]
fn test_pagination_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Pagination { total: { 10u32 }, value: { 3u32 }, siblings: { 1u32 } }
        }
    }
}

#[component]
fn test_stepper_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stepper { active: { 1u32 },
                StepperStep { label: "Account", description: "Create account" }
                StepperStep { label: "Verify", description: "Verify email" }
                StepperStep { label: "Complete", description: "All done" }
            }
        }
    }
}

#[component]
fn test_accordion_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Accordion { default_value: "item-1",
                AccordionItem { value: "item-1",
                    AccordionControl { "First Section" }
                    AccordionPanel { Text { size: "sm", "Content of the first section." } }
                }
                AccordionItem { value: "item-2",
                    AccordionControl { "Second Section" }
                    AccordionPanel { Text { size: "sm", "Content of the second section." } }
                }
                AccordionItem { value: "item-3",
                    AccordionControl { "Third Section" }
                    AccordionPanel { Text { size: "sm", "Content of the third section." } }
                }
            }
        }
    }
}

// ── Feedback ─────────────────────────────────────────────────────────────────

#[component]
fn test_alert_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Alert { color: "blue", title: "Information", "This is an info alert." }
                Alert { color: "green", title: "Success", "Operation completed." }
                Alert { color: "yellow", title: "Warning", "Please be careful." }
                Alert { color: "red", title: "Error", "Something went wrong." }
            }
        }
    }
}

#[component]
fn test_alert_variants() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Alert { variant: "light", color: "blue", title: "Light", "Light variant alert." }
                Alert { variant: "filled", color: "blue", title: "Filled", "Filled variant alert." }
                Alert { variant: "outline", color: "blue", title: "Outline", "Outline variant alert." }
            }
        }
    }
}

#[component]
fn test_loader_types() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "lg",
                Loader { r#type: "oval" }
                Loader { r#type: "dots" }
                Loader { r#type: "bars" }
            }
        }
    }
}

#[component]
fn test_loader_sizes() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "lg", align: "center",
                Loader { size: "xs" }
                Loader { size: "sm" }
                Loader { size: "md" }
                Loader { size: "lg" }
                Loader { size: "xl" }
            }
        }
    }
}

#[component]
fn test_loader_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "lg",
                Loader { color: "blue" }
                Loader { color: "cyan" }
                Loader { color: "green" }
                Loader { color: "violet" }
                Loader { color: "red" }
            }
        }
    }
}

#[component]
fn test_progress_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                Progress { value: 25.0 }
                Progress { value: 50.0, color: "green" }
                Progress { value: 75.0, color: "orange", size: "lg" }
            }
        }
    }
}

#[component]
fn test_progress_colors() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "sm",
                Progress { value: 60.0, size: "xs", color: "cyan" }
                Progress { value: 60.0, size: "sm", color: "teal" }
                Progress { value: 60.0, size: "md", color: "green" }
                Progress { value: 60.0, size: "lg", color: "violet" }
            }
        }
    }
}

#[component]
fn test_skeleton_patterns() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                // Text skeleton
                Skeleton { height: "8px", radius: "xl" }
                Skeleton { height: "8px", width: "70%", radius: "xl" }
                // Profile skeleton
                Group { gap: "sm",
                    Skeleton { height: "40px", width: "40px", circle: true }
                    Stack { gap: "xs",
                        Skeleton { height: "8px", width: "120px" }
                        Skeleton { height: "8px", width: "80px" }
                    }
                }
                // Card skeleton
                Skeleton { height: "120px", radius: "md" }
                Skeleton { height: "12px", width: "60%" }
                Skeleton { height: "8px" }
                Skeleton { height: "8px", width: "80%" }
            }
        }
    }
}

#[component]
fn test_notification_basic() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                Notification { title: "Success", color: "green",
                    "Your changes have been saved."
                }
                Notification { title: "Error", color: "red",
                    "Failed to save changes."
                }
                Notification { title: "Info", color: "blue",
                    "A new update is available."
                }
            }
        }
    }
}

// ── Overlays (static states) ─────────────────────────────────────────────────

#[component]
fn test_tooltip_positions() -> NodeHandle {
    rsx! {
        div { style: "padding: 40px 12px; background: white;",
            Group { gap: "xl", justify: "center",
                Tooltip { label: "Top tooltip", position: "top",
                    Button { variant: "outline", "Top" }
                }
                Tooltip { label: "Right tooltip", position: "right",
                    Button { variant: "outline", "Right" }
                }
                Tooltip { label: "Bottom tooltip", position: "bottom",
                    Button { variant: "outline", "Bottom" }
                }
            }
        }
    }
}

#[component]
fn test_modal_static() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white; position: relative;",
            // Render a modal-like structure statically
            div { style: "background: rgba(0,0,0,0.5); padding: 40px; border-radius: 8px;",
                div { style: "background: white; padding: 20px; border-radius: 8px; max-width: 500px;",
                    Stack { gap: "md",
                        Title { order: 3, "Modal Title" }
                        Text { size: "sm", "This is the modal body content. It demonstrates how a modal looks when rendered." }
                        Group { justify: "flex-end",
                            Button { variant: "subtle", "Cancel" }
                            Button { variant: "filled", "Confirm" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn test_loading_overlay() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            div { style: "position: relative; height: 160px;",
                Stack { gap: "sm", p: "md",
                    Text { "Content behind overlay" }
                    Text { size: "sm", color: "dimmed", "This text should be dimmed by the overlay." }
                }
                LoadingOverlay { visible: true }
            }
        }
    }
}

// ── CSS Primitives ───────────────────────────────────────────────────────────

#[component]
fn test_css_borders() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: white;",
            Group { gap: "md",
                div { style: "width: 80px; height: 80px; border: 2px solid #228be6;", "" }
                div { style: "width: 80px; height: 80px; border: 3px dashed #fa5252;", "" }
                div { style: "width: 80px; height: 80px; border: 1px solid #868e96; border-left: 4px solid #228be6;", "" }
                div { style: "width: 80px; height: 80px; border-top: 2px solid red; border-right: 2px solid green; border-bottom: 2px solid blue; border-left: 2px solid orange;", "" }
            }
        }
    }
}

#[component]
fn test_css_border_radius() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: white;",
            Group { gap: "md",
                div { style: "width: 80px; height: 80px; background: #228be6; border-radius: 4px;", "" }
                div { style: "width: 80px; height: 80px; background: #40c057; border-radius: 12px;", "" }
                div { style: "width: 80px; height: 80px; background: #fab005; border-radius: 50%;", "" }
                div { style: "width: 80px; height: 80px; background: #fa5252; border-radius: 20px 0;", "" }
            }
        }
    }
}

#[component]
fn test_css_shadows() -> NodeHandle {
    rsx! {
        div { style: "padding: 24px; background: white;",
            Group { gap: "lg",
                div { style: "width: 80px; height: 80px; background: white; box-shadow: 0 1px 3px rgba(0,0,0,0.12);", "" }
                div { style: "width: 80px; height: 80px; background: white; box-shadow: 0 4px 8px rgba(0,0,0,0.15);", "" }
                div { style: "width: 80px; height: 80px; background: white; box-shadow: 0 8px 16px rgba(0,0,0,0.2);", "" }
                div { style: "width: 80px; height: 80px; background: white; box-shadow: 0 2px 4px rgba(0,0,255,0.3);", "" }
            }
        }
    }
}

#[component]
fn test_css_gradients() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: white;",
            Group { gap: "md",
                div { style: "width: 120px; height: 80px; background: linear-gradient(to right, #228be6, #40c057);", "" }
                div { style: "width: 120px; height: 80px; background: linear-gradient(135deg, #fa5252, #fab005);", "" }
                div { style: "width: 120px; height: 80px; background: linear-gradient(to bottom, #7950f2, #228be6, #40c057);", "" }
            }
        }
    }
}

#[component]
fn test_css_opacity() -> NodeHandle {
    rsx! {
        div { style: "padding: 16px; background: white;",
            Group { gap: "md",
                div { style: "width: 80px; height: 60px; background: #228be6; opacity: 1.0;", "" }
                div { style: "width: 80px; height: 60px; background: #228be6; opacity: 0.75;", "" }
                div { style: "width: 80px; height: 60px; background: #228be6; opacity: 0.5;", "" }
                div { style: "width: 80px; height: 60px; background: #228be6; opacity: 0.25;", "" }
            }
        }
    }
}

#[component]
fn test_css_transforms() -> NodeHandle {
    rsx! {
        div { style: "padding: 32px; background: white;",
            Group { gap: "lg",
                div { style: "width: 60px; height: 60px; background: #228be6; color: white; display: flex; align-items: center; justify-content: center; transform: rotate(45deg);",
                    Text { size: "xs", style: "color: white", "45deg" }
                }
                div { style: "width: 60px; height: 60px; background: #40c057; color: white; display: flex; align-items: center; justify-content: center; transform: scale(1.2);",
                    Text { size: "xs", style: "color: white", "1.2x" }
                }
                div { style: "width: 60px; height: 60px; background: #fab005; color: white; display: flex; align-items: center; justify-content: center; transform: skewX(10deg);",
                    Text { size: "xs", style: "color: white", "skew" }
                }
                div { style: "width: 60px; height: 60px; background: #fa5252; color: white; display: flex; align-items: center; justify-content: center; transform: rotate(-10deg) scale(0.9);",
                    Text { size: "xs", style: "color: white", "combo" }
                }
            }
        }
    }
}

#[component]
fn test_css_overflow() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Group { gap: "md",
                div { style: "width: 100px; height: 50px; overflow: hidden; background: #f0f0f0; border: 1px solid #ccc;",
                    div { style: "width: 200px; height: 100px; background: #228be6; color: white; padding: 4px;", "Clipped overflow content that is wider and taller" }
                }
                div { style: "width: 100px; height: 50px; overflow: visible; background: #f0f0f0; border: 1px solid #ccc;",
                    div { style: "width: 150px; height: 80px; background: rgba(250,82,82,0.5); padding: 4px;", "Visible overflow" }
                }
            }
        }
    }
}

#[component]
fn test_css_flexbox() -> NodeHandle {
    rsx! {
        div { style: "padding: 12px; background: white;",
            Stack { gap: "md",
                // Flex grow
                div { style: "display: flex; gap: 8px; height: 30px;",
                    div { style: "flex: 1; background: #228be6;", "" }
                    div { style: "flex: 2; background: #40c057;", "" }
                    div { style: "flex: 1; background: #fab005;", "" }
                }
                // Flex align items
                div { style: "display: flex; gap: 8px; height: 60px; align-items: center;",
                    div { style: "width: 40px; height: 20px; background: #228be6;", "" }
                    div { style: "width: 40px; height: 40px; background: #40c057;", "" }
                    div { style: "width: 40px; height: 30px; background: #fab005;", "" }
                }
                // Flex wrap
                div { style: "display: flex; flex-wrap: wrap; gap: 8px; width: 200px;",
                    div { style: "width: 80px; height: 30px; background: #228be6;", "" }
                    div { style: "width: 80px; height: 30px; background: #40c057;", "" }
                    div { style: "width: 80px; height: 30px; background: #fab005;", "" }
                    div { style: "width: 80px; height: 30px; background: #fa5252;", "" }
                }
                // Flex column
                div { style: "display: flex; flex-direction: column; gap: 4px; height: 80px;",
                    div { style: "flex: 1; background: #228be6;", "" }
                    div { style: "flex: 1; background: #40c057;", "" }
                }
            }
        }
    }
}

// ── Images ──────────────────────────────────────────────────────────────────

/// Generate a small test PNG as a data URI.
/// Creates a colored rectangle with a diagonal line pattern for easy visual identification.
fn make_test_image_data_uri(width: u32, height: u32) -> String {
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            // Gradient: red increases left→right, blue increases top→bottom
            let r = (x * 255 / width.max(1)) as u8;
            let g = 80u8;
            let b = (y * 255 / height.max(1)) as u8;
            // Diagonal stripe pattern every 16px
            let on_stripe = ((x + y) / 8) % 2 == 0;
            if on_stripe {
                pixels[i] = r;
                pixels[i + 1] = g;
                pixels[i + 2] = b;
            } else {
                pixels[i] = r.saturating_add(40);
                pixels[i + 1] = g.saturating_add(40);
                pixels[i + 2] = b.saturating_add(40);
            }
            pixels[i + 3] = 255;
        }
    }

    // Encode to PNG
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(&pixels).expect("PNG data");
    }

    // Base64 encode
    let mut b64 = String::new();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut i = 0;
    while i < png_data.len() {
        let b0 = png_data[i] as u32;
        let b1 = if i + 1 < png_data.len() {
            png_data[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < png_data.len() {
            png_data[i + 2] as u32
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        b64.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        b64.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < png_data.len() {
            b64.push(alphabet[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            b64.push('=');
        }
        if i + 2 < png_data.len() {
            b64.push(alphabet[(triple & 0x3F) as usize] as char);
        } else {
            b64.push('=');
        }
        i += 3;
    }

    format!("data:image/png;base64,{}", b64)
}

fn test_image_object_fit(__scope: &mut RenderScope) -> NodeHandle {
    // Use a 100x60 source image in various 120x120 containers to test all object-fit modes
    let data_uri = make_test_image_data_uri(100, 60);
    let tall_uri = make_test_image_data_uri(60, 100);

    // Build imperatively to avoid rsx closure move issues with data_uri
    let root = __scope.create_element("div");
    root.set_attribute(
        "style",
        "display: flex; flex-direction: column; gap: 12px; padding: 12px; background: #f8f9fa;",
    );

    // Helper to create a labeled image cell
    let make_cell = |scope: &mut RenderScope, label: &str, fit: &str, src: &str| {
        let col = scope.create_element("div");
        col.set_attribute(
            "style",
            "display: flex; flex-direction: column; align-items: center; gap: 4px;",
        );

        let lbl = scope.create_element("div");
        lbl.set_attribute("style", "font-size: 12px; text-align: center;");
        let txt = scope.create_text(label);
        lbl.append_child(&txt);
        col.append_child(&lbl);

        let container = scope.create_element("div");
        container.set_attribute(
            "style",
            "width: 120px; height: 120px; border: 1px solid #dee2e6;",
        );

        let img = scope.create_element("img");
        img.set_attribute(
            "style",
            &format!("width: 120px; height: 120px; object-fit: {};", fit),
        );
        img.set_attribute("src", src);
        container.append_child(&img);
        col.append_child(&container);
        col
    };

    // Wide image row (100x60 source)
    let row1 = __scope.create_element("div");
    row1.set_attribute("style", "display: flex; gap: 12px;");
    for (label, fit) in [
        ("fill", "fill"),
        ("contain", "contain"),
        ("cover", "cover"),
        ("none", "none"),
        ("scale-down", "scale-down"),
    ] {
        let cell = make_cell(__scope, label, fit, &data_uri);
        row1.append_child(&cell);
    }
    root.append_child(&row1);

    // Tall image row (60x100 source)
    let row2 = __scope.create_element("div");
    row2.set_attribute("style", "display: flex; gap: 12px;");
    for (label, fit) in [
        ("fill (tall)", "fill"),
        ("contain (tall)", "contain"),
        ("cover (tall)", "cover"),
        ("none (tall)", "none"),
        ("scale-down (tall)", "scale-down"),
    ] {
        let cell = make_cell(__scope, label, fit, &tall_uri);
        row2.append_child(&cell);
    }
    root.append_child(&row2);

    root
}

fn test_image_sizing(__scope: &mut RenderScope) -> NodeHandle {
    let data_uri = make_test_image_data_uri(200, 120);

    let root = __scope.create_element("div");
    root.set_attribute(
        "style",
        "display: flex; flex-direction: column; gap: 12px; padding: 12px; background: #f8f9fa;",
    );

    let make_case = |scope: &mut RenderScope, label: &str, img_style: &str, src: &str| {
        let col = scope.create_element("div");
        col.set_attribute("style", "display: flex; flex-direction: column; gap: 4px;");

        let lbl = scope.create_element("div");
        lbl.set_attribute("style", "font-size: 12px;");
        let txt = scope.create_text(label);
        lbl.append_child(&txt);
        col.append_child(&lbl);

        let container = scope.create_element("div");
        container.set_attribute("style", "border: 1px solid #dee2e6;");

        let img = scope.create_element("img");
        if !img_style.is_empty() {
            img.set_attribute("style", img_style);
        }
        img.set_attribute("src", src);
        container.append_child(&img);
        col.append_child(&container);
        col
    };

    // Row 1: single-dimension sizing (aspect ratio preservation)
    let row1 = __scope.create_element("div");
    row1.set_attribute(
        "style",
        "display: flex; gap: 16px; align-items: flex-start;",
    );
    row1.append_child(&make_case(
        __scope,
        "width: 100px (auto height)",
        "width: 100px;",
        &data_uri,
    ));
    row1.append_child(&make_case(
        __scope,
        "height: 80px (auto width)",
        "height: 80px;",
        &data_uri,
    ));
    row1.append_child(&make_case(__scope, "no size (intrinsic)", "", &data_uri));
    root.append_child(&row1);

    // Row 2: explicit width+height with object-fit
    let row2 = __scope.create_element("div");
    row2.set_attribute(
        "style",
        "display: flex; gap: 16px; align-items: flex-start;",
    );
    row2.append_child(&make_case(
        __scope,
        "100x100 (fill)",
        "width: 100px; height: 100px;",
        &data_uri,
    ));
    row2.append_child(&make_case(
        __scope,
        "100x100 (contain)",
        "width: 100px; height: 100px; object-fit: contain;",
        &data_uri,
    ));
    row2.append_child(&make_case(
        __scope,
        "100x100 (cover)",
        "width: 100px; height: 100px; object-fit: cover;",
        &data_uri,
    ));
    root.append_child(&row2);

    root
}

// ── Rendering backends ───────────────────────────────────────────────────────

fn encode_png(pixels: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(pixels).expect("PNG data");
    }
    png_data
}

/// Software renderer: RinchApp → TinySkiaPainter → RGBA pixels → PNG
#[cfg(not(feature = "gpu"))]
fn render_to_png(
    component: fn(&mut RenderScope) -> NodeHandle,
    width: u32,
    height: u32,
) -> Vec<u8> {
    use rinch::app::RinchApp;

    let mut app = RinchApp::new(component);
    app.mount_component(width as f32, height as f32);
    app.mark_scene_dirty();

    let (pixels, w, h) = app.build_pixels(1.0, (width, height));
    encode_png(pixels, w, h)
}

/// GPU renderer: RinchApp → Vello Scene → wgpu texture → readback → PNG
#[cfg(feature = "gpu")]
fn render_to_png(
    component: fn(&mut RenderScope) -> NodeHandle,
    width: u32,
    height: u32,
) -> Vec<u8> {
    use rinch::app::RinchApp;
    use rinch::embed::RinchOverlayRenderer;
    use rinch::shell::screenshot::capture_texture_rgba;
    use wgpu::TextureFormat;

    // Create headless wgpu device
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::from_build_config().with_env(),
        ..Default::default()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("No suitable GPU adapter found");

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("render-test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .expect("Failed to create device");

    // Build the component and scene
    let mut app = RinchApp::new(component);
    app.mount_component(width as f32, height as f32);
    app.mark_scene_dirty();

    let scene = app.build_scene(1.0, (width, height));

    // Render scene to texture via Vello
    let format = TextureFormat::Rgba8Unorm;
    let mut overlay = RinchOverlayRenderer::new(&device, width, height, format);
    let _view = overlay.render(&device, &queue, scene);

    // Read back pixels from the render texture
    let rgba = capture_texture_rgba(&device, &queue, overlay.texture(), width, height, format)
        .expect("Failed to capture texture");

    encode_png(&rgba, width, height)
}

// ── HTML dump ─────────────────────────────────────────────────────────────────

/// Mount each test component, run layout, then serialize its DOM tree to HTML.
fn dump_html_baselines(tests: &[TestCase], filter: &[&str], output_dir: &Path) {
    use rinch::app::RinchApp;
    use rinch_dom::html_serializer::serialize_to_html;

    // Get the full CSS (theme + components)
    let css = rinch_core::get_current_theme_css().unwrap_or_default();

    let html_dir = output_dir.join("html");
    std::fs::create_dir_all(&html_dir).expect("create html output dir");

    let tests_to_run: Vec<&TestCase> = if filter.is_empty() {
        tests.iter().collect()
    } else {
        tests
            .iter()
            .filter(|t| filter.iter().any(|f| t.name.contains(f)))
            .collect()
    };

    println!(
        "Dumping {} HTML baseline(s) to {}",
        tests_to_run.len(),
        html_dir.display()
    );

    for test in &tests_to_run {
        let mut app = RinchApp::new(test.component);
        app.mount_component(test.width as f32, test.height as f32);

        let doc = app.doc().expect("document should be mounted");
        let doc = doc.borrow();
        let html = serialize_to_html(&doc.tree, &css, test.width, test.height);

        let path = html_dir.join(format!("{}.html", test.name));
        std::fs::write(&path, &html).expect("write HTML");
        println!(
            "  {} ({}x{}) -> {}",
            test.name,
            test.width,
            test.height,
            path.display()
        );
    }

    println!("Done!");
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn renderer_name() -> &'static str {
    if cfg!(feature = "gpu") {
        "gpu"
    } else {
        "software"
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse --output dir
    let default_output = format!("renderer-compare/{}", renderer_name());
    let mut output_dir = PathBuf::from(&default_output);
    let mut filter: Vec<&str> = Vec::new();
    let mut list_mode = false;
    let mut dump_css = false;
    let mut dump_html = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                if i < args.len() {
                    output_dir = PathBuf::from(&args[i]);
                }
            }
            "--list" | "-l" => {
                list_mode = true;
            }
            "--dump-css" => {
                dump_css = true;
            }
            "--dump-html" => {
                dump_html = true;
            }
            arg => {
                filter.push(arg);
            }
        }
        i += 1;
    }

    let tests = test_cases();

    if list_mode {
        for t in &tests {
            println!("  {} ({}x{})", t.name, t.width, t.height);
        }
        println!("\n{} tests total", tests.len());
        return;
    }

    if dump_css {
        // Dump the full CSS (theme + components) to stdout
        rinch::update_theme(&ThemeProviderProps::default());
        if let Some(css) = rinch_core::get_current_theme_css() {
            print!("{}", css);
        }
        return;
    }

    // Initialize theme CSS before rendering any components
    rinch::update_theme(&ThemeProviderProps::default());

    if dump_html {
        dump_html_baselines(&tests, &filter, &output_dir);
        return;
    }

    std::fs::create_dir_all(&output_dir).expect("create output dir");

    let tests_to_run: Vec<&TestCase> = if filter.is_empty() {
        tests.iter().collect()
    } else {
        tests
            .iter()
            .filter(|t| filter.iter().any(|f| t.name.contains(f)))
            .collect()
    };

    println!(
        "Rendering {} test(s) via {} to {}",
        tests_to_run.len(),
        renderer_name(),
        output_dir.display()
    );

    for test in &tests_to_run {
        let png = render_to_png(test.component, test.width, test.height);
        let path = output_dir.join(format!("{}.png", test.name));
        std::fs::write(&path, &png).expect("write PNG");
        println!(
            "  {} ({}x{}) -> {} bytes",
            test.name,
            test.width,
            test.height,
            png.len()
        );
    }

    println!("Done!");
}
