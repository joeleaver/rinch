pub fn styles() -> String {
    r#"
/* Select wrapper */
.rinch-select {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.rinch-select__label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
}

/* Select input */
.rinch-select__input {
    font-family: var(--rinch-font-family);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    padding: 0 2rem 0 var(--rinch-spacing-sm);
    height: 2.625rem;
    cursor: pointer;
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 24 24' fill='none' stroke='%23868e96' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='6,9 12,15 18,9'%3E%3C/polyline%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 0.75rem center;
    transition: border-color 150ms ease;
}

.rinch-select__input:focus {
    outline: none;
    border-color: var(--rinch-primary-color);
}

.rinch-select__input:disabled {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
}

.rinch-select__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

.rinch-select__error {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-red-6);
}

.rinch-select--error .rinch-select__input {
    border-color: var(--rinch-color-red-6);
}

/* Select sizes */
.rinch-select--xs .rinch-select__input { height: 1.875rem; font-size: var(--rinch-font-size-xs); }
.rinch-select--sm .rinch-select__input { height: 2.25rem; }
.rinch-select--md .rinch-select__input { height: 2.625rem; }
.rinch-select--lg .rinch-select__input { height: 3.125rem; font-size: var(--rinch-font-size-md); }
.rinch-select--xl .rinch-select__input { height: 3.75rem; font-size: var(--rinch-font-size-lg); }
"#.to_string()
}
