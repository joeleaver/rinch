pub fn styles() -> String {
    r#"
/* List base */
.rinch-list {
    font-family: var(--rinch-font-family);
    margin: 0;
    padding-left: var(--rinch-spacing-lg);
    list-style-position: outside;
}

/* List types */
.rinch-list--unordered {
    list-style-type: disc;
}

.rinch-list--ordered {
    list-style-type: decimal;
}

/* List without padding (markers inside text) */
.rinch-list--no-padding {
    padding-left: 0;
    list-style-position: inside;
}

/* List sizes */
.rinch-list--xs { font-size: var(--rinch-font-size-xs); }
.rinch-list--sm { font-size: var(--rinch-font-size-sm); }
.rinch-list--md { font-size: var(--rinch-font-size-md); }
.rinch-list--lg { font-size: var(--rinch-font-size-lg); }
.rinch-list--xl { font-size: var(--rinch-font-size-xl); }

/* List spacing */
.rinch-list--spacing-xs .rinch-list__item { margin-bottom: var(--rinch-spacing-xs); }
.rinch-list--spacing-sm .rinch-list__item { margin-bottom: var(--rinch-spacing-sm); }
.rinch-list--spacing-md .rinch-list__item { margin-bottom: var(--rinch-spacing-md); }
.rinch-list--spacing-lg .rinch-list__item { margin-bottom: var(--rinch-spacing-lg); }
.rinch-list--spacing-xl .rinch-list__item { margin-bottom: var(--rinch-spacing-xl); }

.rinch-list__item:last-child {
    margin-bottom: 0;
}

/* List centered */
.rinch-list--center .rinch-list__item {
    display: flex;
    align-items: center;
}

/* List item with custom icon */
.rinch-list__item--with-icon {
    display: flex;
    align-items: flex-start;
    gap: var(--rinch-spacing-sm);
}

.rinch-list__item-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
}

.rinch-list__item-icon svg {
    width: 1em;
    height: 1em;
}

.rinch-list__item-content {
    flex: 1;
}
"#.to_string()
}
