pub fn styles() -> String {
    r#"
/* TextInput base */
.rinch-text-input {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.rinch-text-input__label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
}

.rinch-text-input__input {
    font-family: var(--rinch-font-family);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    transition: border-color 150ms ease;
    min-width: 10rem;
}

.rinch-text-input__input:focus {
    outline: none;
    border-color: var(--rinch-primary-color);
}

.rinch-text-input__input::placeholder {
    color: var(--rinch-color-placeholder);
}

.rinch-text-input__input:disabled {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
}

/* TextInput sizes */
.rinch-text-input--xs .rinch-text-input__input {
    height: 1.875rem;
    padding: 0 0.75rem;
}

.rinch-text-input--sm .rinch-text-input__input {
    height: 2.25rem;
    padding: 0 0.875rem;
}

.rinch-text-input--md .rinch-text-input__input {
    height: 2.625rem;
    padding: 0 1rem;
}

.rinch-text-input--lg .rinch-text-input__input {
    height: 3.125rem;
    padding: 0 1.125rem;
}

.rinch-text-input--xl .rinch-text-input__input {
    height: 3.75rem;
    padding: 0 1.25rem;
}

/* Description and error */
.rinch-text-input__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

.rinch-text-input__error {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-red-6);
}

.rinch-text-input--error .rinch-text-input__input {
    border-color: var(--rinch-color-red-6);
}
"#.to_string()
}
