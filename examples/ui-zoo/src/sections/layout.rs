//! Layout section - Stack, Group, Paper, Divider, Container, and other layout primitives.

use rinch::prelude::*;

pub fn layout_section(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Layout" }
                Text { size: "lg", color: "dimmed",
                    "Components for organizing and structuring content."
                }
            }
            Space { h: "xl" }

            // Flex layout
            Title { order: 3, "Flex Layout" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Stack and Group for vertical and horizontal layouts." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Stack (Vertical)" }
                        Text { size: "sm", color: "dimmed", "Arrange items in a column with consistent gap" }
                        Divider {}
                        Stack { gap: "sm",
                            Paper { p: "sm", shadow: "xs", radius: "sm", "Item 1" }
                            Paper { p: "sm", shadow: "xs", radius: "sm", "Item 2" }
                            Paper { p: "sm", shadow: "xs", radius: "sm", "Item 3" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Group (Horizontal)" }
                        Text { size: "sm", color: "dimmed", "Arrange items in a row with consistent gap" }
                        Divider {}
                        Group { gap: "sm",
                            Paper { p: "sm", shadow: "xs", radius: "sm", "Item 1" }
                            Paper { p: "sm", shadow: "xs", radius: "sm", "Item 2" }
                            Paper { p: "sm", shadow: "xs", radius: "sm", "Item 3" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Group Justify" }
                        Text { size: "sm", color: "dimmed", "Control horizontal alignment" }
                        Divider {}
                        Stack { gap: "sm",
                            Paper { p: "xs", radius: "sm", with_border: true,
                                Group { justify: "start", gap: "sm",
                                    Badge { color: "blue", "Start" }
                                    Badge { color: "blue", "Items" }
                                }
                            }
                            Paper { p: "xs", radius: "sm", with_border: true,
                                Group { justify: "center", gap: "sm",
                                    Badge { color: "cyan", "Center" }
                                    Badge { color: "cyan", "Items" }
                                }
                            }
                            Paper { p: "xs", radius: "sm", with_border: true,
                                Group { justify: "end", gap: "sm",
                                    Badge { color: "teal", "End" }
                                    Badge { color: "teal", "Items" }
                                }
                            }
                            Paper { p: "xs", radius: "sm", with_border: true,
                                Group { justify: "between",
                                    Badge { color: "violet", "Space" }
                                    Badge { color: "violet", "Between" }
                                }
                            }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Center" }
                        Text { size: "sm", color: "dimmed", "Center content horizontally and vertically" }
                        Divider {}
                        Paper { p: "xl", shadow: "xs", radius: "sm",
                            Center {
                                Badge { color: "green", size: "lg", "Centered Content" }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Spacing and dividers
            Title { order: 3, "Spacing & Dividers" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Control spacing between elements." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Space" }
                        Text { size: "sm", color: "dimmed", "Add vertical or horizontal spacing" }
                        Divider {}
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Stack { gap: "0",
                                Text { size: "sm", "Before" }
                                Space { h: "xs" }
                                Text { size: "xs", color: "dimmed", "xs spacing" }
                            }
                        }
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Stack { gap: "0",
                                Text { size: "sm", "Before" }
                                Space { h: "md" }
                                Text { size: "xs", color: "dimmed", "md spacing" }
                            }
                        }
                        Paper { p: "sm", radius: "sm", with_border: true,
                            Stack { gap: "0",
                                Text { size: "sm", "Before" }
                                Space { h: "xl" }
                                Text { size: "xs", color: "dimmed", "xl spacing" }
                            }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Divider" }
                        Text { size: "sm", color: "dimmed", "Visual separation between sections" }
                        Divider {}
                        Stack { gap: "md",
                            Text { size: "sm", "Section one content" }
                            Divider {}
                            Text { size: "sm", "Section two content" }
                            Divider { label: "OR" }
                            Text { size: "sm", "Alternative option" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Containers
            Title { order: 3, "Containers" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Container components for grouping and styling content." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Paper Shadows" }
                        Text { size: "sm", color: "dimmed", "Elevated surfaces with depth" }
                        Divider {}
                        Group { gap: "sm",
                            Paper { shadow: "xs", p: "md", radius: "sm", "xs" }
                            Paper { shadow: "sm", p: "md", radius: "sm", "sm" }
                            Paper { shadow: "md", p: "md", radius: "sm", "md" }
                            Paper { shadow: "lg", p: "md", radius: "sm", "lg" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Paper Radius" }
                        Text { size: "sm", color: "dimmed", "Rounded corners at different sizes" }
                        Divider {}
                        Group { gap: "sm",
                            Paper { radius: "xs", p: "md", shadow: "sm", "xs" }
                            Paper { radius: "sm", p: "md", shadow: "sm", "sm" }
                            Paper { radius: "md", p: "md", shadow: "sm", "md" }
                            Paper { radius: "lg", p: "md", shadow: "sm", "lg" }
                            Paper { radius: "xl", p: "md", shadow: "sm", "xl" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Fieldset" }
                        Text { size: "sm", color: "dimmed", "Group related form fields" }
                        Divider {}
                        Fieldset { legend: "Personal Info",
                            Stack { gap: "sm",
                                TextInput { label: "Name", placeholder: "Your name" }
                                TextInput { label: "Email", placeholder: "you@example.com" }
                            }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "SimpleGrid" }
                        Text { size: "sm", color: "dimmed", "Responsive equal-width column grid" }
                        Divider {}
                        SimpleGrid { cols: Some(3), spacing: Some("xs".to_string()),
                            Paper { p: "sm", shadow: "xs", radius: "sm",
                                Center { Text { size: "xs", "1" } }
                            }
                            Paper { p: "sm", shadow: "xs", radius: "sm",
                                Center { Text { size: "xs", "2" } }
                            }
                            Paper { p: "sm", shadow: "xs", radius: "sm",
                                Center { Text { size: "xs", "3" } }
                            }
                            Paper { p: "sm", shadow: "xs", radius: "sm",
                                Center { Text { size: "xs", "4" } }
                            }
                            Paper { p: "sm", shadow: "xs", radius: "sm",
                                Center { Text { size: "xs", "5" } }
                            }
                            Paper { p: "sm", shadow: "xs", radius: "sm",
                                Center { Text { size: "xs", "6" } }
                            }
                        }
                    }
                }
            }
        }
    }
}
