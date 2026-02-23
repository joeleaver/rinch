//! DevTools overlay rendering.
//!
//! Generates HTML for the devtools panel overlay.

use super::devtools::{DevToolsPanel, DevToolsState};

/// Render the devtools overlay as HTML.
///
/// This generates an HTML string that can be appended to the document
/// to display the devtools panel.
pub fn render_overlay(state: &DevToolsState) -> String {
    if !state.visible {
        return String::new();
    }

    let width = state.panel_width;
    let panel_content = match state.active_panel {
        DevToolsPanel::Elements => render_elements_panel(state),
        DevToolsPanel::Styles => render_styles_panel(state),
    };

    let elements_active = if state.active_panel == DevToolsPanel::Elements {
        "background: #2a2a2a;"
    } else {
        ""
    };
    let styles_active = if state.active_panel == DevToolsPanel::Styles {
        "background: #2a2a2a;"
    } else {
        ""
    };

    let inspect_style = if state.inspect_mode {
        "background: #4a90d9; color: white;"
    } else {
        ""
    };

    format!(
        r#"<div id="rinch-devtools" style="
            position: fixed;
            right: 0;
            top: 0;
            bottom: 0;
            width: {width}px;
            background: #1e1e1e;
            color: #d4d4d4;
            font-family: 'Consolas', 'Monaco', monospace;
            font-size: 12px;
            border-left: 1px solid #3c3c3c;
            display: flex;
            flex-direction: column;
            z-index: 999999;
        ">
            <div style="
                display: flex;
                background: #252525;
                border-bottom: 1px solid #3c3c3c;
            ">
                <button data-devtools-panel="elements" style="
                    flex: 1;
                    padding: 8px;
                    border: none;
                    color: #d4d4d4;
                    cursor: pointer;
                    {elements_active}
                ">Elements</button>
                <button data-devtools-panel="styles" style="
                    flex: 1;
                    padding: 8px;
                    border: none;
                    color: #d4d4d4;
                    cursor: pointer;
                    {styles_active}
                ">Styles</button>
            </div>
            <div style="
                padding: 4px 8px;
                background: #252525;
                border-bottom: 1px solid #3c3c3c;
                display: flex;
                gap: 8px;
            ">
                <button data-devtools-inspect style="
                    padding: 4px 8px;
                    border: 1px solid #3c3c3c;
                    border-radius: 3px;
                    background: #2d2d2d;
                    color: #d4d4d4;
                    cursor: pointer;
                    {inspect_style}
                ">🔍 Inspect</button>
            </div>
            <div style="
                flex: 1;
                overflow: auto;
                padding: 8px;
            ">
                {panel_content}
            </div>
        </div>"#
    )
}

/// Render the Elements panel showing the DOM tree.
fn render_elements_panel(state: &DevToolsState) -> String {
    let selected_info = if let Some(node_id) = state.selected_node {
        format!(
            r#"<div style="margin-bottom: 12px; padding: 8px; background: #2d2d2d; border-radius: 4px;">
                <div style="color: #569cd6;">Selected: Node #{}</div>
            </div>"#,
            node_id
        )
    } else {
        r#"<div style="margin-bottom: 12px; color: #808080;">
            Click an element to inspect it, or enable Inspect mode.
        </div>"#
            .to_string()
    };

    format!(
        r#"<div>
            <div style="font-weight: bold; margin-bottom: 8px; color: #dcdcaa;">DOM Tree</div>
            {selected_info}
            <div style="color: #808080;">
                Use Alt+D to toggle layout debug overlay<br>
                Use Alt+H to toggle hover highlight<br>
                Use Alt+T to print Taffy tree to console
            </div>
        </div>"#
    )
}

/// Render the Styles panel showing computed styles.
fn render_styles_panel(state: &DevToolsState) -> String {
    if let Some(node_id) = state.selected_node {
        format!(
            r#"<div>
                <div style="font-weight: bold; margin-bottom: 8px; color: #dcdcaa;">Computed Styles</div>
                <div style="color: #808080;">
                    Styles for Node #{node_id}<br>
                    (Full style inspection coming soon)
                </div>
            </div>"#
        )
    } else {
        r#"<div>
            <div style="font-weight: bold; margin-bottom: 8px; color: #dcdcaa;">Computed Styles</div>
            <div style="color: #808080;">Select an element to view its styles.</div>
        </div>"#
            .to_string()
    }
}

/// CSS styles for the devtools overlay.
/// These can be included in the document head for proper styling.
pub fn devtools_styles() -> &'static str {
    r#"
    #rinch-devtools button:hover {
        background: #3a3a3a !important;
    }
    #rinch-devtools::-webkit-scrollbar {
        width: 8px;
    }
    #rinch-devtools::-webkit-scrollbar-track {
        background: #1e1e1e;
    }
    #rinch-devtools::-webkit-scrollbar-thumb {
        background: #3c3c3c;
        border-radius: 4px;
    }
    "#
}
