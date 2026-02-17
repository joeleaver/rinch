//! Rich Text Editor section - Working editor with toolbar, content, and status bar.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::dom::RenderScope as CoreRenderScope;
use rinch_core::reactive::Effect;
use rinch_editor::bridge::EditorBridge;
use rinch_editor::editor::{Editor, EditorConfig};
use rinch_editor::schema::Schema;
use rinch_editor_components::{ControlButton, ToolbarConfig, render_status_bar, render_toolbar};

/// State for the Editor section, stored in context.
#[derive(Clone)]
pub struct EditorSectionState {
    pub toolbar_preset: Signal<usize>, // 0=Full, 1=Minimal, 2=Markdown
    pub editor: Rc<RefCell<Editor>>,
}

/// Initialize the Editor section state. Call this from the main app function.
pub fn init_editor_state() {
    let editor = Editor::new(Schema::starter_kit(), EditorConfig::default())
        .expect("Failed to create editor");
    create_context(EditorSectionState {
        toolbar_preset: Signal::new(0),
        editor: Rc::new(RefCell::new(editor)),
    });
}

fn get_toolbar_config(preset: usize) -> ToolbarConfig {
    match preset {
        1 => ToolbarConfig::default_minimal(),
        2 => ToolbarConfig::default_markdown(),
        _ => ToolbarConfig::default_full(),
    }
}

fn preset_name(preset: usize) -> &'static str {
    match preset {
        1 => "Minimal",
        2 => "Markdown",
        _ => "Full",
    }
}

