pub fn styles() -> String {
    r#"
/* Tabs base */
.rinch-tabs {
    display: flex;
    flex-direction: column;
}

.rinch-tabs--vertical {
    flex-direction: row;
}

.rinch-tabs--position-bottom {
    flex-direction: column-reverse;
}

.rinch-tabs--position-right {
    flex-direction: row-reverse;
}

/* Tabs list */
.rinch-tabs__list {
    display: flex;
    flex-wrap: wrap;
    gap: 0;
    border-bottom: 2px solid var(--rinch-color-border);
}

.rinch-tabs--vertical .rinch-tabs__list {
    flex-direction: column;
    border-bottom: none;
    border-right: 2px solid var(--rinch-color-border);
}

.rinch-tabs--position-bottom .rinch-tabs__list {
    border-bottom: none;
    border-top: 2px solid var(--rinch-color-border);
}

.rinch-tabs--position-right .rinch-tabs__list {
    border-right: none;
    border-left: 2px solid var(--rinch-color-border);
}

.rinch-tabs__list--grow {
    justify-content: stretch;
}

.rinch-tabs__list--grow .rinch-tabs__tab {
    flex: 1;
}

/* Tab button */
.rinch-tabs__tab {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-dimmed);
    background: transparent;
    border: none;
    cursor: pointer;
    position: relative;
    transition: color 150ms ease, background-color 150ms ease;
}

.rinch-tabs__tab:hover {
    color: var(--rinch-color-text);
    background-color: var(--rinch-color-filled);
}

/* The active tab. `Tabs::render` sets `data-active="true"` and adds
   `--active` together (#760); it used to set neither and write the colour
   inline on the label span, which is why `transition: color` above never ran.
   Both hooks are live, so either can be styled by a caller. */
.rinch-tabs__tab[data-active="true"],
.rinch-tabs__tab--active {
    color: var(--rinch-tabs-color, var(--rinch-primary-color));
}

.rinch-tabs__tab--disabled {
    opacity: 0.5;
    cursor: not-allowed;
    pointer-events: none;
}

/* Default variant — underline indicator.

   A real element, `.rinch-tabs__tab-indicator`, appended by `Tabs::render` to
   each tab button in the `default` variant. These three rules were written
   against `.rinch-tabs__tab::after` until #760, and the underline did not
   animate on either backend — for two different reasons, which is why one
   element is the cure for both.

   On desktop the pseudo-element was never created at all: rinch materialises
   none whose `content` computes to the empty string, and `content: ''` is the
   only spelling a decorative box wants (Refs #773 for that gap, which is not
   specific to Tabs). In a browser it *was* created, and was simply never given
   `data-active` — nothing set that hook — so it stayed `transparent` under the
   opaque `<div>` the component drew for itself with inline styles. Either way
   the `transition` declared here had nothing to interpolate, which is the one
   user-visible half of #760. */
.rinch-tabs--default .rinch-tabs__tab-indicator {
    position: absolute;
    bottom: -2px;
    left: 0;
    right: 0;
    height: 2px;
    background-color: transparent;
    transition: background-color 150ms ease;
}

.rinch-tabs--default .rinch-tabs__tab[data-active="true"] .rinch-tabs__tab-indicator,
.rinch-tabs--default .rinch-tabs__tab--active .rinch-tabs__tab-indicator {
    background-color: var(--rinch-tabs-color, var(--rinch-primary-color));
}

/* The variant- and orientation-specific rules that key off the active hook, or
   that place the indicator, name the path from the root with child combinators:
   `.rinch-tabs--X > .rinch-tabs__list > .rinch-tabs__tab`. A `Tabs` nested in
   another's panel is inside that root as well, so a descendant rule would give
   a nested `default` tab the outer `pills` fill, or a nested horizontal
   underline the outer vertical side bar — rules that could leak nowhere before
   #760, when the hook they key off was never set (review of #774). That path is
   the one `Tabs::render` walks to find a tab to wire — a `TabsList` that is a
   direct child of the root, and its direct children — so every tab it marks
   active is still matched. (The non-active variant rules — border
   shape, list border — are still descendant and still reach nested Tabs; that
   is older than #760.) */
