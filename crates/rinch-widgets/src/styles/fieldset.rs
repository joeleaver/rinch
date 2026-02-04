pub fn styles() -> String {
    r#"
/* Fieldset base */
.rinch-fieldset {
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    padding: var(--rinch-spacing-md);
    margin: 0;
}

.rinch-fieldset__legend {
    font-size: var(--rinch-font-size-sm);
    font-weight: 600;
    color: var(--rinch-color-text);
    padding: 0 var(--rinch-spacing-xs);
}

/* Fieldset variants */
.rinch-fieldset--unstyled {
    border: none;
    padding: 0;
}

.rinch-fieldset--filled {
    background-color: var(--rinch-color-filled);
}

/* Fieldset sizes */
.rinch-fieldset--xs { padding: var(--rinch-spacing-xs); }
.rinch-fieldset--xs .rinch-fieldset__legend { font-size: var(--rinch-font-size-xs); }

.rinch-fieldset--sm { padding: var(--rinch-spacing-sm); }

.rinch-fieldset--lg { padding: var(--rinch-spacing-lg); }
.rinch-fieldset--lg .rinch-fieldset__legend { font-size: var(--rinch-font-size-md); }

.rinch-fieldset--xl { padding: var(--rinch-spacing-xl); }
.rinch-fieldset--xl .rinch-fieldset__legend { font-size: var(--rinch-font-size-lg); }
"#
    .to_string()
}
