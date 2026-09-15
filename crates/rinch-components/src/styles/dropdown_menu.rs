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

/* Dropdown.

   Hidden here and shown by `.rinch-dropdown-menu--opened` below, which is the
   class `DropdownMenu`'s `opened_fn` effect toggles on the root. Until #760 the
   class matched nothing and the reveal was an inline `style` rewrite on every
   child after the first; the class was emitted all along, so anyone who found
   it in the class list and styled it got silence.

   `display` rather than `visibility`/`opacity` because nothing on the panel
   declares a `transition`: css-transitions-1 §3 starts nothing for a property
   that retargets in the same pass its subtree stops being `display: none`, so
   the day this sheet grows a fade the panel has to stay rendered first, the way
   `styles/popover.rs` does it (#751). */
.rinch-dropdown-menu__dropdown {
    display: none;
    position: absolute;
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border, var(--rinch-color-gray-3));
    border-radius: var(--rinch-radius-md);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    padding: var(--rinch-spacing-xs);
    width: var(--rinch-dropdown-menu-width, auto);
    min-width: 160px;
    /* #474: `DropdownMenu { z_index }` publishes --rinch-dropdown-menu-z-index
       on the root; the backdrop below derives its level from the same number,
       so it stays exactly one under the panel however high the pair moves. */
    z-index: var(--rinch-dropdown-menu-z-index, 100);
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
   close_on_click_outside is true. It covers the viewport, and its z-index sits
   below the dropdown panel's (100) so that a click on an item still lands on
   the item.

   `fixed` is what makes "outside" mean the whole window rather than whatever
   clips the panel. A dismiss region has to be at least as large as the region
   the user thinks of as outside the menu, and inside a sidebar, a table cell or
   any panel narrower than the window an absolutely positioned backdrop is not:
   an absolute box IS clipped by an `overflow` ancestor in its containing-block
   chain — CSS, not a rinch quirk, and measured here, where the `absolute`
   backdrop's entry carries exactly one clip and it is the shell's box — so the
   popup and its dismiss region would share one clip. A fixed box's chain is
   empty instead. It also puts the backdrop above the app's own fixed chrome (a
   hand-rolled titlebar has no z-index, so it enters at 0), which is why
   clicking the titlebar dismisses.

   It was `position: absolute; top: -100vh; right: -100vw; bottom: -100vh;
   left: -100vw` between PR #317 and #324's stage C, and that is worth knowing
   because the reason was not geometry. Rinch used to make an overflow clip a
   stacking context, and used to hoist a fixed box out of every ancestor clip
   *and*, with it, every ancestor stacking context — so behind any `overflow`
   ancestor the 99 and the 100 were compared across two contexts, which is to
   say not compared at all, and the backdrop covered the panel and swallowed
   every tap on the menu. Two fixes undid that, not one: #324 stage B took
   `overflow` out of `Node::creates_stacking_context` and gave each hoisted box
   its own clip chain, and **#545** stopped the hoist at the nearest ancestor
   stacking context instead of the body. Stage B alone is enough for a plain
   `overflow: hidden` shell; #545 is what makes it hold under a *real* stacking
   context, which is the shape of every `Modal` (201), `Drawer` (201) and
   `Notification` (300) a popup might be opened inside.

   Pinned in `rinch/src/app/mod.rs`'s `popup_backdrop_hit_tests`, which mounts
   this stylesheet: a tap on an item runs the item, a tap outside the clipping
   shell dismisses, and a tap on the app's own fixed chrome dismisses. The last
   two fail against the `absolute` spelling — they are the measurement of what
   it cost, not a memory of it. */
.rinch-dropdown-menu__backdrop {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: calc(var(--rinch-dropdown-menu-z-index, 100) - 1);
    display: none;
}

/* The open state (#760). One class on the root reveals both boxes, and it is
   the class `class_string` has always emitted. Each of these carries two class
   selectors to the hidden rule's one, so it wins whatever the source order.

   A **child** combinator, not a descendant one: a `DropdownMenu` nested in an
   open one's panel is inside that root too, and a descendant rule would open
   its panel and show its backdrop — which, fixed and hoisted above the outer
   items, then takes every tap meant for them and runs the inner `on_close`
   (review of #774). The panel and the backdrop are direct children of the root
   by construction, so `>` reaches exactly this menu's own. */
.rinch-dropdown-menu--opened > .rinch-dropdown-menu__dropdown,
.rinch-dropdown-menu--opened > .rinch-dropdown-menu__backdrop {
    display: block;
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
