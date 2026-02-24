pub fn styles() -> String {
    r#"
/* BorderlessWindow - Container for borderless/transparent windows */
.rinch-borderlesswindow {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    background-color: var(--rinch-color-body);
    overflow: hidden;
}

/* Radius variants — only top corners (desktop windows have flat bottom edge) */
.rinch-borderlesswindow--radius-none {
    border-radius: 0;
}

.rinch-borderlesswindow--radius-xs {
    border-top-left-radius: 4px;
    border-top-right-radius: 4px;
}

.rinch-borderlesswindow--radius-sm {
    border-top-left-radius: 6px;
    border-top-right-radius: 6px;
}

.rinch-borderlesswindow--radius-md {
    border-top-left-radius: 8px;
    border-top-right-radius: 8px;
}

.rinch-borderlesswindow--radius-lg {
    border-top-left-radius: 12px;
    border-top-right-radius: 12px;
}

.rinch-borderlesswindow--radius-xl {
    border-top-left-radius: 16px;
    border-top-right-radius: 16px;
}

/* Titlebar - draggable area */
.rinch-borderlesswindow__titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 36px;
    min-height: 36px;
    padding: 0 var(--rinch-spacing-xs);
    background-color: var(--rinch-titlebar-bg);
    border-bottom: none;
    flex-shrink: 0;
    /* Make titlebar draggable for window movement */
    -webkit-app-region: drag;
    app-region: drag;
    user-select: none;
    /* Pass icon color to children */
    color: var(--rinch-titlebar-icon);
}

/* Left section (menu button, etc.) */
.rinch-borderlesswindow__left {
    display: flex;
    align-items: center;
    gap: var(--rinch-spacing-xs);
    -webkit-app-region: no-drag;
    app-region: no-drag;
}

/* Title text */
.rinch-borderlesswindow__title {
    flex: 1;
    text-align: center;
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-titlebar-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 var(--rinch-spacing-md);
}

/* Right section (custom controls before window buttons) */
.rinch-borderlesswindow__right {
    display: flex;
    align-items: center;
    gap: var(--rinch-spacing-xs);
    -webkit-app-region: no-drag;
    app-region: no-drag;
}

/* ActionIcons in titlebar sections use titlebar colors via CSS variable override */
.rinch-borderlesswindow__left,
.rinch-borderlesswindow__right {
    --rinch-action-icon-color: var(--rinch-titlebar-icon);
}

/* Override ActionIcon hover backgrounds in titlebar */
.rinch-borderlesswindow__left .rinch-action-icon--subtle:hover,
.rinch-borderlesswindow__left .rinch-action-icon--transparent:hover,
.rinch-borderlesswindow__right .rinch-action-icon--subtle:hover,
.rinch-borderlesswindow__right .rinch-action-icon--transparent:hover {
    background-color: var(--rinch-titlebar-hover);
}

/* Window controls container */
.rinch-borderlesswindow__controls {
    display: flex;
    align-items: center;
    gap: 0;
    -webkit-app-region: no-drag;
    app-region: no-drag;
}

/* Individual window control button */
.rinch-borderlesswindow__control {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    height: 36px;
    border: none;
    background: transparent;
    color: var(--rinch-titlebar-icon);
    cursor: pointer;
    transition: background-color 150ms ease;
}

.rinch-borderlesswindow__control:hover {
    background-color: var(--rinch-titlebar-hover);
}

.rinch-borderlesswindow__control:active {
    background-color: var(--rinch-titlebar-active);
}

/* Close button special styling */
.rinch-borderlesswindow__control--close:hover {
    background-color: var(--rinch-titlebar-hover);
}

.rinch-borderlesswindow__control--close:active {
    background-color: var(--rinch-titlebar-active);
}

/* SVG icons in controls */
.rinch-borderlesswindow__control svg {
    width: 10px;
    height: 10px;
}

/* Content area */
.rinch-borderlesswindow__content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
}

/* When window has rounded corners, clip content appropriately */
.rinch-borderlesswindow--radius-xs .rinch-borderlesswindow__content,
.rinch-borderlesswindow--radius-sm .rinch-borderlesswindow__content,
.rinch-borderlesswindow--radius-md .rinch-borderlesswindow__content,
.rinch-borderlesswindow--radius-lg .rinch-borderlesswindow__content,
.rinch-borderlesswindow--radius-xl .rinch-borderlesswindow__content {
    /* Ensure content doesn't overflow rounded corners at bottom */
}

/* Rounded corners on titlebar when window has radius */
.rinch-borderlesswindow--radius-xs .rinch-borderlesswindow__titlebar {
    border-top-left-radius: 4px;
    border-top-right-radius: 4px;
}

.rinch-borderlesswindow--radius-sm .rinch-borderlesswindow__titlebar {
    border-top-left-radius: 6px;
    border-top-right-radius: 6px;
}

.rinch-borderlesswindow--radius-md .rinch-borderlesswindow__titlebar {
    border-top-left-radius: 8px;
    border-top-right-radius: 8px;
}

.rinch-borderlesswindow--radius-lg .rinch-borderlesswindow__titlebar {
    border-top-left-radius: 12px;
    border-top-right-radius: 12px;
}

.rinch-borderlesswindow--radius-xl .rinch-borderlesswindow__titlebar {
    border-top-left-radius: 16px;
    border-top-right-radius: 16px;
}

/* Close button at right edge needs rounded corner when window has radius */
.rinch-borderlesswindow--radius-xs .rinch-borderlesswindow__control--close {
    border-top-right-radius: 4px;
}

.rinch-borderlesswindow--radius-sm .rinch-borderlesswindow__control--close {
    border-top-right-radius: 6px;
}

.rinch-borderlesswindow--radius-md .rinch-borderlesswindow__control--close {
    border-top-right-radius: 8px;
}

.rinch-borderlesswindow--radius-lg .rinch-borderlesswindow__control--close {
    border-top-right-radius: 12px;
}

.rinch-borderlesswindow--radius-xl .rinch-borderlesswindow__control--close {
    border-top-right-radius: 16px;
}
"#
    .to_string()
}
