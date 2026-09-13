pub fn styles() -> String {
    r#"
/* NumberInput base */
.rinch-number-input {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.rinch-number-input__label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
}

.rinch-number-input__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

.rinch-number-input__wrapper {
    display: flex;
    align-items: stretch;
    position: relative;
    min-width: 10rem;
}

.rinch-number-input__input {
    flex: 1;
    font-family: var(--rinch-font-family);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    padding: 0 var(--rinch-spacing-sm);
    height: 2.625rem;
    transition: border-color 150ms ease;
}

.rinch-number-input__input:focus {
    outline: none;
    border-color: var(--rinch-primary-color);
}

.rinch-number-input__input:disabled {
    background-color: var(--rinch-color-filled);
    cursor: not-allowed;
}

/* Prefix and suffix */
.rinch-number-input__prefix,
.rinch-number-input__suffix {
    display: flex;
    align-items: center;
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-dimmed);
    background-color: var(--rinch-color-filled);
    border: 1px solid var(--rinch-color-border);
    padding: 0 var(--rinch-spacing-sm);
}

.rinch-number-input__prefix {
    border-right: none;
    border-radius: var(--rinch-radius-default) 0 0 var(--rinch-radius-default);
}

.rinch-number-input__suffix {
    border-left: none;
    border-radius: 0 var(--rinch-radius-default) var(--rinch-radius-default) 0;
}

.rinch-number-input__prefix + .rinch-number-input__input {
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
}

/* Controls */
.rinch-number-input__controls {
    position: absolute;
    right: 1px;
    top: 1px;
    bottom: 1px;
    display: flex;
    flex-direction: column;
    width: 1.75rem;
    border-left: 1px solid var(--rinch-color-border);
}

.rinch-number-input__control {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--rinch-color-filled);
    border: none;
    cursor: pointer;
    color: var(--rinch-color-dimmed);
    transition: background-color 150ms ease, color 150ms ease;
}

.rinch-number-input__control:hover {
    background: var(--rinch-color-filled-hover);
    color: var(--rinch-color-text);
}

.rinch-number-input__control--up {
    border-radius: 0 calc(var(--rinch-radius-default) - 1px) 0 0;
    border-bottom: 1px solid var(--rinch-color-border);
}

.rinch-number-input__control--down {
    border-radius: 0 0 calc(var(--rinch-radius-default) - 1px) 0;
}

.rinch-number-input__error {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-red-6);
}

.rinch-number-input--error .rinch-number-input__input {
    border-color: var(--rinch-color-red-6);
}

/* Hide controls */
.rinch-number-input--no-controls .rinch-number-input__input {
    padding-right: var(--rinch-spacing-sm);
}

/* NumberInput sizes */
.rinch-number-input--xs .rinch-number-input__input { height: 1.875rem; font-size: var(--rinch-font-size-xs); }
.rinch-number-input--xs .rinch-number-input__controls { width: 1.5rem; }
.rinch-number-input--sm .rinch-number-input__input { height: 2.25rem; }
.rinch-number-input--lg .rinch-number-input__input { height: 3.125rem; font-size: var(--rinch-font-size-md); }
.rinch-number-input--lg .rinch-number-input__controls { width: 2rem; }
.rinch-number-input--xl .rinch-number-input__input { height: 3.75rem; font-size: var(--rinch-font-size-lg); }
.rinch-number-input--xl .rinch-number-input__controls { width: 2.25rem; }

/* NumberInput radius (#474). The field, a prefix, a suffix and both stepper
   buttons each derive their corners from --rinch-radius-default — including two
   `calc(... - 1px)` insets — so rebinding that one variable on the container
   moves all five together and keeps every corner relationship intact. The
   component owns its whole subtree (it renders no children), so nothing of the
   caller's inherits the rebinding. */
.rinch-number-input--radius-xs { --rinch-radius-default: var(--rinch-radius-xs); }
.rinch-number-input--radius-sm { --rinch-radius-default: var(--rinch-radius-sm); }
.rinch-number-input--radius-md { --rinch-radius-default: var(--rinch-radius-md); }
.rinch-number-input--radius-lg { --rinch-radius-default: var(--rinch-radius-lg); }
.rinch-number-input--radius-xl { --rinch-radius-default: var(--rinch-radius-xl); }
"#.to_string()
}