#[component]
pub fn editor_section() -> NodeHandle {
    let state = use_context::<EditorSectionState>();

    let state = match state {
        Some(s) => s,
        None => {
            return rsx! {
                div { "Error: EditorSectionState not initialized" }
            };
        }
    };

    let toolbar_preset = state.toolbar_preset;
    let editor = state.editor.clone();

    // Version signal bumped after every editor change for reactive updates
    let version = use_signal(|| 0u64);

    // on_change callback: bumps version signal to trigger reactive toolbar/status updates.
    // The bridge's internal reconcile callback handles DOM reconciliation before calling this.
    let on_change: Rc<dyn Fn()> = Rc::new(move || {
        version.update(|v| *v += 1);
    });

    // ContentEditable div - the editor surface
    let ce_div = __scope.create_element("div");
    ce_div.set_attribute("class", "editor-content");
    ce_div.set_attribute("contenteditable", "true");
    ce_div.set_attribute(
        "style",
        "min-height: 300px; padding: 16px 24px; background: var(--rinch-color-body); \
         font-size: 16px; line-height: 1.6; color: var(--rinch-color-text); \
         outline: none; cursor: text;",
    );

    // Mount the bridge: installs keyboard/CE interceptors + performs initial DOM render
    let bridge = Rc::new(EditorBridge::mount(
        __scope,
        editor.clone(),
        ce_div.clone(),
        on_change.clone(),
    ));

    // Keep bridge alive for the component's lifetime
    let bridge_store = use_ref(|| None::<Rc<EditorBridge>>);
    *bridge_store.borrow_mut() = Some(bridge.clone());

    // Toolbar on_change: reconcile DOM after toolbar commands + bump version
    let toolbar_on_change: Rc<dyn Fn()> = {
        let bridge = bridge.clone();
        Rc::new(move || {
            bridge.reconcile();
            version.update(|v| *v += 1);
        })
    };

    // Render the working toolbar
    let toolbar_config = get_toolbar_config(toolbar_preset.get());
    let toolbar_node = render_toolbar(
        __scope,
        editor.clone(),
        &toolbar_config,
        toolbar_on_change,
    );

    // Build reactive status bar
    let status_div = __scope.create_element("div");
    {
        let status_handle = status_div.clone();
        let editor_for_status = editor.clone();
        let doc_weak = __scope.doc_weak();
        let container_id = status_div.node_id();
        let current_content: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let current_scope: Rc<RefCell<Option<CoreRenderScope>>> = Rc::new(RefCell::new(None));
        let prev_version: Rc<RefCell<Option<u64>>> = Rc::new(RefCell::new(None));

        let effect = Effect::new(move || {
            let v = version.get();
            if let Some(prev) = *prev_version.borrow()
                && v == prev
            {
                return;
            }
            *prev_version.borrow_mut() = Some(v);

            if let Some(old_scope) = current_scope.borrow_mut().take() {
                old_scope.dispose();
            }
            if let Some(old_content) = current_content.borrow_mut().take() {
                old_content.clear_animations();
                old_content.remove();
            }

            if let Some(doc) = doc_weak.upgrade() {
                let mut child_scope = CoreRenderScope::new(doc, container_id);
                let new_status = render_status_bar(&mut child_scope, &editor_for_status);
                status_handle.append_child(&new_status);
                *current_content.borrow_mut() = Some(new_status);
                *current_scope.borrow_mut() = Some(child_scope);
            }
        });
        __scope.create_effect_from(effect);
    }

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Rich Text Editor" }
                Text { size: "lg", color: "dimmed",
                    "Working rich-text editor with toolbar, keyboard shortcuts, and content rendering."
                }
            }
            Space { h: "xl" }

            // Preset selector
            Group { gap: "sm",
                Button {
                    variant: "filled",
                    onclick: move || toolbar_preset.set(0),
                    "Full"
                }
                Button {
                    variant: "light",
                    onclick: move || toolbar_preset.set(1),
                    "Minimal"
                }
                Button {
                    variant: "light",
                    onclick: move || toolbar_preset.set(2),
                    "Markdown"
                }
                Text { size: "sm", color: "dimmed",
                    "Active: "
                    {|| preset_name(toolbar_preset.get()).to_string()}
                }
            }
            Space { h: "md" }

            // Editor container
            Paper { p: "0", radius: "md", with_border: true,
                // Toolbar
                {toolbar_node}

                // Content area (contentEditable, managed by bridge)
                {ce_div}

                // Status bar (reactive)
                {status_div}
            }

            Space { h: "xl" }

            // Editor CSS
            div {
                style: "display: none;",
                {render_editor_styles(__scope)}
            }

            // Keyboard Shortcuts Reference
            Title { order: 3, "Keyboard Shortcuts" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Controls with keyboard shortcuts from the current toolbar preset." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                div {
                    {render_shortcuts_list(__scope, toolbar_preset)}
                }
            }

            Space { h: "xl" }

            // Configuration info
            Title { order: 3, "Configuration" }
            Space { h: "sm" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Text { size: "sm",
                        "The toolbar is configured using the builder API. Choose from built-in presets or create a custom configuration with ToolbarConfig."
                    }
                    Space { h: "xs" }

                    SimpleGrid { cols: Some(3), spacing: "md",
                        Paper { p: "md", radius: "sm", with_border: true,
                            Stack { gap: "xs",
                                Text { weight: "600", "Full" }
                                Text { size: "sm", color: "dimmed",
                                    {format!("{} controls in {} groups",
                                        ToolbarConfig::default_full().control_count(),
                                        ToolbarConfig::default_full().groups.len())}
                                }
                            }
                        }
                        Paper { p: "md", radius: "sm", with_border: true,
                            Stack { gap: "xs",
                                Text { weight: "600", "Minimal" }
                                Text { size: "sm", color: "dimmed",
                                    {format!("{} controls in {} groups",
                                        ToolbarConfig::default_minimal().control_count(),
                                        ToolbarConfig::default_minimal().groups.len())}
                                }
                            }
                        }
                        Paper { p: "md", radius: "sm", with_border: true,
                            Stack { gap: "xs",
                                Text { weight: "600", "Markdown" }
                                Text { size: "sm", color: "dimmed",
                                    {format!("{} controls in {} groups",
                                        ToolbarConfig::default_markdown().control_count(),
                                        ToolbarConfig::default_markdown().groups.len())}
                                }
                            }
                        }
                    }

                    Space { h: "xs" }
                    Text { size: "sm", color: "dimmed",
                        "Current preset: "
                        {|| {
                            let config = get_toolbar_config(toolbar_preset.get());
                            format!("{} controls across {} groups", config.control_count(), config.groups.len())
                        }}
                    }
                }
            }
        }
    }
}

