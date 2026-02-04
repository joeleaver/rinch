pub fn styles() -> String {
    r#"
/* Code inline */
.rinch-code {
    font-family: var(--rinch-font-family-monospace);
    font-size: var(--rinch-font-size-sm);
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
    padding: 0.125rem 0.375rem;
    border-radius: var(--rinch-radius-xs);
}

/* Code block */
.rinch-code--block {
    display: block;
    padding: var(--rinch-spacing-md);
    border-radius: var(--rinch-radius-default);
    overflow-x: auto;
    white-space: pre;
}

/* Code colors */
.rinch-code--primary {
    background-color: var(--rinch-primary-color-0);
    color: var(--rinch-primary-color-7);
}

/* Code sizes */
.rinch-code--xs { font-size: var(--rinch-font-size-xs); }
.rinch-code--sm { font-size: var(--rinch-font-size-sm); }
.rinch-code--md { font-size: var(--rinch-font-size-md); }
.rinch-code--lg { font-size: var(--rinch-font-size-lg); }
"#.to_string()
}
