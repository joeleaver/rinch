pub fn styles() -> String {
    r#"
/* CloseButton base */
.rinch-close-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 0;
    color: var(--rinch-color-gray-6);
    border-radius: var(--rinch-radius-sm);
    transition: background-color 150ms ease, color 150ms ease;
}

.rinch-close-button:hover {
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
}

.rinch-close-button:disabled,
.rinch-close-button--disabled {
    cursor: not-allowed;
    opacity: 0.6;
    pointer-events: none;
}

/* The glyph's size: `--rinch-close-button-icon-size`, published on the glyph
   from `icon_size` or the size step. One class, so an app rule such as
   `.rinch-close-button svg { width: 20px; height: 20px; }` outranks it. */
.rinch-icon--close {
    width: var(--rinch-close-button-icon-size, 16px);
    height: var(--rinch-close-button-icon-size, 16px);
    flex-shrink: 0;
}

/* CloseButton sizes */
.rinch-close-button--xs { width: 1rem; height: 1rem; }
.rinch-close-button--sm { width: 1.25rem; height: 1.25rem; }
.rinch-close-button--md { width: 1.5rem; height: 1.5rem; }
.rinch-close-button--lg { width: 2rem; height: 2rem; }
.rinch-close-button--xl { width: 2.5rem; height: 2.5rem; }

/* CloseButton radius */
.rinch-close-button--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-close-button--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-close-button--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-close-button--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-close-button--radius-xl { border-radius: var(--rinch-radius-xl); }
"#
    .to_string()
}
