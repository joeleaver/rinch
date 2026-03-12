pub fn styles() -> String {
    r#"
/* Fieldset base — rinch doesn't natively support <fieldset>/<legend> semantics,
   so we emulate the browser behavior with flex layout + negative margin. */
.rinch-fieldset {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    padding: var(--rinch-spacing-md);
    padding-top: 0;
    margin: 0;
}

.rinch-fieldset__legend {
    font-size: var(--rinch-font-size-sm);
    font-weight: 600;
    color: var(--rinch-color-text);
    padding: 0 var(--rinch-spacing-xs);
    margin-top: -0.6em;
    margin-bottom: var(--rinch-spacing-xs);
    align-self: flex-start;
    background-color: var(--rinch-color-body);
}

/* Fieldset variants */
.rinch-fieldset--unstyled {
    border: none;
    padding: 0;
}

.rinch-fieldset--unstyled .rinch-fieldset__legend {
    margin-top: 0;
    background-color: transparent;
}

.rinch-fieldset--filled {
    background-color: var(--rinch-color-filled);
}

.rinch-fieldset--filled .rinch-fieldset__legend {
    background-color: var(--rinch-color-filled);
}

/* Fieldset sizes */
.rinch-fieldset--xs { padding: var(--rinch-spacing-xs); padding-top: 0; }
.rinch-fieldset--xs .rinch-fieldset__legend { font-size: var(--rinch-font-size-xs); }

.rinch-fieldset--sm { padding: var(--rinch-spacing-sm); padding-top: 0; }

.rinch-fieldset--lg { padding: var(--rinch-spacing-lg); padding-top: 0; }
.rinch-fieldset--lg .rinch-fieldset__legend { font-size: var(--rinch-font-size-md); }

.rinch-fieldset--xl { padding: var(--rinch-spacing-xl); padding-top: 0; }
.rinch-fieldset--xl .rinch-fieldset__legend { font-size: var(--rinch-font-size-lg); }
"#
    .to_string()
}
