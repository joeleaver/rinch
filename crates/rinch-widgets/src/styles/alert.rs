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
    background-color: rgba(0, 0, 0, 0.05);
}

/* Light variant (default) */
.rinch-alert--light {
    border: 1px solid transparent;
}

.rinch-alert--light.rinch-alert--blue {
    background-color: var(--rinch-color-blue-0);
    color: var(--rinch-color-blue-9);
}
.rinch-alert--light.rinch-alert--blue .rinch-alert__title { color: var(--rinch-color-blue-7); }

.rinch-alert--light.rinch-alert--green {
    background-color: var(--rinch-color-green-0);
    color: var(--rinch-color-green-9);
}
.rinch-alert--light.rinch-alert--green .rinch-alert__title { color: var(--rinch-color-green-7); }

.rinch-alert--light.rinch-alert--yellow {
    background-color: var(--rinch-color-yellow-0);
    color: var(--rinch-color-yellow-9);
}
.rinch-alert--light.rinch-alert--yellow .rinch-alert__title { color: var(--rinch-color-yellow-7); }

.rinch-alert--light.rinch-alert--red {
    background-color: var(--rinch-color-red-0);
    color: var(--rinch-color-red-9);
}
.rinch-alert--light.rinch-alert--red .rinch-alert__title { color: var(--rinch-color-red-7); }

.rinch-alert--light.rinch-alert--gray {
    background-color: var(--rinch-color-filled);
    color: var(--rinch-color-text);
}
.rinch-alert--light.rinch-alert--gray .rinch-alert__title { color: var(--rinch-color-gray-7); }

.rinch-alert--light.rinch-alert--primary {
    background-color: var(--rinch-primary-color-0);
    color: var(--rinch-primary-color-9);
}
.rinch-alert--light.rinch-alert--primary .rinch-alert__title { color: var(--rinch-primary-color-7); }

/* Filled variant */
.rinch-alert--filled {
    color: white;
}
.rinch-alert--filled .rinch-alert__title { color: white; }
.rinch-alert--filled .rinch-alert__close { color: white; }
.rinch-alert--filled .rinch-alert__close:hover { background-color: rgba(255, 255, 255, 0.15); }

.rinch-alert--filled.rinch-alert--blue { background-color: var(--rinch-color-blue-6); }
.rinch-alert--filled.rinch-alert--green { background-color: var(--rinch-color-green-6); }
.rinch-alert--filled.rinch-alert--yellow { background-color: var(--rinch-color-yellow-6); color: var(--rinch-color-yellow-9); }
.rinch-alert--filled.rinch-alert--yellow .rinch-alert__title { color: var(--rinch-color-yellow-9); }
.rinch-alert--filled.rinch-alert--yellow .rinch-alert__close { color: var(--rinch-color-yellow-9); }
.rinch-alert--filled.rinch-alert--red { background-color: var(--rinch-color-red-6); }
.rinch-alert--filled.rinch-alert--gray { background-color: var(--rinch-color-gray-6); }
.rinch-alert--filled.rinch-alert--primary { background-color: var(--rinch-primary-color); }

/* Outline variant */
.rinch-alert--outline {
    background-color: transparent;
}

.rinch-alert--outline.rinch-alert--blue {
    border: 1px solid var(--rinch-color-blue-6);
    color: var(--rinch-color-blue-6);
}
.rinch-alert--outline.rinch-alert--green {
    border: 1px solid var(--rinch-color-green-6);
    color: var(--rinch-color-green-6);
}
.rinch-alert--outline.rinch-alert--yellow {
    border: 1px solid var(--rinch-color-yellow-6);
    color: var(--rinch-color-yellow-7);
}
.rinch-alert--outline.rinch-alert--red {
    border: 1px solid var(--rinch-color-red-6);
    color: var(--rinch-color-red-6);
}
.rinch-alert--outline.rinch-alert--gray {
    border: 1px solid var(--rinch-color-gray-6);
    color: var(--rinch-color-gray-6);
}
.rinch-alert--outline.rinch-alert--primary {
    border: 1px solid var(--rinch-primary-color);
    color: var(--rinch-primary-color);
}

/* Radius variants */
.rinch-alert--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-alert--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-alert--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-alert--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-alert--radius-xl { border-radius: var(--rinch-radius-xl); }
"#.to_string()
}
