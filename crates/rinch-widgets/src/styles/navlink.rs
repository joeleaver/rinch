pub fn styles() -> String {
    r#"
/* NavLink wrapper */
.rinch-navlink__wrapper {
    display: flex;
    flex-direction: column;
}

/* NavLink base */
.rinch-navlink {
    display: flex;
    align-items: center;
    width: 100%;
    padding: 0.625rem var(--rinch-spacing-sm);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    text-decoration: none;
    background-color: transparent;
    border: none;
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    transition: background-color 150ms ease, color 150ms ease;
    text-align: left;
}

.rinch-navlink:hover {
    background-color: var(--rinch-color-default);
}

/* Default variant */
.rinch-navlink--default.rinch-navlink--active {
    background-color: var(--rinch-navlink-color, var(--rinch-primary-color));
    color: white;
}

.rinch-navlink--default.rinch-navlink--active:hover {
    background-color: var(--rinch-navlink-color, var(--rinch-primary-color));
}

/* Light variant */
.rinch-navlink--light.rinch-navlink--active {
    background-color: color-mix(in srgb, var(--rinch-navlink-color, var(--rinch-primary-color)) 10%, transparent);
    color: var(--rinch-navlink-color, var(--rinch-primary-color));
}

/* Filled variant */
.rinch-navlink--filled {
    background-color: var(--rinch-color-filled);
}

.rinch-navlink--filled.rinch-navlink--active {
    background-color: var(--rinch-navlink-color, var(--rinch-primary-color));
    color: white;
}

/* Subtle variant */
.rinch-navlink--subtle.rinch-navlink--active {
    color: var(--rinch-navlink-color, var(--rinch-primary-color));
}

/* NavLink inner */
.rinch-navlink__inner {
    display: flex;
    align-items: center;
    gap: var(--rinch-spacing-sm);
    flex: 1;
    min-width: 0;
}

/* NavLink sections */
.rinch-navlink__left,
.rinch-navlink__right {
    display: flex;
    align-items: center;
    flex-shrink: 0;
}

.rinch-navlink__left svg,
.rinch-navlink__right svg {
    width: 1.25rem;
    height: 1.25rem;
}

/* NavLink body */
.rinch-navlink__body {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
}

/* NavLink label */
.rinch-navlink__label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
}

.rinch-navlink--no-wrap .rinch-navlink__label {
    white-space: nowrap;
}

/* NavLink description */
.rinch-navlink__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
    overflow: hidden;
    text-overflow: ellipsis;
}

.rinch-navlink--active .rinch-navlink__description {
    color: inherit;
    opacity: 0.7;
}

/* NavLink chevron (for expandable) */
.rinch-navlink__chevron {
    width: 1rem;
    height: 1rem;
    transition: transform 200ms ease;
    color: var(--rinch-color-dimmed);
}

.rinch-navlink--active .rinch-navlink__chevron {
    color: inherit;
}

.rinch-navlink__chevron--opened {
    transform: rotate(90deg);
}

/* NavLink children (nested links) */
.rinch-navlink__children {
    padding-left: var(--rinch-navlink-children-offset, var(--rinch-spacing-lg));
    overflow: hidden;
    max-height: 1000px;
    transition: max-height 200ms ease;
}

.rinch-navlink__children--collapsed {
    max-height: 0;
}

/* Disabled state */
.rinch-navlink--disabled {
    opacity: 0.5;
    cursor: not-allowed;
    pointer-events: none;
}
"#.to_string()
}
