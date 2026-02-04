pub fn styles() -> String {
    r#"
/* Loader base */
.rinch-loader {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--rinch-loader-color, var(--rinch-primary-color));
}

/* Loader sizes */
.rinch-loader--xs { width: 1rem; height: 1rem; }
.rinch-loader--sm { width: 1.25rem; height: 1.25rem; }
.rinch-loader--md { width: 2rem; height: 2rem; }
.rinch-loader--lg { width: 2.5rem; height: 2.5rem; }
.rinch-loader--xl { width: 3rem; height: 3rem; }

/* Oval loader (spinner) - CSS-based spinning circle */
.rinch-loader__oval {
    width: 100%;
    height: 100%;
    border: 3px solid transparent;
    border-top-color: currentColor;
    border-radius: 50%;
    animation: rinch-loader-spin 0.8s linear infinite;
}

/* Size adjustments for oval border */
.rinch-loader--xs .rinch-loader__oval { border-width: 2px; }
.rinch-loader--sm .rinch-loader__oval { border-width: 2px; }
.rinch-loader--lg .rinch-loader__oval { border-width: 4px; }
.rinch-loader--xl .rinch-loader__oval { border-width: 4px; }

@keyframes rinch-loader-spin {
    0% { transform: rotate(0deg); }
    100% { transform: rotate(360deg); }
}

/* SVG variant for oval loader (if used) */
.rinch-loader--oval svg {
    width: 100%;
    height: 100%;
}

/* Bars loader */
.rinch-loader--bars {
    gap: 0.125rem;
}

.rinch-loader__bar {
    width: 0.1875rem;
    height: 100%;
    background-color: currentColor;
    border-radius: 0.125rem;
    animation: rinch-loader-bars 1s ease-in-out infinite;
}

.rinch-loader__bar:nth-child(1) { animation-delay: 0s; }
.rinch-loader__bar:nth-child(2) { animation-delay: 0.1s; }
.rinch-loader__bar:nth-child(3) { animation-delay: 0.2s; }
.rinch-loader__bar:nth-child(4) { animation-delay: 0.3s; }

@keyframes rinch-loader-bars {
    0%, 40%, 100% {
        transform: scaleY(0.4);
        opacity: 0.5;
    }
    20% {
        transform: scaleY(1);
        opacity: 1;
    }
}

/* Dots loader */
.rinch-loader--dots {
    gap: 0.25rem;
}

.rinch-loader__dot {
    width: 25%;
    height: 25%;
    background-color: currentColor;
    border-radius: 50%;
    animation: rinch-loader-dots 1.4s ease-in-out infinite both;
}

.rinch-loader__dot:nth-child(1) { animation-delay: -0.32s; }
.rinch-loader__dot:nth-child(2) { animation-delay: -0.16s; }
.rinch-loader__dot:nth-child(3) { animation-delay: 0s; }

@keyframes rinch-loader-dots {
    0%, 80%, 100% {
        transform: scale(0);
        opacity: 0.5;
    }
    40% {
        transform: scale(1);
        opacity: 1;
    }
}

/* Size adjustments for bars */
.rinch-loader--bars.rinch-loader--xs .rinch-loader__bar { width: 0.125rem; }
.rinch-loader--bars.rinch-loader--sm .rinch-loader__bar { width: 0.125rem; }
.rinch-loader--bars.rinch-loader--lg .rinch-loader__bar { width: 0.25rem; }
.rinch-loader--bars.rinch-loader--xl .rinch-loader__bar { width: 0.3125rem; }
"#
    .to_string()
}
