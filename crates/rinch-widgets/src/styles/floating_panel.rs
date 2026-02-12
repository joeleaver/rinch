pub fn styles() -> String {
    r#"
/* FloatingPanel - draggable/resizable floating panel */
.rinch-floating-panel {
    position: absolute;
    z-index: 150;
    display: flex;
    flex-direction: column;
    background-color: var(--rinch-color-surface);
    border: 1px solid var(--rinch-color-default-border);
    border-radius: var(--rinch-radius-md);
    box-shadow: var(--rinch-shadow-lg);
    overflow: hidden;
}

/* Header - drag handle */
.rinch-floating-panel__header {
    display: flex;
    align-items: center;
    padding: 6px 8px 6px 12px;
    background-color: var(--rinch-color-default);
    cursor: grab;
    user-select: none;
    flex-shrink: 0;
}

.rinch-floating-panel__header:active {
    cursor: grabbing;
}

/* Title */
.rinch-floating-panel__title {
    flex: 1;
    font-size: var(--rinch-font-size-sm);
    font-weight: 600;
    color: var(--rinch-color-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

/* Close button */
.rinch-floating-panel__close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    margin-left: 4px;
    border: none;
    border-radius: var(--rinch-radius-sm);
    background: transparent;
    color: var(--rinch-color-dimmed);
    cursor: pointer;
    flex-shrink: 0;
    font-size: 18px;
    line-height: 1;
}

.rinch-floating-panel__close:hover {
    background-color: var(--rinch-color-default-hover);
    color: var(--rinch-color-text);
}

/* Body - scrollable content area */
.rinch-floating-panel__body {
    flex: 1;
    overflow: auto;
    padding: 12px;
}

/* Footer spacer - keeps scrollbar above resize handle */
.rinch-floating-panel__footer {
    flex-shrink: 0;
    height: 8px;
}

/* Resize handle - bottom-right corner grip */
.rinch-floating-panel__resize {
    position: absolute;
    bottom: 0;
    right: 0;
    width: 20px;
    height: 20px;
    cursor: nwse-resize;
    background-image: linear-gradient(
        135deg,
        transparent 30%,
        var(--rinch-color-dimmed) 30%,
        var(--rinch-color-dimmed) 33%,
        transparent 33%,
        transparent 55%,
        var(--rinch-color-dimmed) 55%,
        var(--rinch-color-dimmed) 58%,
        transparent 58%,
        transparent 80%,
        var(--rinch-color-dimmed) 80%,
        var(--rinch-color-dimmed) 83%,
        transparent 83%
    );
    opacity: 0.4;
    border-bottom-right-radius: var(--rinch-radius-md);
}

.rinch-floating-panel:hover .rinch-floating-panel__resize {
    opacity: 0.7;
}
"#
    .to_string()
}
