pub fn styles() -> String {
    r#"
/* Button base */
.rinch-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--rinch-font-family);
    font-weight: 600;
    border: 1px solid transparent;
    cursor: pointer;
    text-decoration: none;
    white-space: nowrap;
    transition: background-color 150ms ease, border-color 150ms ease;
}

.rinch-button:disabled {
    cursor: not-allowed;
    opacity: 0.6;
}

/* Button sizes */
.rinch-button--xs {
    height: 1.875rem;
    padding: 0 0.875rem;
    font-size: var(--rinch-font-size-xs);
    border-radius: var(--rinch-radius-xs);
}

.rinch-button--sm {
    height: 2.25rem;
    padding: 0 1.125rem;
    font-size: var(--rinch-font-size-sm);
    border-radius: var(--rinch-radius-sm);
}

.rinch-button--md {
    height: 2.625rem;
    padding: 0 1.375rem;
    font-size: var(--rinch-font-size-md);
    border-radius: var(--rinch-radius-default);
}

.rinch-button--lg {
    height: 3.125rem;
    padding: 0 1.625rem;
    font-size: var(--rinch-font-size-lg);
    border-radius: var(--rinch-radius-default);
}

.rinch-button--xl {
    height: 3.75rem;
    padding: 0 2rem;
    font-size: var(--rinch-font-size-xl);
    border-radius: var(--rinch-radius-default);
}

/* Button variants - filled */
.rinch-button--filled {
    background-color: var(--rinch-primary-color);
    color: white;
}

.rinch-button--filled:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-7);
}

/* Button variants - outline */
.rinch-button--outline {
    background-color: transparent;
    color: var(--rinch-primary-color);
    border-color: var(--rinch-primary-color);
}

.rinch-button--outline:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-0);
}

/* Button variants - light */
.rinch-button--light {
    background-color: var(--rinch-primary-color-0);
    color: var(--rinch-primary-color-6);
}

.rinch-button--light:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-1);
}

/* Button variants - subtle */
.rinch-button--subtle {
    background-color: transparent;
    color: var(--rinch-primary-color);
}

.rinch-button--subtle:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-0);
}

/* Button variants - default */
.rinch-button--default {
    background-color: var(--rinch-color-filled);
    color: var(--rinch-color-text);
    border-color: var(--rinch-color-border);
}

.rinch-button--default:hover:not(:disabled) {
    background-color: var(--rinch-color-default);
}

/* Full width */
.rinch-button--full-width {
    width: 100%;
}

/* Button label wrapper */
.rinch-button__label {
    display: inline-flex;
    align-items: center;
}

/* Disabled state (via class) */
.rinch-button--disabled {
    cursor: not-allowed;
    opacity: 0.6;
    pointer-events: none;
}

/* Loading state */
.rinch-button--loading {
    position: relative;
    pointer-events: none;
}

.rinch-button--loading .rinch-button__label {
    opacity: 0;
}

.rinch-button__loader {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
}

.rinch-button__loader::after {
    content: '';
    width: 1rem;
    height: 1rem;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: rinch-button-spin 0.6s linear infinite;
}

@keyframes rinch-button-spin {
    to { transform: rotate(360deg); }
}

/* Radius overrides */
.rinch-button--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-button--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-button--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-button--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-button--radius-xl { border-radius: var(--rinch-radius-xl); }

/* Custom color support (via CSS custom properties) */
.rinch-button--colored.rinch-button--filled {
    background-color: var(--rinch-button-color);
}
.rinch-button--colored.rinch-button--filled:hover:not(:disabled) {
    background-color: var(--rinch-button-color-hover);
}
.rinch-button--colored.rinch-button--light {
    background-color: var(--rinch-button-color-light);
    color: var(--rinch-button-color);
}
.rinch-button--colored.rinch-button--light:hover:not(:disabled) {
    background-color: var(--rinch-button-color-light-hover);
}
.rinch-button--colored.rinch-button--outline {
    color: var(--rinch-button-color);
    border-color: var(--rinch-button-color);
}
.rinch-button--colored.rinch-button--outline:hover:not(:disabled) {
    background-color: var(--rinch-button-color-light);
}
.rinch-button--colored.rinch-button--subtle {
    color: var(--rinch-button-color);
}
.rinch-button--colored.rinch-button--subtle:hover:not(:disabled) {
    background-color: var(--rinch-button-color-light);
}

/* Button variants - transparent */
.rinch-button--transparent {
    background-color: transparent;
    color: var(--rinch-primary-color);
    border-color: transparent;
}
.rinch-button--transparent:hover:not(:disabled) {
    background-color: var(--rinch-color-filled);
}

/* Button variants - white */
.rinch-button--white {
    background-color: white;
    color: var(--rinch-color-text);
    border-color: var(--rinch-color-border);
}
.rinch-button--white:hover:not(:disabled) {
    background-color: var(--rinch-color-filled);
}
"#.to_string()
}
