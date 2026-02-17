pub fn styles() -> String {
    r#"
/* Group base */
.rinch-group {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
}

/* Group gap */
.rinch-group--gap-xs { gap: var(--rinch-spacing-xs); }
.rinch-group--gap-sm { gap: var(--rinch-spacing-sm); }
.rinch-group--gap-md { gap: var(--rinch-spacing-md); }
.rinch-group--gap-lg { gap: var(--rinch-spacing-lg); }
.rinch-group--gap-xl { gap: var(--rinch-spacing-xl); }

/* Group alignment */
.rinch-group--align-stretch { align-items: stretch; }
.rinch-group--align-start { align-items: flex-start; }
.rinch-group--align-center { align-items: center; }
.rinch-group--align-end { align-items: flex-end; }
.rinch-group--align-baseline { align-items: baseline; }

/* Group justification */
.rinch-group--justify-start { justify-content: flex-start; }
.rinch-group--justify-center { justify-content: center; }
.rinch-group--justify-end { justify-content: flex-end; }
.rinch-group--justify-between { justify-content: space-between; }
.rinch-group--justify-around { justify-content: space-around; }

/* Group no wrap */
.rinch-group--nowrap { flex-wrap: nowrap; }

/* Group grow children */
.rinch-group--grow > * { flex-grow: 1; }
"#
    .to_string()
}
