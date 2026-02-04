pub fn styles() -> String {
    r#"
/* Checkbox wrapper */
.rinch-checkbox {
    display: inline-flex;
    align-items: center;
    gap: var(--rinch-spacing-sm);
    cursor: pointer;
    user-select: none;
}

.rinch-checkbox--disabled {
    cursor: not-allowed;
    opacity: 0.6;
}

/* Hide native checkbox but keep it interactive */
.rinch-checkbox__input {
    position: absolute;
    opacity: 0;
    width: 1.25rem;
    height: 1.25rem;
    margin: 0;
    cursor: pointer;
    z-index: 1;
}

/* Custom checkbox box */
.rinch-checkbox__box {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-xs);
    background-color: var(--rinch-color-surface);
    transition: all 150ms ease;
    flex-shrink: 0;
}

/* Checked state via class */
.rinch-checkbox--checked .rinch-checkbox__box {
    background-color: var(--rinch-primary-color);
    border-color: var(--rinch-primary-color);
}

.rinch-checkbox__input:focus + .rinch-checkbox__box {
    box-shadow: 0 0 0 2px var(--rinch-primary-color-2);
}

/* Checkmark icon inside box */
.rinch-checkbox__icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 0.75rem;
    height: 0.75rem;
    color: white;
    opacity: 0;
    transform: scale(0.5);
    transition: all 150ms ease;
}

.rinch-checkbox__icon svg {
    width: 100%;
    height: 100%;
}

.rinch-checkbox--checked .rinch-checkbox__icon {
    opacity: 1;
    transform: scale(1);
}

/* Checkbox label */
.rinch-checkbox__label {
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    white-space: nowrap;
}

/* Checkbox sizes - matches Mantine: xs=16px, sm=20px, md=24px, lg=30px */
.rinch-checkbox--xs .rinch-checkbox__box { width: 1rem; height: 1rem; }
.rinch-checkbox--xs .rinch-checkbox__icon { width: 0.625rem; height: 0.625rem; }
.rinch-checkbox--xs .rinch-checkbox__label { font-size: var(--rinch-font-size-xs); }

.rinch-checkbox--sm .rinch-checkbox__box { width: 1.25rem; height: 1.25rem; }
.rinch-checkbox--sm .rinch-checkbox__icon { width: 0.75rem; height: 0.75rem; }

/* Default md size is in .rinch-checkbox__box above: 1.25rem */

.rinch-checkbox--md .rinch-checkbox__box { width: 1.5rem; height: 1.5rem; }
.rinch-checkbox--md .rinch-checkbox__icon { width: 0.875rem; height: 0.875rem; }

.rinch-checkbox--lg .rinch-checkbox__box { width: 1.875rem; height: 1.875rem; }
.rinch-checkbox--lg .rinch-checkbox__icon { width: 1.125rem; height: 1.125rem; }
.rinch-checkbox--lg .rinch-checkbox__label { font-size: var(--rinch-font-size-md); }

/* Indeterminate state */
.rinch-checkbox--indeterminate .rinch-checkbox__box {
    background-color: var(--rinch-primary-color);
    border-color: var(--rinch-primary-color);
}

.rinch-checkbox--indeterminate .rinch-checkbox__icon {
    opacity: 1;
    transform: scale(1);
}
"#.to_string()
}
