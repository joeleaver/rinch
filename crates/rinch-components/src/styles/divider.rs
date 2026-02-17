pub fn styles() -> String {
    r#"
/* Divider base */
.rinch-divider {
    border: 0;
    margin: 0;
}

/* Horizontal divider (default) */
.rinch-divider--horizontal {
    width: 100%;
    height: 1px;
    background-color: var(--rinch-color-border);
}

/* Vertical divider */
.rinch-divider--vertical {
    width: 1px;
    height: auto;
    align-self: stretch;
    background-color: var(--rinch-color-border);
}

/* Divider sizes */
.rinch-divider--xs { margin: var(--rinch-spacing-xs) 0; }
.rinch-divider--sm { margin: var(--rinch-spacing-sm) 0; }
.rinch-divider--md { margin: var(--rinch-spacing-md) 0; }
.rinch-divider--lg { margin: var(--rinch-spacing-lg) 0; }
.rinch-divider--xl { margin: var(--rinch-spacing-xl) 0; }

/* Vertical divider sizes */
.rinch-divider--vertical.rinch-divider--xs { margin: 0 var(--rinch-spacing-xs); }
.rinch-divider--vertical.rinch-divider--sm { margin: 0 var(--rinch-spacing-sm); }
.rinch-divider--vertical.rinch-divider--md { margin: 0 var(--rinch-spacing-md); }
.rinch-divider--vertical.rinch-divider--lg { margin: 0 var(--rinch-spacing-lg); }
.rinch-divider--vertical.rinch-divider--xl { margin: 0 var(--rinch-spacing-xl); }

/* Divider with label */
.rinch-divider--with-label {
    display: flex;
    align-items: center;
    height: auto;
    background: none;
}

.rinch-divider--with-label::before,
.rinch-divider--with-label::after {
    content: '';
    flex: 1;
    height: 1px;
    background-color: var(--rinch-color-border);
}

.rinch-divider__label {
    padding: 0 var(--rinch-spacing-md);
    color: var(--rinch-color-dimmed);
    font-size: var(--rinch-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

/* Label positions */
.rinch-divider--label-left::before { display: none; }
.rinch-divider--label-left .rinch-divider__label { padding-left: 0; }
.rinch-divider--label-right::after { display: none; }
.rinch-divider--label-right .rinch-divider__label { padding-right: 0; }
"#
    .to_string()
}
