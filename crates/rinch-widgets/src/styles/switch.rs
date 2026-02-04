pub fn styles() -> String {
    r#"
/* Switch wrapper */
.rinch-switch {
    display: inline-flex;
    align-items: center;
    gap: var(--rinch-spacing-sm);
    cursor: pointer;
    user-select: none;
}

.rinch-switch--disabled {
    cursor: not-allowed;
    opacity: 0.6;
}

/* Hide native checkbox but keep it interactive */
.rinch-switch__input {
    position: absolute;
    opacity: 0;
    width: 2.5rem;
    height: 1.5rem;
    margin: 0;
    cursor: pointer;
    z-index: 1;
}

/* Switch track */
.rinch-switch__track {
    position: relative;
    width: 2.5rem;
    height: 1.5rem;
    background-color: var(--rinch-color-default);
    border-radius: 1rem;
    transition: background-color 150ms ease;
    flex-shrink: 0;
}

/* Checked state via class */
.rinch-switch--checked .rinch-switch__track {
    background-color: var(--rinch-primary-color);
}

.rinch-switch__input:focus + .rinch-switch__track {
    box-shadow: 0 0 0 2px var(--rinch-primary-color-2);
}

/* Switch thumb */
.rinch-switch__thumb {
    position: absolute;
    top: 0.125rem;
    left: 0.125rem;
    width: 1.25rem;
    height: 1.25rem;
    background-color: var(--rinch-color-surface);
    border-radius: 50%;
    box-shadow: var(--rinch-shadow-xs);
    transition: transform 150ms ease;
}

.rinch-switch--checked .rinch-switch__thumb {
    transform: translateX(1rem);
}

/* Switch label */
.rinch-switch__label {
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    white-space: nowrap;
}

/* Switch sizes */
.rinch-switch--xs .rinch-switch__track { width: 1.75rem; height: 1rem; }
.rinch-switch--xs .rinch-switch__thumb { width: 0.75rem; height: 0.75rem; }
.rinch-switch--xs.rinch-switch--checked .rinch-switch__thumb { transform: translateX(0.75rem); }
.rinch-switch--xs .rinch-switch__label { font-size: var(--rinch-font-size-xs); }

.rinch-switch--sm .rinch-switch__track { width: 2rem; height: 1.25rem; }
.rinch-switch--sm .rinch-switch__thumb { width: 1rem; height: 1rem; top: 0.125rem; }
.rinch-switch--sm.rinch-switch--checked .rinch-switch__thumb { transform: translateX(0.75rem); }

.rinch-switch--lg .rinch-switch__track { width: 3rem; height: 1.75rem; }
.rinch-switch--lg .rinch-switch__thumb { width: 1.5rem; height: 1.5rem; }
.rinch-switch--lg.rinch-switch--checked .rinch-switch__thumb { transform: translateX(1.25rem); }
.rinch-switch--lg .rinch-switch__label { font-size: var(--rinch-font-size-md); }

/* Label position */
.rinch-switch--label-start {
    flex-direction: row-reverse;
}
"#.to_string()
}
