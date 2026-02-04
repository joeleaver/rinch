//! Typography section - Text, Title, Code, Badge, and other text components.

use rinch::prelude::*;

pub fn typography_section(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Typography" }
                Text { size: "lg", color: "dimmed",
                    "Text components for displaying and formatting content."
                }
            }
            Space { h: "xl" }

            // Headings
            Title { order: 3, "Headings" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Six levels of headings for document structure." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Group { align: "center", gap: "lg",
                        div { style: "width: 60px;",
                            Badge { color: "gray", variant: "light", "h1" }
                        }
                        Title { order: 1, "Heading One" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 60px;",
                            Badge { color: "gray", variant: "light", "h2" }
                        }
                        Title { order: 2, "Heading Two" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 60px;",
                            Badge { color: "gray", variant: "light", "h3" }
                        }
                        Title { order: 3, "Heading Three" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 60px;",
                            Badge { color: "gray", variant: "light", "h4" }
                        }
                        Title { order: 4, "Heading Four" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 60px;",
                            Badge { color: "gray", variant: "light", "h5" }
                        }
                        Title { order: 5, "Heading Five" }
                    }
                    Divider {}
                    Group { align: "center", gap: "lg",
                        div { style: "width: 60px;",
                            Badge { color: "gray", variant: "light", "h6" }
                        }
                        Title { order: 6, "Heading Six" }
                    }
                }
            }

            Space { h: "xl" }

            // Text styling
            Title { order: 3, "Text Styling" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Size, weight, color, and alignment options." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
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
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Colors" }
                        Divider {}
                        Text { "Default text color" }
                        Text { color: "dimmed", "Dimmed text color" }
                        Group { gap: "lg",
                            Text { color: "blue", "Blue" }
                            Text { color: "red", "Red" }
                            Text { color: "green", "Green" }
                            Text { color: "orange", "Orange" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Alignment" }
                        Divider {}
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Text { align: "left", "Left aligned" }
                        }
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Text { align: "center", "Center aligned" }
                        }
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Text { align: "right", "Right aligned" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Code and inline elements
            Title { order: 3, "Code & Inline Elements" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Inline elements for code, keyboard shortcuts, and links." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Code" }
                        Text { size: "sm", color: "dimmed", "Inline and block code" }
                        Divider {}
                        Text { "Use " Code { "inline code" } " within text." }
                        Space { h: "sm" }
                        Code { block: true,
                            "fn main() {\n    println!(\"Hello, Rinch!\");\n}"
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Keyboard Shortcuts" }
                        Text { size: "sm", color: "dimmed", "Display key combinations" }
                        Divider {}
                        Text {
                            "Press " Kbd { "Ctrl" } " + " Kbd { "C" } " to copy"
                        }
                        Text {
                            "Press " Kbd { "Ctrl" } " + " Kbd { "V" } " to paste"
                        }
                        Text {
                            Kbd { "Cmd" } " + " Kbd { "Shift" } " + " Kbd { "P" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Badge" }
                        Text { size: "sm", color: "dimmed", "Status indicators" }
                        Divider {}
                        Group { gap: "sm",
                            Badge { "Default" }
                            Badge { variant: "outline", "Outline" }
                            Badge { variant: "light", "Light" }
                            Badge { variant: "dot", "Dot" }
                        }
                        Group { gap: "sm",
                            Badge { color: "red", "Error" }
                            Badge { color: "green", "Success" }
                            Badge { color: "yellow", "Warning" }
                            Badge { color: "blue", "Info" }
                        }
                        Group { gap: "sm",
                            Badge { size: "xs", "XS" }
                            Badge { size: "sm", "SM" }
                            Badge { size: "md", "MD" }
                            Badge { size: "lg", "LG" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Links" }
                        Text { size: "sm", color: "dimmed", "Anchor elements for navigation" }
                        Divider {}
                        Text {
                            "Visit " Anchor { href: "https://github.com", "GitHub" } " for source"
                        }
                        Text {
                            "Check " Anchor { href: "https://rust-lang.org", "Rust" } " docs"
                        }
                    }
                }
            }
        }
    }
}
