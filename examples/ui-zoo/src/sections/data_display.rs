//! Data Display section - Avatar, Badge, Card, Tooltip, and other display components.

use rinch::prelude::*;

pub fn data_display_section(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Data Display" }
                Text { size: "lg", color: "dimmed",
                    "Components for presenting data and content."
                }
            }
            Space { h: "xl" }

            // Avatar
            Title { order: 3, "Avatar" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "User avatars with initials fallback." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Sizes" }
                        Divider {}
                        Group { gap: "md",
                            Avatar { size: "xs", name: "Alice" }
                            Avatar { size: "sm", name: "Bob" }
                            Avatar { size: "md", name: "Carol" }
                            Avatar { size: "lg", name: "Dave" }
                            Avatar { size: "xl", name: "Eve" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Colors" }
                        Divider {}
                        Group { gap: "md",
                            Avatar { color: "blue", name: "AB" }
                            Avatar { color: "red", name: "CD" }
                            Avatar { color: "green", name: "EF" }
                            Avatar { color: "orange", name: "GH" }
                            Avatar { color: "grape", name: "IJ" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Card
            Title { order: 3, "Card" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Container with sections for structured content." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Card { shadow: "sm", radius: "md", with_border: true,
                    CardSection {
                        div { style: "height: 80px; background: linear-gradient(135deg, var(--rinch-color-blue-5), var(--rinch-color-cyan-5));" }
                    }
                    Stack { gap: "sm",
                        Space { h: "sm" }
                        Text { weight: "600", "Getting Started" }
                        Text { size: "sm", color: "dimmed",
                            "Learn how to build applications with Rinch widgets and the reactive rendering system."
                        }
                        Space { h: "xs" }
                        Button { variant: "light", "Read More" }
                    }
                }
                Card { shadow: "sm", radius: "md", with_border: true,
                    CardSection {
                        div { style: "height: 80px; background: linear-gradient(135deg, var(--rinch-color-grape-5), var(--rinch-color-pink-5));" }
                    }
                    Stack { gap: "sm",
                        Space { h: "sm" }
                        Text { weight: "600", "Theme System" }
                        Text { size: "sm", color: "dimmed",
                            "Customize colors, spacing, and typography with the built-in theme system."
                        }
                        Space { h: "xs" }
                        Button { variant: "light", color: "grape", "Explore" }
                    }
                }
            }

            Space { h: "xl" }

            // Tooltip
            Title { order: 3, "Tooltip" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Show additional information on hover." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Group { gap: "md",
                    Tooltip { label: "This is a tooltip",
                        Button { variant: "outline", "Hover me" }
                    }
                    Tooltip { label: "Bottom tooltip", position: "bottom",
                        Button { variant: "light", "Bottom" }
                    }
                    Tooltip { label: "Left tooltip", position: "left",
                        Button { variant: "subtle", "Left" }
                    }
                }
            }

            Space { h: "xl" }

            // Blockquote and List
            Title { order: 3, "Blockquote & List" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Content blocks for structured text." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Blockquote" }
                        Divider {}
                        Blockquote { cite: "Alan Kay",
                            "The best way to predict the future is to invent it."
                        }
                        Blockquote { cite: "Linus Torvalds", color: "green",
                            "Talk is cheap. Show me the code."
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "List" }
                        Divider {}
                        Group { align: "start", gap: "xl",
                            Stack { gap: "xs",
                                Text { size: "sm", weight: "500", "Unordered" }
                                List {
                                    ListItem { "First item" }
                                    ListItem { "Second item" }
                                    ListItem { "Third item" }
                                }
                            }
                            Stack { gap: "xs",
                                Text { size: "sm", weight: "500", "Ordered" }
                                List { r#type: "ordered",
                                    ListItem { "Step one" }
                                    ListItem { "Step two" }
                                    ListItem { "Step three" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
