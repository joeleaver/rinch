pub fn styles() -> String {
    r#"
/* HoverCard base */
.rinch-hover-card {
    position: relative;
    display: inline-block;
}

/* Target */
.rinch-hover-card__target {
    display: inline-block;
}

/* Dropdown */
.rinch-hover-card__dropdown {
    position: absolute;
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    box-shadow: var(--rinch-shadow-md);
    padding: var(--rinch-spacing-md);
    width: var(--rinch-hover-card-width, 280px);
    z-index: 100;
    opacity: 0;
    visibility: hidden;
    transition: opacity var(--rinch-hover-card-open-delay, 150ms) ease,
                visibility var(--rinch-hover-card-open-delay, 150ms) ease;
}

.rinch-hover-card:hover .rinch-hover-card__dropdown {
    opacity: 1;
    visibility: visible;
    transition-delay: var(--rinch-hover-card-open-delay, 0ms);
}

.rinch-hover-card__dropdown:hover {
    opacity: 1;
    visibility: visible;
}

/* Keep dropdown visible while moving to it */
.rinch-hover-card:not(:hover) .rinch-hover-card__dropdown {
    transition-delay: var(--rinch-hover-card-close-delay, 150ms);
}

/* Positions */
.rinch-hover-card--bottom .rinch-hover-card__dropdown {
    top: 100%;
    left: 50%;
    transform: translateX(-50%);
    margin-top: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--bottom-start .rinch-hover-card__dropdown {
    top: 100%;
    left: 0;
    margin-top: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--bottom-end .rinch-hover-card__dropdown {
    top: 100%;
    right: 0;
    margin-top: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--top .rinch-hover-card__dropdown {
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%);
    margin-bottom: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--top-start .rinch-hover-card__dropdown {
    bottom: 100%;
    left: 0;
    margin-bottom: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--top-end .rinch-hover-card__dropdown {
    bottom: 100%;
    right: 0;
    margin-bottom: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--left .rinch-hover-card__dropdown {
    right: 100%;
    top: 50%;
    transform: translateY(-50%);
    margin-right: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--left-start .rinch-hover-card__dropdown {
    right: 100%;
    top: 0;
    margin-right: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--left-end .rinch-hover-card__dropdown {
    right: 100%;
    bottom: 0;
    margin-right: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--right .rinch-hover-card__dropdown {
    left: 100%;
    top: 50%;
    transform: translateY(-50%);
    margin-left: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--right-start .rinch-hover-card__dropdown {
    left: 100%;
    top: 0;
    margin-left: var(--rinch-hover-card-offset, 8px);
}

.rinch-hover-card--right-end .rinch-hover-card__dropdown {
    left: 100%;
    bottom: 0;
    margin-left: var(--rinch-hover-card-offset, 8px);
}

/* Radius */
.rinch-hover-card--radius-xs .rinch-hover-card__dropdown { border-radius: var(--rinch-radius-xs); }
.rinch-hover-card--radius-sm .rinch-hover-card__dropdown { border-radius: var(--rinch-radius-sm); }
.rinch-hover-card--radius-md .rinch-hover-card__dropdown { border-radius: var(--rinch-radius-md); }
.rinch-hover-card--radius-lg .rinch-hover-card__dropdown { border-radius: var(--rinch-radius-lg); }
.rinch-hover-card--radius-xl .rinch-hover-card__dropdown { border-radius: var(--rinch-radius-xl); }

/* Shadow */
.rinch-hover-card--shadow-xs .rinch-hover-card__dropdown { box-shadow: var(--rinch-shadow-xs); }
.rinch-hover-card--shadow-sm .rinch-hover-card__dropdown { box-shadow: var(--rinch-shadow-sm); }
.rinch-hover-card--shadow-md .rinch-hover-card__dropdown { box-shadow: var(--rinch-shadow-md); }
.rinch-hover-card--shadow-lg .rinch-hover-card__dropdown { box-shadow: var(--rinch-shadow-lg); }
.rinch-hover-card--shadow-xl .rinch-hover-card__dropdown { box-shadow: var(--rinch-shadow-xl); }
"#
    .to_string()
}
