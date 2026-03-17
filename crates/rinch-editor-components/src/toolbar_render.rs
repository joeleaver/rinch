//! Toolbar DOM rendering with Tabler Icons.
//!
//! Renders ToolbarConfig groups and controls as interactive buttons
//! that dispatch editor commands when clicked. All commands go through
//! the CE API (`with_active_ce_api`) — same code path as keyboard shortcuts.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::ce::with_active_ce_api;
use rinch_core::dom::{NodeHandle, RenderScope};

use crate::{ControlButton, ToolbarConfig, ToolbarControl, ToolbarGroup};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// Shared dropdown open/close state threaded through toolbar rendering functions.
#[derive(Clone)]
struct DropdownState {
    heading_open: Rc<RefCell<bool>>,
    color_open: Rc<RefCell<bool>>,
    link_input_open: Rc<RefCell<bool>>,
    link_url: Rc<RefCell<String>>,
}

/// Render the full toolbar from a ToolbarConfig.
///
/// The toolbar uses the active CE API (thread-local) for all commands and
/// state queries. No Editor dependency needed.
pub fn render_toolbar(
    scope: &mut RenderScope,
    config: &ToolbarConfig,
    on_change: Rc<dyn Fn()>,
) -> NodeHandle {
    let __scope = scope;

    // Dropdown open/close state (replaces thread_local).
    let ds = DropdownState {
        heading_open: Rc::new(RefCell::new(false)),
        color_open: Rc::new(RefCell::new(false)),
        link_input_open: Rc::new(RefCell::new(false)),
        link_url: Rc::new(RefCell::new(String::from("https://"))),
    };

    let toolbar = rinch_macros::rsx! {
        div {
            class: "editor-toolbar",
            style: "display: flex; flex-wrap: wrap; gap: 8px; align-items: center; \
                    padding: 8px 12px; border-bottom: 1px solid var(--rinch-color-gray-3); \
                    background: var(--rinch-color-gray-0);",
        }
    };

    for (i, group) in config.groups.iter().enumerate() {
        if i > 0 {
            let divider = rinch_macros::rsx! {
                div {
                    style: "width: 1px; height: 24px; background: var(--rinch-color-gray-3); margin: 0 4px;",
                }
            };
            toolbar.append_child(&divider);
        }

        let group_node = render_toolbar_group(__scope, group, on_change.clone(), ds.clone());
        toolbar.append_child(&group_node);
    }

    // Backdrop overlay: when any dropdown is open, render a transparent full-screen
    // div behind the dropdowns. Clicking it closes all dropdowns.
    let any_open =
        *ds.heading_open.borrow() || *ds.color_open.borrow() || *ds.link_input_open.borrow();
    if any_open {
        let h = ds.heading_open.clone();
        let c = ds.color_open.clone();
        let l = ds.link_input_open.clone();
        let on_change_clone = on_change.clone();
        let backdrop = rinch_macros::rsx! {
            div {
                style: "position: fixed; top: 0; left: 0; width: 100vw; height: 100vh; \
                        z-index: 999; background: transparent;",
                onclick: move || {
                    *h.borrow_mut() = false;
                    *c.borrow_mut() = false;
                    *l.borrow_mut() = false;
                    on_change_clone();
                },
            }
        };
        toolbar.append_child(&backdrop);
    }

    toolbar
}

/// Render a single toolbar group.
fn render_toolbar_group(
    scope: &mut RenderScope,
    group: &ToolbarGroup,
    on_change: Rc<dyn Fn()>,
    ds: DropdownState,
) -> NodeHandle {
    let __scope = scope;
    let group_div = rinch_macros::rsx! {
        div { style: "display: flex; gap: 2px; align-items: center;" }
    };

    for control in &group.controls {
        let btn_node = render_toolbar_button(__scope, control, on_change.clone(), ds.clone());
        group_div.append_child(&btn_node);
    }

    group_div
}