/// Render editor-specific CSS styles.
#[component]
fn render_editor_styles() -> NodeHandle {
    let style = __scope.create_element("style");
    let css = r#"
        .editor-content p { margin: 0 0 8px 0; }
        .editor-content h1 { font-size: 2em; font-weight: 700; margin: 16px 0 8px 0; }
        .editor-content h2 { font-size: 1.5em; font-weight: 700; margin: 14px 0 6px 0; }
        .editor-content h3 { font-size: 1.25em; font-weight: 600; margin: 12px 0 6px 0; }
        .editor-content h4 { font-size: 1.1em; font-weight: 600; margin: 10px 0 4px 0; }
        .editor-content h5 { font-size: 1em; font-weight: 600; margin: 8px 0 4px 0; }
        .editor-content h6 { font-size: 0.9em; font-weight: 600; margin: 8px 0 4px 0; }
        .editor-content blockquote {
            border-left: 3px solid var(--rinch-color-gray-4);
            padding-left: 16px; margin: 8px 0;
            color: var(--rinch-color-dimmed);
        }
        .editor-content pre {
            background: var(--rinch-color-gray-1);
            border-radius: var(--rinch-radius-sm);
            padding: 12px; margin: 8px 0;
            font-family: monospace; font-size: 14px;
            overflow-x: auto;
        }
        .editor-content code {
            background: var(--rinch-color-gray-1);
            padding: 2px 4px; border-radius: 3px;
            font-size: 0.9em;
        }
        .editor-content pre code {
            background: none; padding: 0; border-radius: 0;
        }
        .editor-content ul, .editor-content ol {
            margin: 8px 0; padding-left: 24px;
        }
        .editor-content li { margin: 2px 0; }
        .editor-content hr {
            border: none; border-top: 1px solid var(--rinch-color-gray-3);
            margin: 16px 0;
        }
        .editor-content mark {
            background: var(--rinch-color-yellow-2);
            padding: 1px 2px; border-radius: 2px;
        }
        .editor-content a {
            color: var(--rinch-primary-color);
            text-decoration: underline;
        }
        .editor-content strong { font-weight: 700; }
        .editor-content em { font-style: italic; }
        .editor-content u { text-decoration: underline; }
        .editor-content s { text-decoration: line-through; }
        .editor-content sub { vertical-align: sub; font-size: smaller; }
        .editor-content sup { vertical-align: super; font-size: smaller; }
        .editor-document { min-height: 100px; }
        .editor-toolbar div:hover {
            background: var(--rinch-color-gray-1);
        }
    "#;

    style.set_text(css);
    style
}

/// Renders a list of controls that have keyboard shortcuts.
#[component]
fn render_shortcuts_list(preset: Signal<usize>) -> NodeHandle {
    let config = get_toolbar_config(preset.get());

    let container = __scope.create_element("div");
    container.set_attribute("style", "display: flex; flex-direction: column; gap: 6px;");

    for group in &config.groups {
        for control in &group.controls {
            let btn_meta = ControlButton::from_control(control.clone());
            if let Some(shortcut) = btn_meta.shortcut_hint() {
                let row = __scope.create_element("div");
                row.set_attribute(
                    "style",
                    "display: flex; justify-content: space-between; align-items: center; \
                     padding: 4px 8px; border-radius: 4px; \
                     background: var(--rinch-color-gray-0);",
                );

                let label = __scope.create_element("span");
                label.set_attribute("style", "font-size: 13px;");
                label.set_text(btn_meta.tooltip());
                row.append_child(&label);

                let badge = __scope.create_element("span");
                badge.set_attribute(
                    "style",
                    "font-size: 11px; font-family: monospace; \
                     padding: 2px 8px; border-radius: 4px; \
                     background: var(--rinch-color-gray-2); \
                     color: var(--rinch-color-text);",
                );
                badge.set_text(shortcut);
                row.append_child(&badge);

                container.append_child(&row);
            }
        }
    }

    container
}
