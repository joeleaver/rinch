//! Virtual List section - demonstrates virtualized rendering of large lists.

use rinch::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct ListItem {
    id: usize,
    label: String,
}

#[component]
pub fn virtual_list_section() -> NodeHandle {
    let item_count = Signal::new(100_000usize);
    let items: Signal<Vec<ListItem>> = Signal::new(
        (0..100_000)
            .map(|i| ListItem {
                id: i,
                label: format!("Item {i}"),
            })
            .collect(),
    );

    // Rebuild items when count changes
    let rebuild = move || {
        let count = item_count.get();
        items.set(
            (0..count)
                .map(|i| ListItem {
                    id: i,
                    label: format!("Item {i}"),
                })
                .collect(),
        );
    };

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Virtual List" }
                Text { size: "lg", color: "dimmed",
                    "Virtualized rendering for large datasets — only visible items are in the DOM"
                }
            }
            Space { h: "xl" }

            // Controls
            Group { gap: "md",
                Button {
                    variant: "light",
                    onclick: move || { item_count.set(1_000); rebuild(); },
                    "1K items"
                }
                Button {
                    variant: "light",
                    onclick: move || { item_count.set(10_000); rebuild(); },
                    "10K items"
                }
                Button {
                    variant: "filled",
                    onclick: move || { item_count.set(100_000); rebuild(); },
                    "100K items"
                }
            }
            Space { h: "xs" }
            Text { size: "sm", color: "dimmed",
                {|| format!("Showing {} items (only ~15 DOM nodes in viewport)", items.get().len())}
            }
            Space { h: "md" }

            // The virtual list in a fixed-height container
            Paper { p: "0", radius: "md", with_border: true,
                div { style: "height: 500px;",
                    {virtual_list(
                        __scope,
                        36.0,
                        move || items.get(),
                        |item: &ListItem| item.id,
                        5,
                        |item: ListItem, __scope: &mut RenderScope| {
                            let bg = if item.id.is_multiple_of(2) {
                                "var(--rinch-color-dark-8)"
                            } else {
                                "transparent"
                            };
                            rsx! {
                                div {
                                    style: {format!(
                                        "height:36px;display:flex;align-items:center;padding:0 16px;background:{bg};\
                                         border-bottom:1px solid var(--rinch-color-dark-6)"
                                    )},
                                    Text { size: "sm", {item.label} }
                                }
                            }
                        },
                    )}
                }
            }
        }
    }
}
