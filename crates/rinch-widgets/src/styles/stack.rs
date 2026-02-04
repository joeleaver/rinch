pub fn styles() -> String {
    r#"
/* Stack base */
.rinch-stack {
    display: flex;
    flex-direction: column;
}

/* Stack gap */
.rinch-stack--gap-xs { gap: var(--rinch-spacing-xs); }
.rinch-stack--gap-sm { gap: var(--rinch-spacing-sm); }
.rinch-stack--gap-md { gap: var(--rinch-spacing-md); }
.rinch-stack--gap-lg { gap: var(--rinch-spacing-lg); }
.rinch-stack--gap-xl { gap: var(--rinch-spacing-xl); }

/* Stack alignment */
.rinch-stack--align-stretch { align-items: stretch; }
.rinch-stack--align-start { align-items: flex-start; }
.rinch-stack--align-center { align-items: center; }
.rinch-stack--align-end { align-items: flex-end; }

/* Stack justification */
.rinch-stack--justify-start { justify-content: flex-start; }
.rinch-stack--justify-center { justify-content: center; }
.rinch-stack--justify-end { justify-content: flex-end; }
.rinch-stack--justify-between { justify-content: space-between; }
.rinch-stack--justify-around { justify-content: space-around; }
"#.to_string()
}