/// Render a single toolbar button with icon and click handler.
fn render_toolbar_button(
    scope: &mut RenderScope,
    control: &ToolbarControl,
    on_change: Rc<dyn Fn()>,
    ds: DropdownState,
) -> NodeHandle {
    let __scope = scope;

    // Dispatch to specialized renderers for dropdown controls.
    match control {
        ToolbarControl::HeadingDropdown => {
            return render_heading_dropdown(__scope, on_change, ds);
        }
        ToolbarControl::TextColorPicker => {
            return render_color_picker(__scope, on_change, ds);
        }
        ToolbarControl::Link => {
            return render_link_popover(__scope, on_change, ds);
        }
        _ => {}
    }

    let meta = ControlButton::from_control(control.clone());

    // Check active state via CE API
    let is_active = is_control_active(control);

    let style = if is_active {
        "display: inline-flex; align-items: center; justify-content: center; \
         width: 32px; height: 32px; border-radius: 4px; cursor: pointer; \
         border: 1px solid var(--rinch-primary-color); \
         background: var(--rinch-color-blue-1); \
         color: var(--rinch-primary-color); \
         transition: background 0.15s;"
    } else {
        "display: inline-flex; align-items: center; justify-content: center; \
         width: 32px; height: 32px; border-radius: 4px; cursor: pointer; \
         border: 1px solid transparent; \
         transition: background 0.15s;"
    };

    // Tooltip with shortcut hint
    let title = if let Some(shortcut) = meta.shortcut_hint() {
        format!("{} ({})", meta.tooltip(), shortcut)
    } else {
        meta.tooltip().to_string()
    };

    // Try to render a Tabler icon; fall back to text label
    if let Some(tabler_icon) = control_to_tabler_icon(control) {
        let icon_node = render_tabler_icon(__scope, tabler_icon, TablerIconStyle::Outline);
        let cc = control.clone();
        let oc = on_change.clone();
        let el = rinch_macros::rsx! {
            div {
                onclick: move || {
                    execute_toolbar_command(&cc, &*oc);
                },
                {icon_node}
            }
        };
        el.set_attribute("style", style);
        el.set_attribute("title", &title);
        el
    } else {
        let label_text = meta.label().to_string();
        let cc = control.clone();
        let oc = on_change.clone();
        let label_span = rinch_macros::rsx! {
            span { style: "font-size: 12px; font-weight: 600;" }
        };
        label_span.set_text(&label_text);
        let el = rinch_macros::rsx! {
            div {
                onclick: move || {
                    execute_toolbar_command(&cc, &*oc);
                },
                {label_span}
            }
        };
        el.set_attribute("style", style);
        el.set_attribute("title", &title);
        el
    }
}

/// Get the current block type label for the heading dropdown via CE API.
fn current_block_label_from_ce() -> &'static str {
    let tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
    match tag.as_deref() {
        Some("h1") => "Heading 1",
        Some("h2") => "Heading 2",
        Some("h3") => "Heading 3",
        Some("h4") => "Heading 4",
        Some("h5") => "Heading 5",
        Some("h6") => "Heading 6",
        Some("blockquote") => "Blockquote",
        Some("pre") => "Code Block",
        Some("ul") => "Bullet List",
        Some("ol") => "Ordered List",
        _ => "Paragraph",
    }
}

/// Render the heading dropdown (replaces separate H1-H6 + Paragraph buttons).
fn render_heading_dropdown(
    scope: &mut RenderScope,
    on_change: Rc<dyn Fn()>,
    ds: DropdownState,
) -> NodeHandle {
    let __scope = scope;
    let is_open = *ds.heading_open.borrow();

    let label = current_block_label_from_ce();
    let chevron = render_tabler_icon(__scope, TablerIcon::ChevronDown, TablerIconStyle::Outline);

    // Toggle dropdown on click
    let on_change_toggle = on_change.clone();
    let ds_toggle = ds.clone();

    let container = rinch_macros::rsx! {
        div {
            style: "position: relative; display: inline-flex; align-items: center;",
            div {
                style: "display: inline-flex; align-items: center; gap: 4px; \
                        padding: 4px 8px; border-radius: 4px; cursor: pointer; \
                        border: 1px solid transparent; font-size: 13px; \
                        transition: background 0.15s; min-width: 90px;",
                title: "Block type",
                onclick: move || {
                    let mut v = ds_toggle.heading_open.borrow_mut();
                    *v = !*v;
                    drop(v);
                    // Close other dropdowns if open
                    *ds_toggle.color_open.borrow_mut() = false;
                    *ds_toggle.link_input_open.borrow_mut() = false;
                    on_change_toggle();
                },
                span { {label} }
                {chevron}
            }
        }
    };

    // Dropdown menu
    if is_open {
        let dropdown = rinch_macros::rsx! {
            div {
                style: "position: absolute; top: 100%; left: 0; z-index: 1000; \
                        background: var(--rinch-color-body, #fff); \
                        border: 1px solid var(--rinch-color-gray-3); \
                        border-radius: 4px; box-shadow: 0 2px 8px rgba(0,0,0,0.12); \
                        padding: 4px 0; min-width: 150px;",
            }
        };

        let items: &[(&str, Option<&str>)] = &[
            ("Paragraph", Some("p")),
            ("Heading 1", Some("h1")),
            ("Heading 2", Some("h2")),
            ("Heading 3", Some("h3")),
            ("Heading 4", Some("h4")),
            ("Heading 5", Some("h5")),
            ("Heading 6", Some("h6")),
        ];

        for &(item_label, target_tag) in items {
            let font_size = match target_tag {
                Some("h1") => "18px",
                Some("h2") => "16px",
                Some("h3") => "15px",
                _ => "13px",
            };
            let font_weight = if target_tag != Some("p") {
                "600"
            } else {
                "400"
            };

            let item_style = format!(
                "padding: 6px 12px; cursor: pointer; font-size: {}; font-weight: {}; \
                 transition: background 0.1s;",
                font_size, font_weight,
            );

            let on_change_clone = on_change.clone();
            let ho_clone = ds.heading_open.clone();
            let tag = target_tag.unwrap_or("p").to_string();
            let item = rinch_macros::rsx! {
                div {
                    onclick: move || {
                        *ho_clone.borrow_mut() = false;
                        with_active_ce_api(|api| {
                            api.borrow_mut().set_block_type(&tag);
                        });
                        on_change_clone();
                    },
                }
            };
            item.set_attribute("style", &item_style);
            item.set_text(item_label);

            dropdown.append_child(&item);
        }

        container.append_child(&dropdown);
    }

    container
}

