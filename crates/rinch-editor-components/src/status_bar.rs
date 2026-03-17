//! Status bar rendering for the editor.
//!
//! Shows cursor block type and active marks via the CE API.
//! Reactively updates when `change_count` signal changes.

use rinch_core::ce::with_active_ce_api;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::reactive::{Effect, Signal};

/// Render the status bar showing editor state.
///
/// The status bar reactively updates whenever `change_count` changes.
/// The caller should increment `change_count` on each `on_change` callback
/// from the editor.
pub fn render_status_bar(scope: &mut RenderScope, change_count: Signal<usize>) -> NodeHandle {
    let __scope = scope;

    let block_label = rinch_macros::rsx! {
        span {
            style: "padding: 2px 8px; border-radius: 4px; background: var(--rinch-color-gray-2); \
                    font-weight: 500;",
        }
    };

    let marks_container = rinch_macros::rsx! {
        span { style: "display: flex; gap: 4px;" }
    };

    let block_label_clone = block_label.clone();
    let marks_container_clone = marks_container.clone();

    let bar = rinch_macros::rsx! {
        div {
            class: "editor-status-bar",
            style: "display: flex; gap: 16px; align-items: center; padding: 6px 12px; \
                    font-size: 12px; color: var(--rinch-color-dimmed); \
                    border-top: 1px solid var(--rinch-color-gray-3); \
                    background: var(--rinch-color-gray-0);",
            {block_label}
            {marks_container}
            div { style: "flex: 1;" }
        }
    };

    // Single effect that updates all dynamic content when change_count changes
    Effect::new(move || {
        let _ = change_count.get(); // Track the signal

        // Update block type label via CE API
        let block_tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
        let block_type_str = match block_tag.as_deref() {
            Some("h1") => "Heading 1",
            Some("h2") => "Heading 2",
            Some("h3") => "Heading 3",
            Some("h4") => "Heading 4",
            Some("h5") => "Heading 5",
            Some("h6") => "Heading 6",
            Some("p") | Some("div") => "Paragraph",
            Some("blockquote") => "Blockquote",
            Some("pre") => "Code Block",
            Some("ul") => "Bullet List",
            Some("ol") => "Ordered List",
            Some("hr") => "Horizontal Rule",
            Some(other) => other,
            None => "Unknown",
        };
        block_label_clone.set_text(block_type_str);

        // Update active marks from CE API
        let mut mark_names = Vec::new();
        let mark_tags = ["strong", "em", "u", "s", "code", "mark", "sub", "sup"];
        for &tag in &mark_tags {
            let active =
                with_active_ce_api(|api| api.borrow().has_active_mark(tag)).unwrap_or(false);
            if active {
                mark_names.push(match tag {
                    "strong" => "bold",
                    "em" => "italic",
                    "u" => "underline",
                    "s" => "strike",
                    _ => tag,
                });
            }
        }
        if mark_names.is_empty() {
            marks_container_clone.set_text("");
        } else {
            marks_container_clone.set_text(&mark_names.join(" | "));
        }
    });

    bar
}
