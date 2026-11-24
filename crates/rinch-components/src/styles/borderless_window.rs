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
    /* `--rinch-titlebar-height` is written by BorderlessWindow from its
       `titlebar_height` prop (issue #2196), which also feeds the top inset. */
    height: var(--rinch-titlebar-height, 36px);
    min-height: var(--rinch-titlebar-height, 36px);
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

/* Spacer that reserves titlebar space for inline menu items overlay.
   Its inline `width` is written by BorderlessWindow: the shell's estimate on
   the first frame, then the menu row's measured box (issue #529). The custom
   property is only the fallback for a spacer nobody sized. */
.rinch-borderlesswindow__menu-spacer {
    flex-shrink: 0;
    width: var(--rinch-menu-spacer-width, 0px);
}

/* The title in the inline menu row: plain text in the titlebar's text colour,
   as a client-side-decorated titlebar draws it (issue #2196; it used to be a
   rainbow pill). `title_section` puts the app's own nodes here instead. */
.rinch-borderlesswindow__brand {
    display: flex;
    align-items: center;
    font-size: var(--rinch-font-size-sm);
    font-weight: 600;
    white-space: nowrap;
    padding: 0 var(--rinch-spacing-xs);
    color: var(--rinch-titlebar-text);
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
    height: var(--rinch-titlebar-height, 36px);
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

/* The control at each outer edge of the titlebar follows the window's rounded
   corner: the last one on the right, the first one on the left (issue #2196:
   the controls can sit at either end). */
.rinch-borderlesswindow--radius-xs .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--right > .rinch-borderlesswindow__control:last-child {
    border-top-right-radius: 4px;
}

.rinch-borderlesswindow--radius-xs .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--left > .rinch-borderlesswindow__control:first-child {
    border-top-left-radius: 4px;
}

.rinch-borderlesswindow--radius-sm .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--right > .rinch-borderlesswindow__control:last-child {
    border-top-right-radius: 6px;
}

.rinch-borderlesswindow--radius-sm .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--left > .rinch-borderlesswindow__control:first-child {
    border-top-left-radius: 6px;
}

.rinch-borderlesswindow--radius-md .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--right > .rinch-borderlesswindow__control:last-child {
    border-top-right-radius: 8px;
}

.rinch-borderlesswindow--radius-md .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--left > .rinch-borderlesswindow__control:first-child {
    border-top-left-radius: 8px;
}

.rinch-borderlesswindow--radius-lg .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--right > .rinch-borderlesswindow__control:last-child {
    border-top-right-radius: 12px;
}

.rinch-borderlesswindow--radius-lg .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--left > .rinch-borderlesswindow__control:first-child {
    border-top-left-radius: 12px;
}

.rinch-borderlesswindow--radius-xl .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--right > .rinch-borderlesswindow__control:last-child {
    border-top-right-radius: 16px;
}

.rinch-borderlesswindow--radius-xl .rinch-borderlesswindow__titlebar > .rinch-borderlesswindow__controls--left > .rinch-borderlesswindow__control:first-child {
    border-top-left-radius: 16px;
}
"#
    .to_string()
}
