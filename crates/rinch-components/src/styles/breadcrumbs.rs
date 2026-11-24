pub fn styles() -> String {
    r#"
/* Breadcrumbs base */
.rinch-breadcrumbs {
    display: inline-block;
}

/* Breadcrumbs list */
.rinch-breadcrumbs__list {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    list-style: none;
    margin: 0;
    padding: 0;
    gap: var(--rinch-breadcrumbs-margin, 0.5rem);
}

/* Breadcrumbs item */
.rinch-breadcrumbs__item {
    display: flex;
    align-items: center;
    gap: var(--rinch-breadcrumbs-margin, 0.5rem);
    font-size: var(--rinch-font-size-sm);
}

/* Separator: a real element appended to every item but the last (#2193).
   `aria-hidden`, so it is not read. Drawn in the dimmed colour, at the item's
   font size; a Tabler icon in it is 1em. */
.rinch-breadcrumbs__separator {
    display: flex;
    align-items: center;
    color: var(--rinch-color-dimmed);
}

/* Breadcrumbs link */
.rinch-breadcrumbs__link {
    color: var(--rinch-color-dimmed);
    text-decoration: none;
    transition: color 150ms ease;
}

/* An item with `onclick` and no `href` is a <button>, drawn as the link. */
button.rinch-breadcrumbs__link {
    background: none;
    border: none;
    padding: 0;
    margin: 0;
    font: inherit;
    cursor: pointer;
}

.rinch-breadcrumbs__link:hover {
    color: var(--rinch-primary-color);
    text-decoration: underline;
}

/* Active item (current page) */
.rinch-breadcrumbs__item--active {
    color: var(--rinch-color-text);
    font-weight: 500;
}
"#
    .to_string()
}
