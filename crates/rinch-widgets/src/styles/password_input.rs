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
    position: relative;
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    height: 2.625rem;
    overflow: hidden;
    transition: border-color 150ms ease;
}

.rinch-password-input__wrapper:focus-within {
    border-color: var(--rinch-primary-color);
}

/* Mask layer - shows bullets, positioned behind the input */
.rinch-password-input__mask {
    position: absolute;
    top: 0;
    left: 0;
    right: 2.5rem; /* Leave space for toggle button */
    bottom: 0;
    display: flex;
    align-items: center;
    padding: 0 var(--rinch-spacing-sm);
    font-family: var(--rinch-font-family);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    letter-spacing: 0.15em;
    pointer-events: none;
    z-index: 0;
    overflow: hidden;
    white-space: nowrap;
}

/* Hide mask when password is visible */
.rinch-password-input--visible .rinch-password-input__mask {
    display: none;
}

/* Input element - always contains real password */
.rinch-password-input__input {
    position: relative;
    z-index: 1;
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

/* When masked: hide the text but keep the caret visible */
.rinch-password-input--masked .rinch-password-input__input {
    color: transparent;
    -webkit-text-fill-color: transparent;
    caret-color: var(--rinch-color-text, #000);
}

/* When visible: show normal text */
.rinch-password-input--visible .rinch-password-input__input {
    color: var(--rinch-color-text);
    -webkit-text-fill-color: var(--rinch-color-text);
}

.rinch-password-input__input:focus {
    outline: none;
}

.rinch-password-input__input::placeholder {
    color: var(--rinch-color-placeholder);
}

/* Placeholder should be visible even when text is transparent */
.rinch-password-input:not(.rinch-password-input--visible) .rinch-password-input__input::placeholder {
    color: var(--rinch-color-placeholder);
}

.rinch-password-input__input:disabled {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
}

/* Visibility toggle - positioned inside wrapper using flex */
.rinch-password-input__toggle {
    position: relative;
    z-index: 2;
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
.rinch-password-input--xs .rinch-password-input__input,
.rinch-password-input--xs .rinch-password-input__mask { font-size: var(--rinch-font-size-xs); }
.rinch-password-input--xs .rinch-password-input__toggle { width: 1.5rem; height: 1.5rem; }
.rinch-password-input--xs .rinch-password-input__mask { right: 1.75rem; }

.rinch-password-input--sm .rinch-password-input__wrapper { height: 2.25rem; }
.rinch-password-input--sm .rinch-password-input__mask { right: 2.25rem; }

.rinch-password-input--lg .rinch-password-input__wrapper { height: 3.125rem; }
.rinch-password-input--lg .rinch-password-input__input,
.rinch-password-input--lg .rinch-password-input__mask { font-size: var(--rinch-font-size-md); }
.rinch-password-input--lg .rinch-password-input__toggle { width: 2rem; height: 2rem; }
.rinch-password-input--lg .rinch-password-input__mask { right: 2.5rem; }

.rinch-password-input--xl .rinch-password-input__wrapper { height: 3.75rem; }
.rinch-password-input--xl .rinch-password-input__input,
.rinch-password-input--xl .rinch-password-input__mask { font-size: var(--rinch-font-size-lg); }
.rinch-password-input--xl .rinch-password-input__toggle { width: 2.25rem; height: 2.25rem; }
.rinch-password-input--xl .rinch-password-input__mask { right: 2.75rem; }

/* Disabled state on wrapper */
.rinch-password-input--disabled .rinch-password-input__wrapper {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
}
"#.to_string()
}
