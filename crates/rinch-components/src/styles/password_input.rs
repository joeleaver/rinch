pub fn styles() -> String {
    r#"
/* PasswordInput base */
.rinch-password-input {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.rinch-password-input__label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
}

.rinch-password-input__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

.rinch-password-input__wrapper {
    display: flex;
    align-items: center;
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    height: 2.625rem;
    overflow: hidden;
    transition: border-color 150ms ease;
    min-width: 10rem;
}

.rinch-password-input__wrapper:focus-within {
    border-color: var(--rinch-primary-color);
}

.rinch-password-input__input {
    flex: 1;
    min-width: 0;
    font-family: var(--rinch-font-family);
    font-size: var(--rinch-font-size-sm);
    background-color: transparent;
    border: none;
    padding: 0 var(--rinch-spacing-sm);
    height: 100%;
    color: var(--rinch-color-text);
}

.rinch-password-input__input:focus {
    outline: none;
}

.rinch-password-input__input::placeholder {
    color: var(--rinch-color-placeholder);
}

.rinch-password-input__input:disabled {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
}

/* Visibility toggle */
.rinch-password-input__toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 2rem;
    height: 1.75rem;
    margin-right: 0.25rem;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--rinch-color-dimmed);
    border-radius: var(--rinch-radius-sm);
}

.rinch-password-input__toggle:hover {
    background-color: var(--rinch-color-filled);
    color: var(--rinch-color-text);
}

.rinch-password-input__toggle svg {
    width: 1.125rem;
    height: 1.125rem;
}

.rinch-password-input__error {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-red-6);
}

.rinch-password-input--error .rinch-password-input__wrapper {
    border-color: var(--rinch-color-red-6);
}

/* PasswordInput sizes */
.rinch-password-input--xs .rinch-password-input__wrapper { height: 1.875rem; }
.rinch-password-input--xs .rinch-password-input__input { font-size: var(--rinch-font-size-xs); }
.rinch-password-input--xs .rinch-password-input__toggle { width: 1.5rem; height: 1.5rem; }

.rinch-password-input--sm .rinch-password-input__wrapper { height: 2.25rem; }

.rinch-password-input--lg .rinch-password-input__wrapper { height: 3.125rem; }
.rinch-password-input--lg .rinch-password-input__input { font-size: var(--rinch-font-size-md); }
.rinch-password-input--lg .rinch-password-input__toggle { width: 2rem; height: 2rem; }

.rinch-password-input--xl .rinch-password-input__wrapper { height: 3.75rem; }
.rinch-password-input--xl .rinch-password-input__input { font-size: var(--rinch-font-size-lg); }
.rinch-password-input--xl .rinch-password-input__toggle { width: 2.25rem; height: 2.25rem; }

/* Disabled state on wrapper */
.rinch-password-input--disabled .rinch-password-input__wrapper {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
}
"#.to_string()
}
