pub fn styles() -> String {
    r#"
/* Stepper base */
.rinch-stepper {
    display: flex;
    flex-direction: column;
}

/* Stepper steps container */
.rinch-stepper__steps {
    display: flex;
    gap: var(--rinch-spacing-md);
}

.rinch-stepper--horizontal .rinch-stepper__steps {
    flex-direction: row;
    align-items: flex-start;
}

.rinch-stepper--vertical .rinch-stepper__steps {
    flex-direction: column;
}

/* Stepper step */
.rinch-stepper__step {
    display: flex;
    flex: 1;
    position: relative;
}

.rinch-stepper--horizontal .rinch-stepper__step {
    flex-direction: column;
    align-items: center;
}

.rinch-stepper--vertical .rinch-stepper__step {
    flex-direction: row;
    align-items: flex-start;
    gap: var(--rinch-spacing-sm);
}

/* Last step has no separator flex */
.rinch-stepper__step:last-child {
    flex: 0;
}

/* Step icon */
.rinch-stepper__step-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--rinch-stepper-icon-size, 2.25rem);
    height: var(--rinch-stepper-icon-size, 2.25rem);
    border-radius: 50%;
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-dimmed);
    font-size: var(--rinch-font-size-sm);
    font-weight: 600;
    flex-shrink: 0;
    transition: background-color 150ms ease, color 150ms ease;
    z-index: 1;
}

.rinch-stepper__step-icon svg {
    width: 60%;
    height: 60%;
}

/* Completed step */
.rinch-stepper__step--completed .rinch-stepper__step-icon {
    background-color: var(--rinch-stepper-color, var(--rinch-primary-color));
    color: white;
}

/* Progress (current) step */
.rinch-stepper__step--progress .rinch-stepper__step-icon {
    background-color: var(--rinch-stepper-color, var(--rinch-primary-color));
    color: white;
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--rinch-stepper-color, var(--rinch-primary-color)) 20%, transparent);
}

/* Step body */
.rinch-stepper__step-body {
    display: flex;
    flex-direction: column;
    text-align: center;
    margin-top: var(--rinch-spacing-xs);
}

.rinch-stepper--vertical .rinch-stepper__step-body {
    text-align: left;
    margin-top: 0;
    padding-top: 0.25rem;
}

/* Step label */
.rinch-stepper__step-label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
}

.rinch-stepper__step--inactive .rinch-stepper__step-label {
    color: var(--rinch-color-dimmed);
}

/* Step description */
.rinch-stepper__step-description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
    margin-top: 0.125rem;
}

/* Step content (optional content for each step) */
.rinch-stepper__step-content {
    display: none;
}

/* Separator line */
.rinch-stepper__separator {
    position: absolute;
    background-color: var(--rinch-color-border);
    transition: background-color 150ms ease;
}

.rinch-stepper--horizontal .rinch-stepper__separator {
    top: calc(var(--rinch-stepper-icon-size, 2.25rem) / 2);
    left: calc(50% + var(--rinch-stepper-icon-size, 2.25rem) / 2 + 0.5rem);
    right: calc(-50% + var(--rinch-stepper-icon-size, 2.25rem) / 2 + 0.5rem);
    height: 2px;
}

.rinch-stepper--vertical .rinch-stepper__separator {
    left: calc(var(--rinch-stepper-icon-size, 2.25rem) / 2 - 1px);
    top: calc(var(--rinch-stepper-icon-size, 2.25rem) + 0.5rem);
    bottom: calc(-0.5rem);
    width: 2px;
}

.rinch-stepper__step:last-child .rinch-stepper__separator {
    display: none;
}

/* Completed separator */
.rinch-stepper__step--completed .rinch-stepper__separator {
    background-color: var(--rinch-stepper-color, var(--rinch-primary-color));
}

/* Clickable step */
.rinch-stepper__step--clickable .rinch-stepper__step-icon {
    cursor: pointer;
}

.rinch-stepper__step--clickable .rinch-stepper__step-icon:hover {
    opacity: 0.9;
}

/* Loading state */
.rinch-stepper__step--loading .rinch-stepper__step-icon {
    background-color: transparent;
}

.rinch-stepper__loader {
    width: 60%;
    height: 60%;
    border: 2px solid var(--rinch-stepper-color, var(--rinch-primary-color));
    border-top-color: transparent;
    border-radius: 50%;
    animation: rinch-stepper-spin 0.6s linear infinite;
}

@keyframes rinch-stepper-spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
}

/* Size variants */
.rinch-stepper--xs { --rinch-stepper-icon-size: 1.5rem; }
.rinch-stepper--xs .rinch-stepper__step-label { font-size: var(--rinch-font-size-xs); }
.rinch-stepper--xs .rinch-stepper__step-description { font-size: 0.625rem; }

.rinch-stepper--sm { --rinch-stepper-icon-size: 1.875rem; }
.rinch-stepper--sm .rinch-stepper__step-label { font-size: var(--rinch-font-size-xs); }

.rinch-stepper--lg { --rinch-stepper-icon-size: 2.75rem; }
.rinch-stepper--lg .rinch-stepper__step-label { font-size: var(--rinch-font-size-md); }

.rinch-stepper--xl { --rinch-stepper-icon-size: 3.25rem; }
.rinch-stepper--xl .rinch-stepper__step-label { font-size: var(--rinch-font-size-lg); }

/* Radius variants */
.rinch-stepper--radius-xs .rinch-stepper__step-icon { border-radius: var(--rinch-radius-xs); }
.rinch-stepper--radius-sm .rinch-stepper__step-icon { border-radius: var(--rinch-radius-sm); }
.rinch-stepper--radius-md .rinch-stepper__step-icon { border-radius: var(--rinch-radius-md); }
.rinch-stepper--radius-lg .rinch-stepper__step-icon { border-radius: var(--rinch-radius-lg); }
.rinch-stepper--radius-xl .rinch-stepper__step-icon { border-radius: var(--rinch-radius-xl); }

/* Completed state content */
.rinch-stepper__completed {
    padding: var(--rinch-spacing-md);
    text-align: center;
}
"#.to_string()
}
