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

/* #1339: this used to be a plain descendant rule, `.rinch-hover-card:hover
   .rinch-hover-card__dropdown` — the same shape as #778's Popover bug (and
   the pre-#774 DropdownMenu/Tooltip bugs), but reached through `:hover`
   rather than a toggled class. `:hover` propagates to every ancestor of
   whatever the pointer is physically over, and a descendant combinator
   matches through ANY ancestor carrying it — so a closed `HoverCard` nested
   inside an open (hovered) one's dropdown had a `.rinch-hover-card` ancestor
   matching `:hover` (the outer card), and its own `__dropdown` is a
   descendant of that ancestor, so the selector matched it too, even though
   the inner card's own root was never hovered.

   The fix is the same `:not()`-exclusion pattern #778 applies to Popover
   (`.rinch-popover--opened .rinch-popover__dropdown:not(…)`), respelled for
   a pseudo-class instead of a toggled class: `:not(:hover)` stands in for
   "closed" since there is no class to negate. Reveal a dropdown under a
   hovered ancestor UNLESS a *closed* `.rinch-hover-card` root sits between
   that ancestor and the dropdown. For the outer card's own dropdown there is
   no such intervening root, so it still opens; for a closed card nested
   inside an open one's dropdown, its own root is exactly that intervening
   closed root, so its dropdown stays hidden. Same known limit as the
   Popover/DropdownMenu fixes (an open card C inside the target of a closed
   card B inside an open card A stays hidden, because B sits between A and
   C's panel) — not separately measured here since the mechanism is
   identical. */
.rinch-hover-card:hover .rinch-hover-card__dropdown:not(.rinch-hover-card:hover .rinch-hover-card:not(:hover) .rinch-hover-card__dropdown) {
    opacity: 1;
    visibility: visible;
    transition-delay: var(--rinch-hover-card-open-delay, 0ms);
}

/* Keeps the panel open while the pointer moves from the target onto the
   panel itself. This needs NO matching `:not()` exclusion (#1339): for this
   selector to match a dropdown `D` at all, `D` itself (or something inside
   it) must be the genuinely-hovered node — and hovering marks `D`'s *entire*
   ancestor chain `:hover`, `D`'s own direct parent card included, with zero
   gap. So whenever this rule matches `D`, `D`'s own card is unconditionally
   open too, which can never be the "closed card between a hovered ancestor
   and the dropdown" the rule above excludes: matching this selector already
   proves the card is open, so it cannot leak a closed nested card's panel. */
.rinch-hover-card__dropdown:hover {
    opacity: 1;
    visibility: visible;
}

/* Paused while hidden (#912). Hidden is the card's resting state, and a hidden
   dropdown is `visibility: hidden`, which is rendered — so an animation inside
   it ran, and kept the app awake, for as long as nobody hovered. The note on
   `.rinch-drawer__root--hidden *` in `styles/drawer.rs` has the reasoning, the
   `!important`, the `*`, and why no pseudo-elements. The `:not()` is the exact
   complement of the widened reveal rule above (#1339 widened both together,
   as #778 does for Popover): the dropdown is a descendant of the card, so a
   hovered dropdown is inside a hovered card and the second rule above needs
   no clause of its own here either, by the same proof. The pause lands as the
   close delay starts, so in a browser the spinner stops while the card is
   still fading out — a closed card nested in an open one's dropdown included. */
.rinch-hover-card__dropdown:not(.rinch-hover-card:hover .rinch-hover-card__dropdown:not(.rinch-hover-card:hover .rinch-hover-card:not(:hover) .rinch-hover-card__dropdown)) * {
    animation-play-state: paused !important;
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
