pub fn styles() -> String {
    r#"
/* Card base */
.rinch-card {
    background-color: var(--rinch-color-body);
    border-radius: var(--rinch-radius-default);
    overflow: hidden;
}

/* Card shadows */
.rinch-card--shadow-xs { box-shadow: var(--rinch-shadow-xs); }
.rinch-card--shadow-sm { box-shadow: var(--rinch-shadow-sm); }
.rinch-card--shadow-md { box-shadow: var(--rinch-shadow-md); }
.rinch-card--shadow-lg { box-shadow: var(--rinch-shadow-lg); }
.rinch-card--shadow-xl { box-shadow: var(--rinch-shadow-xl); }

/* Card padding */
.rinch-card--p-xs { padding: var(--rinch-spacing-xs); }
.rinch-card--p-sm { padding: var(--rinch-spacing-sm); }
.rinch-card--p-md { padding: var(--rinch-spacing-md); }
.rinch-card--p-lg { padding: var(--rinch-spacing-lg); }
.rinch-card--p-xl { padding: var(--rinch-spacing-xl); }

/* Card radius */
.rinch-card--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-card--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-card--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-card--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-card--radius-xl { border-radius: var(--rinch-radius-xl); }

/* Card with border */
.rinch-card--with-border {
    border: 1px solid var(--rinch-color-border);
}

/* Card section */
.rinch-card__section {
    margin-left: calc(var(--rinch-spacing-md) * -1);
    margin-right: calc(var(--rinch-spacing-md) * -1);
}

.rinch-card--p-xs > .rinch-card__section {
    margin-left: calc(var(--rinch-spacing-xs) * -1);
    margin-right: calc(var(--rinch-spacing-xs) * -1);
}

.rinch-card--p-sm > .rinch-card__section {
    margin-left: calc(var(--rinch-spacing-sm) * -1);
    margin-right: calc(var(--rinch-spacing-sm) * -1);
}

.rinch-card--p-lg > .rinch-card__section {
    margin-left: calc(var(--rinch-spacing-lg) * -1);
    margin-right: calc(var(--rinch-spacing-lg) * -1);
}

.rinch-card--p-xl > .rinch-card__section {
    margin-left: calc(var(--rinch-spacing-xl) * -1);
    margin-right: calc(var(--rinch-spacing-xl) * -1);
}

.rinch-card__section--inherit-padding {
    padding-left: var(--rinch-spacing-md);
    padding-right: var(--rinch-spacing-md);
}

.rinch-card__section--with-border {
    border-top: 1px solid var(--rinch-color-border);
}

.rinch-card__section img {
    display: block;
    width: 100%;
}
"#.to_string()
}
