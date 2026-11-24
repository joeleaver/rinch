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
    /* Every badge sets its own colour variables (#2201), so a badge inside
       another badge's section never inherits the outer one's. */
    --rinch-badge-color: var(--rinch-primary-color);
    --rinch-badge-light: var(--rinch-primary-color-light);
    --rinch-badge-light-color: var(--rinch-primary-color-light-color);
    --rinch-badge-text: var(--rinch-primary-color-contrast, white);
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

/* Colour (#2201). Every variant reads the --rinch-badge-* variables, falling
   back to the theme's primary colour:
     --rinch-badge-color        solid colour (filled background, outline, dot)
     --rinch-badge-light        light background
     --rinch-badge-light-color  text on the light background
     --rinch-badge-text         text on the filled background (the
                                colour's contrast token, #2208)
   `.rinch-badge` sets the primary defaults on every badge; a built-in
   palette name's [data-color] rule below sets them all; any other colour (a
   raw CSS colour, `primary`, an app's token family) sets them all inline. */

/* Badge variants - filled */
.rinch-badge--filled {
    background-color: var(--rinch-badge-color, var(--rinch-primary-color));
    color: var(--rinch-badge-text, var(--rinch-primary-color-contrast, white));
}

/* Badge variants - light */
.rinch-badge--light {
    background-color: var(--rinch-badge-light, var(--rinch-primary-color-light));
    color: var(--rinch-badge-light-color, var(--rinch-primary-color-light-color));
}

/* Badge variants - outline */
.rinch-badge--outline {
    background-color: transparent;
    color: var(--rinch-badge-color, var(--rinch-primary-color));
    border: 1px solid var(--rinch-badge-color, var(--rinch-primary-color));
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
    background-color: var(--rinch-badge-color, var(--rinch-primary-color));
    margin-right: 0.375rem;
}

/* Badge with full width */
.rinch-badge--full-width {
    width: 100%;
}

/* Sections (#2201): beside the label, 0.25rem from it. */
.rinch-badge__section {
    display: inline-flex;
    align-items: center;
}
.rinch-badge__section[data-position="left"] { margin-right: 0.25rem; }
.rinch-badge__section[data-position="right"] { margin-left: 0.25rem; }

/* Badge colours - the built-in palette names */
.rinch-badge[data-color="dark"] { --rinch-badge-color: var(--rinch-color-dark-6); --rinch-badge-light: var(--rinch-color-dark-light); --rinch-badge-light-color: var(--rinch-color-dark-light-color); --rinch-badge-text: var(--rinch-color-dark-contrast, white); }
.rinch-badge[data-color="gray"] { --rinch-badge-color: var(--rinch-color-gray-6); --rinch-badge-light: var(--rinch-color-gray-light); --rinch-badge-light-color: var(--rinch-color-gray-light-color); --rinch-badge-text: var(--rinch-color-gray-contrast, white); }
.rinch-badge[data-color="red"] { --rinch-badge-color: var(--rinch-color-red-6); --rinch-badge-light: var(--rinch-color-red-light); --rinch-badge-light-color: var(--rinch-color-red-light-color); --rinch-badge-text: var(--rinch-color-red-contrast, white); }
.rinch-badge[data-color="pink"] { --rinch-badge-color: var(--rinch-color-pink-6); --rinch-badge-light: var(--rinch-color-pink-light); --rinch-badge-light-color: var(--rinch-color-pink-light-color); --rinch-badge-text: var(--rinch-color-pink-contrast, white); }
.rinch-badge[data-color="grape"] { --rinch-badge-color: var(--rinch-color-grape-6); --rinch-badge-light: var(--rinch-color-grape-light); --rinch-badge-light-color: var(--rinch-color-grape-light-color); --rinch-badge-text: var(--rinch-color-grape-contrast, white); }
.rinch-badge[data-color="violet"] { --rinch-badge-color: var(--rinch-color-violet-6); --rinch-badge-light: var(--rinch-color-violet-light); --rinch-badge-light-color: var(--rinch-color-violet-light-color); --rinch-badge-text: var(--rinch-color-violet-contrast, white); }
.rinch-badge[data-color="indigo"] { --rinch-badge-color: var(--rinch-color-indigo-6); --rinch-badge-light: var(--rinch-color-indigo-light); --rinch-badge-light-color: var(--rinch-color-indigo-light-color); --rinch-badge-text: var(--rinch-color-indigo-contrast, white); }
.rinch-badge[data-color="blue"] { --rinch-badge-color: var(--rinch-color-blue-6); --rinch-badge-light: var(--rinch-color-blue-light); --rinch-badge-light-color: var(--rinch-color-blue-light-color); --rinch-badge-text: var(--rinch-color-blue-contrast, white); }
.rinch-badge[data-color="cyan"] { --rinch-badge-color: var(--rinch-color-cyan-6); --rinch-badge-light: var(--rinch-color-cyan-light); --rinch-badge-light-color: var(--rinch-color-cyan-light-color); --rinch-badge-text: var(--rinch-color-cyan-contrast, white); }
.rinch-badge[data-color="teal"] { --rinch-badge-color: var(--rinch-color-teal-6); --rinch-badge-light: var(--rinch-color-teal-light); --rinch-badge-light-color: var(--rinch-color-teal-light-color); --rinch-badge-text: var(--rinch-color-teal-contrast, white); }
.rinch-badge[data-color="green"] { --rinch-badge-color: var(--rinch-color-green-6); --rinch-badge-light: var(--rinch-color-green-light); --rinch-badge-light-color: var(--rinch-color-green-light-color); --rinch-badge-text: var(--rinch-color-green-contrast, white); }
.rinch-badge[data-color="lime"] { --rinch-badge-color: var(--rinch-color-lime-6); --rinch-badge-light: var(--rinch-color-lime-light); --rinch-badge-light-color: var(--rinch-color-lime-light-color); --rinch-badge-text: var(--rinch-color-lime-contrast, black); }
.rinch-badge[data-color="yellow"] { --rinch-badge-color: var(--rinch-color-yellow-6); --rinch-badge-light: var(--rinch-color-yellow-light); --rinch-badge-light-color: var(--rinch-color-yellow-light-color); --rinch-badge-text: var(--rinch-color-yellow-contrast, black); }
.rinch-badge[data-color="orange"] { --rinch-badge-color: var(--rinch-color-orange-6); --rinch-badge-light: var(--rinch-color-orange-light); --rinch-badge-light-color: var(--rinch-color-orange-light-color); --rinch-badge-text: var(--rinch-color-orange-contrast, white); }

/* Badge radius (#474) — overrides the pill base above, so it comes after it. */
.rinch-badge--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-badge--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-badge--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-badge--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-badge--radius-xl { border-radius: var(--rinch-radius-xl); }
"#.to_string()
}
