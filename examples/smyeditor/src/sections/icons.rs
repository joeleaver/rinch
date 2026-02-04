//! Icons section - Displays all Tabler Icons organized into pages.

use rinch::prelude::*;
use rinch_tabler_icons::{render_tabler_icon, TablerIcon, TablerIconStyle, ALL_ICONS, ICON_COUNT};
use std::rc::Rc;

/// Number of icons per page (10 rows x 10 columns)
const ICONS_PER_PAGE: usize = 100;

/// State for the icons section
#[derive(Clone)]
pub struct IconsSectionState {
    pub current_page: Signal<usize>,
    pub use_filled: Signal<bool>,
}

/// Initialize the icons section state
pub fn init_icons_state() {
    create_context(IconsSectionState {
        current_page: Signal::new(0),
        use_filled: Signal::new(false),
    });
}

/// Icons section component
pub fn icons_section(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<IconsSectionState>();

    let (current_page, use_filled) = match state {
        Some(s) => (s.current_page, s.use_filled),
        None => {
            return rsx! { div { "Icons section state not initialized" } };
        }
    };

    // Calculate total pages
    let total_pages = (ICON_COUNT + ICONS_PER_PAGE - 1) / ICONS_PER_PAGE;

    rsx! {
        Fragment {
            // Header
            Title { order: 1, "Tabler Icons" }
            Text { color: "dimmed",
                "5000+ free SVG icons from "
                Anchor { href: "https://tabler.io/icons", target: "_blank", "Tabler Icons" }
                ". Available in Outline and Filled styles."
            }
            Space { h: "xl" }

            // Stats and controls
            Paper { p: "md", radius: "md", with_border: true,
                Group { justify: "space-between",
                    Group { gap: "xl",
                        Stack { gap: "0",
                            Text { size: "xl", weight: "bold", {ICON_COUNT.to_string()} }
                            Text { size: "sm", color: "dimmed", "Total Icons" }
                        }
                        Stack { gap: "0",
                            Text { size: "xl", weight: "bold",
                                {|| format!("Page {}", current_page.get() + 1)}
                            }
                            Text { size: "sm", color: "dimmed",
                                {format!("of {}", total_pages)}
                            }
                        }
                    }

                    // Style toggle
                    Switch {
                        label: Some("Filled Style".to_string()),
                        checked_fn: Some(Rc::new(move || use_filled.get())),
                        onchange: move || use_filled.update(|v| *v = !*v)
                    }
                }
            }
            Space { h: "md" }

            // Pagination controls
            Paper { p: "sm", radius: "md", with_border: true,
                Center {
                    Group { gap: "sm",
                        // First page
                        ActionIcon {
                            variant: "subtle",
                            size: "lg",
                            onclick: move || current_page.set(0),
                            {render_tabler_icon(__scope, TablerIcon::ChevronsLeft, TablerIconStyle::Outline)}
                        }
                        // Previous page
                        ActionIcon {
                            variant: "subtle",
                            size: "lg",
                            onclick: move || current_page.update(|p| *p = p.saturating_sub(1)),
                            {render_tabler_icon(__scope, TablerIcon::ChevronLeft, TablerIconStyle::Outline)}
                        }

                        // Page indicator
                        Text { size: "sm",
                            {|| format!("Page {} of {}", current_page.get() + 1, total_pages)}
                        }

                        // Next page
                        ActionIcon {
                            variant: "subtle",
                            size: "lg",
                            onclick: {
                                let max_page = total_pages - 1;
                                move || current_page.update(|p| *p = (*p + 1).min(max_page))
                            },
                            {render_tabler_icon(__scope, TablerIcon::ChevronRight, TablerIconStyle::Outline)}
                        }
                        // Last page
                        ActionIcon {
                            variant: "subtle",
                            size: "lg",
                            onclick: {
                                let max_page = total_pages - 1;
                                move || current_page.set(max_page)
                            },
                            {render_tabler_icon(__scope, TablerIcon::ChevronsRight, TablerIconStyle::Outline)}
                        }
                    }
                }
            }
            Space { h: "md" }

            // Icon grid - using for_each_dom for reactive updates
            Paper { p: "md", radius: "md", with_border: true,
                {reactive_icon_grid(__scope, current_page, use_filled)}
            }

            Space { h: "xl" }

            // Usage examples
            Title { order: 3, "Usage" }
            Space { h: "md" }

            Code { block: true, r#"use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

// Render an outline icon
let icon = render_tabler_icon(__scope, TablerIcon::Home, TablerIconStyle::Outline);

// Render a filled icon
let icon = render_tabler_icon(__scope, TablerIcon::Heart, TablerIconStyle::Filled);

// Use the macro for convenience
let icon = tabler_icon!(__scope, AlertCircle);
let icon = tabler_icon!(__scope, AlertCircle, Filled);"# }

            Space { h: "xl" }

            // Sample icons by category
            Title { order: 3, "Sample Icons by Category" }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("md".to_string()),
                // Navigation icons
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Title { order: 5, "Navigation" }
                        Group { gap: "md",
                            {render_tabler_icon(__scope, TablerIcon::Home, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::ArrowLeft, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::ArrowRight, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::ChevronUp, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::ChevronDown, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Menu2, TablerIconStyle::Outline)}
                        }
                    }
                }

                // Actions
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Title { order: 5, "Actions" }
                        Group { gap: "md",
                            {render_tabler_icon(__scope, TablerIcon::Plus, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Minus, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::X, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Check, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Edit, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Trash, TablerIconStyle::Outline)}
                        }
                    }
                }

                // Status
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Title { order: 5, "Status" }
                        Group { gap: "md",
                            {render_tabler_icon(__scope, TablerIcon::AlertCircle, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::AlertTriangle, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::CircleCheck, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::InfoCircle, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::QuestionMark, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::CircleX, TablerIconStyle::Outline)}
                        }
                    }
                }

                // Communication
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Title { order: 5, "Communication" }
                        Group { gap: "md",
                            {render_tabler_icon(__scope, TablerIcon::Mail, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Phone, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Message, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Bell, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Send, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Share, TablerIconStyle::Outline)}
                        }
                    }
                }

                // Media
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Title { order: 5, "Media" }
                        Group { gap: "md",
                            {render_tabler_icon(__scope, TablerIcon::Photo, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Video, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Music, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Microphone, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Camera, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::PlayerPlay, TablerIconStyle::Outline)}
                        }
                    }
                }

                // Files
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Title { order: 5, "Files" }
                        Group { gap: "md",
                            {render_tabler_icon(__scope, TablerIcon::File, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Folder, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Download, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Upload, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::Copy, TablerIconStyle::Outline)}
                            {render_tabler_icon(__scope, TablerIcon::ClipboardCopy, TablerIconStyle::Outline)}
                        }
                    }
                }
            }
        }
    }
}

