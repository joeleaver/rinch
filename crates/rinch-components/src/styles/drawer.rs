pub fn styles() -> String {
    r#"
/* Drawer root.
   `top` clears any window chrome rinch draws itself (the in-app menu bar, the
   BorderlessWindow titlebar). Those reserve space with in-document padding,
   which a fixed element correctly ignores — so without this the drawer slides
   up underneath them. The fallback is 0px, so a plain window is unaffected.
   Everything inside the root is absolute, so this shifts the whole drawer. */
.rinch-drawer__root {
    position: fixed;
    top: var(--rinch-window-top-inset, 0px);
    left: 0;
    right: 0;
    bottom: 0;
    /* #474: see the note in `styles::modal` — `z_index` publishes
       --rinch-drawer-z-index here and the panel follows one above. */
    z-index: var(--rinch-drawer-z-index, 200);
}

/* Hidden state (#751).

   `visibility`, NOT `display`. The panel below carries `transition: transform
   300ms ease`, and the component's one effect removes this class and adds the
   panel's `--opened` class in a single batch — so the panel's `transform`
   retargets in the very pass that its ancestor stops being hidden. Under
   `display: none` css-transitions-1 §3 starts nothing there: an element that
   was not being rendered has no before-change style to animate from, so the
   panel snapped to its open position. A browser refuses it for the same reason
   — which is why `@starting-style` and `transition-behavior: allow-discrete`
   exist — so the Drawer had never animated on rinch-web either.

   A `visibility: hidden` element **is** being rendered: it keeps its box, it
   transitions, and it is still excluded from paint, from hit testing and from
   the Tab order on both backends. `Popover` reaches the same place from the
   other side (`styles/popover.rs`) — it has never touched `display` on the
   panel it animates.

   The close slides out too (#759, #413). The hidden state holds the root
   `visible` for the panel's 300ms and only then hides it — `visibility 0s
   linear 300ms`, a zero-length transition after a 300ms delay, which is the
   canonical spelling: a browser keeps the old value through a transition's
   delay, and css-values-4 interpolates `visibility` so that nothing strictly
   inside a transition with a `visible` end is hidden. rinch-dom implements
   both, and hands the held value down to the panel and everything else that
   inherits it, so the close is the same on both backends.

   Only the hidden state declares it. Opening is instant — the root has no
   `visibility` transition to run — and reopening before the 300ms are up
   cancels the pending hide (css-transitions-1 §3 item 3: the open state's
   `transition-property` no longer matches `visibility`), so the panel simply
   reverses.

   While it slides out the root is visible, so for those 300ms it paints, takes
   clicks and is in the Tab order, exactly as in a browser. Its focus trap and
   its hold on the page scroll are released at the close itself. And the pause
   below applies from the close, so a spinner inside a closing drawer stops
   where it is and slides out still — also what a browser does with this CSS. */
.rinch-drawer__root--hidden {
    visibility: hidden !important;
    transition: visibility 0s linear 300ms;
}

/* Paused while closed (#912).

   Being rendered has a cost the note above does not: an animation under a
   `visibility: hidden` box runs — in rinch and in a browser alike — and an
   `animation: … infinite` (a `Loader`, a loading `Button`, a `Skeleton`) has no
   duration to expire, so a closed drawer holding one kept the app asking for a
   frame on every turn, forever, for pixels nobody could see. A paused animation
   asks for none (#763), and resumes where it stopped when the drawer opens, as
   a browser keeps its `currentTime`.

   Every descendant, not a list of known spinners: any component or app rule
   can declare an animation. `!important` because the `animation` shorthand
   resets `animation-play-state` to `running`, so without it the pause would
   lose to any shorthand of equal or higher specificity declared after it —
   `.rinch-loader__oval` ties with this selector and `.rinch-button--loading
   .rinch-button__loader` beats it. `animation-play-state` does not inherit,
   hence the `*`.

   Not `*::before` / `*::after`, deliberately, although `animation-play-state`
   does not inherit into a pseudo-element either: on rinch-web a spinner drawn
   by an `::after` under a closed overlay is therefore **not** paused, and
   since #1004 neither is one on desktop, which now runs a pseudo-element's
   animation (#925, #1023). The selectors are not free there: rinch-dom matches `::before` /
   `::after` for every element with no ancestor bloom filter (#935), so two
   rules with a universal rightmost compound cost +10% of style instructions
   on a page with no overlay in it, and +71% under a closed drawer (measured by
   PR #929's review). The `drawer_toggle` bench in `rinch-bench` pins this
   stylesheet's cost. */
.rinch-drawer__root--hidden * {
    animation-play-state: paused !important;
}

/* Drawer overlay — absolute within the fixed root */
.rinch-drawer__overlay {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    /* #474: see the note in `styles::modal` — `overlay_opacity` rides the
       alpha channel, with the old hard-coded value as the fallback. */
    background-color: rgba(0, 0, 0, var(--rinch-drawer-overlay-opacity, 0.75));
}

/* Drawer container */
.rinch-drawer {
    position: absolute;
    background-color: var(--rinch-color-body);
    box-shadow: var(--rinch-shadow-xl);
    display: flex;
    flex-direction: column;
    z-index: calc(var(--rinch-drawer-z-index, 200) + 1);
    transition: transform 300ms ease;
    overflow: hidden;
    /* Use absolute within the fixed root */
    top: 0;
    bottom: 0;
}

/* Position: Left */
.rinch-drawer--left {
    left: 0;
    transform: translateX(-100%);
}

.rinch-drawer--left.rinch-drawer--opened {
    transform: translateX(0);
}

/* Position: Right */
.rinch-drawer--right {
    right: 0;
    left: auto;
    transform: translateX(100%);
}

.rinch-drawer--right.rinch-drawer--opened {
    transform: translateX(0);
}

/* Position: Top */
.rinch-drawer--top {
    top: 0;
    bottom: auto;
    left: 0;
    right: 0;
    transform: translateY(-100%);
}

.rinch-drawer--top.rinch-drawer--opened {
    transform: translateY(0);
}

/* Position: Bottom */
.rinch-drawer--bottom {
    bottom: 0;
    top: auto;
    left: 0;
    right: 0;
    transform: translateY(100%);
}

.rinch-drawer--bottom.rinch-drawer--opened {
    transform: translateY(0);
}

/* Drawer sizes (for left/right) */
.rinch-drawer--left.rinch-drawer--xs,
.rinch-drawer--right.rinch-drawer--xs { width: 280px; }

.rinch-drawer--left.rinch-drawer--sm,
.rinch-drawer--right.rinch-drawer--sm { width: 320px; }

.rinch-drawer--left.rinch-drawer--md,
.rinch-drawer--right.rinch-drawer--md { width: 380px; }

.rinch-drawer--left.rinch-drawer--lg,
.rinch-drawer--right.rinch-drawer--lg { width: 500px; }

.rinch-drawer--left.rinch-drawer--xl,
.rinch-drawer--right.rinch-drawer--xl { width: 680px; }

.rinch-drawer--left.rinch-drawer--full,
.rinch-drawer--right.rinch-drawer--full { width: 100%; }

/* Drawer sizes (for top/bottom) */
.rinch-drawer--top.rinch-drawer--xs,
.rinch-drawer--bottom.rinch-drawer--xs { height: 200px; }

.rinch-drawer--top.rinch-drawer--sm,
.rinch-drawer--bottom.rinch-drawer--sm { height: 280px; }

.rinch-drawer--top.rinch-drawer--md,
.rinch-drawer--bottom.rinch-drawer--md { height: 380px; }

.rinch-drawer--top.rinch-drawer--lg,
.rinch-drawer--bottom.rinch-drawer--lg { height: 500px; }

.rinch-drawer--top.rinch-drawer--xl,
.rinch-drawer--bottom.rinch-drawer--xl { height: 680px; }

.rinch-drawer--top.rinch-drawer--full,
.rinch-drawer--bottom.rinch-drawer--full { height: 100%; }

/* Drawer header */
.rinch-drawer__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--rinch-spacing-sm);
    padding: var(--rinch-spacing-md) var(--rinch-spacing-lg);
    border-bottom: 1px solid var(--rinch-color-border);
    flex-shrink: 0;
}

.rinch-drawer__title {
    margin: 0;
    flex: 1;
    font-size: var(--rinch-font-size-lg);
    font-weight: 600;
    color: var(--rinch-color-text);
}

.rinch-drawer__spacer {
    flex: 1;
}

/* Drawer body */
.rinch-drawer__body {
    flex: 1;
    padding: var(--rinch-spacing-lg);
    overflow-y: auto;
}

/* Drawer close button */
.rinch-drawer__close {
    flex-shrink: 0;
    width: 2rem;
    height: 2rem;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    color: var(--rinch-color-dimmed);
    transition: background-color 150ms ease, color 150ms ease;
}

.rinch-drawer__close:hover {
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
}

.rinch-drawer__close svg {
    width: 1rem;
    height: 1rem;
}
"#
    .to_string()
}
