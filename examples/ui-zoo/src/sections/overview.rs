//! Overview section - dashboard with stats and quick intro.

use rinch::prelude::*;

#[component]
pub fn overview_section() -> NodeHandle {
    rsx! {
        Fragment {
            // Hero section
            Center {
                Stack { align: "center", gap: "md",
                    Title { order: 1, "Welcome to UI Zoo" }
                    Text { size: "lg", color: "dimmed",
                        "A comprehensive showcase of all Rinch components"
                    }
                }
            }

            Space { h: "xl" }

            // Stats cards
            SimpleGrid { cols: Some(4), spacing: "md",
                Paper { p: "xl", radius: "md", shadow: "sm",
                    Center {
                        Stack { align: "center", gap: "xs",
                            Text { size: "xl", weight: "700", color: "blue", "50+" }
                            Text { size: "sm", color: "dimmed", "Components" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", shadow: "sm",
                    Center {
                        Stack { align: "center", gap: "xs",
                            Text { size: "xl", weight: "700", color: "cyan", "9" }
                            Text { size: "sm", color: "dimmed", "Categories" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", shadow: "sm",
                    Center {
                        Stack { align: "center", gap: "xs",
                            Text { size: "xl", weight: "700", color: "teal", "18" }
                            Text { size: "sm", color: "dimmed", "Color Palettes" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", shadow: "sm",
                    Center {
                        Stack { align: "center", gap: "xs",
                            Text { size: "xl", weight: "700", color: "violet", "5" }
                            Text { size: "sm", color: "dimmed", "Size Variants" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Feature highlights
            Title { order: 3, "Key Features" }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { gap: "sm",
                            Badge { color: "blue", variant: "light", size: "lg", "Theming" }
                        }
                        Title { order: 4, "Theme System" }
                        Text { size: "sm", color: "dimmed",
                            "Full theming support with CSS variables. Switch between light and dark mode, customize colors, spacing, and typography. All components automatically adapt to your theme."
                        }
                        Group { gap: "xs",
                            Badge { color: "blue", "CSS Variables" }
                            Badge { color: "cyan", "Dark Mode" }
                            Badge { color: "teal", "Customizable" }
                        }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { gap: "sm",
                            Badge { color: "violet", variant: "light", size: "lg", "Reactivity" }
                        }
                        Title { order: 4, "Reactive State" }
                        Text { size: "sm", color: "dimmed",
                            "Built-in reactive primitives with use_signal, use_effect, and use_derived for automatic UI updates. Fine-grained reactivity ensures minimal re-renders."
                        }
                        Group { gap: "xs",
                            Badge { color: "violet", "Signals" }
                            Badge { color: "grape", "Effects" }
                            Badge { color: "pink", "Derived" }
                        }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { gap: "sm",
                            Badge { color: "orange", variant: "light", size: "lg", "Layout" }
                        }
                        Title { order: 4, "Layout Components" }
                        Text { size: "sm", color: "dimmed",
                            "Flexbox-based layout primitives: Stack, Group, Grid, Center, and Container. Build complex layouts with simple, composable components."
                        }
                        Group { gap: "xs",
                            Badge { color: "orange", "Stack" }
                            Badge { color: "yellow", "Group" }
                            Badge { color: "lime", "Grid" }
                        }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { gap: "sm",
                            Badge { color: "green", variant: "light", size: "lg", "Forms" }
                        }
                        Title { order: 4, "Form Controls" }
                        Text { size: "sm", color: "dimmed",
                            "Complete form input suite with validation, labels, descriptions, and error states. Build accessible forms with minimal effort."
                        }
                        Group { gap: "xs",
                            Badge { color: "green", "TextInput" }
                            Badge { color: "teal", "Checkbox" }
                            Badge { color: "cyan", "Select" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Getting started
            Title { order: 3, "Getting Started" }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "1. Add to Cargo.toml" }
                        Code { block: true,
                            "[dependencies]\nrinch = { version = \"0.1\", features = [\"components\", \"theme\"] }"
                        }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "2. Create your app" }
                        Code { block: true,
                            "use rinch::prelude::*;\n\nfn app(__scope: &mut RenderScope) -> NodeHandle {\n    rsx! {\n        Button { \"Hello Rinch!\" }\n    }\n}"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Navigation hint
            Paper { p: "lg", radius: "md", shadow: "sm",
                Center {
                    Group { gap: "md",
                        Text { color: "dimmed", "Use the" }
                        Badge { color: "blue", variant: "light", "☰ Menu" }
                        Text { color: "dimmed", "to explore all component categories" }
                    }
                }
            }
        }
    }
}
