pub fn styles() -> String {
    r#"
/* Loading overlay base */
.rinch-loading-overlay {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
    opacity: 0;
    visibility: hidden;
    transition: opacity var(--rinch-loading-overlay-transition, 150ms) ease,
                visibility var(--rinch-loading-overlay-transition, 150ms) ease;
}

.rinch-loading-overlay--visible {
    opacity: 1;
    visibility: visible;
}

/* Overlay background - uses CSS variable for dark mode support */
.rinch-loading-overlay__overlay {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: var(--rinch-loading-overlay-bg, var(--rinch-color-body));
    opacity: var(--rinch-loading-overlay-opacity, 0.75);
    backdrop-filter: blur(var(--rinch-loading-overlay-blur, 0));
    border-radius: inherit;
}

/* Loader (reuses loader styles) */
.rinch-loading-overlay .rinch-loader {
    z-index: 1;
    --rinch-loader-color: var(--rinch-loading-overlay-loader-color, var(--rinch-primary-color));
}

/* Radius variants */
.rinch-loading-overlay--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-loading-overlay--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-loading-overlay--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-loading-overlay--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-loading-overlay--radius-xl { border-radius: var(--rinch-radius-xl); }
"#
    .to_string()
}
