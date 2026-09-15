pub fn styles() -> String {
    r#"
/* Code inline */
/* `margin: 0` is load-bearing on the *block* form, which is a `<pre>`: since
   #674 the UA stylesheet gives `<pre>` the browser's `margin-block: 1em`, and
   without this declaration a `Code` block would gain a line of space above and
   below on desktop — and has always had one on rinch-web, which runs on the
   browser's own UA sheet. Declaring it here converges the two on the
   component's own look, which is what `.rinch-list`, `.rinch-divider`,
   `.rinch-blockquote` and `.rinch-breadcrumbs__list` already do for the same
   reason.

   The `monospace` fallback in the `var()` is load-bearing too, and for a
   subtler reason: in a build with `components` but not `theme` the custom
   property is undefined, which makes the whole declaration *invalid at
   computed-value time* — that computes to `unset`, i.e. `inherit` for an
   inherited property, and does **not** fall through to the UA sheet's
   `code, kbd, samp, pre { font-family: monospace }`. Without the fallback a
   theme-less `Code` renders in the body font. */
.rinch-code {
    margin: 0;
    font-family: var(--rinch-font-family-monospace, monospace);
    font-size: var(--rinch-font-size-sm);
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
    padding: 0.125rem 0.375rem;
    border-radius: var(--rinch-radius-xs);
}

/* Code block */
.rinch-code--block {
    display: block;
    padding: var(--rinch-spacing-md);
    border-radius: var(--rinch-radius-default);
    overflow-x: auto;
    white-space: pre;
}

/* Code colors */
.rinch-code--primary {
    background-color: var(--rinch-primary-color-0);
    color: var(--rinch-primary-color-7);
}

/* Code sizes */
.rinch-code--xs { font-size: var(--rinch-font-size-xs); }
.rinch-code--sm { font-size: var(--rinch-font-size-sm); }
.rinch-code--md { font-size: var(--rinch-font-size-md); }
.rinch-code--lg { font-size: var(--rinch-font-size-lg); }
"#
    .to_string()
}