/// Color palette for the color picker.
const COLOR_PALETTE: &[(&str, &str)] = &[
    ("Black", "#000000"),
    ("Gray", "#868e96"),
    ("Red", "#e03131"),
    ("Orange", "#e8590c"),
    ("Yellow", "#fcc419"),
    ("Green", "#2f9e44"),
    ("Cyan", "#1098ad"),
    ("Blue", "#1971c2"),
    ("Purple", "#7048e8"),
    ("Pink", "#c2255c"),
];

/// Render the color picker dropdown.
///
/// Note: Text color requires `wrap_selection_with_attrs()` which isn't in the CE API yet.
/// For now, color swatches are rendered but clicking them is a no-op.
fn render_color_picker(
    scope: &mut RenderScope,
    on_change: Rc<dyn Fn()>,
    ds: DropdownState,
) -> NodeHandle {
    let __scope = scope;
    let is_open = *ds.color_open.borrow();

    let icon_node = render_tabler_icon(__scope, TablerIcon::Palette, TablerIconStyle::Outline);

    // Toggle dropdown on click
    let on_change_toggle = on_change.clone();
    let ds_toggle = ds.clone();

    let container = rinch_macros::rsx! {
        div {
            style: "position: relative; display: inline-flex; align-items: center;",
            div {
                style: "display: inline-flex; align-items: center; justify-content: center; \
                        flex-direction: column; width: 32px; height: 32px; \
                        border-radius: 4px; cursor: pointer; border: 1px solid transparent; \
                        transition: background 0.15s;",
                title: "Text color",
                onclick: move || {
                    let mut v = ds_toggle.color_open.borrow_mut();
                    *v = !*v;
                    drop(v);
                    // Close other dropdowns if open
                    *ds_toggle.heading_open.borrow_mut() = false;
                    *ds_toggle.link_input_open.borrow_mut() = false;
                    on_change_toggle();
                },
                {icon_node}
                div {
                    style: "width: 16px; height: 3px; border-radius: 1px; \
                            background: var(--rinch-primary-color, #1971c2); margin-top: -2px;",
                }
            }
        }
    };

    // Dropdown color grid
    if is_open {
        let dropdown = rinch_macros::rsx! {
            div {
                style: "position: absolute; top: 100%; left: 0; z-index: 1000; \
                        background: var(--rinch-color-body, #fff); \
                        border: 1px solid var(--rinch-color-gray-3); \
                        border-radius: 4px; box-shadow: 0 2px 8px rgba(0,0,0,0.12); \
                        padding: 8px; display: flex; flex-wrap: wrap; gap: 4px; width: 140px;",
            }
        };

        for &(color_name, color_hex) in COLOR_PALETTE {
            let swatch_style = format!(
                "width: 24px; height: 24px; border-radius: 4px; cursor: pointer; \
                 background: {}; border: 1px solid var(--rinch-color-gray-3); \
                 transition: transform 0.1s;",
                color_hex,
            );

            let on_change_clone = on_change.clone();
            let co_clone = ds.color_open.clone();
            let color_name_str = color_name.to_string();
            let swatch = rinch_macros::rsx! {
                div {
                    onclick: move || {
                        // Close dropdown
                        *co_clone.borrow_mut() = false;
                        // TODO: Text color needs wrap_selection_with_attrs() — deferred
                        on_change_clone();
                    },
                }
            };
            swatch.set_attribute("style", &swatch_style);
            swatch.set_attribute("title", &color_name_str);

            dropdown.append_child(&swatch);
        }

        container.append_child(&dropdown);
    }

    container
}

