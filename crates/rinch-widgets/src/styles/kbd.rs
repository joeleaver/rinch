pub fn styles() -> String {
    r#"
/* Kbd base */
.rinch-kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--rinch-font-family-monospace);
    font-size: var(--rinch-font-size-xs);
    font-weight: 700;
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
    border: 1px solid var(--rinch-color-border);
    border-bottom-width: 3px;
    border-radius: var(--rinch-radius-xs);
    padding: 0.125rem 0.5rem;
    min-width: 1.5rem;
    text-align: center;
}

/* Kbd sizes */
.rinch-kbd--xs {
    font-size: 0.625rem;
    padding: 0.0625rem 0.375rem;
    min-width: 1.25rem;
}

.rinch-kbd--sm {
    font-size: var(--rinch-font-size-xs);
    padding: 0.125rem 0.4375rem;
    min-width: 1.375rem;
}

.rinch-kbd--md {
    font-size: var(--rinch-font-size-sm);
    padding: 0.1875rem 0.5rem;
    min-width: 1.625rem;
}

.rinch-kbd--lg {
    font-size: var(--rinch-font-size-md);
    padding: 0.25rem 0.625rem;
    min-width: 2rem;
}
"#.to_string()
}
