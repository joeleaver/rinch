pub fn styles() -> String {
    r#"
/* Radio base */
.rinch-radio {
    display: inline-flex;
    align-items: flex-start;
    gap: var(--rinch-spacing-sm);
    cursor: pointer;
    user-select: none;
    position: relative;
}

.rinch-radio--disabled {
    cursor: not-allowed;
    opacity: 0.6;
}

/* Hide native radio but keep interactive */
.rinch-radio__input {
    position: absolute;
    opacity: 0;
    width: 1.25rem;
    height: 1.25rem;
    margin: 0;
    cursor: pointer;
    z-index: 1;
}

/* Custom radio indicator */
.rinch-radio__indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    border: 2px solid var(--rinch-color-border);
    border-radius: 50%;
    background-color: var(--rinch-color-body);
    transition: all 150ms ease;
    flex-shrink: 0;
}

/* Checked state */
.rinch-radio--checked .rinch-radio__indicator {
    background-color: var(--rinch-radio-color, var(--rinch-primary-color));
    border-color: var(--rinch-radio-color, var(--rinch-primary-color));
}

.rinch-radio__input:focus + .rinch-radio__indicator {
    box-shadow: 0 0 0 2px var(--rinch-primary-color-2);
}

/* Radio dot */
.rinch-radio__dot {
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 50%;
    background-color: var(--rinch-color-surface);
    opacity: 0;
    transform: scale(0);
    transition: all 150ms ease;
}

.rinch-radio--checked .rinch-radio__dot {
    opacity: 1;
    transform: scale(1);
}

/* Radio body */
.rinch-radio__body {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
}

.rinch-radio__label {
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    line-height: 1.4;
    white-space: nowrap;
}

.rinch-radio__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

/* Radio sizes */
.rinch-radio--xs .rinch-radio__indicator { width: 1rem; height: 1rem; }
.rinch-radio--xs .rinch-radio__dot { width: 0.375rem; height: 0.375rem; }
.rinch-radio--xs .rinch-radio__label { font-size: var(--rinch-font-size-xs); }

.rinch-radio--sm .rinch-radio__indicator { width: 1.125rem; height: 1.125rem; }
.rinch-radio--sm .rinch-radio__dot { width: 0.4375rem; height: 0.4375rem; }

.rinch-radio--lg .rinch-radio__indicator { width: 1.5rem; height: 1.5rem; }
.rinch-radio--lg .rinch-radio__dot { width: 0.625rem; height: 0.625rem; }
.rinch-radio--lg .rinch-radio__label { font-size: var(--rinch-font-size-md); }

.rinch-radio--xl .rinch-radio__indicator { width: 1.75rem; height: 1.75rem; }
.rinch-radio--xl .rinch-radio__dot { width: 0.75rem; height: 0.75rem; }

/* Radio error state */
.rinch-radio--error .rinch-radio__indicator {
    border-color: var(--rinch-color-red-6);
}

/* RadioGroup */
.rinch-radio-group {
    display: flex;
    flex-direction: column;
    gap: var(--rinch-spacing-xs);
}

.rinch-radio-group__label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
}

.rinch-radio-group__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

.rinch-radio-group__radios {
    display: flex;
    flex-direction: column;
    gap: var(--rinch-spacing-sm);
}

.rinch-radio-group--horizontal .rinch-radio-group__radios {
    flex-direction: row;
    flex-wrap: wrap;
}

.rinch-radio-group__error {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-red-6);
}
"#.to_string()
}
