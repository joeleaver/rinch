pub fn styles() -> String {
    r#"
/* Space - creates empty space */
.rinch-space {
    display: block;
}

/* Horizontal space (width) */
.rinch-space--w-xs { width: var(--rinch-spacing-xs); }
.rinch-space--w-sm { width: var(--rinch-spacing-sm); }
.rinch-space--w-md { width: var(--rinch-spacing-md); }
.rinch-space--w-lg { width: var(--rinch-spacing-lg); }
.rinch-space--w-xl { width: var(--rinch-spacing-xl); }

/* Vertical space (height) */
.rinch-space--h-xs { height: var(--rinch-spacing-xs); }
.rinch-space--h-sm { height: var(--rinch-spacing-sm); }
.rinch-space--h-md { height: var(--rinch-spacing-md); }
.rinch-space--h-lg { height: var(--rinch-spacing-lg); }
.rinch-space--h-xl { height: var(--rinch-spacing-xl); }
"#.to_string()
}
