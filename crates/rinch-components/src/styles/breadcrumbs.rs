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

/* Separator via data attribute */
.rinch-breadcrumbs__item:not(:last-child)::after {
    content: attr(data-separator);
    color: var(--rinch-color-dimmed);
}

/* Fallback separator when no data attribute */
.rinch-breadcrumbs__list[data-separator] .rinch-breadcrumbs__item:not(:last-child)::after {
    content: '/';
}

.rinch-breadcrumbs__list[data-separator="/"] .rinch-breadcrumbs__item:not(:last-child)::after {
    content: '/';
}

.rinch-breadcrumbs__list[data-separator=">"] .rinch-breadcrumbs__item:not(:last-child)::after {
    content: '>';
}

.rinch-breadcrumbs__list[data-separator=">>"] .rinch-breadcrumbs__item:not(:last-child)::after {
    content: '>>';
}

.rinch-breadcrumbs__list[data-separator="-"] .rinch-breadcrumbs__item:not(:last-child)::after {
    content: '-';
}

.rinch-breadcrumbs__list[data-separator="•"] .rinch-breadcrumbs__item:not(:last-child)::after {
    content: '•';
}

/* Breadcrumbs link */
.rinch-breadcrumbs__link {
    color: var(--rinch-color-dimmed);
    text-decoration: none;
    transition: color 150ms ease;
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