/// Render the link button with a URL input popover.
///
/// Note: Link insertion requires `wrap_selection_with_attrs()` which isn't in
/// the CE API yet. For now, the popover renders but Apply is a no-op.
fn render_link_popover(
    scope: &mut RenderScope,
    on_change: Rc<dyn Fn()>,
    ds: DropdownState,
) -> NodeHandle {
    let __scope = scope;
    let is_open = *ds.link_input_open.borrow();
    let link_url = ds.link_url.clone();

    // Check active state — link is a formatting ancestor in the DOM
    let is_active = with_active_ce_api(|api| api.borrow().has_active_mark("a")).unwrap_or(false);

    let style = if is_active {
        "display: inline-flex; align-items: center; justify-content: center; \
         width: 32px; height: 32px; border-radius: 4px; cursor: pointer; \
         border: 1px solid var(--rinch-primary-color); \
         background: var(--rinch-color-blue-1); \
         color: var(--rinch-primary-color); \
         transition: background 0.15s;"
    } else {
        "display: inline-flex; align-items: center; justify-content: center; \
         width: 32px; height: 32px; border-radius: 4px; cursor: pointer; \
         border: 1px solid transparent; \
         transition: background 0.15s;"
    };

    let icon_node = render_tabler_icon(__scope, TablerIcon::Link, TablerIconStyle::Outline);

    // Toggle popover on click
    let on_change_toggle = on_change.clone();
    let ds_toggle = ds.clone();

    let container = rinch_macros::rsx! {
        div {
            style: "position: relative; display: inline-flex; align-items: center;",
            div {
                style: {style},
                title: "Insert link",
                onclick: move || {
                    let mut v = ds_toggle.link_input_open.borrow_mut();
                    *v = !*v;
                    drop(v);
                    // Close other dropdowns if open
                    *ds_toggle.heading_open.borrow_mut() = false;
                    *ds_toggle.color_open.borrow_mut() = false;
                    on_change_toggle();
                },
                {icon_node}
            }
        }
    };

    // URL input popover
    if is_open {
        let url_state = link_url.clone();
        let url_value = link_url.borrow().clone();
        let link_open_clone = ds.link_input_open.clone();
        let on_change_clone = on_change.clone();
        let popover = rinch_macros::rsx! {
            div {
                style: "position: absolute; top: 100%; left: 0; z-index: 1000; \
                        background: var(--rinch-color-body, #fff); \
                        border: 1px solid var(--rinch-color-gray-3); \
                        border-radius: 4px; box-shadow: 0 2px 8px rgba(0,0,0,0.12); \
                        padding: 8px; display: flex; gap: 4px; align-items: center;",
                input {
                    r#type: "text",
                    value: {url_value.as_str()},
                    placeholder: "Enter URL...",
                    style: "width: 200px; padding: 4px 8px; \
                            border: 1px solid var(--rinch-color-gray-4); \
                            border-radius: 4px; font-size: 13px;",
                    oninput: move |value: String| {
                        *url_state.borrow_mut() = value;
                    },
                }
                div {
                    style: "padding: 4px 12px; border-radius: 4px; cursor: pointer; \
                            background: var(--rinch-primary-color, #1971c2); color: #fff; \
                            font-size: 13px; font-weight: 500; white-space: nowrap;",
                    onclick: move || {
                        *link_open_clone.borrow_mut() = false;
                        // TODO: Link insertion needs wrap_selection_with_attrs() — deferred
                        on_change_clone();
                    },
                    "Apply"
                }
            }
        };

        container.append_child(&popover);
    }

    container
}

