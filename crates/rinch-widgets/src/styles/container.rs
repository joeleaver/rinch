pub fn styles() -> String {
    r#"
/* Container base */
.rinch-container {
    width: 100%;
    margin-left: auto;
    margin-right: auto;
    padding-left: var(--rinch-spacing-md);
    padding-right: var(--rinch-spacing-md);
}

/* Container sizes */
.rinch-container--xs { max-width: 576px; }
.rinch-container--sm { max-width: 768px; }
.rinch-container--md { max-width: 992px; }
.rinch-container--lg { max-width: 1200px; }
.rinch-container--xl { max-width: 1400px; }

/* Fluid container (no max-width) */
.rinch-container--fluid { max-width: none; }
"#.to_string()
}
