pub fn styles() -> String {
    r#"
/* Textarea base */
.rinch-textarea {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.rinch-textarea__label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
}

.rinch-textarea__input {
    font-family: var(--rinch-font-family);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    padding: var(--rinch-spacing-sm);
    resize: vertical;
    min-height: 80px;
    transition: border-color 150ms ease;
}

.rinch-textarea__input:focus {
    outline: none;
    border-color: var(--rinch-primary-color);
}

.rinch-textarea__input::placeholder {
    color: var(--rinch-color-placeholder);
}

.rinch-textarea__input:disabled {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
    resize: none;
}

.rinch-textarea__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

.rinch-textarea__error {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-red-6);
}

.rinch-textarea--error .rinch-textarea__input {
    border-color: var(--rinch-color-red-6);
}

/* Textarea sizes */
.rinch-textarea--xs .rinch-textarea__input { min-height: 60px; font-size: var(--rinch-font-size-xs); }
.rinch-textarea--sm .rinch-textarea__input { min-height: 70px; }
.rinch-textarea--md .rinch-textarea__input { min-height: 80px; }
.rinch-textarea--lg .rinch-textarea__input { min-height: 100px; font-size: var(--rinch-font-size-md); }
.rinch-textarea--xl .rinch-textarea__input { min-height: 120px; font-size: var(--rinch-font-size-md); }

/* Auto resize (no resize handle) */
.rinch-textarea--autosize .rinch-textarea__input {
    resize: none;
}
"#.to_string()
}
