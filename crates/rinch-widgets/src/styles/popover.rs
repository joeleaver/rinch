pub fn styles() -> String {
    r#"
/* Popover base */
.rinch-popover {
    position: relative;
    display: inline-block;
}

/* Target */
.rinch-popover__target {
    display: inline-block;
}

/* Dropdown */
.rinch-popover__dropdown {
    position: absolute;
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    box-shadow: var(--rinch-shadow-md);
    padding: var(--rinch-spacing-sm);
    width: var(--rinch-popover-width, auto);
    min-width: 150px;
    z-index: 100;
    opacity: 0;
    visibility: hidden;
    transition: opacity 150ms ease, visibility 150ms ease, transform 150ms ease;
}

.rinch-popover--opened .rinch-popover__dropdown {
    opacity: 1;
    visibility: visible;
}

/* Positions */
.rinch-popover--bottom .rinch-popover__dropdown {
    top: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(var(--rinch-popover-offset, 8px));
}

.rinch-popover--bottom-start .rinch-popover__dropdown {
    top: 100%;
    left: 0;
    transform: translateY(var(--rinch-popover-offset, 8px));
}

.rinch-popover--bottom-end .rinch-popover__dropdown {
    top: 100%;
    right: 0;
    transform: translateY(var(--rinch-popover-offset, 8px));
}

.rinch-popover--top .rinch-popover__dropdown {
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(calc(-1 * var(--rinch-popover-offset, 8px)));
}

.rinch-popover--top-start .rinch-popover__dropdown {
    bottom: 100%;
    left: 0;
    transform: translateY(calc(-1 * var(--rinch-popover-offset, 8px)));
}

.rinch-popover--top-end .rinch-popover__dropdown {
    bottom: 100%;
    right: 0;
    transform: translateY(calc(-1 * var(--rinch-popover-offset, 8px)));
}

.rinch-popover--left .rinch-popover__dropdown {
    right: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(calc(-1 * var(--rinch-popover-offset, 8px)));
}

.rinch-popover--left-start .rinch-popover__dropdown {
    right: 100%;
    top: 0;
    transform: translateX(calc(-1 * var(--rinch-popover-offset, 8px)));
}

.rinch-popover--left-end .rinch-popover__dropdown {
    right: 100%;
    bottom: 0;
    transform: translateX(calc(-1 * var(--rinch-popover-offset, 8px)));
}

.rinch-popover--right .rinch-popover__dropdown {
    left: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(var(--rinch-popover-offset, 8px));
}

.rinch-popover--right-start .rinch-popover__dropdown {
    left: 100%;
    top: 0;
    transform: translateX(var(--rinch-popover-offset, 8px));
}

.rinch-popover--right-end .rinch-popover__dropdown {
    left: 100%;
    bottom: 0;
    transform: translateX(var(--rinch-popover-offset, 8px));
}

/* Arrow */
.rinch-popover--with-arrow .rinch-popover__dropdown::before {
    content: '';
    position: absolute;
    width: var(--rinch-popover-arrow-size, 8px);
    height: var(--rinch-popover-arrow-size, 8px);
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    transform: rotate(45deg);
}

.rinch-popover--with-arrow.rinch-popover--bottom .rinch-popover__dropdown::before,
.rinch-popover--with-arrow.rinch-popover--bottom-start .rinch-popover__dropdown::before,
.rinch-popover--with-arrow.rinch-popover--bottom-end .rinch-popover__dropdown::before {
    top: calc(-1 * var(--rinch-popover-arrow-size, 8px) / 2);
    left: var(--rinch-popover-arrow-offset, 50%);
    border-right: none;
    border-bottom: none;
}

.rinch-popover--with-arrow.rinch-popover--top .rinch-popover__dropdown::before,
.rinch-popover--with-arrow.rinch-popover--top-start .rinch-popover__dropdown::before,
.rinch-popover--with-arrow.rinch-popover--top-end .rinch-popover__dropdown::before {
    bottom: calc(-1 * var(--rinch-popover-arrow-size, 8px) / 2);
    left: var(--rinch-popover-arrow-offset, 50%);
    border-left: none;
    border-top: none;
}

/* Radius */
.rinch-popover--radius-xs .rinch-popover__dropdown { border-radius: var(--rinch-radius-xs); }
.rinch-popover--radius-sm .rinch-popover__dropdown { border-radius: var(--rinch-radius-sm); }
.rinch-popover--radius-md .rinch-popover__dropdown { border-radius: var(--rinch-radius-md); }
.rinch-popover--radius-lg .rinch-popover__dropdown { border-radius: var(--rinch-radius-lg); }
.rinch-popover--radius-xl .rinch-popover__dropdown { border-radius: var(--rinch-radius-xl); }

/* Shadow */
.rinch-popover--shadow-xs .rinch-popover__dropdown { box-shadow: var(--rinch-shadow-xs); }
.rinch-popover--shadow-sm .rinch-popover__dropdown { box-shadow: var(--rinch-shadow-sm); }
.rinch-popover--shadow-md .rinch-popover__dropdown { box-shadow: var(--rinch-shadow-md); }
.rinch-popover--shadow-lg .rinch-popover__dropdown { box-shadow: var(--rinch-shadow-lg); }
.rinch-popover--shadow-xl .rinch-popover__dropdown { box-shadow: var(--rinch-shadow-xl); }
"#.to_string()
}
