pub fn styles() -> String {
    r#"
/* Paper base */
.rinch-paper {
    background-color: var(--rinch-color-body);
    border-radius: var(--rinch-radius-default);
}

/* Paper shadows */
.rinch-paper--shadow-xs { box-shadow: var(--rinch-shadow-xs); }
.rinch-paper--shadow-sm { box-shadow: var(--rinch-shadow-sm); }
.rinch-paper--shadow-md { box-shadow: var(--rinch-shadow-md); }
.rinch-paper--shadow-lg { box-shadow: var(--rinch-shadow-lg); }
.rinch-paper--shadow-xl { box-shadow: var(--rinch-shadow-xl); }

/* Paper with border */
.rinch-paper--with-border {
    border: 1px solid var(--rinch-color-border);
}

/* Paper padding */
.rinch-paper--p-xs { padding: var(--rinch-spacing-xs); }
.rinch-paper--p-sm { padding: var(--rinch-spacing-sm); }
.rinch-paper--p-md { padding: var(--rinch-spacing-md); }
.rinch-paper--p-lg { padding: var(--rinch-spacing-lg); }
.rinch-paper--p-xl { padding: var(--rinch-spacing-xl); }

/* Paper radius */
.rinch-paper--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-paper--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-paper--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-paper--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-paper--radius-xl { border-radius: var(--rinch-radius-xl); }
"#
    .to_string()
}
