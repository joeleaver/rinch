pub fn styles() -> String {
    r#"
/* Select wrapper */
.rinch-select {
    display: flex;
    flex-direction: column;
    gap: 4px;
    /* min-width: 0 lets the Select shrink below its intrinsic min-content
       size when its parent is constrained (e.g., a 110px timeline cell).
       Without this, flex containers/items default to min-width: auto,
       which is the trigger's min-content width — typically ~75-150px. */
    min-width: 0;
}

.rinch-select__label {
    font-size: var(--rinch-font-size-sm);
    font-weight: 500;
    color: var(--rinch-color-text);
    display: inline;
}

.rinch-select__required {
    color: var(--rinch-color-red-6);
    font-weight: 500;
}

.rinch-select__wrapper {
    position: relative;
}

/* Trigger button — matches TextInput styling */
.rinch-select__input {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-family: var(--rinch-font-family);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    background-color: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    padding: 0 var(--rinch-spacing-sm);
    height: 2.25rem;
    cursor: pointer;
    user-select: none;
    gap: 8px;
}

.rinch-select__input:hover {
    border-color: var(--rinch-primary-color);
}

.rinch-select__input--disabled {
    background-color: var(--rinch-color-state-disabled);
    color: var(--rinch-color-dimmed);
    cursor: not-allowed;
    pointer-events: none;
}

.rinch-select__display {
    flex: 1;
    /* Pair with overflow: hidden + text-overflow: ellipsis to allow the
       display label to shrink below its intrinsic content width. */
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
}

.rinch-select__display--placeholder {
    color: var(--rinch-color-placeholder);
}

.rinch-select__chevron {
    width: 18px;
    height: 18px;
    color: var(--rinch-color-dimmed);
    flex-shrink: 0;
    transition: transform 200ms ease;
}

/* Dropdown overlay
   Width: trigger width is the floor (min-width: 100%), but the panel grows
   to fit the widest option (width: max-content). max-width caps it so
   pathologically long labels don't escape the viewport. See GH #33. */
.rinch-select__dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    min-width: 100%;
    width: max-content;
    max-width: min(calc(100vw - 16px), 480px);
    margin-top: 4px;
    display: none;
    flex-direction: column;
    background: var(--rinch-color-body);
    border: 1px solid var(--rinch-color-border);
    border-radius: var(--rinch-radius-default);
    box-shadow: var(--rinch-shadow-md);
    z-index: 300;
    max-height: 200px;
    overflow-y: auto;
    padding: 4px;
}

/* Flipped variant — opens above the trigger when below-space is too small. */
.rinch-select__dropdown--above {
    top: auto;
    bottom: 100%;
    margin-top: 0;
    margin-bottom: 4px;
}

/* Options demand a single-line intrinsic width via `white-space: nowrap`.
   The column-flex panel sizes its cross axis to the widest child's
   intrinsic width, so this is what actually grows the panel past
   `min-width: 100%` up to the `max-width` cap (the `width: max-content`
   above is a defensive hint — Stylo currently parses it as Auto). Without
   nowrap, options wrap inside the trigger-width panel and nothing grows. */
.rinch-select__option {
    padding: 8px var(--rinch-spacing-sm);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-text);
    border-radius: var(--rinch-radius-sm);
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
}

.rinch-select__option:hover {
    background-color: var(--rinch-color-option-hover);
}

.rinch-select__option--selected {
    background-color: var(--rinch-color-option-selected);
    color: var(--rinch-primary-color);
    font-weight: 500;
}

/* Backdrop — invisible full-screen overlay to catch outside clicks */
.rinch-select__backdrop {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 299;
    display: none;
}

/* Error state */
.rinch-select--error .rinch-select__input {
    border-color: var(--rinch-color-red-6);
}

.rinch-select__description {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-dimmed);
}

.rinch-select__error {
    font-size: var(--rinch-font-size-xs);
    color: var(--rinch-color-red-6);
}

/* Disabled state */
.rinch-select--disabled .rinch-select__input {
    background-color: var(--rinch-color-state-disabled);
    color: var(--rinch-color-dimmed);
    cursor: not-allowed;
}

/* Sizes */
.rinch-select--xs .rinch-select__input { height: 1.875rem; font-size: var(--rinch-font-size-xs); }
.rinch-select--sm .rinch-select__input { height: 2rem; }
.rinch-select--lg .rinch-select__input { height: 2.625rem; font-size: var(--rinch-font-size-md); }
.rinch-select--xl .rinch-select__input { height: 3.125rem; font-size: var(--rinch-font-size-lg); }
"#
    .to_string()
}
