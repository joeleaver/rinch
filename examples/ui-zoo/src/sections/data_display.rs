//! Data Display section - Components for displaying data.

use rinch::prelude::*;

pub fn data_display_section(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Data Display" }
                Text { size: "lg", color: "dimmed",
                    "Components for presenting and organizing data"
                }
            }
            Space { h: "xl" }

            // ============================================
            // BADGES & STATUS
            // ============================================
            Title { order: 3, "Badges & Status" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Visual indicators for status, labels, and counts." }
            Space { h: "md" }

            SimpleGrid { cols: Some(3), spacing: Some("lg".to_string()),
                // Badge colors
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Badge Colors" }
                        Divider {}
                        Group { gap: "sm",
                            Badge { color: "blue", "Blue" }
                            Badge { color: "cyan", "Cyan" }
                            Badge { color: "teal", "Teal" }
                        }
                        Group { gap: "sm",
                            Badge { color: "green", "Green" }
                            Badge { color: "yellow", "Yellow" }
                            Badge { color: "orange", "Orange" }
                        }
                        Group { gap: "sm",
                            Badge { color: "red", "Red" }
                            Badge { color: "violet", "Violet" }
                            Badge { color: "grape", "Grape" }
                        }
                    }
                }

                // Badge variants
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Badge Variants" }
                        Divider {}
                        Group { gap: "lg",
                            Stack { gap: "xs", align: "center",
                                Badge { variant: "filled", "Filled" }
                                Text { size: "xs", color: "dimmed", "Solid" }
                            }
                            Stack { gap: "xs", align: "center",
                                Badge { variant: "light", "Light" }
                                Text { size: "xs", color: "dimmed", "Subtle" }
                            }
                            Stack { gap: "xs", align: "center",
                                Badge { variant: "outline", "Outline" }
                                Text { size: "xs", color: "dimmed", "Border" }
                            }
                            Stack { gap: "xs", align: "center",
                                Badge { variant: "dot", "Dot" }
                                Text { size: "xs", color: "dimmed", "Status" }
                            }
                        }
                    }
                }

                // Badge sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Badge Sizes" }
                        Divider {}
                        Group { gap: "sm", align: "center",
                            Badge { size: "xs", "xs" }
                            Badge { size: "sm", "sm" }
                            Badge { size: "md", "md" }
                            Badge { size: "lg", "lg" }
                            Badge { size: "xl", "xl" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // AVATAR
            // ============================================
            Title { order: 3, "Avatar" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "User profile images with initials fallback." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Avatar with names
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Avatar with Initials" }
                        Text { size: "sm", color: "dimmed", "Automatically generates initials from name" }
                        Divider {}
                        Group { gap: "md",
                            Stack { align: "center", gap: "xs",
                                Avatar { name: "John Doe", size: "lg" }
                                Text { size: "xs", color: "dimmed", "John Doe" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { name: "Jane Smith", size: "lg" }
                                Text { size: "xs", color: "dimmed", "Jane Smith" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { name: "Bob Wilson", size: "lg" }
                                Text { size: "xs", color: "dimmed", "Bob Wilson" }
                            }
                        }
                    }
                }

                // Avatar sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Avatar Sizes" }
                        Text { size: "sm", color: "dimmed", "Five size options" }
                        Divider {}
                        Group { gap: "md", align: "end",
                            Stack { align: "center", gap: "xs",
                                Avatar { size: "xs", name: "A" }
                                Text { size: "xs", color: "dimmed", "xs" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { size: "sm", name: "B" }
                                Text { size: "xs", color: "dimmed", "sm" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { size: "md", name: "C" }
                                Text { size: "xs", color: "dimmed", "md" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { size: "lg", name: "D" }
                                Text { size: "xs", color: "dimmed", "lg" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { size: "xl", name: "E" }
                                Text { size: "xs", color: "dimmed", "xl" }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // CARDS
            // ============================================
            Title { order: 3, "Cards" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Container components for grouping related content." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Basic card
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Basic Card" }
                        Divider {}
                        Card { shadow: "sm", padding: "lg", radius: "md",
                            Stack { gap: "sm",
                                Text { weight: "600", size: "lg", "Card Title" }
                                Text { size: "sm", color: "dimmed",
                                    "Cards are used to group and display related content in a clear, consistent way."
                                }
                                Space { h: "xs" }
                                Button { variant: "light", color: "blue", full_width: true, "Learn More" }
                            }
                        }
                    }
                }

                // Card with sections
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Card with Sections" }
                        Divider {}
                        Card { shadow: "sm", padding: "0", radius: "md",
                            CardSection { inherit_padding: true,
                                div { style: "padding: var(--rinch-spacing-md);",
                                    Text { weight: "600", "Header" }
                                }
                            }
                            CardSection { inherit_padding: true, with_border: true,
                                div { style: "padding: var(--rinch-spacing-md);",
                                    Text { size: "sm", color: "dimmed",
                                        "Main content area with border."
                                    }
                                }
                            }
                            CardSection { inherit_padding: true,
                                div { style: "padding: var(--rinch-spacing-md);",
                                    Group { justify: "end", gap: "sm",
                                        Button { variant: "subtle", size: "sm", "Cancel" }
                                        Button { size: "sm", "Save" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // ACCORDION
            // ============================================
            Title { order: 3, "Accordion" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Expandable content sections." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Accordion {
                    AccordionItem { value: "item1",
                        AccordionControl { "What is Rinch?" }
                        AccordionPanel {
                            Text { size: "sm", color: "dimmed",
                                "Rinch is a lightweight cross-platform GUI library for Rust. It uses HTML/CSS for layout with a Vello-based renderer for high-performance graphics."
                            }
                        }
                    }
                    AccordionItem { value: "item2",
                        AccordionControl { "How does the reactive system work?" }
                        AccordionPanel {
                            Text { size: "sm", color: "dimmed",
                                "Rinch uses signals, effects, and memos for reactive state management. When a signal changes, dependent UI automatically updates without manual DOM manipulation."
                            }
                        }
                    }
                    AccordionItem { value: "item3",
                        AccordionControl { "What styling options are available?" }
                        AccordionPanel {
                            Text { size: "sm", color: "dimmed",
                                "The theme system provides CSS variables for colors, spacing, typography, and shadows. Components accept color, size, and variant props for customization."
                            }
                        }
                    }
                }
            }
        }
    }
}
