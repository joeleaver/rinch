pub fn styles() -> String {
    r#"
/* Mark (highlighted text) */
.rinch-mark {
    background-color: var(--rinch-mark-color, var(--rinch-color-yellow-2));
    color: inherit;
    padding: 0 0.125rem;
    border-radius: var(--rinch-radius-xs);
}
"#.to_string()
}
