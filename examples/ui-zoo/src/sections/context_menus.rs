//! Context Menu section - rich interactive context menus demo.

use rinch::prelude::*;

#[component]
pub fn context_menus_section() -> NodeHandle {
    let last_action = Signal::new(String::new());
    let bold = Signal::new(false);
    let italic = Signal::new(false);
    let underline = Signal::new(false);
    let font_size = Signal::new(14.0);
    let opacity = Signal::new(100.0);

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Context Menu" }
                Text { size: "lg", color: "dimmed",
                    "Rich, interactive context menus with arbitrary components — not just text labels."
                }
            }
            Space { h: "xl" }

            // Last action display
            Paper { p: "md", radius: "md", with_border: true,
                Group { gap: "sm",
                    Text { weight: "600", "Last action:" }
                    Text { color: "dimmed", {|| {
                        let v = last_action.get();
                        if v.is_empty() { "None".to_string() } else { v }
                    }} }
                }
            }
            Space { h: "lg" }

            // --- Rich Context Menu Demo ---
            Title { order: 3, "Rich Context Menu" }
            Text { size: "sm", color: "dimmed",
                "Right-click the text below to get a formatting menu with live controls."
            }
            Space { h: "sm" }

            ContextMenu {
                ContextMenuTarget {
                    Paper { p: "lg", radius: "md", with_border: true, style: "cursor: context-menu; min-height: 80px;",
                        p {
                            style: {|| {
                                let mut s = format!("font-size: {}px; opacity: {};",
                                    font_size.get(),
                                    opacity.get() / 100.0);
                                if bold.get() { s.push_str(" font-weight: bold;"); }
                                if italic.get() { s.push_str(" font-style: italic;"); }
                                if underline.get() { s.push_str(" text-decoration: underline;"); }
                                s
                            }},
                            "Right-click me to format this text. The context menu contains checkboxes, a slider, and action buttons — things native OS menus can't do."
                        }
                    }
                }
                ContextMenuDropdown {
                    // Checkboxes for text formatting
                    div { style: "padding: 6px 8px;",
                        Checkbox {
                            label: "Bold",
                            checked_fn: move || bold.get(),
                            onchange: move || bold.update(|v| *v = !*v)
                        }
                    }
                    div { style: "padding: 6px 8px;",
                        Checkbox {
                            label: "Italic",
                            checked_fn: move || italic.get(),
                            onchange: move || italic.update(|v| *v = !*v)
                        }
                    }
                    div { style: "padding: 6px 8px;",
                        Checkbox {
                            label: "Underline",
                            checked_fn: move || underline.get(),
                            onchange: move || underline.update(|v| *v = !*v)
                        }
                    }

                    DropdownMenuDivider {}

                    // Font size slider
                    div { style: "padding: 8px 12px;",
                        Text { size: "xs", color: "dimmed", "Font Size" }
                        Space { h: "xs" }
                        Slider {
                            min: 8.0,
                            max: 32.0,
                            step: 1.0,
                            value_signal: Some(font_size),
                            onchange: move |v: f64| font_size.set(v),
                            size: "sm"
                        }
                    }

                    // Opacity slider
                    div { style: "padding: 8px 12px;",
                        Text { size: "xs", color: "dimmed", "Opacity" }
                        Space { h: "xs" }
                        Slider {
                            min: 10.0,
                            max: 100.0,
                            step: 5.0,
                            value_signal: Some(opacity),
                            onchange: move |v: f64| opacity.set(v),
                            size: "sm"
                        }
                    }

                    DropdownMenuDivider {}

                    // Regular menu items
                    DropdownMenuItem {
                        onclick: {
                            let la = last_action;
                            move || {
                                bold.set(false);
                                italic.set(false);
                                underline.set(false);
                                font_size.set(14.0);
                                opacity.set(100.0);
                                la.set("Reset formatting".into());
                            }
                        },
                        "Reset All"
                    }
                }
            }

            Space { h: "xl" }

            // --- Simple Context Menu ---
            Title { order: 3, "Simple Context Menu" }
            Text { size: "sm", color: "dimmed",
                "A basic context menu with action items. Right-click the targets below."
            }
            Space { h: "sm" }

            Group { gap: "md",
                {corner_target(__scope, last_action, "Target A")}
                {corner_target(__scope, last_action, "Target B")}
            }
        }
    }
}

fn corner_target(
    __scope: &mut RenderScope,
    last_action: Signal<String>,
    label: &str,
) -> NodeHandle {
    let label_owned = label.to_string();
    rsx! {
        ContextMenu {
            ContextMenuTarget {
                div {
                    style: "padding: 24px 32px; background: var(--rinch-color-default-hover); border-radius: var(--rinch-radius-default); cursor: context-menu;",
                    Text { color: "dimmed", size: "sm", {label_owned.clone()} }
                }
            }
            ContextMenuDropdown {
                DropdownMenuItem {
                    onclick: { let l = label_owned.clone(); move || last_action.set(format!("Edit ({})", l)) },
                    "Edit"
                }
                DropdownMenuItem {
                    onclick: { let l = label_owned.clone(); move || last_action.set(format!("Duplicate ({})", l)) },
                    "Duplicate"
                }
                DropdownMenuDivider {}
                DropdownMenuItem {
                    color: "red",
                    onclick: { let l = label_owned.clone(); move || last_action.set(format!("Delete ({})", l)) },
                    "Delete"
                }
            }
        }
    }
}
