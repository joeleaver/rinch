pub fn styles() -> String {
    r#"
/* ===== ColorSwatch ===== */
.rinch-color-swatch {
    position: relative;
    overflow: hidden;
    flex-shrink: 0;
    cursor: default;
    /* Checkerboard background for transparency indication */
    background-image:
        linear-gradient(45deg, #ccc 25%, transparent 25%),
        linear-gradient(-45deg, #ccc 25%, transparent 25%),
        linear-gradient(45deg, transparent 75%, #ccc 75%),
        linear-gradient(-45deg, transparent 75%, #ccc 75%);
    background-size: 8px 8px;
    background-position: 0 0, 0 4px, 4px -4px, -4px 0;
}

.rinch-color-swatch--clickable {
    cursor: pointer;
}

.rinch-color-swatch__overlay {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
}

.rinch-color-swatch--shadow {
    box-shadow: var(--rinch-shadow-sm);
}

/* ===== ColorPicker ===== */
.rinch-color-picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 200px;
    padding: 0;
}

.rinch-color-picker--sm { width: 180px; }
.rinch-color-picker--md { width: 200px; }
.rinch-color-picker--lg { width: 240px; }
.rinch-color-picker--xl { width: 280px; }

/* Saturation panel */
.rinch-color-picker__saturation {
    position: relative;
    height: 150px;
    border-radius: var(--rinch-radius-sm);
    overflow: hidden;
    cursor: crosshair;
}

.rinch-color-picker__saturation-bg {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
}

.rinch-color-picker__saturation-white {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: linear-gradient(to right, white, transparent);
}

.rinch-color-picker__saturation-black {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: linear-gradient(to top, black, transparent);
}

.rinch-color-picker__saturation-overlay {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: 10;
}

/* Shared thumb style for saturation, hue, and alpha */
.rinch-color-picker__thumb-wrapper {
    position: absolute;
    width: 100%;
    height: 100%;
    top: 0;
    left: 0;
    pointer-events: none;
}

.rinch-color-picker__thumb {
    position: absolute;
    width: 16px;
    height: 16px;
    border: 2px solid white;
    border-radius: 50%;
    box-shadow: 0 0 2px rgba(0, 0, 0, 0.6);
    pointer-events: none;
    /* Centered on its position via negative margin */
    margin-left: -8px;
    margin-top: -8px;
}

/* Hue slider */
.rinch-color-picker__hue {
    position: relative;
    height: 12px;
    border-radius: 6px;
    background: linear-gradient(to right,
        #ff0000 0%,
        #ffff00 17%,
        #00ff00 33%,
        #00ffff 50%,
        #0000ff 67%,
        #ff00ff 83%,
        #ff0000 100%
    );
    cursor: pointer;
}

.rinch-color-picker__hue-overlay {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: 10;
}

.rinch-color-picker__hue-thumb {
    position: absolute;
    top: 50%;
    width: 16px;
    height: 16px;
    border: 2px solid white;
    border-radius: 50%;
    box-shadow: 0 0 2px rgba(0, 0, 0, 0.6);
    pointer-events: none;
    margin-left: -8px;
    margin-top: -8px;
}

/* Alpha slider */
.rinch-color-picker__alpha {
    position: relative;
    height: 12px;
    border-radius: 6px;
    overflow: hidden;
    cursor: pointer;
}

.rinch-color-picker__alpha-checkerboard {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-image:
        linear-gradient(45deg, #ccc 25%, transparent 25%),
        linear-gradient(-45deg, #ccc 25%, transparent 25%),
        linear-gradient(45deg, transparent 75%, #ccc 75%),
        linear-gradient(-45deg, transparent 75%, #ccc 75%);
    background-size: 8px 8px;
    background-position: 0 0, 0 4px, 4px -4px, -4px 0;
}

.rinch-color-picker__alpha-gradient {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    border-radius: 6px;
}

.rinch-color-picker__alpha-overlay {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: 10;
}

.rinch-color-picker__alpha-thumb {
    position: absolute;
    top: 50%;
    width: 16px;
    height: 16px;
    border: 2px solid white;
    border-radius: 50%;
    box-shadow: 0 0 2px rgba(0, 0, 0, 0.6);
    pointer-events: none;
    margin-left: -8px;
    margin-top: -8px;
}

/* Controls row: preview swatch + hex input */
.rinch-color-picker__controls {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
}

.rinch-color-picker__preview {
    flex-shrink: 0;
}

.rinch-color-picker__hex-input {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--rinch-color-default-border, #ced4da);
    border-radius: var(--rinch-radius-sm);
    background-color: var(--rinch-color-surface, #fff);
    color: var(--rinch-color-text, #000);
    font-size: var(--rinch-font-size-sm, 14px);
    font-family: monospace;
    outline: none;
}

.rinch-color-picker__hex-input:focus {
    border-color: var(--rinch-primary-color);
}

/* Swatches grid */
.rinch-color-picker__swatches {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
}

/* ===== ColorInput ===== */
.rinch-color-input {
    display: flex;
    flex-direction: column;
    gap: 4px;
}

.rinch-color-input__label {
    font-size: var(--rinch-font-size-sm, 14px);
    font-weight: 500;
    color: var(--rinch-color-text);
}

.rinch-color-input__wrapper {
    position: relative;
}

.rinch-color-input__input-group {
    display: flex;
    align-items: center;
    border: 1px solid var(--rinch-color-default-border, #ced4da);
    border-radius: var(--rinch-radius-sm);
    background-color: var(--rinch-color-surface, #fff);
    overflow: hidden;
    cursor: pointer;
}

.rinch-color-input__input-group:focus-within {
    border-color: var(--rinch-primary-color);
}

.rinch-color-input--error .rinch-color-input__input-group {
    border-color: var(--rinch-color-red-6, #fa5252);
}

.rinch-color-input__swatch-preview {
    flex-shrink: 0;
    margin-left: 8px;
}

.rinch-color-input__input {
    flex: 1;
    min-width: 0;
    height: 34px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--rinch-color-text, #000);
    font-size: var(--rinch-font-size-sm, 14px);
    outline: none;
}

.rinch-color-input__dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    z-index: 1000;
    background-color: var(--rinch-color-surface, #fff);
    border: 1px solid var(--rinch-color-default-border, #ced4da);
    border-radius: var(--rinch-radius-sm);
    box-shadow: var(--rinch-shadow-md);
    padding: 8px;
    display: none;
}

.rinch-color-input--opened .rinch-color-input__dropdown {
    display: block;
}

.rinch-color-input__description {
    font-size: var(--rinch-font-size-xs, 12px);
    color: var(--rinch-color-dimmed);
}

.rinch-color-input__error {
    font-size: var(--rinch-font-size-xs, 12px);
    color: var(--rinch-color-red-6, #fa5252);
}

.rinch-color-input--disabled .rinch-color-input__input-group {
    opacity: 0.6;
    pointer-events: none;
    cursor: default;
}
"#
    .to_string()
}
