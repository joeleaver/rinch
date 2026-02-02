pub fn styles() -> String {
    r#"
/* Badge base */
.rinch-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--rinch-font-family);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.25px;
    border-radius: 2rem;
    white-space: nowrap;
}

/* Badge sizes */
.rinch-badge--xs {
    height: 1rem;
    padding: 0 0.375rem;
    font-size: 0.5625rem;
}

.rinch-badge--sm {
    height: 1.125rem;
    padding: 0 0.5rem;
    font-size: 0.625rem;
}

.rinch-badge--md {
    height: 1.25rem;
    padding: 0 0.625rem;
    font-size: 0.6875rem;
}

.rinch-badge--lg {
    height: 1.625rem;
    padding: 0 0.75rem;
    font-size: 0.8125rem;
}

.rinch-badge--xl {
    height: 2rem;
    padding: 0 1rem;
    font-size: 0.875rem;
}

/* Badge variants - filled */
.rinch-badge--filled {
    background-color: var(--rinch-primary-color);
    color: white;
}

/* Badge variants - light */
.rinch-badge--light {
    background-color: var(--rinch-primary-color-0);
    color: var(--rinch-primary-color-6);
}

/* Badge variants - outline */
.rinch-badge--outline {
    background-color: transparent;
    color: var(--rinch-primary-color);
    border: 1px solid var(--rinch-primary-color);
}

/* Badge variants - dot */
.rinch-badge--dot {
    background-color: transparent;
    color: var(--rinch-color-text);
    border: 1px solid var(--rinch-color-border);
}

.rinch-badge--dot::before {
    content: '';
    display: inline-block;
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 50%;
    background-color: var(--rinch-primary-color);
    margin-right: 0.375rem;
}

/* Badge with full width */
.rinch-badge--full-width {
    width: 100%;
}
"#.to_string()
}
