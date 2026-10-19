//! Tree component styles.

pub fn styles() -> String {
    r#"
/* Tree container */
.rinch-tree {
    --tree-level-offset: var(--rinch-spacing-md);
    /* Where a guide sits in its level: the middle of the chevron above it.
       A stylesheet that resizes the chevron moves this with it. */
    --tree-guide-offset: 0.625rem;
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: var(--rinch-font-size-sm);
}

/* Tree node */
.rinch-tree__node {
    list-style: none;
    margin: 0;
    padding: 0;
}

/* Node content wrapper */
.rinch-tree__node-content {
    display: flex;
    align-items: center;
    padding: var(--rinch-spacing-xs) var(--rinch-spacing-sm);
    padding-left: 0;
    cursor: pointer;
    border-radius: var(--rinch-radius-sm);
    user-select: none;
}

.rinch-tree__node-content:hover {
    background-color: var(--rinch-color-gray-1);
}

.rinch-tree__node-content--selected {
    background-color: var(--rinch-primary-color-1);
    color: var(--rinch-primary-color);
}

.rinch-tree__node-content--selected:hover {
    background-color: var(--rinch-primary-color-2);
}

.rinch-tree__node-content--disabled {
    opacity: 0.5;
    cursor: not-allowed;
    pointer-events: none;
}

/* Chevron icon */
.rinch-tree__chevron {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    margin-right: var(--rinch-spacing-xs);
    color: var(--rinch-color-dimmed);
    transition: transform 200ms ease;
    flex-shrink: 0;
}

.rinch-tree__chevron svg {
    width: 1rem;
    height: 1rem;
}

.rinch-tree__chevron--expanded {
    transform: rotate(90deg);
}

/* Spacer for leaf nodes (alignment) */
.rinch-tree__spacer {
    width: 1.25rem;
    height: 1.25rem;
    margin-right: var(--rinch-spacing-xs);
    flex-shrink: 0;
}

/* Node icon */
.rinch-tree__icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    margin-right: var(--rinch-spacing-xs);
    color: var(--rinch-color-dimmed);
    flex-shrink: 0;
}

.rinch-tree__icon svg {
    width: 1rem;
    height: 1rem;
}

/* Node label */
.rinch-tree__label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

/* Subtree container */
.rinch-tree__subtree {
    list-style: none;
    margin: 0;
    padding: 0;
}

/* Focus styles */
.rinch-tree__node-content:focus-visible {
    outline: 2px solid var(--rinch-primary-color);
    outline-offset: 2px;
}

/* Selected node icon color */
.rinch-tree__node-content--selected .rinch-tree__icon {
    color: var(--rinch-primary-color);
}

/* Drag and drop: the row under the pointer draws where the drop lands, a
   line before or after it at the indent of --tree-drop-level, or an outline
   for "inside". */
.rinch-tree__node-content {
    position: relative;
}
/* The dragged row's contents fade, not the row: a drop line drawn on the
   row itself (its own edge, when it is dragged out of its group) stays
   bright. */
.rinch-tree__node-content--dragging > * {
    opacity: 0.4;
}
.rinch-tree__node-content--drop-before::after,
.rinch-tree__node-content--drop-after::after,
.rinch-tree__node-content--drop-inside::after {
    content: "";
    position: absolute;
    left: calc(var(--tree-level-offset) * var(--tree-drop-level, 0));
    right: 0;
    pointer-events: none;
    z-index: 2;
}
.rinch-tree__node-content--drop-before::after,
.rinch-tree__node-content--drop-after::after {
    height: 2px;
    border-radius: 1px;
    background-color: var(--rinch-primary-color);
}
.rinch-tree__node-content--drop-before::after {
    top: -1px;
}
.rinch-tree__node-content--drop-after::after {
    bottom: -1px;
}
.rinch-tree__node-content--drop-inside::after {
    top: 0;
    bottom: 0;
    border: 2px solid var(--rinch-primary-color);
    border-radius: var(--rinch-radius-sm);
}
.rinch-tree__node-content--drop-refused::after {
    background-color: var(--rinch-color-red-6);
    border-color: var(--rinch-color-red-6);
}
.rinch-tree__node-content--drop-inside.rinch-tree__node-content--drop-refused::after {
    background-color: transparent;
}

/* Indent guides: one line down each expanded row's children */
.rinch-tree__subtree-wrap {
    position: relative;
}
.rinch-tree__guide {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background-color: var(--rinch-color-border);
    pointer-events: none;
}

/* Selected node chevron color */
.rinch-tree__node-content--selected .rinch-tree__chevron {
    color: var(--rinch-primary-color);
}
"#
    .to_string()
}
