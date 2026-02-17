pub fn styles() -> String {
    r#"
/* Accordion base */
.rinch-accordion {
    display: flex;
    flex-direction: column;
}

/* Accordion item */
.rinch-accordion__item {
    border-bottom: 1px solid var(--rinch-color-border);
}

.rinch-accordion__item:last-child {
    border-bottom: none;
}

/* Default variant */
.rinch-accordion--default .rinch-accordion__item {
    border-bottom: 1px solid var(--rinch-color-border);
}

/* Contained variant */
.rinch-accordion--contained {
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
}

.rinch-accordion--contained .rinch-accordion__item {
    border-bottom: 1px solid var(--rinch-color-border);
}

.rinch-accordion--contained .rinch-accordion__item:last-child {
    border-bottom: none;
}

/* Filled variant */
.rinch-accordion--filled .rinch-accordion__control {
    background-color: var(--rinch-color-filled);
}

.rinch-accordion--filled .rinch-accordion__control:hover {
    background-color: var(--rinch-color-default);
}

/* Separated variant */
.rinch-accordion--separated .rinch-accordion__item {
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    margin-bottom: var(--rinch-spacing-sm);
    background-color: var(--rinch-color-body);
}

.rinch-accordion--separated .rinch-accordion__item:last-child {
    margin-bottom: 0;
}

/* Accordion control */
.rinch-accordion__control {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: var(--rinch-spacing-md);
    font-size: var(--rinch-font-size-md);
    font-weight: 500;
    color: var(--rinch-color-text);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    transition: background-color 150ms ease;
}

.rinch-accordion__control:hover {
    background-color: var(--rinch-color-filled);
}

.rinch-accordion__control--disabled {
    opacity: 0.5;
    cursor: not-allowed;
    pointer-events: none;
}

/* Chevron position */
.rinch-accordion--chevron-left .rinch-accordion__control {
    flex-direction: row-reverse;
    justify-content: flex-end;
    gap: var(--rinch-spacing-sm);
}

/* Accordion label */
.rinch-accordion__label {
    flex: 1;
}

/* Accordion chevron */
.rinch-accordion__chevron {
    width: 1.25rem;
    height: 1.25rem;
    transition: transform 200ms ease;
    color: var(--rinch-color-dimmed);
}

.rinch-accordion__item[data-active="true"] .rinch-accordion__chevron,
.rinch-accordion__item--active .rinch-accordion__chevron {
    transform: rotate(180deg);
}

/* Accordion panel */
.rinch-accordion__panel {
    overflow: hidden;
    max-height: 0;
    transition: max-height 200ms ease;
}

.rinch-accordion__item[data-active="true"] .rinch-accordion__panel,
.rinch-accordion__item--active .rinch-accordion__panel {
    max-height: 1000px;
}

.rinch-accordion__content {
    padding: 0 var(--rinch-spacing-md) var(--rinch-spacing-md);
    color: var(--rinch-color-dimmed);
    font-size: var(--rinch-font-size-sm);
}

/* Radius variants */
.rinch-accordion--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-accordion--radius-xs .rinch-accordion__item:first-child .rinch-accordion__control {
    border-radius: var(--rinch-radius-xs) var(--rinch-radius-xs) 0 0;
}
.rinch-accordion--radius-xs .rinch-accordion__item:last-child .rinch-accordion__control {
    border-radius: 0 0 var(--rinch-radius-xs) var(--rinch-radius-xs);
}

.rinch-accordion--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-accordion--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-accordion--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-accordion--radius-xl { border-radius: var(--rinch-radius-xl); }

/* Separated variant radius */
.rinch-accordion--separated.rinch-accordion--radius-xs .rinch-accordion__item { border-radius: var(--rinch-radius-xs); }
.rinch-accordion--separated.rinch-accordion--radius-sm .rinch-accordion__item { border-radius: var(--rinch-radius-sm); }
.rinch-accordion--separated.rinch-accordion--radius-md .rinch-accordion__item { border-radius: var(--rinch-radius-md); }
.rinch-accordion--separated.rinch-accordion--radius-lg .rinch-accordion__item { border-radius: var(--rinch-radius-lg); }
.rinch-accordion--separated.rinch-accordion--radius-xl .rinch-accordion__item { border-radius: var(--rinch-radius-xl); }
"#.to_string()
}
