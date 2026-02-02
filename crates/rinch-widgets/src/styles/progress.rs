pub fn styles() -> String {
    r#"
/* Progress base */
.rinch-progress {
    width: 100%;
    background-color: var(--rinch-color-default);
    border-radius: var(--rinch-radius-default);
    overflow: hidden;
}

/* Progress sizes */
.rinch-progress--xs { height: 0.25rem; }
.rinch-progress--sm { height: 0.5rem; }
.rinch-progress--md { height: 0.75rem; }
.rinch-progress--lg { height: 1rem; }
.rinch-progress--xl { height: 1.5rem; }

/* Progress bar fill */
.rinch-progress__bar {
    height: 100%;
    background-color: var(--rinch-progress-color, var(--rinch-primary-color));
    border-radius: inherit;
    transition: width 200ms ease;
}

/* Striped pattern */
.rinch-progress__bar--striped {
    background-image: linear-gradient(
        45deg,
        rgba(255, 255, 255, 0.15) 25%,
        transparent 25%,
        transparent 50%,
        rgba(255, 255, 255, 0.15) 50%,
        rgba(255, 255, 255, 0.15) 75%,
        transparent 75%,
        transparent
    );
    background-size: 1rem 1rem;
}

/* Animated stripes */
.rinch-progress__bar--animated {
    animation: rinch-progress-stripes 1s linear infinite;
}

@keyframes rinch-progress-stripes {
    from { background-position: 1rem 0; }
    to { background-position: 0 0; }
}

/* Progress radius */
.rinch-progress--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-progress--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-progress--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-progress--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-progress--radius-xl { border-radius: var(--rinch-radius-xl); }
"#.to_string()
}
