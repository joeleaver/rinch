pub fn styles() -> String {
    r#"
/* Highlight (search highlight) */
.rinch-highlight {
    display: inline;
}

.rinch-highlight__match {
    background-color: var(--rinch-highlight-color, var(--rinch-color-yellow-2));
    color: inherit;
    padding: 0 0.0625rem;
    border-radius: var(--rinch-radius-xs);
}
"#
    .to_string()
}
