pub fn styles() -> String {
    r#"
/* SimpleGrid. The props write only the variables (inline); the grid is this
   rule, so an app stylesheet can override it, e.g. per breakpoint. */
.rinch-simple-grid {
    display: grid;
    grid-template-columns: var(
        --rinch-simple-grid-template,
        repeat(var(--rinch-simple-grid-cols, 1), 1fr)
    );
    column-gap: var(--rinch-simple-grid-gap, var(--rinch-spacing-md));
    row-gap: var(
        --rinch-simple-grid-row-gap,
        var(--rinch-simple-grid-gap, var(--rinch-spacing-md))
    );
}

/* SimpleGrid - prevent content from expanding grid columns. The variables
   inherit, so they stop at the grid's children: a SimpleGrid inside another
   keeps its own defaults, not the outer grid's props. */
.rinch-simple-grid > * {
    min-width: 0;
    --rinch-simple-grid-template: initial;
    --rinch-simple-grid-cols: initial;
    --rinch-simple-grid-gap: initial;
    --rinch-simple-grid-row-gap: initial;
}
"#
    .to_string()
}
