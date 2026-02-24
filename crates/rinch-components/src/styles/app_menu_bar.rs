pub fn styles() -> String {
    r#"
/* App menu bar wrapper - relative container for absolute bar */
.rinch-app-menu-bar-wrapper {
    position: relative;
    width: 100%;
    height: 100vh;
}

/* Menu bar row - positioned via inline style (top offset varies by window type) */
.rinch-app-menu-bar {
    display: flex;
    flex-direction: row;
    align-items: center;
    background: var(--rinch-color-body);
    border-bottom: 1px solid var(--rinch-color-border, var(--rinch-color-gray-3));
    padding: 0 var(--rinch-spacing-xs);
}

/* Top-level menu item container */
.rinch-app-menu-item {
    position: relative;
}

/* Label button */
.rinch-app-menu-item__label {
    padding: 4px 8px;
    font-size: var(--rinch-font-size-sm);
    cursor: pointer;
    border-radius: var(--rinch-radius-sm);
    color: var(--rinch-color-text);
    user-select: none;
}

.rinch-app-menu-item__label:hover {
    background: var(--rinch-color-default);
}

/* Dropdown panel (hidden by default) */
.rinch-app-menu-item__dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    min-width: 220px;
    background: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border, var(--rinch-color-gray-3));
    border-radius: var(--rinch-radius-md);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    padding: 4px;
    z-index: 200;
    opacity: 0;
    visibility: hidden;
}

/* Show dropdown for the opened menu (toggled by active_menu signal) */
.rinch-app-menu-item__dropdown--visible {
    opacity: 1;
    visibility: visible;
}

/* Highlight the label of the currently opened menu item */
.rinch-app-menu-item--opened > .rinch-app-menu-item__label {
    background: var(--rinch-color-default);
}

/* Menu entries */
.rinch-app-menu-entry {
    display: flex;
    align-items: center;
    gap: var(--rinch-spacing-sm);
    width: 100%;
    padding: 4px 10px;
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    cursor: pointer;
    border-radius: var(--rinch-radius-sm);
}

.rinch-app-menu-entry:hover {
    background: var(--rinch-color-default);
}

.rinch-app-menu-entry--disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.rinch-app-menu-entry__label {
    flex: 1;
}

.rinch-app-menu-entry__shortcut {
    margin-left: auto;
    color: var(--rinch-color-dimmed);
    font-size: var(--rinch-font-size-xs);
}

/* Separator */
.rinch-app-menu-separator {
    height: 1px;
    background: var(--rinch-color-border, var(--rinch-color-gray-3));
    margin: 4px 0;
}

/* Click-outside overlay */
.rinch-app-menu-bar__overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    z-index: 199;
    pointer-events: auto;
}

/* Submenu (nested) */
.rinch-app-menu-submenu {
    position: relative;
    cursor: pointer;
}

.rinch-app-menu-submenu__trigger {
    display: flex;
    align-items: center;
    width: 100%;
    padding: 4px 10px;
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    border-radius: var(--rinch-radius-sm);
}

.rinch-app-menu-submenu__trigger:hover {
    background: var(--rinch-color-default);
}

.rinch-app-menu-submenu__label {
    flex: 1;
}

.rinch-app-menu-submenu__arrow {
    margin-left: auto;
    color: var(--rinch-color-dimmed);
    font-size: var(--rinch-font-size-xs);
}

.rinch-app-menu-submenu__dropdown {
    position: absolute;
    left: 100%;
    top: 0;
    min-width: 200px;
    background: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border, var(--rinch-color-gray-3));
    border-radius: var(--rinch-radius-md);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    padding: 4px;
    z-index: 200;
    opacity: 0;
    visibility: hidden;
}

.rinch-app-menu-submenu:hover > .rinch-app-menu-submenu__dropdown {
    opacity: 1;
    visibility: visible;
}

/* Content padding-top is set via inline style (varies by window type) */

/* ── Inline titlebar layout (VS Code style) ───────────────────────────── */

/* Floating layer overlapping the titlebar — LAST child for hit testing */
.rinch-app-menu-bar__inline-layer {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 0;
    overflow: visible;
    pointer-events: none;
    z-index: 200;
}

/* Row of menu items inside the titlebar */
.rinch-app-menu-bar__inline-row {
    position: absolute;
    top: 0;
    left: 0;
    height: 36px;
    display: flex;
    align-items: center;
    padding: 0 var(--rinch-spacing-xs);
    gap: 2px;
    pointer-events: auto;
    z-index: 201;
}

/* ActionIcons in the inline row use titlebar colors */
.rinch-app-menu-bar__inline-row {
    --rinch-action-icon-color: var(--rinch-titlebar-icon);
    color: var(--rinch-titlebar-icon);
}

.rinch-app-menu-bar__inline-row .rinch-action-icon--subtle:hover,
.rinch-app-menu-bar__inline-row .rinch-action-icon--transparent:hover {
    background-color: var(--rinch-titlebar-hover);
}

/* Menu items container within the inline row */
.rinch-app-menu-bar__inline-items {
    display: flex;
    align-items: center;
    padding: 0;
}

/* Titlebar-themed colors for inline menu labels */
.rinch-app-menu-bar__inline-row .rinch-app-menu-item__label {
    color: var(--rinch-titlebar-text);
}

.rinch-app-menu-bar__inline-row .rinch-app-menu-item__label:hover,
.rinch-app-menu-bar__inline-row .rinch-app-menu-item--opened > .rinch-app-menu-item__label {
    background: var(--rinch-titlebar-hover);
}
"#
    .to_string()
}
