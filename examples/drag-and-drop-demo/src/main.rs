//! Drag-and-drop demo — showcases the rinch DnD system with:
//! 1. Kanban board: drag cards between columns
//! 2. Reorderable list: drag items to reorder

use rinch::prelude::*;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

// =============================================================================
// Data types
// =============================================================================

#[derive(Clone, Debug, PartialEq)]
struct Card {
    id: u32,
    title: String,
    color: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
struct ListItem {
    id: u32,
    label: String,
    icon: TablerIcon,
}

/// What we're dragging in the kanban board.
#[derive(Clone, Debug)]
struct KanbanDrag {
    card: Card,
    from_column: usize,
}

/// What we're dragging in the reorderable list — just the item index.
#[derive(Clone, Debug)]
struct ListDrag {
    item_id: u32,
}

// =============================================================================
// App
// =============================================================================

#[component]
fn app() -> NodeHandle {
    rsx! {
        Stack { gap: "xl", p: "xl", maw: "1000px",
            Title { order: 1, "Drag & Drop Demo" }
            Text { size: "lg", color: "dimmed",
                "Showcasing the rinch internal drag-and-drop system."
            }

            {kanban_section(__scope)}
            Divider {}
            {reorderable_list_section(__scope)}
        }
    }
}

// =============================================================================
// Section 1: Kanban Board
// =============================================================================

#[component]
fn kanban_section() -> NodeHandle {
    let drag_ctx = DragContext::<KanbanDrag>::new();

    // Three columns: To Do, In Progress, Done
    let columns: Signal<Vec<Vec<Card>>> = Signal::new(vec![
        vec![
            Card {
                id: 1,
                title: "Design mockups".into(),
                color: "blue",
            },
            Card {
                id: 2,
                title: "Write documentation".into(),
                color: "teal",
            },
            Card {
                id: 3,
                title: "Set up CI/CD".into(),
                color: "violet",
            },
        ],
        vec![
            Card {
                id: 4,
                title: "Implement DnD".into(),
                color: "orange",
            },
            Card {
                id: 5,
                title: "Code review".into(),
                color: "cyan",
            },
        ],
        vec![Card {
            id: 6,
            title: "Deploy v1.0".into(),
            color: "green",
        }],
    ]);

    let column_names = ["To Do", "In Progress", "Done"];
    let column_colors = ["blue", "orange", "green"];

    // Drop target: (col_idx, Some(card_id)) = before that card, (col_idx, None) = end of column
    let kanban_drop_target = Signal::new(Option::<(usize, Option<u32>)>::None);

    // Pre-render icons
    let icon_clipboard =
        render_tabler_icon(__scope, TablerIcon::ClipboardList, TablerIconStyle::Outline);
    let icon_progress = render_tabler_icon(__scope, TablerIcon::Progress, TablerIconStyle::Outline);
    let icon_check = render_tabler_icon(__scope, TablerIcon::CircleCheck, TablerIconStyle::Outline);
    let col_icons = [icon_clipboard, icon_progress, icon_check];

    rsx! {
        Stack { gap: "md",
            Title { order: 3, "Kanban Board" }
            Text { color: "dimmed", size: "sm",
                "Drag cards between columns. A placeholder shows where the card will land."
            }

            Group { gap: "md", align: "flex-start",
                style: "width: 100%;",

                // Column 0: To Do
                {kanban_column(__scope, drag_ctx, columns, kanban_drop_target, 0, column_names[0], column_colors[0], col_icons[0].clone())}
                // Column 1: In Progress
                {kanban_column(__scope, drag_ctx, columns, kanban_drop_target, 1, column_names[1], column_colors[1], col_icons[1].clone())}
                // Column 2: Done
                {kanban_column(__scope, drag_ctx, columns, kanban_drop_target, 2, column_names[2], column_colors[2], col_icons[2].clone())}
            }
        }
    }
}

#[component]
fn kanban_column(
    drag_ctx: DragContext<KanbanDrag>,
    columns: Signal<Vec<Vec<Card>>>,
    kanban_drop_target: Signal<Option<(usize, Option<u32>)>>,
    col_idx: usize,
    col_name: &str,
    col_color: &str,
    col_icon: NodeHandle,
) -> NodeHandle {
    let col_name = col_name.to_string();
    let col_color = col_color.to_string();
    let col_color_badge = col_color.clone();

    rsx! {
        div {
            style: "flex: 1; min-height: 200px; padding: 12px; border-radius: var(--rinch-radius-md); \
                    background: var(--rinch-color-gray-0); border: 2px solid var(--rinch-color-gray-3);",
            // Drop target for column empty space (appends to end)
            ondrop: move || {
                kanban_drop_target.set(None);
                if let Some(drag_data) = drag_ctx.take() {
                    columns.update(|cols| {
                        if let Some(src) = cols.get_mut(drag_data.from_column) {
                            src.retain(|c| c.id != drag_data.card.id);
                        }
                        if let Some(dst) = cols.get_mut(col_idx) {
                            dst.push(drag_data.card);
                        }
                    });
                }
            },
            ondragenter: move || {
                kanban_drop_target.set(Some((col_idx, None)));
            },
            // Column header
            Group { gap: "xs", align: "center",
                style: "margin-bottom: 12px;",
                {col_icon}
                Text { weight: "700", size: "sm", {col_name.clone()} }
                Badge {
                    color: col_color_badge.clone(),
                    variant: "light",
                    size: "sm",
                    {|| columns.get()[col_idx].len().to_string()}
                }
            }

            // Cards with placeholders
            Stack { gap: "xs",
                for card in columns.get()[col_idx].clone() {
                    div { key: card.id, style: "display: contents;",
                        // Placeholder before this card
                        if kanban_drop_target.get() == Some((col_idx, Some(card.id)))
                            && drag_ctx.get().map(|d| d.card.id) != Some(card.id)
                        {
                            {kanban_placeholder(__scope, drag_ctx, kanban_drop_target, columns, col_idx, Some(card.id))}
                        }
                        {kanban_card(__scope, drag_ctx, kanban_drop_target, card.clone(), col_idx, columns)}
                    }
                }
                // End-of-column placeholder
                if kanban_drop_target.get() == Some((col_idx, None))
                    && drag_ctx.is_active()
                {
                    {kanban_placeholder(__scope, drag_ctx, kanban_drop_target, columns, col_idx, None)}
                }
            }
        }
    }
}

#[component]
fn kanban_placeholder(
    drag_ctx: DragContext<KanbanDrag>,
    kanban_drop_target: Signal<Option<(usize, Option<u32>)>>,
    columns: Signal<Vec<Vec<Card>>>,
    col_idx: usize,
    before_card_id: Option<u32>,
) -> NodeHandle {
    let title = drag_ctx
        .get()
        .map(|d| d.card.title.clone())
        .unwrap_or_default();
    let color = drag_ctx.get().map(|d| d.card.color).unwrap_or("gray");

    rsx! {
        div {
            style: {
                format!(
                    "padding: 10px 14px; border-radius: var(--rinch-radius-sm); \
                     border: 2px dashed var(--rinch-color-{}-4); \
                     background: var(--rinch-color-{}-0); \
                     border-left: 3px solid var(--rinch-color-{}-4);",
                    color, color, color,
                )
            },
            ondragenter: move || {
                kanban_drop_target.set(Some((col_idx, before_card_id)));
            },
            ondrop: move || {
                kanban_drop_target.set(None);
                if let Some(drag_data) = drag_ctx.take() {
                    columns.update(|cols| {
                        // Remove from source
                        if let Some(src) = cols.get_mut(drag_data.from_column) {
                            src.retain(|c| c.id != drag_data.card.id);
                        }
                        // Insert at target position
                        if let Some(dst) = cols.get_mut(col_idx) {
                            if let Some(target_id) = before_card_id {
                                if let Some(pos) = dst.iter().position(|c| c.id == target_id) {
                                    dst.insert(pos, drag_data.card);
                                } else {
                                    dst.push(drag_data.card);
                                }
                            } else {
                                dst.push(drag_data.card);
                            }
                        }
                    });
                }
            },
            Text { size: "sm", color: "dimmed", {title} }
        }
    }
}

#[component]
fn kanban_card(
    drag_ctx: DragContext<KanbanDrag>,
    kanban_drop_target: Signal<Option<(usize, Option<u32>)>>,
    card: Card,
    from_column: usize,
    columns: Signal<Vec<Vec<Card>>>,
) -> NodeHandle {
    let card_id = card.id;
    let card_for_drag = card.clone();

    rsx! {
        div {
            key: card.id,
            draggable: "true",
            ondragstart: move || {
                drag_ctx.set(KanbanDrag {
                    card: card_for_drag.clone(),
                    from_column,
                });
            },
            ondragend: move || {
                drag_ctx.clear();
                kanban_drop_target.set(None);
            },
            ondragenter: move || {
                kanban_drop_target.set(Some((from_column, Some(card_id))));
            },
            ondrop: move || {
                kanban_drop_target.set(None);
                if let Some(drag_data) = drag_ctx.take()
                    && drag_data.card.id != card_id
                {
                    columns.update(|cols| {
                        // Remove from source
                        if let Some(src) = cols.get_mut(drag_data.from_column) {
                            src.retain(|c| c.id != drag_data.card.id);
                        }
                        // Insert before this card in the destination column
                        if let Some(dst) = cols.get_mut(from_column) {
                            if let Some(pos) = dst.iter().position(|c| c.id == card_id) {
                                dst.insert(pos, drag_data.card);
                            } else {
                                dst.push(drag_data.card);
                            }
                        }
                    });
                }
            },
            style: {
                let color = card.color;
                move || {
                    let is_dragging = drag_ctx.is_active()
                        && drag_ctx.get().map(|d| d.card.id) == Some(card_id);
                    let opacity = if is_dragging { "0.4" } else { "1" };
                    format!(
                        "padding: 10px 14px; background: white; border-radius: var(--rinch-radius-sm); \
                         border-left: 3px solid var(--rinch-color-{}-5); cursor: grab; \
                         box-shadow: var(--rinch-shadow-xs); opacity: {};",
                        color, opacity,
                    )
                }
            },
            Text { size: "sm", {card.title.clone()} }
        }
    }
}

// =============================================================================
// Section 2: Reorderable List
// =============================================================================

#[component]
fn reorderable_list_section() -> NodeHandle {
    let drag_ctx = DragContext::<ListDrag>::new();

    let items = Signal::new(vec![
        ListItem {
            id: 1,
            label: "Rust".into(),
            icon: TablerIcon::Code,
        },
        ListItem {
            id: 2,
            label: "TypeScript".into(),
            icon: TablerIcon::BrandTypescript,
        },
        ListItem {
            id: 3,
            label: "Python".into(),
            icon: TablerIcon::BrandPython,
        },
        ListItem {
            id: 4,
            label: "Go".into(),
            icon: TablerIcon::BrandGolang,
        },
        ListItem {
            id: 5,
            label: "Zig".into(),
            icon: TablerIcon::ZoomCode,
        },
    ]);

    // Track which item is being hovered as a drop target
    let drop_target_id = Signal::new(Option::<u32>::None);

    rsx! {
        Stack { gap: "md",
            Title { order: 3, "Reorderable List" }
            Text { color: "dimmed", size: "sm",
                "Drag items to reorder. A placeholder shows where the item will be inserted."
            }

            Paper { p: "lg", radius: "md", with_border: true,
                maw: "400px",
                Stack { gap: "0",
                    for item in items.get() {
                        div { key: item.id, style: "display: contents;",
                            // Placeholder before this item
                            if drop_target_id.get() == Some(item.id)
                                && drag_ctx.get().map(|d| d.item_id) != Some(item.id)
                            {
                                {list_placeholder(__scope, drag_ctx, items, drop_target_id, item.id)}
                            }
                            {reorderable_item(__scope, drag_ctx, items, drop_target_id, item.clone())}
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn list_placeholder(
    drag_ctx: DragContext<ListDrag>,
    items: Signal<Vec<ListItem>>,
    drop_target_id: Signal<Option<u32>>,
    target_item_id: u32,
) -> NodeHandle {
    rsx! {
        div {
            style: "display: flex; align-items: center; gap: 12px; padding: 10px 14px; \
                    border: 2px dashed var(--rinch-color-blue-4); \
                    border-radius: var(--rinch-radius-sm); \
                    background: var(--rinch-color-blue-0); \
                    min-height: 17px;",
            ondragenter: move || {
                drop_target_id.set(Some(target_item_id));
            },
            ondrop: move || {
                drop_target_id.set(None);
                if let Some(drag_data) = drag_ctx.take()
                    && drag_data.item_id != target_item_id
                {
                    items.update(|list| {
                        let src_idx = list.iter().position(|i| i.id == drag_data.item_id);
                        let dst_idx = list.iter().position(|i| i.id == target_item_id);
                        if let (Some(src), Some(dst)) = (src_idx, dst_idx) {
                            let dragged = list.remove(src);
                            let insert_at = if src < dst { dst - 1 } else { dst };
                            list.insert(insert_at, dragged);
                        }
                    });
                }
            },
        }
    }
}

#[component]
fn reorderable_item(
    drag_ctx: DragContext<ListDrag>,
    items: Signal<Vec<ListItem>>,
    drop_target_id: Signal<Option<u32>>,
    item: ListItem,
) -> NodeHandle {
    let item_id = item.id;
    let icon_el = render_tabler_icon(__scope, item.icon, TablerIconStyle::Outline);

    rsx! {
        div {
            key: item.id,
            draggable: "true",
            ondragstart: move || {
                drag_ctx.set(ListDrag { item_id });
            },
            ondragend: move || {
                drag_ctx.clear();
                drop_target_id.set(None);
            },
            ondrop: move || {
                drop_target_id.set(None);
                if let Some(drag_data) = drag_ctx.take()
                    && drag_data.item_id != item_id
                {
                    items.update(|list| {
                        let src_idx = list.iter().position(|i| i.id == drag_data.item_id);
                        let dst_idx = list.iter().position(|i| i.id == item_id);
                        if let (Some(src), Some(dst)) = (src_idx, dst_idx) {
                            let dragged = list.remove(src);
                            // After remove, indices >= src shift down by 1
                            let insert_at = if src < dst { dst - 1 } else { dst };
                            list.insert(insert_at, dragged);
                        }
                    });
                }
            },
            ondragenter: move || {
                drop_target_id.set(Some(item_id));
            },
            style: {
                move || {
                    let is_dragging = drag_ctx.is_active()
                        && drag_ctx.get().map(|d| d.item_id) == Some(item_id);
                    let opacity = if is_dragging { "0.4" } else { "1" };
                    format!(
                        "display: flex; align-items: center; gap: 12px; padding: 10px 14px; \
                         opacity: {}; cursor: grab; background: white;",
                        opacity,
                    )
                }
            },

            span { style: "color: var(--rinch-color-gray-5); display: flex;",
                {render_tabler_icon(__scope, TablerIcon::GripVertical, TablerIconStyle::Outline)}
            }
            span { style: "display: flex; color: var(--rinch-color-blue-6);",
                {icon_el}
            }
            Text { size: "sm", weight: "500", {item.label.clone()} }
        }
    }
}

// =============================================================================
// Entry point
// =============================================================================

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        ..Default::default()
    };
    run_with_theme("Drag & Drop Demo", 900, 700, app, theme);
}
