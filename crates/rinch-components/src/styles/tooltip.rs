pub fn styles() -> String {
    r#"
/* Tooltip base */
.rinch-tooltip {
    position: relative;
    display: inline-block;
}

/* Tooltip target */
.rinch-tooltip__target {
    display: inline-block;
}

/* Tooltip content.

   Hidden here and shown by `.rinch-tooltip--opened` below, which is the class
   `Tooltip`'s hover effect toggles on the root. Until #760 the class matched
   nothing and the reveal was an inline `style` rewrite on this node instead;
   the class was emitted all along, so anyone who found it in the class list and
   styled it got silence.

   `display` rather than `visibility`/`opacity` because nothing here declares a
   `transition`: css-transitions-1 §3 starts nothing for a property that
   retargets in the same pass its subtree stops being `display: none`, so the
   day this sheet grows a fade the content has to stay rendered first, the way
   `styles/popover.rs` does it. */
.rinch-tooltip__content {
    position: absolute;
    z-index: 1000;
    padding: 0.375rem 0.625rem;
    background-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
    color: white;
    font-size: var(--rinch-font-size-xs);
    border-radius: var(--rinch-radius-sm);
    white-space: nowrap;
    pointer-events: none;
    display: none;
}

.rinch-tooltip--opened .rinch-tooltip__content {
    display: block;
    /* Redundant against this sheet, which never sets `opacity` on the content —
       and kept anyway, because the inline write this rule replaced carried it
       and so outranked a caller's own `opacity` on `.rinch-tooltip__content`.
       Dropping it would change that caller's tooltip from opaque to whatever
       they set. It is also where a fade would start from, if one is ever added
       (see the note on the base rule for what that would take). */
    opacity: 1;
}

/* Tooltip positions */
.rinch-tooltip--top .rinch-tooltip__content {
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(-0.25rem);
    margin-bottom: 0.375rem;
}

.rinch-tooltip--bottom .rinch-tooltip__content {
    top: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(0.25rem);
    margin-top: 0.375rem;
}

.rinch-tooltip--left .rinch-tooltip__content {
    right: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(-0.25rem);
    margin-right: 0.375rem;
}

.rinch-tooltip--right .rinch-tooltip__content {
    left: 100%;
    top: 50%;
    transform: translateY(-50%) translateX(0.25rem);
    margin-left: 0.375rem;
}

/* Tooltip with arrow */
.rinch-tooltip--with-arrow .rinch-tooltip__content::after {
    content: '';
    position: absolute;
    border: 0.375rem solid transparent;
}

.rinch-tooltip--with-arrow.rinch-tooltip--top .rinch-tooltip__content::after {
    top: 100%;
    left: 50%;
    transform: translateX(-50%);
    border-top-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

.rinch-tooltip--with-arrow.rinch-tooltip--bottom .rinch-tooltip__content::after {
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%);
    border-bottom-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

.rinch-tooltip--with-arrow.rinch-tooltip--left .rinch-tooltip__content::after {
    left: 100%;
    top: 50%;
    transform: translateY(-50%);
    border-left-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

.rinch-tooltip--with-arrow.rinch-tooltip--right .rinch-tooltip__content::after {
    right: 100%;
    top: 50%;
    transform: translateY(-50%);
    border-right-color: var(--rinch-tooltip-color, var(--rinch-color-gray-9));
}

/* Tooltip multiline */
.rinch-tooltip--multiline .rinch-tooltip__content {
    white-space: normal;
    width: var(--rinch-tooltip-width, 200px);
    text-align: center;
}

/* Tooltip disabled.

   Ties with `.rinch-tooltip--opened .rinch-tooltip__content` on specificity, so
   it has to stay **after** it in this file to win. (The hover handlers refuse to
   set `hovered` on a disabled tooltip anyway, so the pair only meets when a
   caller passes `opened: true` and `disabled: true` together.) */
.rinch-tooltip--disabled .rinch-tooltip__content {
    display: none;
}
"#
    .to_string()
}
