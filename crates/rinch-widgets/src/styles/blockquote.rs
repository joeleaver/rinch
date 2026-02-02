pub fn styles() -> String {
    r#"
/* Blockquote base */
.rinch-blockquote {
    margin: 0;
    padding: var(--rinch-spacing-md) var(--rinch-spacing-lg);
    border-left: 4px solid var(--rinch-blockquote-color, var(--rinch-primary-color));
    background-color: var(--rinch-color-filled);
    border-radius: var(--rinch-radius-default);
}

/* Blockquote body */
.rinch-blockquote__body {
    margin: 0;
    font-size: var(--rinch-font-size-md);
    line-height: 1.6;
    color: var(--rinch-color-text);
    font-style: italic;
}

/* Blockquote citation */
.rinch-blockquote__cite {
    display: block;
    margin-top: var(--rinch-spacing-sm);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-dimmed);
    font-style: normal;
}

.rinch-blockquote__cite::before {
    content: '— ';
}

/* Blockquote with icon */
.rinch-blockquote--with-icon {
    display: flex;
    gap: var(--rinch-spacing-md);
}

.rinch-blockquote__icon {
    flex-shrink: 0;
    color: var(--rinch-blockquote-color, var(--rinch-primary-color));
}

.rinch-blockquote__icon svg {
    width: 1.5rem;
    height: 1.5rem;
}

/* Blockquote radius */
.rinch-blockquote--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-blockquote--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-blockquote--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-blockquote--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-blockquote--radius-xl { border-radius: var(--rinch-radius-xl); }
"#.to_string()
}