/// Check if a toolbar control's format is currently active at the cursor.
///
/// Queries the CE API's DOM to check formatting ancestors and block type.
fn is_control_active(control: &ToolbarControl) -> bool {
    match control {
        ToolbarControl::Bold => {
            with_active_ce_api(|api| api.borrow().has_active_mark("strong")).unwrap_or(false)
        }
        ToolbarControl::Italic => {
            with_active_ce_api(|api| api.borrow().has_active_mark("em")).unwrap_or(false)
        }
        ToolbarControl::Underline => {
            with_active_ce_api(|api| api.borrow().has_active_mark("u")).unwrap_or(false)
        }
        ToolbarControl::Strike => {
            with_active_ce_api(|api| api.borrow().has_active_mark("s")).unwrap_or(false)
        }
        ToolbarControl::Code => {
            with_active_ce_api(|api| api.borrow().has_active_mark("code")).unwrap_or(false)
        }
        ToolbarControl::Highlight => {
            with_active_ce_api(|api| api.borrow().has_active_mark("mark")).unwrap_or(false)
        }
        ToolbarControl::Subscript => {
            with_active_ce_api(|api| api.borrow().has_active_mark("sub")).unwrap_or(false)
        }
        ToolbarControl::Superscript => {
            with_active_ce_api(|api| api.borrow().has_active_mark("sup")).unwrap_or(false)
        }
        ToolbarControl::Heading(level) => {
            let tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
            tag.as_deref() == Some(&format!("h{}", level))
        }
        ToolbarControl::Paragraph => {
            let tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
            tag.as_deref() == Some("p") || tag.as_deref() == Some("div")
        }
        ToolbarControl::BulletList => {
            let tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
            tag.as_deref() == Some("ul")
        }
        ToolbarControl::OrderedList => {
            let tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
            tag.as_deref() == Some("ol")
        }
        ToolbarControl::Blockquote => {
            let tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
            tag.as_deref() == Some("blockquote")
        }
        ToolbarControl::CodeBlock => {
            let tag = with_active_ce_api(|api| api.borrow().cursor_block_tag()).flatten();
            tag.as_deref() == Some("pre")
        }
        ToolbarControl::Link => {
            with_active_ce_api(|api| api.borrow().has_active_mark("a")).unwrap_or(false)
        }
        // Alignment, dropdowns, and table controls don't have simple active state
        _ => false,
    }
}

/// Map a ToolbarControl to the corresponding TablerIcon variant.
fn control_to_tabler_icon(control: &ToolbarControl) -> Option<TablerIcon> {
    match control {
        ToolbarControl::Bold => Some(TablerIcon::Bold),
        ToolbarControl::Italic => Some(TablerIcon::Italic),
        ToolbarControl::Underline => Some(TablerIcon::Underline),
        ToolbarControl::Strike => Some(TablerIcon::Strikethrough),
        ToolbarControl::Code => Some(TablerIcon::Code),
        ToolbarControl::Highlight => Some(TablerIcon::Highlight),
        ToolbarControl::Subscript => Some(TablerIcon::Subscript),
        ToolbarControl::Superscript => Some(TablerIcon::Superscript),
        ToolbarControl::Link => Some(TablerIcon::Link),
        ToolbarControl::TextColor(_) => Some(TablerIcon::Palette),
        ToolbarControl::HeadingDropdown => Some(TablerIcon::Heading),
        ToolbarControl::TextColorPicker => Some(TablerIcon::Palette),
        ToolbarControl::Heading(1) => Some(TablerIcon::H1),
        ToolbarControl::Heading(2) => Some(TablerIcon::H2),
        ToolbarControl::Heading(3) => Some(TablerIcon::H3),
        ToolbarControl::Heading(4) => Some(TablerIcon::H4),
        ToolbarControl::Heading(5) => Some(TablerIcon::H5),
        ToolbarControl::Heading(6) => Some(TablerIcon::H6),
        ToolbarControl::Heading(_) => Some(TablerIcon::Heading),
        ToolbarControl::Paragraph => Some(TablerIcon::Pilcrow),
        ToolbarControl::BulletList => Some(TablerIcon::List),
        ToolbarControl::OrderedList => Some(TablerIcon::ListNumbers),
        ToolbarControl::Blockquote => Some(TablerIcon::Blockquote),
        ToolbarControl::CodeBlock => Some(TablerIcon::SourceCode),
        ToolbarControl::HorizontalRule => Some(TablerIcon::SeparatorHorizontal),
        ToolbarControl::HardBreak => Some(TablerIcon::TextWrap),
        ToolbarControl::Undo => Some(TablerIcon::ArrowBackUp),
        ToolbarControl::Redo => Some(TablerIcon::ArrowForwardUp),
        ToolbarControl::ClearFormatting => Some(TablerIcon::ClearFormatting),
        ToolbarControl::InsertTable => Some(TablerIcon::Table),
        ToolbarControl::AlignLeft => Some(TablerIcon::AlignLeft),
        ToolbarControl::AlignCenter => Some(TablerIcon::AlignCenter),
        ToolbarControl::AlignRight => Some(TablerIcon::AlignRight),
        ToolbarControl::AlignJustify => Some(TablerIcon::AlignJustified),
        ToolbarControl::InsertRowBefore => Some(TablerIcon::RowInsertTop),
        ToolbarControl::InsertRowAfter => Some(TablerIcon::RowInsertBottom),
        ToolbarControl::InsertColBefore => Some(TablerIcon::ColumnInsertLeft),
        ToolbarControl::InsertColAfter => Some(TablerIcon::ColumnInsertRight),
        ToolbarControl::DeleteRow => Some(TablerIcon::RowRemove),
        ToolbarControl::DeleteCol => Some(TablerIcon::ColumnRemove),
        ToolbarControl::ToggleHeaderRow => Some(TablerIcon::TableRow),
        ToolbarControl::MergeCells => Some(TablerIcon::TableShortcut),
        ToolbarControl::SplitCell => Some(TablerIcon::LayoutColumns),
        ToolbarControl::DeleteTable => Some(TablerIcon::TableOff),
        ToolbarControl::Custom { .. } => None,
    }
}

