//! Sortable list styles (#2091).

pub fn styles() -> String {
    r#"
/* A sortable list lays its items out along its axis. Override with class:
   or style: (a grid, a wrap) — the drag reads positions along the axis. */
.rinch-sortable {
    --rinch-sortable-gap: var(--rinch-spacing-xs);
    --rinch-sortable-caret-color: var(--rinch-primary-color);
    --rinch-sortable-refused-color: var(--rinch-color-red-6);
    --rinch-sortable-caret-size: 2px;
    position: relative;
    display: flex;
    gap: var(--rinch-sortable-gap);
}
.rinch-sortable--vertical {
    flex-direction: column;
}
.rinch-sortable--horizontal {
    flex-direction: row;
}

/* Items are positioned: the caret and the reason are drawn against them. */
.rinch-sortable__item {
    position: relative;
}
.rinch-sortable__item--grab,
.rinch-sortable__handle {
    cursor: grab;
}
/* What a drag carries fades; a keyboard lift is ringed. */
.rinch-sortable__item--dragging {
    opacity: 0.4;
}
.rinch-sortable__item--lifted {
    outline: 2px solid var(--rinch-sortable-caret-color);
    outline-offset: 2px;
}

/* The caret: a line along the item's edge, inside its box. A line hanging
   into the gap would be clipped at a scrolling list's edge (before its first
   item) and would grow its scrollable overflow (after its last). */
.rinch-sortable__item--drop-before::after,
.rinch-sortable__item--drop-after::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    height: var(--rinch-sortable-caret-size);
    border-radius: 1px;
    background-color: var(--rinch-sortable-caret-color);
    pointer-events: none;
    z-index: 2;
}
.rinch-sortable__item--drop-before::after {
    top: 0;
}
.rinch-sortable__item--drop-after::after {
    bottom: 0;
}
.rinch-sortable__item--drop-horizontal::after {
    top: 0;
    bottom: 0;
    left: auto;
    right: auto;
    height: auto;
    width: var(--rinch-sortable-caret-size);
}
.rinch-sortable__item--drop-horizontal.rinch-sortable__item--drop-before::after {
    left: 0;
}
.rinch-sortable__item--drop-horizontal.rinch-sortable__item--drop-after::after {
    right: 0;
}
.rinch-sortable__item--refused::after {
    background-color: var(--rinch-sortable-refused-color);
}
/* A drop into an empty list outlines the list. */
.rinch-sortable--drop-into {
    outline: 2px dashed var(--rinch-sortable-caret-color);
    outline-offset: -2px;
}
.rinch-sortable--drop-into.rinch-sortable--refused {
    outline-color: var(--rinch-sortable-refused-color);
}

/* Why a drop is refused, at the caret's edge of the item, inside its box: a
   box hanging outside it would grow a scrolling list's scrollable overflow and
   scroll it. Restyle .rinch-sortable__reason. */
.rinch-sortable__reason {
    display: none;
}
.rinch-sortable__reason--shown {
    display: block;
    position: absolute;
    right: 4px;
    bottom: 4px;
    z-index: 3;
    pointer-events: none;
    padding: 2px 6px;
    border-radius: var(--rinch-radius-sm);
    background-color: var(--rinch-sortable-refused-color);
    color: var(--rinch-color-white, #ffffff);
    font-size: var(--rinch-font-size-xs);
    line-height: 1.4;
    white-space: nowrap;
}
.rinch-sortable__reason--above {
    bottom: auto;
    top: 4px;
}
.rinch-sortable > .rinch-sortable__reason--shown {
    bottom: auto;
    top: 4px;
}

/* The keyboard flow's announcements: read, not shown. */
.rinch-sortable__live {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
}
"#
    .to_string()
}
