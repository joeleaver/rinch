pub fn styles() -> String {
    r#"
/* Alert base */
.rinch-alert {
    position: relative;
    display: flex;
    padding: var(--rinch-spacing-md);
    border-radius: var(--rinch-radius-default);
    font-family: var(--rinch-font-family);
}

.rinch-alert__wrapper {
    flex: 1;
    min-width: 0;
}

.rinch-alert__title {
    font-weight: 700;
    font-size: var(--rinch-font-size-sm);
    line-height: 1.4;
    margin-bottom: 0.25rem;
}

.rinch-alert__message {
    font-size: var(--rinch-font-size-sm);
    line-height: 1.5;
}

.rinch-alert--with-title .rinch-alert__message {
    color: inherit;
    opacity: 0.9;
}

/* Alert icon */
.rinch-alert__icon {
    display: flex;
    align-items: flex-start;
    margin-right: var(--rinch-spacing-sm);
    width: 1.25rem;
    height: 1.25rem;
    flex-shrink: 0;
}

.rinch-alert__icon svg {
    width: 100%;
    height: 100%;
}

/* Close button */
.rinch-alert__close {
    position: absolute;
    top: var(--rinch-spacing-xs);
    right: var(--rinch-spacing-xs);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    opacity: 0.7;
    transition: opacity 150ms ease, background-color 150ms ease;
}

.rinch-alert__close:hover {
    opacity: 1;
    background-color: var(--rinch-alert-close-hover, rgba(0, 0, 0, 0.05));
}

/* Colour (#2200). Every variant reads the --rinch-alert-* variables; a
   built-in colour's class below sets them, and any other colour (a palette
   name, an app's token family, a raw CSS colour) sets them inline:
     --rinch-alert-color         solid colour (filled background, outline)
     --rinch-alert-light         light background
     --rinch-alert-light-text    text on the light background
     --rinch-alert-light-strong  title on the light background
     --rinch-alert-filled-text   text on the filled background (the
                                 colour's contrast token, #2208)
     --rinch-alert-outline-text  text of the outline variant (the colour)
   Each colour sets all six, so an Alert nested in another never inherits the
   outer one's values. */
.rinch-alert--blue {
    --rinch-alert-color: var(--rinch-color-blue-6);
    --rinch-alert-light: var(--rinch-color-blue-light);
    --rinch-alert-light-text: var(--rinch-color-blue-light-text);
    --rinch-alert-light-strong: var(--rinch-color-blue-light-color-strong);
    --rinch-alert-filled-text: var(--rinch-color-blue-contrast, white);
    --rinch-alert-outline-text: var(--rinch-color-blue-6);
}
.rinch-alert--green {
    --rinch-alert-color: var(--rinch-color-green-6);
    --rinch-alert-light: var(--rinch-color-green-light);
    --rinch-alert-light-text: var(--rinch-color-green-light-text);
    --rinch-alert-light-strong: var(--rinch-color-green-light-color-strong);
    --rinch-alert-filled-text: var(--rinch-color-green-contrast, white);
    --rinch-alert-outline-text: var(--rinch-color-green-6);
}
.rinch-alert--yellow {
    --rinch-alert-color: var(--rinch-color-yellow-6);
    --rinch-alert-light: var(--rinch-color-yellow-light);
    --rinch-alert-light-text: var(--rinch-color-yellow-light-text);
    --rinch-alert-light-strong: var(--rinch-color-yellow-light-color-strong);
    --rinch-alert-filled-text: var(--rinch-color-yellow-9);
    --rinch-alert-outline-text: var(--rinch-color-yellow-7);
}
.rinch-alert--red {
    --rinch-alert-color: var(--rinch-color-red-6);
    --rinch-alert-light: var(--rinch-color-red-light);
    --rinch-alert-light-text: var(--rinch-color-red-light-text);
    --rinch-alert-light-strong: var(--rinch-color-red-light-color-strong);
    --rinch-alert-filled-text: var(--rinch-color-red-contrast, white);
    --rinch-alert-outline-text: var(--rinch-color-red-6);
}
.rinch-alert--gray {
    --rinch-alert-color: var(--rinch-color-gray-6);
    --rinch-alert-light: var(--rinch-color-filled);
    --rinch-alert-light-text: var(--rinch-color-text);
    --rinch-alert-light-strong: var(--rinch-color-gray-light-color-strong);
    --rinch-alert-filled-text: var(--rinch-color-gray-contrast, white);
    --rinch-alert-outline-text: var(--rinch-color-gray-6);
}
.rinch-alert--primary {
    --rinch-alert-color: var(--rinch-primary-color);
    --rinch-alert-light: var(--rinch-primary-color-light);
    --rinch-alert-light-text: var(--rinch-primary-color-light-text);
    --rinch-alert-light-strong: var(--rinch-primary-color-light-color-strong);
    --rinch-alert-filled-text: var(--rinch-primary-color-contrast, white);
    --rinch-alert-outline-text: var(--rinch-primary-color);
}

/* Light variant (default) */
.rinch-alert--light {
    border: 1px solid transparent;
    background-color: var(--rinch-alert-light);
    color: var(--rinch-alert-light-text);
}
.rinch-alert--light .rinch-alert__title { color: var(--rinch-alert-light-strong); }

/* Filled variant. With no colour class (hand-written markup) it falls back
   to the primary fill and its contrast token, as a filled Badge does. */
.rinch-alert--filled {
    background-color: var(--rinch-alert-color, var(--rinch-primary-color));
    color: var(--rinch-alert-filled-text, var(--rinch-primary-color-contrast, white));
}
.rinch-alert--filled .rinch-alert__title { color: var(--rinch-alert-filled-text, var(--rinch-primary-color-contrast, white)); }
.rinch-alert--filled .rinch-alert__close { color: var(--rinch-alert-filled-text, var(--rinch-primary-color-contrast, white)); }
.rinch-alert--filled .rinch-alert__close:hover {
    background-color: var(--rinch-alert-close-hover, rgba(255, 255, 255, 0.15));
}

/* Outline variant */
.rinch-alert--outline {
    background-color: transparent;
    border: 1px solid var(--rinch-alert-color);
    color: var(--rinch-alert-outline-text, var(--rinch-alert-color));
}

/* Radius variants */
.rinch-alert--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-alert--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-alert--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-alert--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-alert--radius-xl { border-radius: var(--rinch-radius-xl); }
"#
    .to_string()
}
