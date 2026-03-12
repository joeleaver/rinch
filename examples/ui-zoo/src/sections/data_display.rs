//! Data Display section - Components for displaying data.

use rinch::prelude::*;

#[component]
pub fn data_display_section() -> NodeHandle {
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

            SimpleGrid { cols: Some(3), spacing: "lg",
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

            SimpleGrid { cols: Some(3), spacing: "lg",
                // Avatar with image
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Avatar with Image" }
                        Text { size: "sm", color: "dimmed", "Display user photos with image source" }
                        Divider {}
                        Group { gap: "md",
                            Stack { align: "center", gap: "xs",
                                Avatar { src: "test_image.png", size: "lg", alt: "User 1" }
                                Text { size: "xs", color: "dimmed", "Image" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { src: "test_image.png", size: "xl", alt: "User 2" }
                                Text { size: "xs", color: "dimmed", "Large" }
                            }
                            Stack { align: "center", gap: "xs",
                                Avatar { src: "test_image.png", size: "md", radius: "md", alt: "User 3" }
                                Text { size: "xs", color: "dimmed", "Rounded" }
                            }
                        }
                    }
                }

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

            SimpleGrid { cols: Some(2), spacing: "lg",
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

            Space { h: "xl" }

            // ============================================
            // IMAGE
            // ============================================
            Title { order: 3, "Image" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Image rendering with object-fit modes and background-image CSS." }
            Space { h: "md" }

            // Object-fit modes — 1024x1024 source in 200x120 containers
            SimpleGrid { cols: Some(5), spacing: "md",
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "fill" }
                        Text { size: "xs", color: "dimmed", "Stretches to fill" }
                        div { style: "border: 1px dashed var(--rinch-color-border);",
                            Image { src: "test_image.png", width: "180", height: "120", fit: "fill" }
                        }
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "contain" }
                        Text { size: "xs", color: "dimmed", "Fits inside, letterboxed" }
                        div { style: "border: 1px dashed var(--rinch-color-border);",
                            Image { src: "test_image.png", width: "180", height: "120", fit: "contain" }
                        }
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "cover" }
                        Text { size: "xs", color: "dimmed", "Fills and clips overflow" }
                        div { style: "border: 1px dashed var(--rinch-color-border);",
                            Image { src: "test_image.png", width: "180", height: "120", fit: "cover" }
                        }
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "none" }
                        Text { size: "xs", color: "dimmed", "Natural size, centered" }
                        div { style: "border: 1px dashed var(--rinch-color-border);",
                            Image { src: "test_image.png", width: "180", height: "120", fit: "none" }
                        }
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", size: "sm", "scale-down" }
                        Text { size: "xs", color: "dimmed", "Like contain, never upscales" }
                        div { style: "border: 1px dashed var(--rinch-color-border);",
                            Image { src: "test_image.png", width: "180", height: "120", fit: "scale-down" }
                        }
                    }
                }
            }

            Space { h: "md" }

            // Raw img element and background-image
            SimpleGrid { cols: Some(3), spacing: "lg",
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Raw img element" }
                        Divider {}
                        img { style: "width: 200px; height: 150px; object-fit: contain;", src: "test_image.png" }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "background-image" }
                        Divider {}
                        div {
                            style: "width: 200px; height: 150px; background-image: url(test_image.png); background-size: cover;",
                        }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Auto sizing" }
                        Divider {}
                        img { style: "width: 150px;", src: "test_image.png" }
                    }
                }
            }
        }
    }
}
