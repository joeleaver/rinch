//! Sortable list styles (#2091).
//!
//! Every value an app may want to change is a custom property read with a
//! fallback, never defined here, so it inherits: set it inline on a
//! `SortableList` (or any ancestor) — the one lever a sandboxed plugin guest,
//! which cannot add a stylesheet, has.

pub fn styles() -> String {
    r#"
/* A sortable list lays its items out along its axis. Override with class:
   or style: (a grid, a wrap) — the drag reads positions along the axis. */
.rinch-sortable {
    position: relative;
    display: flex;
    gap: var(--rinch-sortable-gap, var(--rinch-spacing-xs));
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
.rinch-sortable--disabled .rinch-sortable__item--grab,
.rinch-sortable--disabled .rinch-sortable__handle {
    cursor: default;
}
/* What a drag carries fades; a keyboard lift is ringed. */
.rinch-sortable__item--dragging {
    opacity: var(--rinch-sortable-dragging-opacity, 0.4);
}
.rinch-sortable__item--lifted {
    outline: 2px solid var(--rinch-sortable-caret-color, var(--rinch-primary-color));
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
    height: var(--rinch-sortable-caret-size, 2px);
    border-radius: 1px;
    background-color: var(--rinch-sortable-caret-color, var(--rinch-primary-color));
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
    width: var(--rinch-sortable-caret-size, 2px);
}
.rinch-sortable__item--drop-horizontal.rinch-sortable__item--drop-before::after {
    left: 0;
}
.rinch-sortable__item--drop-horizontal.rinch-sortable__item--drop-after::after {
    right: 0;
}
.rinch-sortable__item--refused::after {
    background-color: var(--rinch-sortable-refused-color, var(--rinch-color-red-6));
}
/* A drop into an empty list outlines the list. */
.rinch-sortable--drop-into {
    outline: var(--rinch-sortable-caret-size, 2px) dashed
        var(--rinch-sortable-caret-color, var(--rinch-primary-color));
    outline-offset: -2px;
}
.rinch-sortable--drop-into.rinch-sortable--refused {
    outline-color: var(--rinch-sortable-refused-color, var(--rinch-color-red-6));
}

/* Why a drop is refused, at the caret's edge of the item, inside its box: a
   box hanging outside it would grow a scrolling list's scrollable overflow and
   scroll it. Shown and hidden by an inline `display`; everything else here is
   a --rinch-sortable-reason-* property. */
.rinch-sortable__reason--shown {
    position: absolute;
    right: var(--rinch-sortable-reason-inset, 4px);
    bottom: var(--rinch-sortable-reason-inset, 4px);
    z-index: 3;
    pointer-events: none;
    max-width: var(--rinch-sortable-reason-max-width, calc(100% - 8px));
    padding: var(--rinch-sortable-reason-padding, 2px 6px);
    border-radius: var(--rinch-sortable-reason-radius, var(--rinch-radius-sm));
    background-color: var(
        --rinch-sortable-reason-bg,
        var(--rinch-sortable-refused-color, var(--rinch-color-red-6))
    );
    color: var(--rinch-sortable-reason-color, var(--rinch-color-white, #ffffff));
    font-size: var(--rinch-sortable-reason-font-size, var(--rinch-font-size-xs));
    line-height: 1.4;
    white-space: var(--rinch-sortable-reason-white-space, nowrap);
    overflow: hidden;
    text-overflow: ellipsis;
}
.rinch-sortable__reason--above {
    bottom: auto;
    top: var(--rinch-sortable-reason-inset, 4px);
}
.rinch-sortable > .rinch-sortable__reason--shown {
    bottom: auto;
    top: var(--rinch-sortable-reason-inset, 4px);
}

/* The keyboard flow's announcements: read, not shown (the element carries the
   same rules inline, for a document without this stylesheet). */
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