/// Data for an icon cell in the grid
#[derive(Clone)]
struct IconCellData {
    icon: TablerIcon,
    style: TablerIconStyle,
}

/// Renders the icon grid reactively using for_each_dom
fn reactive_icon_grid(
    __scope: &mut RenderScope,
    current_page: Signal<usize>,
    use_filled: Signal<bool>,
) -> NodeHandle {
    // Create a grid container for the icon list
    let grid_container = __scope.create_element("div");
    grid_container.set_attribute(
        "style",
        "display: grid; grid-template-columns: repeat(10, 1fr); gap: 8px;",
    );

    // Create reactive list using for_each_dom - items are inserted as
    // siblings after a comment marker inside the grid container
    let _grid_marker = for_each_dom(
        __scope,
        &grid_container,
        // Each closure - returns ForItems based on current page and style
        move || {
            let page = current_page.get();
            let use_filled_val = use_filled.get();
            let style = if use_filled_val {
                TablerIconStyle::Filled
            } else {
                TablerIconStyle::Outline
            };

            let start = page * ICONS_PER_PAGE;
            let end = (start + ICONS_PER_PAGE).min(ALL_ICONS.len());
            let page_icons = &ALL_ICONS[start..end];

            page_icons
                .iter()
                .map(|icon| {
                    // Key includes page and style so items change when these change
                    let key = format!("{}_{}_{}",icon.name(), page, use_filled_val);
                    ForItem::new(key, IconCellData { icon: *icon, style })
                })
                .collect()
        },
        // View function - renders a single icon cell
        |item, scope| {
            let data = item.downcast::<IconCellData>().unwrap();

            let cell = scope.create_element("div");
            cell.set_attribute("class", "icon-cell");
            cell.set_attribute(
                "style",
                "display: flex; flex-direction: column; align-items: center; padding: 12px; \
                 border-radius: var(--rinch-radius-sm); cursor: pointer; transition: background 0.15s;",
            );
            cell.set_attribute("title", data.icon.name());

            // Icon wrapper
            let icon_wrapper = scope.create_element("div");
            icon_wrapper.set_attribute(
                "style",
                "width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;",
            );

            // Render the icon
            let icon_svg = render_tabler_icon(scope, data.icon, data.style);
            icon_wrapper.append_child(&icon_svg);
            cell.append_child(&icon_wrapper);

            // Icon name
            let name_text = scope.create_element("span");
            name_text.set_attribute(
                "style",
                "font-size: 10px; color: var(--rinch-color-dimmed); margin-top: 4px; \
                 max-width: 80px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: center;",
            );
            name_text.set_text(data.icon.name());
            cell.append_child(&name_text);

            cell
        },
    );

    grid_container
}
