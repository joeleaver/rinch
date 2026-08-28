pub fn styles() -> String {
    r#"
/* Dropdown menu base */
.rinch-dropdown-menu {
    position: relative;
    display: inline-block;
}

/* Target */
.rinch-dropdown-menu__target {
    display: inline-block;
}

/* Dropdown — visibility controlled by inline styles (Stylo limitation) */
.rinch-dropdown-menu__dropdown {
    position: absolute;
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border, var(--rinch-color-gray-3));
    border-radius: var(--rinch-radius-md);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    padding: var(--rinch-spacing-xs);
    width: var(--rinch-dropdown-menu-width, auto);
    min-width: 160px;
    z-index: 100;
}

/* Positions - default to bottom-start for better UX */
.rinch-dropdown-menu--bottom .rinch-dropdown-menu__dropdown,
.rinch-dropdown-menu--bottom-start .rinch-dropdown-menu__dropdown {
    top: 100%;
    left: 0;
    margin-top: var(--rinch-dropdown-menu-offset, 4px);
}

.rinch-dropdown-menu--bottom-end .rinch-dropdown-menu__dropdown {
    top: 100%;
    right: 0;
    margin-top: var(--rinch-dropdown-menu-offset, 4px);
}

.rinch-dropdown-menu--top .rinch-dropdown-menu__dropdown,
.rinch-dropdown-menu--top-start .rinch-dropdown-menu__dropdown {
    bottom: 100%;
    left: 0;
    margin-bottom: var(--rinch-dropdown-menu-offset, 4px);
}

.rinch-dropdown-menu--top-end .rinch-dropdown-menu__dropdown {
    bottom: 100%;
    right: 0;
    margin-bottom: var(--rinch-dropdown-menu-offset, 4px);
}

/* Menu item */
.rinch-dropdown-menu__item {
    display: flex;
    align-items: center;
    gap: var(--rinch-spacing-sm);
    width: 100%;
    padding: 0.625rem 0.875rem;
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-dropdown-menu-item-color, var(--rinch-color-text));
    background: transparent;
    border: none;
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    text-align: left;
    transition: background-color 100ms ease;
}

.rinch-dropdown-menu__item:hover:not(:disabled) {
    background-color: var(--rinch-color-default);
}

.rinch-dropdown-menu__item--disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

/* Item sections */
.rinch-dropdown-menu__item-left,
.rinch-dropdown-menu__item-right {
    display: flex;
    align-items: center;
    flex-shrink: 0;
}

.rinch-dropdown-menu__item-left svg,
.rinch-dropdown-menu__item-right svg {
    width: 1rem;
    height: 1rem;
}

.rinch-dropdown-menu__item-label {
    flex: 1;
}

.rinch-dropdown-menu__item-right {
    margin-left: auto;
    color: var(--rinch-color-dimmed);
    font-size: var(--rinch-font-size-xs);
}

/* Menu label */
.rinch-dropdown-menu__label {
    padding: 0.625rem 0.875rem;
    font-size: var(--rinch-font-size-xs);
    font-weight: 500;
    color: var(--rinch-color-dimmed);
    text-transform: uppercase;
    letter-spacing: 0.05em;
}

/* Menu divider */
.rinch-dropdown-menu__divider {
    height: 1px;
    background-color: var(--rinch-color-border);
    margin: var(--rinch-spacing-xs) 0;
}

/* Backdrop — the invisible overlay that catches outside clicks when
   close_on_click_outside is true. Its z-index sits below the dropdown panel's
   (100) so that a click on an item still lands on the item.

   It is `position: absolute`, and it has to be. A `position: fixed` box is
   viewport-level content in Rinch: it is hoisted out of every ancestor clip,
   and — because Rinch makes an overflow clip a stacking context — out of every
   ancestor stacking context along with it. A fixed backdrop is therefore above
   everything that is not itself fixed, whatever the z-indexes say, because the
   99 and the 100 are then being compared across two stacking contexts, which
   is to say not compared at all. Behind any `overflow` ancestor — every scroll
   container, and most app roots — a fixed backdrop covered the panel and
   swallowed every tap on the menu: the menu closed and the item never ran.

   Absolute puts the backdrop back in the panel's own stacking context, where
   the two z-indexes are comparable and the panel wins. The insets are what a
   viewport-covering box costs once it can no longer be viewport-positioned:
   100vw/100vh in each direction from a root that is on screen by construction
   (the menu is open, so its target is visible) covers the viewport at any
   scroll position. The price is that the backdrop is now clipped by whatever
   clips the panel, so a click beyond *that* box does not dismiss — the popup
   and its dismiss region share one clip, which is the trade a fixed backdrop
   was never able to make in the other direction. */
.rinch-dropdown-menu__backdrop {
    position: absolute;
    top: -100vh;
    right: -100vw;
    bottom: -100vh;
    left: -100vw;
    z-index: 99;
    display: none;
}

/* Radius */
.rinch-dropdown-menu--radius-xs .rinch-dropdown-menu__dropdown { border-radius: var(--rinch-radius-xs); }
.rinch-dropdown-menu--radius-sm .rinch-dropdown-menu__dropdown { border-radius: var(--rinch-radius-sm); }
.rinch-dropdown-menu--radius-md .rinch-dropdown-menu__dropdown { border-radius: var(--rinch-radius-md); }
.rinch-dropdown-menu--radius-lg .rinch-dropdown-menu__dropdown { border-radius: var(--rinch-radius-lg); }
.rinch-dropdown-menu--radius-xl .rinch-dropdown-menu__dropdown { border-radius: var(--rinch-radius-xl); }

/* Shadow */
.rinch-dropdown-menu--shadow-xs .rinch-dropdown-menu__dropdown { box-shadow: var(--rinch-shadow-xs); }
.rinch-dropdown-menu--shadow-sm .rinch-dropdown-menu__dropdown { box-shadow: var(--rinch-shadow-sm); }
.rinch-dropdown-menu--shadow-md .rinch-dropdown-menu__dropdown { box-shadow: var(--rinch-shadow-md); }
.rinch-dropdown-menu--shadow-lg .rinch-dropdown-menu__dropdown { box-shadow: var(--rinch-shadow-lg); }
.rinch-dropdown-menu--shadow-xl .rinch-dropdown-menu__dropdown { box-shadow: var(--rinch-shadow-xl); }
"#.to_string()
}
