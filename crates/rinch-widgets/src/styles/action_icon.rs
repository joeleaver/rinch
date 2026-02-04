pub fn styles() -> String {
    r#"
/* ActionIcon base */
.rinch-action-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background-color 150ms ease, border-color 150ms ease;
    padding: 0;
    color: var(--rinch-action-icon-color, var(--rinch-primary-color));
}

/* Icon sizing per button size */
.rinch-action-icon--xs svg { width: 0.75rem; height: 0.75rem; }
.rinch-action-icon--sm svg { width: 0.875rem; height: 0.875rem; }
.rinch-action-icon--md svg { width: 1.125rem; height: 1.125rem; }
.rinch-action-icon--lg svg { width: 1.375rem; height: 1.375rem; }
.rinch-action-icon--xl svg { width: 1.75rem; height: 1.75rem; }

.rinch-action-icon:disabled,
.rinch-action-icon--disabled {
    cursor: not-allowed;
    opacity: 0.6;
    pointer-events: none;
}

/* ActionIcon sizes */
.rinch-action-icon--xs { width: 1.125rem; height: 1.125rem; border-radius: var(--rinch-radius-xs); }
.rinch-action-icon--sm { width: 1.375rem; height: 1.375rem; border-radius: var(--rinch-radius-sm); }
.rinch-action-icon--md { width: 1.75rem; height: 1.75rem; border-radius: var(--rinch-radius-default); }
.rinch-action-icon--lg { width: 2.125rem; height: 2.125rem; border-radius: var(--rinch-radius-default); }
.rinch-action-icon--xl { width: 2.75rem; height: 2.75rem; border-radius: var(--rinch-radius-default); }

/* ActionIcon variants */
.rinch-action-icon--filled {
    background-color: var(--rinch-action-icon-color, var(--rinch-primary-color));
    color: white;
}
.rinch-action-icon--filled:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-7);
}

.rinch-action-icon--light {
    background-color: var(--rinch-primary-color-0);
    color: var(--rinch-action-icon-color, var(--rinch-primary-color-6));
}
.rinch-action-icon--light:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-1);
}

.rinch-action-icon--outline {
    background-color: transparent;
    border-color: var(--rinch-action-icon-color, var(--rinch-primary-color));
}
.rinch-action-icon--outline:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-0);
}

.rinch-action-icon--subtle {
    background-color: transparent;
}
.rinch-action-icon--subtle:hover:not(:disabled) {
    background-color: var(--rinch-primary-color-0);
}

.rinch-action-icon--transparent {
    background-color: transparent;
}
.rinch-action-icon--transparent:hover:not(:disabled) {
    background-color: var(--rinch-color-filled);
}

.rinch-action-icon--default {
    background-color: var(--rinch-color-filled);
    color: var(--rinch-color-text);
    border-color: var(--rinch-color-border);
}
.rinch-action-icon--default:hover:not(:disabled) {
    background-color: var(--rinch-color-default);
}

/* ActionIcon radius */
.rinch-action-icon--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-action-icon--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-action-icon--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-action-icon--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-action-icon--radius-xl { border-radius: var(--rinch-radius-xl); }

/* ActionIcon loading */
.rinch-action-icon--loading {
    pointer-events: none;
}

.rinch-action-icon__loader {
    width: 60%;
    height: 60%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: rinch-action-icon-spin 0.6s linear infinite;
}

@keyframes rinch-action-icon-spin {
    to { transform: rotate(360deg); }
}
"#.to_string()
}
