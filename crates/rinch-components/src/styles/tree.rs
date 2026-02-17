//! Tree component styles.

pub fn styles() -> String {
    r#"
/* Tree container */
.rinch-tree {
    --tree-level-offset: var(--rinch-spacing-md);
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
    transition: background-color 150ms ease;
    user-select: none;
}

.rinch-tree__node-content:hover {
    background-color: var(--rinch-color-gray-1);
}

.rinch-tree__node-content--selected {
    background-color: var(--rinch-color-primary-1);
    color: var(--rinch-primary-color);
}

.rinch-tree__node-content--selected:hover {
    background-color: var(--rinch-color-primary-2);
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

/* Selected node chevron color */
.rinch-tree__node-content--selected .rinch-tree__chevron {
    color: var(--rinch-primary-color);
}
"#
    .to_string()
}
