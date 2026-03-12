pub fn styles() -> String {
    r#"
/* Tooltip base */
.rinch-tooltip {
    position: relative;
    display: inline-block;
}

/* Tooltip target */
.rinch-tooltip__target {
    display: inline-block;
}

/* Tooltip content */
.rinch-tooltip__content {
    position: absolute;
    z-index: 1000;
    padding: 0.375rem 0.625rem;
    background-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
    color: white;
    font-size: var(--rinch-font-size-xs);
    border-radius: var(--rinch-radius-sm);
    white-space: nowrap;
    pointer-events: none;
}

/* Tooltip positions */
.rinch-tooltip--top .rinch-tooltip__content {
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(-0.25rem);
    margin-bottom: 0.375rem;
}

.rinch-tooltip--bottom .rinch-tooltip__content {
    top: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(0.25rem);
    margin-top: 0.375rem;
}

.rinch-tooltip--left .rinch-tooltip__content {
    right: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(-0.25rem);
    margin-right: 0.375rem;
}

.rinch-tooltip--right .rinch-tooltip__content {
    left: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(0.25rem);
    margin-left: 0.375rem;
}

/* Tooltip with arrow */
.rinch-tooltip--with-arrow .rinch-tooltip__content::after {
    content: '';
    position: absolute;
    border: 0.375rem solid transparent;
}

.rinch-tooltip--with-arrow.rinch-tooltip--top .rinch-tooltip__content::after {
    top: 100%;
    left: 50%;
    transform: translateX(-50%);
    border-top-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

.rinch-tooltip--with-arrow.rinch-tooltip--bottom .rinch-tooltip__content::after {
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%);
    border-bottom-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

.rinch-tooltip--with-arrow.rinch-tooltip--left .rinch-tooltip__content::after {
    left: 100%;
    top: 50%;
    transform: translateY(-50%);
    border-left-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

.rinch-tooltip--with-arrow.rinch-tooltip--right .rinch-tooltip__content::after {
    right: 100%;
    top: 50%;
    transform: translateY(-50%);
    border-right-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

/* Tooltip multiline */
.rinch-tooltip--multiline .rinch-tooltip__content {
    white-space: normal;
    width: var(--rinch-tooltip-width, 200px);
    text-align: center;
}

/* Tooltip disabled */
.rinch-tooltip--disabled .rinch-tooltip__content {
    display: none;
}
"#
    .to_string()
}
