pub fn styles() -> String {
    r#"
/* Avatar base */
.rinch-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    user-select: none;
    background-color: var(--rinch-avatar-color, var(--rinch-primary-color));
}

/* Avatar sizes */
.rinch-avatar--xs { width: 1rem; height: 1rem; font-size: 0.5rem; }
.rinch-avatar--sm { width: 1.625rem; height: 1.625rem; font-size: 0.625rem; }
.rinch-avatar--md { width: 2.375rem; height: 2.375rem; font-size: 0.875rem; }
.rinch-avatar--lg { width: 3.5rem; height: 3.5rem; font-size: 1.25rem; }
.rinch-avatar--xl { width: 5.25rem; height: 5.25rem; font-size: 1.75rem; }

/* Avatar radius */
.rinch-avatar--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-avatar--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-avatar--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-avatar--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-avatar--radius-xl { border-radius: 50%; }

/* Avatar variants */
.rinch-avatar--filled {
    color: white;
}

.rinch-avatar--light {
    background-color: var(--rinch-primary-color-1);
    color: var(--rinch-avatar-color, var(--rinch-primary-color));
}

.rinch-avatar--outline {
    background-color: transparent;
    border: 2px solid var(--rinch-avatar-color, var(--rinch-primary-color));
    color: var(--rinch-avatar-color, var(--rinch-primary-color));
}

/* Avatar image */
.rinch-avatar__image {
    width: 100%;
    height: 100%;
    object-fit: cover;
}

/* Avatar placeholder (initials or icon) */
.rinch-avatar__placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    text-transform: uppercase;
}

.rinch-avatar__placeholder svg {
    width: 70%;
    height: 70%;
}

/* Avatar group */
.rinch-avatar-group {
    display: flex;
    flex-direction: row;
}

.rinch-avatar-group > .rinch-avatar {
    margin-left: var(--rinch-avatar-group-spacing, -0.5rem);
    border: 2px solid var(--rinch-color-body);
}

.rinch-avatar-group > .rinch-avatar:first-child {
    margin-left: 0;
}
"#.to_string()
}
