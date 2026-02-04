pub fn styles() -> String {
    r#"
/* Notification base */
.rinch-notification {
    display: flex;
    align-items: flex-start;
    gap: var(--rinch-spacing-sm);
    padding: var(--rinch-spacing-md);
    background-color: var(--rinch-color-body);
    border-radius: var(--rinch-radius-default);
    box-shadow: var(--rinch-shadow-md);
    min-width: 300px;
    max-width: 400px;
    position: fixed;
    z-index: 300;
}

/* Hidden state */
.rinch-notification--hidden {
    display: none !important;
}

/* Positions */
.rinch-notification--top-left {
    top: var(--rinch-spacing-md);
    left: var(--rinch-spacing-md);
}

.rinch-notification--top-center {
    top: var(--rinch-spacing-md);
    left: 50%;
    transform: translateX(-50%);
}

.rinch-notification--top-right {
    top: var(--rinch-spacing-md);
    right: var(--rinch-spacing-md);
}

.rinch-notification--bottom-left {
    bottom: var(--rinch-spacing-md);
    left: var(--rinch-spacing-md);
}

.rinch-notification--bottom-center {
    bottom: var(--rinch-spacing-md);
    left: 50%;
    transform: translateX(-50%);
}

.rinch-notification--bottom-right {
    bottom: var(--rinch-spacing-md);
    right: var(--rinch-spacing-md);
}

/* With colored border */
.rinch-notification--with-border {
    border-left: 4px solid var(--rinch-notification-color, var(--rinch-primary-color));
}

/* Icon */
.rinch-notification__icon {
    flex-shrink: 0;
    width: 1.5rem;
    height: 1.5rem;
    color: var(--rinch-notification-color, var(--rinch-primary-color));
}

.rinch-notification__icon svg {
    width: 100%;
    height: 100%;
}

/* Loader icon */
.rinch-notification__loader {
    width: 1.5rem;
    height: 1.5rem;
    border: 2px solid var(--rinch-color-border);
    border-top-color: var(--rinch-notification-color, var(--rinch-primary-color));
    border-radius: 50%;
    animation: rinch-notification-spin 0.6s linear infinite;
}

@keyframes rinch-notification-spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
}

/* Body */
.rinch-notification__body {
    flex: 1;
    min-width: 0;
}

/* Title */
.rinch-notification__title {
    font-size: var(--rinch-font-size-sm);
    font-weight: 600;
    color: var(--rinch-color-text);
    margin-bottom: 0.25rem;
}

/* Description */
.rinch-notification__description {
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-dimmed);
}

/* Close button */
.rinch-notification__close {
    flex-shrink: 0;
    width: 1.5rem;
    height: 1.5rem;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    color: var(--rinch-color-dimmed);
    transition: background-color 150ms ease, color 150ms ease;
    margin-left: auto;
}

.rinch-notification__close:hover {
    background-color: var(--rinch-color-default);
    color: var(--rinch-color-text);
}

.rinch-notification__close svg {
    width: 0.875rem;
    height: 0.875rem;
}

/* Radius variants */
.rinch-notification--radius-xs { border-radius: var(--rinch-radius-xs); }
.rinch-notification--radius-sm { border-radius: var(--rinch-radius-sm); }
.rinch-notification--radius-md { border-radius: var(--rinch-radius-md); }
.rinch-notification--radius-lg { border-radius: var(--rinch-radius-lg); }
.rinch-notification--radius-xl { border-radius: var(--rinch-radius-xl); }

/* Loading state */
.rinch-notification--loading .rinch-notification__icon {
    color: transparent;
}
"#
    .to_string()
}
