pub fn styles() -> String {
    r#"
/* Skeleton base */
.rinch-skeleton {
    background-color: var(--rinch-color-default);
    border-radius: var(--rinch-radius-default);
    min-height: 1rem;
}

/* Skeleton animation */
.rinch-skeleton--animate {
    background: linear-gradient(
        90deg,
        var(--rinch-color-default) 0%,
        var(--rinch-color-filled) 50%,
        var(--rinch-color-default) 100%
    );
    background-size: 200% 100%;
    animation: rinch-skeleton-pulse 1.5s ease-in-out infinite;
}

@keyframes rinch-skeleton-pulse {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
}

/* Skeleton circle */
.rinch-skeleton--circle {
    border-radius: 50%;
}

/* Skeleton radius */
.rinch-skeleton--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-skeleton--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-skeleton--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-skeleton--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-skeleton--radius-xl { border-radius: var(--rinch-radius-xl); }
"#
    .to_string()
}
