//! Navigation section - Tabs, Accordion, Breadcrumbs, Pagination, and NavLink demos.

use rinch::prelude::*;

pub fn navigation_section(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Navigation" }
                Text { size: "lg", color: "dimmed",
                    "Components for organizing content and navigating between views."
                }
            }
            Space { h: "xl" }

            // Tabs
            Title { order: 3, "Tabs" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Tab-based navigation for switching between panels." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Default Tabs" }
                        Divider {}
                        Tabs { default_value: "first",
                            TabsList {
                                Tab { value: "first", "First" }
                                Tab { value: "second", "Second" }
                                Tab { value: "third", "Third" }
                            }
                            TabsPanel { value: "first",
                                Space { h: "sm" }
                                Text { size: "sm", "Content of the first tab panel." }
                            }
                            TabsPanel { value: "second",
                                Space { h: "sm" }
                                Text { size: "sm", "Content of the second tab panel." }
                            }
                            TabsPanel { value: "third",
                                Space { h: "sm" }
                                Text { size: "sm", "Content of the third tab panel." }
                            }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Pills Variant" }
                        Divider {}
                        Tabs { default_value: "overview", variant: "pills",
                            TabsList {
                                Tab { value: "overview", "Overview" }
                                Tab { value: "settings", "Settings" }
                                Tab { value: "disabled", disabled: true, "Disabled" }
                            }
                            TabsPanel { value: "overview",
                                Space { h: "sm" }
                                Text { size: "sm", "Overview content with pills-style tabs." }
                            }
                            TabsPanel { value: "settings",
                                Space { h: "sm" }
                                Text { size: "sm", "Settings panel content." }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Accordion
            Title { order: 3, "Accordion" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Collapsible content sections." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Default" }
                        Divider {}
                        Accordion { default_value: "item1",
                            AccordionItem { value: "item1",
                                AccordionControl { "What is Rinch?" }
                                AccordionPanel {
                                    Text { size: "sm",
                                        "Rinch is a lightweight cross-platform GUI library for Rust."
                                    }
                                }
                            }
                            AccordionItem { value: "item2",
                                AccordionControl { "How does rendering work?" }
                                AccordionPanel {
                                    Text { size: "sm",
                                        "Rinch uses fine-grained reactive rendering with Vello for GPU-accelerated 2D graphics."
                                    }
                                }
                            }
                            AccordionItem { value: "item3",
                                AccordionControl { "What about styling?" }
                                AccordionPanel {
                                    Text { size: "sm",
                                        "Rinch supports HTML/CSS for layout via Taffy and Parley for text."
                                    }
                                }
                            }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Separated Variant" }
                        Divider {}
                        Accordion { variant: "separated",
                            AccordionItem { value: "a",
                                AccordionControl { "Getting Started" }
                                AccordionPanel {
                                    Text { size: "sm", "Add rinch to your Cargo.toml and start building." }
                                }
                            }
                            AccordionItem { value: "b",
                                AccordionControl { "Widget Library" }
                                AccordionPanel {
                                    Text { size: "sm", "Over 55 widgets inspired by Mantine." }
                                }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Breadcrumbs and Pagination
            Title { order: 3, "Breadcrumbs & Pagination" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Navigation trail and page controls." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Breadcrumbs" }
                        Divider {}
                        Breadcrumbs {
                            BreadcrumbsItem { "Home" }
                            BreadcrumbsItem { "Components" }
                            BreadcrumbsItem { "Navigation" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Pagination" }
                        Divider {}
                        Pagination { total: {10_u32}, value: {3_u32} }
                    }
                }
            }

            Space { h: "xl" }

            // NavLink
            Title { order: 3, "NavLink" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Navigation links with active state." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "0",
                    NavLink { label: Some("Dashboard".to_string()), active: true }
                    NavLink { label: Some("Analytics".to_string()) }
                    NavLink { label: Some("Settings".to_string()) }
                    NavLink { label: Some("Disabled".to_string()), disabled: true }
                }
            }
        }
    }
}
