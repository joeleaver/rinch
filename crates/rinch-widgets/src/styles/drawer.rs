pub fn styles() -> String {
    r#"
/* Drawer root */
.rinch-drawer__root {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 200;
}

/* Hidden state - use class instead of inline style for reliability */
.rinch-drawer__root--hidden {
    display: none !important;
}

/* Drawer overlay */
.rinch-drawer__overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: rgba(0, 0, 0, var(--rinch-drawer-overlay-opacity, 0.75));
}

/* Drawer container */
.rinch-drawer {
    position: absolute;
    background-color: var(--rinch-color-body);
    box-shadow: var(--rinch-shadow-xl);
    display: flex;
    flex-direction: column;
    z-index: 201;
    transition: transform 300ms ease;
    overflow: hidden;
    /* Use absolute within the fixed root */
    top: 0;
    bottom: 0;
}

/* Position: Left */
.rinch-drawer--left {
    left: 0;
    transform: translateX(-100%);
}

.rinch-drawer--left.rinch-drawer--opened {
    transform: translateX(0);
}

/* Position: Right */
.rinch-drawer--right {
    right: 0;
    left: auto;
    transform: translateX(100%);
}

.rinch-drawer--right.rinch-drawer--opened {
    transform: translateX(0);
}

/* Position: Top */
.rinch-drawer--top {
    top: 0;
    bottom: auto;
    left: 0;
    right: 0;
    transform: translateY(-100%);
}

.rinch-drawer--top.rinch-drawer--opened {
    transform: translateY(0);
}

/* Position: Bottom */
.rinch-drawer--bottom {
    bottom: 0;
    top: auto;
    left: 0;
    right: 0;
    transform: translateY(100%);
}

.rinch-drawer--bottom.rinch-drawer--opened {
    transform: translateY(0);
}

/* Drawer sizes (for left/right) */
.rinch-drawer--left.rinch-drawer--xs,
.rinch-drawer--right.rinch-drawer--xs { width: 280px; }

.rinch-drawer--left.rinch-drawer--sm,
.rinch-drawer--right.rinch-drawer--sm { width: 320px; }

.rinch-drawer--left.rinch-drawer--md,
.rinch-drawer--right.rinch-drawer--md { width: 380px; }

.rinch-drawer--left.rinch-drawer--lg,
.rinch-drawer--right.rinch-drawer--lg { width: 500px; }

.rinch-drawer--left.rinch-drawer--xl,
.rinch-drawer--right.rinch-drawer--xl { width: 680px; }

.rinch-drawer--left.rinch-drawer--full,
.rinch-drawer--right.rinch-drawer--full { width: 100%; }

/* Drawer sizes (for top/bottom) */
.rinch-drawer--top.rinch-drawer--xs,
.rinch-drawer--bottom.rinch-drawer--xs { height: 200px; }

.rinch-drawer--top.rinch-drawer--sm,
.rinch-drawer--bottom.rinch-drawer--sm { height: 280px; }

.rinch-drawer--top.rinch-drawer--md,
.rinch-drawer--bottom.rinch-drawer--md { height: 380px; }

.rinch-drawer--top.rinch-drawer--lg,
.rinch-drawer--bottom.rinch-drawer--lg { height: 500px; }

.rinch-drawer--top.rinch-drawer--xl,
.rinch-drawer--bottom.rinch-drawer--xl { height: 680px; }

.rinch-drawer--top.rinch-drawer--full,
.rinch-drawer--bottom.rinch-drawer--full { height: 100%; }

/* Drawer header */
.rinch-drawer__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--rinch-spacing-sm);
    padding: var(--rinch-spacing-md) var(--rinch-spacing-lg);
    border-bottom: 1px solid var(--rinch-color-border);
    flex-shrink: 0;
}

.rinch-drawer__title {
    margin: 0;
    flex: 1;
    font-size: var(--rinch-font-size-lg);
    font-weight: 600;
    color: var(--rinch-color-text);
}

.rinch-drawer__spacer {
    flex: 1;
}

/* Drawer body */
.rinch-drawer__body {
    flex: 1;
    padding: var(--rinch-spacing-lg);
    overflow-y: auto;
}

/* Drawer close button */
.rinch-drawer__close {
    flex-shrink: 0;
    width: 2rem;
    height: 2rem;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    color: var(--rinch-color-dimmed);
    transition: background-color 150ms ease, color 150ms ease;
}

.rinch-drawer__close:hover {
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
}

.rinch-drawer__close svg {
    width: 1rem;
    height: 1rem;
}
"#
    .to_string()
}
