//! CSS styles for the DevTools panel.

/// CSS for the DevTools window UI.
pub const DEVTOOLS_CSS: &str = r#"
body {
    margin: 0;
    padding: 0;
    background: #1e1e1e;
    color: #d4d4d4;
    font-family: "Consolas", "Monaco", "Menlo", monospace;
    font-size: 12px;
    overflow: hidden;
}

.devtools-root {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100vh;
}

/* ── Tab bar ─────────────────────────────────────────────────── */

.devtools-tabbar {
    display: flex;
    background: #252525;
    border-bottom: 1px solid #3c3c3c;
    flex-shrink: 0;
}

.devtools-tab {
    flex: 1;
    padding: 8px 12px;
    color: #808080;
    cursor: pointer;
    text-align: center;
    border-bottom: 2px solid transparent;
}

.devtools-tab:hover {
    color: #d4d4d4;
    background: #2a2a2a;
}

.devtools-tab--active {
    color: #d4d4d4;
    border-bottom: 2px solid #569cd6;
    background: #2a2a2a;
}

/* ── Toolbar ─────────────────────────────────────────────────── */

.devtools-toolbar {
    display: flex;
    padding: 4px 8px;
    background: #252525;
    border-bottom: 1px solid #3c3c3c;
    gap: 8px;
    flex-shrink: 0;
    align-items: center;
}

.devtools-btn {
    padding: 4px 8px;
    border: 1px solid #3c3c3c;
    border-radius: 3px;
    background: #2d2d2d;
    color: #d4d4d4;
    cursor: pointer;
    font-size: 11px;
}

.devtools-btn:hover {
    background: #3a3a3a;
}

.devtools-btn--active {
    background: #264f78;
    border-color: #569cd6;
    color: white;
}

/* ── Panel content ───────────────────────────────────────────── */

.devtools-panel {
    flex: 1;
    overflow: auto;
    padding: 8px;
}

/* ── Elements tree ───────────────────────────────────────────── */

.tree-row {
    padding: 2px 4px;
    cursor: pointer;
    white-space: nowrap;
    display: flex;
    align-items: center;
    min-height: 20px;
}

.tree-row:hover {
    background: #2a2d2e;
}

.tree-row--selected {
    background: #264f78;
}

.tree-chevron {
    width: 16px;
    display: inline-flex;
    justify-content: center;
    align-items: center;
    color: #808080;
    flex-shrink: 0;
    cursor: pointer;
    font-size: 10px;
}

.tree-tag {
    color: #569cd6;
}

.tree-id {
    color: #ce9178;
}

.tree-class {
    color: #9cdcfe;
}

.tree-text {
    color: #808080;
    font-style: italic;
    overflow: hidden;
    text-overflow: ellipsis;
}

.tree-layout {
    color: #4ec9b0;
    margin-left: 8px;
    font-size: 10px;
    opacity: 0.7;
}

/* ── Styles panel ────────────────────────────────────────────── */

.styles-empty {
    color: #808080;
    padding: 16px;
    text-align: center;
}

.styles-category {
    margin-bottom: 8px;
}

.styles-category-header {
    color: #dcdcaa;
    font-weight: bold;
    padding: 4px 0;
    border-bottom: 1px solid #3c3c3c;
    margin-bottom: 4px;
    cursor: pointer;
}

.styles-category-header:hover {
    color: #e8e8a8;
}

.styles-prop {
    display: flex;
    padding: 1px 0 1px 12px;
    gap: 4px;
}

.styles-prop-name {
    color: #9cdcfe;
}

.styles-prop-value {
    color: #ce9178;
}

/* ── Performance panel ───────────────────────────────────────── */

.perf-stat {
    display: flex;
    justify-content: space-between;
    padding: 4px 0;
    border-bottom: 1px solid #2a2a2a;
}

.perf-stat-label {
    color: #808080;
}

.perf-stat-value {
    color: #4ec9b0;
    font-weight: bold;
}

.perf-fps {
    font-size: 48px;
    font-weight: bold;
    color: #4ec9b0;
    text-align: center;
    padding: 16px 0;
}

.perf-bar-chart {
    display: flex;
    align-items: flex-end;
    height: 60px;
    gap: 1px;
    padding: 8px 0;
    border-bottom: 1px solid #3c3c3c;
}

.perf-bar {
    flex: 1;
    min-width: 2px;
    background: #4ec9b0;
}
"#;
