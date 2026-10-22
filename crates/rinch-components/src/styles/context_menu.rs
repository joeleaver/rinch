//! CSS styles for the ContextMenu component.

pub fn styles() -> String {
    r#"
.rinch-context-menu {
    display: contents;
}

.rinch-context-menu__target {
    display: contents;
}

/* #1761: the spacers placing the dropdown are physical, so a right-to-left
   page must not mirror the row holding them; the dropdown's own content keeps
   the page's direction. */
.rinch-context-menu__line:dir(rtl) {
    direction: ltr;
}

.rinch-context-menu__dropdown:dir(rtl) {
    direction: rtl;
}

.rinch-context-menu__dropdown {
    min-width: 180px;
    padding: var(--rinch-spacing-xs, 4px) 0;
    background-color: var(--rinch-color-body, #ffffff);
    border: 1px solid var(--rinch-color-default-border, #dee2e6);
    border-radius: var(--rinch-radius-default, 4px);
    box-shadow: var(--rinch-shadow-md, 0 1px 3px rgba(0,0,0,.1), 0 4px 6px rgba(0,0,0,.08));
}
"#
    .into()
}
