pub fn styles() -> String {
    r#"
/* Slider base - clickable container */
.rinch-slider {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    height: 1.5rem;
    touch-action: none;
    user-select: none;
    cursor: pointer;
}

/* Slider track - pointer-events: none so clicks pass through to container */
.rinch-slider__track {
    position: absolute;
    left: 0;
    right: 0;
    width: 100%;
    background-color: var(--rinch-color-default);
    border-radius: var(--rinch-radius-xl);
    overflow: hidden;
    pointer-events: none;
}

/* Slider bar (filled portion) - pointer-events: none so clicks pass through */
.rinch-slider__bar {
    position: absolute;
    top: 0;
    left: 0;
    height: 100%;
    width: var(--rinch-slider-value, 0%);
    background-color: var(--rinch-slider-color, var(--rinch-primary-color));
    border-radius: inherit;
    pointer-events: none;
}

/* Slider thumb wrapper - positioned via CSS variable */
.rinch-slider__thumb-wrapper {
    position: absolute;
    left: var(--rinch-slider-value, 0%);
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    pointer-events: none;
    z-index: 1;
}

/* Invisible click overlay - captures all mouse events for the slider */
.rinch-slider__overlay {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: 10;
    cursor: pointer;
}

/* Slider thumb */
.rinch-slider__thumb {
    width: 1rem;
    height: 1rem;
    background-color: var(--rinch-slider-color, var(--rinch-primary-color));
    border: 4px solid var(--rinch-color-surface);
    border-radius: 50%;
    box-shadow: var(--rinch-shadow-sm);
}

/* Slider label (tooltip) */
.rinch-slider__label {
    position: absolute;
    bottom: 100%;
    margin-bottom: 0.5rem;
    padding: 0.25rem 0.5rem;
    background-color: var(--rinch-color-text);
    color: var(--rinch-color-body);
    font-size: var(--rinch-font-size-xs);
    border-radius: var(--rinch-radius-sm);
    white-space: nowrap;
    opacity: 0;
    transform: translateY(0.25rem);
    transition: opacity 150ms ease, transform 150ms ease;
    pointer-events: none;
}

.rinch-slider:hover .rinch-slider__label,
.rinch-slider--label-always-on .rinch-slider__label {
    opacity: 1;
    transform: translateY(0);
}

/* Slider sizes */
.rinch-slider--xs .rinch-slider__track { height: 0.25rem; }
.rinch-slider--xs .rinch-slider__thumb { width: 0.75rem; height: 0.75rem; border-width: 3px; }

.rinch-slider--sm .rinch-slider__track { height: 0.375rem; }
.rinch-slider--sm .rinch-slider__thumb { width: 0.875rem; height: 0.875rem; }

.rinch-slider--md .rinch-slider__track { height: 0.5rem; }

.rinch-slider--lg .rinch-slider__track { height: 0.625rem; }
.rinch-slider--lg .rinch-slider__thumb { width: 1.25rem; height: 1.25rem; }

.rinch-slider--xl .rinch-slider__track { height: 0.75rem; }
.rinch-slider--xl .rinch-slider__thumb { width: 1.5rem; height: 1.5rem; }

/* Slider radius */
.rinch-slider--radius-xs .rinch-slider__track { border-radius: var(--rinch-radius-xs); }
.rinch-slider--radius-sm .rinch-slider__track { border-radius: var(--rinch-radius-sm); }
.rinch-slider--radius-md .rinch-slider__track { border-radius: var(--rinch-radius-md); }
.rinch-slider--radius-lg .rinch-slider__track { border-radius: var(--rinch-radius-lg); }
.rinch-slider--radius-xl .rinch-slider__track { border-radius: var(--rinch-radius-xl); }

/* Slider disabled */
.rinch-slider--disabled {
    opacity: 0.6;
    pointer-events: none;
    cursor: default;
}
"#.to_string()
}