/// Execute the editor command associated with a toolbar control.
///
/// All commands go through the CE API — same code path as keyboard shortcuts.
fn execute_toolbar_command(control: &ToolbarControl, on_change: &dyn Fn()) {
    let had_api = with_active_ce_api(|api| {
        let mut api = api.borrow_mut();
        match control {
            ToolbarControl::Bold => api.toggle_wrap("strong"),
            ToolbarControl::Italic => api.toggle_wrap("em"),
            ToolbarControl::Underline => api.toggle_wrap("u"),
            ToolbarControl::Strike => api.toggle_wrap("s"),
            ToolbarControl::Code => api.toggle_wrap("code"),
            ToolbarControl::Highlight => api.toggle_wrap("mark"),
            ToolbarControl::Subscript => api.toggle_wrap("sub"),
            ToolbarControl::Superscript => api.toggle_wrap("sup"),
            ToolbarControl::Heading(level) => api.set_block_type(&format!("h{}", level)),
            ToolbarControl::Paragraph => api.set_block_type("p"),
            ToolbarControl::BulletList => api.set_block_type("ul"),
            ToolbarControl::OrderedList => api.set_block_type("ol"),
            ToolbarControl::Blockquote => api.set_block_type("blockquote"),
            ToolbarControl::CodeBlock => api.set_block_type("pre"),
            ToolbarControl::HorizontalRule => {
                api.split_block();
                api.set_block_type("hr");
            }
            ToolbarControl::HardBreak => api.split_block(),
            ToolbarControl::Undo => api.undo(),
            ToolbarControl::Redo => api.redo(),
            ToolbarControl::ClearFormatting => api.clear_formatting(),
            // Link, TextColor, Table operations — deferred (need additional CE API)
            ToolbarControl::Link
            | ToolbarControl::TextColor(_)
            | ToolbarControl::TextColorPicker
            | ToolbarControl::InsertTable
            | ToolbarControl::AlignLeft
            | ToolbarControl::AlignCenter
            | ToolbarControl::AlignRight
            | ToolbarControl::AlignJustify
            | ToolbarControl::InsertRowBefore
            | ToolbarControl::InsertRowAfter
            | ToolbarControl::InsertColBefore
            | ToolbarControl::InsertColAfter
            | ToolbarControl::DeleteRow
            | ToolbarControl::DeleteCol
            | ToolbarControl::ToggleHeaderRow
            | ToolbarControl::MergeCells
            | ToolbarControl::SplitCell
            | ToolbarControl::DeleteTable => {}
            // HeadingDropdown and TextColorPicker handled by their own renderers
            ToolbarControl::HeadingDropdown => {}
            ToolbarControl::Custom { .. } => {}
        }
    })
    .is_some();

    #[cfg(debug_assertions)]
    {
        if had_api {
            eprintln!(
                "[toolbar] execute_toolbar_command: {:?} — CE API active, command dispatched",
                control
            );
        } else {
            eprintln!(
                "[toolbar] execute_toolbar_command: {:?} — CE API NOT active (click into editor first)",
                control
            );
        }
    }

    on_change();
}
