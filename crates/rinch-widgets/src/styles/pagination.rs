pub fn styles() -> String {
    r#"
/* Pagination base */
.rinch-pagination {
    display: inline-flex;
    align-items: center;
}

.rinch-pagination__controls {
    display: flex;
    align-items: center;
    gap: 0.25rem;
}

/* Pagination items container */
.rinch-pagination__items {
    display: flex;
    align-items: center;
    gap: 0.25rem;
}

/* Pagination item (page number) */
.rinch-pagination__item {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 2rem;
    height: 2rem;
    padding: 0 0.5rem;
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
    background-color: transparent;
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    cursor: pointer;
    transition: background-color 150ms ease, border-color 150ms ease;
}

.rinch-pagination__item:hover:not(:disabled):not(.rinch-pagination__item--active) {
    background-color: var(--rinch-color-filled);
}

.rinch-pagination__item--active {
    background-color: var(--rinch-pagination-color, var(--rinch-primary-color));
    border-color: var(--rinch-pagination-color, var(--rinch-primary-color));
    color: white;
}

.rinch-pagination__item:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

/* Pagination control (prev/next/first/last) */
.rinch-pagination__control {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    background-color: transparent;
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    cursor: pointer;
    transition: background-color 150ms ease;
}

.rinch-pagination__control:hover:not(:disabled) {
    background-color: var(--rinch-color-filled);
}

.rinch-pagination__control:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.rinch-pagination__control svg {
    width: 1rem;
    height: 1rem;
    color: var(--rinch-color-dimmed);
}

/* Pagination dots */
.rinch-pagination__dots {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 2rem;
    height: 2rem;
    color: var(--rinch-color-dimmed);
    font-size: var(--rinch-font-size-sm);
}

/* Size variants */
.rinch-pagination--xs .rinch-pagination__item,
.rinch-pagination--xs .rinch-pagination__control {
    min-width: 1.5rem;
    height: 1.5rem;
    font-size: var(--rinch-font-size-xs);
}
.rinch-pagination--xs .rinch-pagination__control svg {
    width: 0.75rem;
    height: 0.75rem;
}

.rinch-pagination--sm .rinch-pagination__item,
.rinch-pagination--sm .rinch-pagination__control {
    min-width: 1.75rem;
    height: 1.75rem;
    font-size: var(--rinch-font-size-xs);
}
.rinch-pagination--sm .rinch-pagination__control svg {
    width: 0.875rem;
    height: 0.875rem;
}

.rinch-pagination--lg .rinch-pagination__item,
.rinch-pagination--lg .rinch-pagination__control {
    min-width: 2.25rem;
    height: 2.25rem;
    font-size: var(--rinch-font-size-md);
}
.rinch-pagination--lg .rinch-pagination__control svg {
    width: 1.125rem;
    height: 1.125rem;
}

.rinch-pagination--xl .rinch-pagination__item,
.rinch-pagination--xl .rinch-pagination__control {
    min-width: 2.5rem;
    height: 2.5rem;
    font-size: var(--rinch-font-size-md);
}
.rinch-pagination--xl .rinch-pagination__control svg {
    width: 1.25rem;
    height: 1.25rem;
}

/* Radius variants */
.rinch-pagination--radius-xs .rinch-pagination__item,
.rinch-pagination--radius-xs .rinch-pagination__control { border-radius: var(--rinch-radius-xs); }
.rinch-pagination--radius-sm .rinch-pagination__item,
.rinch-pagination--radius-sm .rinch-pagination__control { border-radius: var(--rinch-radius-sm); }
.rinch-pagination--radius-md .rinch-pagination__item,
.rinch-pagination--radius-md .rinch-pagination__control { border-radius: var(--rinch-radius-md); }
.rinch-pagination--radius-lg .rinch-pagination__item,
.rinch-pagination--radius-lg .rinch-pagination__control { border-radius: var(--rinch-radius-lg); }
.rinch-pagination--radius-xl .rinch-pagination__item,
.rinch-pagination--radius-xl .rinch-pagination__control { border-radius: var(--rinch-radius-xl); }

/* Disabled state */
.rinch-pagination--disabled {
    opacity: 0.6;
    pointer-events: none;
}
"#
    .to_string()
}
