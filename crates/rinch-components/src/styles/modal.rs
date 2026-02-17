pub fn styles() -> String {
    r#"
/* Modal root */
.rinch-modal__root {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: var(--rinch-spacing-xl);
    z-index: 200;
}

/* Hidden state */
.rinch-modal__root--hidden {
    display: none !important;
}

/* Modal overlay */
.rinch-modal__overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: rgba(0, 0, 0, var(--rinch-modal-overlay-opacity, 0.75));
    backdrop-filter: blur(var(--rinch-modal-overlay-blur, 0));
}

/* Modal container */
.rinch-modal {
    position: relative;
    background-color: var(--rinch-color-body);
    border-radius: var(--rinch-radius-default);
    box-shadow: var(--rinch-shadow-xl);
    max-height: calc(100vh - var(--rinch-spacing-xl) * 2);
    overflow-y: auto;
    margin-top: var(--rinch-spacing-xl);
    z-index: 201;
}

/* Centered modal */
.rinch-modal--centered {
    margin-top: auto;
    margin-bottom: auto;
}

.rinch-modal__root:has(.rinch-modal--centered) {
    align-items: center;
}

/* Modal sizes */
.rinch-modal--xs { width: 320px; }
.rinch-modal--sm { width: 380px; }
.rinch-modal--md { width: 440px; }
.rinch-modal--lg { width: 620px; }
.rinch-modal--xl { width: 780px; }
.rinch-modal--full {
    width: calc(100vw - var(--rinch-spacing-xl) * 2);
    max-width: none;
}

/* Modal header */
.rinch-modal__header {
    padding: var(--rinch-spacing-md) var(--rinch-spacing-lg);
    padding-right: calc(var(--rinch-spacing-lg) + 2rem);
    border-bottom: 1px solid var(--rinch-color-border);
}

.rinch-modal__title {
    margin: 0;
    font-size: var(--rinch-font-size-lg);
    font-weight: 600;
    color: var(--rinch-color-text);
}

/* Modal body */
.rinch-modal__body {
    padding: var(--rinch-spacing-lg);
}

/* Modal close button */
.rinch-modal__close {
    position: absolute;
    top: var(--rinch-spacing-sm);
    right: var(--rinch-spacing-sm);
    width: 2rem;
    height: 2rem;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    color: var(--rinch-color-dimmed);
    transition: background-color 150ms ease, color 150ms ease;
}

.rinch-modal__close:hover {
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
}

.rinch-modal__close svg {
    width: 1rem;
    height: 1rem;
}

/* Radius variants */
.rinch-modal--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-modal--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-modal--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-modal--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-modal--radius-xl { border-radius: var(--rinch-radius-xl); }
"#
    .to_string()
}