.rinch-tabs--vertical.rinch-tabs--default > .rinch-tabs__list > .rinch-tabs__tab > .rinch-tabs__tab-indicator {
    bottom: auto;
    left: auto;
    right: -2px;
    top: 0;
    width: 2px;
    height: 100%;
}

/* Outline variant */
.rinch-tabs--outline .rinch-tabs__list {
    border-bottom: 1px solid var(--rinch-color-border);
}

.rinch-tabs--outline .rinch-tabs__tab {
    border: 1px solid transparent;
    border-bottom: none;
    margin-bottom: -1px;
    border-radius: var(--rinch-radius-sm) var(--rinch-radius-sm) 0 0;
}

.rinch-tabs--outline > .rinch-tabs__list > .rinch-tabs__tab[data-active="true"],
.rinch-tabs--outline > .rinch-tabs__list > .rinch-tabs__tab--active {
    border-color: var(--rinch-color-border);
    background-color: var(--rinch-color-body);
}

/* Pills variant */
.rinch-tabs--pills .rinch-tabs__list {
    border: none;
    gap: 0.25rem;
}

.rinch-tabs--pills .rinch-tabs__tab {
    border-radius: var(--rinch-radius-sm);
}

.rinch-tabs--pills > .rinch-tabs__list > .rinch-tabs__tab[data-active="true"],
.rinch-tabs--pills > .rinch-tabs__list > .rinch-tabs__tab--active {
    background-color: var(--rinch-tabs-color, var(--rinch-primary-color));
    color: white;
}

/* Tab sections */
.rinch-tabs__tab-left,
.rinch-tabs__tab-right {
    display: flex;
    align-items: center;
}

.rinch-tabs__tab-left svg,
.rinch-tabs__tab-right svg {
    width: 1rem;
    height: 1rem;
}

/* Tabs panel */
.rinch-tabs__panel {
    padding: var(--rinch-spacing-md) 0;
}

/* The inactive panel. `Tabs::render` sets and removes `hidden` by presence
   (#760); it used to write an inline `display` instead and this rule matched
   nothing. */
.rinch-tabs__panel[hidden] {
    display: none;
}

/* Grow modifier */
.rinch-tabs--grow .rinch-tabs__tab {
    flex: 1;
}

/* Tabs radius (#474) — the tab buttons are what has corners. `outline` tabs sit
   on the panel's top edge and keep their bottom corners square; that rule
   carries three classes to the plain one's two, so it wins whatever the order. */
.rinch-tabs--radius-xs .rinch-tabs__tab { border-radius: var(--rinch-radius-xs); }
.rinch-tabs--radius-sm .rinch-tabs__tab { border-radius: var(--rinch-radius-sm); }
.rinch-tabs--radius-md .rinch-tabs__tab { border-radius: var(--rinch-radius-md); }
.rinch-tabs--radius-lg .rinch-tabs__tab { border-radius: var(--rinch-radius-lg); }
.rinch-tabs--radius-xl .rinch-tabs__tab { border-radius: var(--rinch-radius-xl); }
.rinch-tabs--outline.rinch-tabs--radius-xs .rinch-tabs__tab { border-radius: var(--rinch-radius-xs) var(--rinch-radius-xs) 0 0; }
.rinch-tabs--outline.rinch-tabs--radius-sm .rinch-tabs__tab { border-radius: var(--rinch-radius-sm) var(--rinch-radius-sm) 0 0; }
.rinch-tabs--outline.rinch-tabs--radius-md .rinch-tabs__tab { border-radius: var(--rinch-radius-md) var(--rinch-radius-md) 0 0; }
.rinch-tabs--outline.rinch-tabs--radius-lg .rinch-tabs__tab { border-radius: var(--rinch-radius-lg) var(--rinch-radius-lg) 0 0; }
.rinch-tabs--outline.rinch-tabs--radius-xl .rinch-tabs__tab { border-radius: var(--rinch-radius-xl) var(--rinch-radius-xl) 0 0; }
"#
    .to_string()
}
