pub fn styles() -> String {
    r#"
/* Tabs base */
.rinch-tabs {
    display: flex;
    flex-direction: column;
}

.rinch-tabs--vertical {
    flex-direction: row;
}

.rinch-tabs--position-bottom {
    flex-direction: column-reverse;
}

.rinch-tabs--position-right {
    flex-direction: row-reverse;
}

/* Tabs list */
.rinch-tabs__list {
    display: flex;
    flex-wrap: wrap;
    gap: 0;
    border-bottom: 2px solid var(--rinch-color-border);
}

.rinch-tabs--vertical .rinch-tabs__list {
    flex-direction: column;
    border-bottom: none;
    border-right: 2px solid var(--rinch-color-border);
}

.rinch-tabs--position-bottom .rinch-tabs__list {
    border-bottom: none;
    border-top: 2px solid var(--rinch-color-border);
}

.rinch-tabs--position-right .rinch-tabs__list {
    border-right: none;
    border-left: 2px solid var(--rinch-color-border);
}

.rinch-tabs__list--grow {
    justify-content: stretch;
}

.rinch-tabs__list--grow .rinch-tabs__tab {
    flex: 1;
}

/* Tab button */
.rinch-tabs__tab {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-dimmed);
    background: transparent;
    border: none;
    cursor: pointer;
    position: relative;
    transition: color 150ms ease, background-color 150ms ease;
}

.rinch-tabs__tab:hover {
    color: var(--rinch-color-text);
    background-color: var(--rinch-color-filled);
}

.rinch-tabs__tab[data-active="true"],
.rinch-tabs__tab--active {
    color: var(--rinch-tabs-color, var(--rinch-primary-color));
}

.rinch-tabs__tab--disabled {
    opacity: 0.5;
    cursor: not-allowed;
    pointer-events: none;
}

/* Default variant - underline indicator */
.rinch-tabs--default .rinch-tabs__tab::after {
    content: '';
    position: absolute;
    bottom: -2px;
    left: 0;
    right: 0;
    height: 2px;
    background-color: transparent;
    transition: background-color 150ms ease;
}

.rinch-tabs--default .rinch-tabs__tab[data-active="true"]::after,
.rinch-tabs--default .rinch-tabs__tab--active::after {
    background-color: var(--rinch-tabs-color, var(--rinch-primary-color));
}

.rinch-tabs--vertical.rinch-tabs--default .rinch-tabs__tab::after {
    bottom: auto;
    left: auto;
    right: -2px;
    top: 0;
    width: 2px;
    height: 100%;
}

/* Outline variant */
.rinch-tabs--outline .rinch-tabs__list {
    border-bottom: 1px solid var(--rinch-color-border);
}

.rinch-tabs--outline .rinch-tabs__tab {
    border: 1px solid transparent;
    border-bottom: none;
    margin-bottom: -1px;
    border-radius: var(--rinch-radius-sm) var(--rinch-radius-sm) 0 0;
}

.rinch-tabs--outline .rinch-tabs__tab[data-active="true"],
.rinch-tabs--outline .rinch-tabs__tab--active {
    border-color: var(--rinch-color-border);
    background-color: var(--rinch-color-body);
}

/* Pills variant */
.rinch-tabs--pills .rinch-tabs__list {
    border: none;
    gap: 0.25rem;
}

.rinch-tabs--pills .rinch-tabs__tab {
    border-radius: var(--rinch-radius-sm);
}

.rinch-tabs--pills .rinch-tabs__tab[data-active="true"],
.rinch-tabs--pills .rinch-tabs__tab--active {
    background-color: var(--rinch-tabs-color, var(--rinch-primary-color));
    color: white;
}

/* Tab sections */
.rinch-tabs__tab-left,
.rinch-tabs__tab-right {
    display: flex;
    align-items: center;
}

.rinch-tabs__tab-left svg,
.rinch-tabs__tab-right svg {
    width: 1rem;
    height: 1rem;
}

/* Tabs panel */
.rinch-tabs__panel {
    padding: var(--rinch-spacing-md) 0;
}

.rinch-tabs__panel[hidden] {
    display: none;
}

/* Grow modifier */
.rinch-tabs--grow .rinch-tabs__tab {
    flex: 1;
}
"#
    .to_string()
}
