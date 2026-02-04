pub fn styles() -> String {
    r#"
/* Anchor base */
.rinch-anchor {
    color: var(--rinch-primary-color);
    text-decoration: none;
    cursor: pointer;
    transition: color 150ms ease;
}

.rinch-anchor:hover {
    text-decoration: underline;
}

/* Anchor with underline always */
.rinch-anchor--underline {
    text-decoration: underline;
}

/* Anchor sizes */
.rinch-anchor--xs { font-size: var(--rinch-font-size-xs); }
.rinch-anchor--sm { font-size: var(--rinch-font-size-sm); }
.rinch-anchor--md { font-size: var(--rinch-font-size-md); }
.rinch-anchor--lg { font-size: var(--rinch-font-size-lg); }
.rinch-anchor--xl { font-size: var(--rinch-font-size-xl); }

/* Anchor inherit color from parent */
.rinch-anchor--inherit {
    color: inherit;
}
"#.to_string()
}
