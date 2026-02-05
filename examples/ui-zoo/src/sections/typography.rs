//! Typography section - Text, Title, and other text components.

use rinch::prelude::*;

pub fn typography_section(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Typography" }
                Text { size: "lg", color: "dimmed",
                    "Text components for displaying and formatting content"
                }
            }
            Space { h: "xl" }

            // ============================================
            // HEADINGS
            // ============================================
            Title { order: 3, "Headings" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Six levels of headings for document structure." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Group { align: "center", gap: "lg",
                        div { style: "width: 80px;",
                            Badge { color: "gray", variant: "light", "h1" }
                        }
                        Title { order: 1, "Heading One" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 80px;",
                            Badge { color: "gray", variant: "light", "h2" }
                        }
                        Title { order: 2, "Heading Two" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 80px;",
                            Badge { color: "gray", variant: "light", "h3" }
                        }
                        Title { order: 3, "Heading Three" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 80px;",
                            Badge { color: "gray", variant: "light", "h4" }
                        }
                        Title { order: 4, "Heading Four" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 80px;",
                            Badge { color: "gray", variant: "light", "h5" }
                        }
                        Title { order: 5, "Heading Five" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 80px;",
                            Badge { color: "gray", variant: "light", "h6" }
                        }
                        Title { order: 6, "Heading Six" }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // TEXT STYLING
            // ============================================
            Title { order: 3, "Text Styling" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Size, weight, color, and alignment options for text." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Sizes" }
                        Divider {}
                        Text { size: "xs", "Extra small (xs)" }
                        Text { size: "sm", "Small (sm)" }
                        Text { size: "md", "Medium (md) - default" }
                        Text { size: "lg", "Large (lg)" }
                        Text { size: "xl", "Extra large (xl)" }
                    }
                }

                // Weights
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Weights" }
                        Divider {}
                        Text { weight: "normal", "Normal weight (400)" }
                        Text { weight: "500", "Medium weight (500)" }
                        Text { weight: "600", "Semibold weight (600)" }
                        Text { weight: "bold", "Bold weight (700)" }
                    }
                }

                // Colors
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Colors" }
                        Divider {}
                        Text { "Default text color" }
                        Text { color: "dimmed", "Dimmed text color" }
                        Group { gap: "lg",
                            Text { color: "blue", "Blue" }
                            Text { color: "cyan", "Cyan" }
                            Text { color: "teal", "Teal" }
                            Text { color: "green", "Green" }
                        }
                        Group { gap: "lg",
                            Text { color: "red", "Red" }
                            Text { color: "orange", "Orange" }
                            Text { color: "yellow", "Yellow" }
                            Text { color: "violet", "Violet" }
                        }
                    }
                }

                // Alignment
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Alignment" }
                        Divider {}
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Text { align: "left", "Left aligned text" }
                        }
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Text { align: "center", "Center aligned text" }
                        }
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Text { align: "right", "Right aligned text" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // CODE & INLINE ELEMENTS
            // ============================================
            Title { order: 3, "Code & Inline Elements" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Inline and block elements for code, keyboard shortcuts, and emphasis." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Code
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Code" }
                        Text { size: "sm", color: "dimmed", "Inline and block code formatting" }
                        Divider {}
                        Text { "Use " Code { "inline code" } " within text." }
                        Space { h: "sm" }
                        Code { block: true,
                            "fn main() {\n    println!(\"Hello, Rinch!\");\n}"
                        }
                    }
                }

                // Keyboard
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Keyboard Shortcuts" }
                        Text { size: "sm", color: "dimmed", "Display keyboard key combinations" }
                        Divider {}
                        Text {
                            "Press " Kbd { "Ctrl" } " + " Kbd { "C" } " to copy"
                        }
                        Text {
                            "Press " Kbd { "Ctrl" } " + " Kbd { "V" } " to paste"
                        }
                        Text {
                            Kbd { "Cmd" } " + " Kbd { "Shift" } " + " Kbd { "P" } " opens palette"
                        }
                    }
                }

                // Highlight & Mark
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Highlight & Mark" }
                        Text { size: "sm", color: "dimmed", "Emphasize important text" }
                        Divider {}
                        Text {
                            "Use " Highlight { "highlight" } " for important text"
                        }
                        Text {
                            Highlight { color: "yellow", "Yellow" } " "
                            Highlight { color: "cyan", "Cyan" } " "
                            Highlight { color: "pink", "Pink" } " "
                            Highlight { color: "lime", "Lime" }
                        }
                        Text {
                            "The " Mark { "mark element" } " for search results"
                        }
                    }
                }

                // Links
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Links" }
                        Text { size: "sm", color: "dimmed", "Anchor elements for navigation" }
                        Divider {}
                        Text {
                            "Visit " Anchor { href: "https://github.com", "GitHub" } " for source"
                        }
                        Text {
                            "Check out " Anchor { href: "https://rust-lang.org", "Rust" } " documentation"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // CONTENT BLOCKS
            // ============================================
            Title { order: 3, "Content Blocks" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Block-level typography components for structured content." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Blockquote
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Blockquote" }
                        Text { size: "sm", color: "dimmed", "Quoted text with attribution" }
                        Divider {}
                        Blockquote { cite: "Alan Kay",
                            "The best way to predict the future is to invent it."
                        }
                        Blockquote { cite: "Linus Torvalds", color: "green",
                            "Talk is cheap. Show me the code."
                        }
                    }
                }

                // Lists
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Lists" }
                        Text { size: "sm", color: "dimmed", "Ordered and unordered lists" }
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
