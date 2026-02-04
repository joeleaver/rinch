pub fn styles() -> String {
    r#"
/* Image wrapper */
.rinch-image__wrapper {
    display: inline-block;
    position: relative;
    margin: 0;
}

/* Image base */
.rinch-image {
    display: block;
    width: 100%;
    height: auto;
}

/* Image fit modes */
.rinch-image--fit-cover { object-fit: cover; }
.rinch-image--fit-contain { object-fit: contain; }
.rinch-image--fit-fill { object-fit: fill; }
.rinch-image--fit-none { object-fit: none; }
.rinch-image--fit-scale-down { object-fit: scale-down; }

/* Image radius */
.rinch-image--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-image--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-image--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-image--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-image--radius-xl { border-radius: var(--rinch-radius-xl); }

/* Image caption */
.rinch-image__caption {
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-dimmed);
    text-align: center;
    margin-top: var(--rinch-spacing-xs);
}
"#
    .to_string()
}
