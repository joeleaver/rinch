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
    border: none;
    background: transparent;
    color: var(--rinch-color-text, #000);
    outline: none;
}

/* ColorInput sizes (issue #263 — `size` used to reach nothing).

   Every ColorInput carries exactly one of these, `--md` when `size` is unset or
   unrecognised, so `md` reproduces the height, padding and font-size the field
   was hard-coded to before and the default rendering is unchanged. The preview
   swatch scales alongside, through the `size` the component hands `ColorSwatch`.

   The steps are ColorInput's own rather than TextInput's: anchoring `md` at
   TextInput's 2.625rem would have resized every existing ColorInput. */
.rinch-color-input--xs .rinch-color-input__input {
    height: 26px;
    padding: 0 6px;
    font-size: var(--rinch-font-size-xs, 12px);
}

.rinch-color-input--sm .rinch-color-input__input {
    height: 30px;
    padding: 0 7px;
    font-size: var(--rinch-font-size-xs, 12px);
}

.rinch-color-input--md .rinch-color-input__input {
    height: 34px;
    padding: 0 8px;
    font-size: var(--rinch-font-size-sm, 14px);
}

.rinch-color-input--lg .rinch-color-input__input {
    height: 40px;
    padding: 0 10px;
    font-size: var(--rinch-font-size-md, 16px);
}

.rinch-color-input--xl .rinch-color-input__input {
    height: 48px;
    padding: 0 12px;
    font-size: var(--rinch-font-size-lg, 18px);
}

/* ColorInput radius (issue #263 — `radius` used to reach nothing).

   Unlike size there is no `--radius-md` default class: an unset or unrecognised
   `radius` emits none of these and the base rule's `--rinch-radius-sm` stands.
   That is the idiom DropdownMenu, Modal and Card already use. */
.rinch-color-input--radius-xs .rinch-color-input__input-group { border-radius: var(--rinch-radius-xs); }
.rinch-color-input--radius-sm .rinch-color-input__input-group { border-radius: var(--rinch-radius-sm); }
.rinch-color-input--radius-md .rinch-color-input__input-group { border-radius: var(--rinch-radius-md); }
.rinch-color-input--radius-lg .rinch-color-input__input-group { border-radius: var(--rinch-radius-lg); }
.rinch-color-input--radius-xl .rinch-color-input__input-group { border-radius: var(--rinch-radius-xl); }

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

/* Backdrop — the invisible box that catches outside clicks while the dropdown
   is open (issue #465). `ColorInput::render` appends it to the wrapper itself.

   `fixed` is what makes "outside" mean the whole window rather than whatever
   clips the field. An absolute box IS clipped by an `overflow` ancestor in its
   containing-block chain — CSS, not a rinch quirk — so inside a sidebar, a
   table cell or any form panel narrower than the window the dismiss region
   would stop exactly where the picker does. A fixed box's clip chain is empty.
   It also puts the backdrop above the app's own fixed chrome (a hand-rolled
   titlebar has no z-index, so it enters at 0), which is why clicking the
   titlebar dismisses.

   That is the same rule, and the same history, as
   `.rinch-dropdown-menu__backdrop`: #317 respelled that one `absolute` because
   rinch used to make an `overflow` clip a stacking context *and* hoist a fixed
   box out of every ancestor stacking context, so the backdrop outranked the
   panel it was supposed to sit under. #324 stage B and #545 undid both halves
   and stage C took it back to `fixed`. The long note above that rule is the
   full account; do not respell this one without reading it. Measured here by
   `color_input_dismiss_465_tests::a_tap_outside_the_clipping_shell_dismisses`,
   which taps outside an `overflow: hidden` shell — the one place the two
   spellings disagree. */
.rinch-color-input__backdrop {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 999;
    display: none;
}

/* Revealed by the same `--opened` class as the panel, and spelled with child
   combinators all the way down.

   Both steps are direct children by construction — `render` appends the
   wrapper to the root and the backdrop to the wrapper — so unlike a *caller's*
   child this can never grow a `display: contents` wrapper in front of it, which
   is the trap #774 documents for `DropdownMenu`'s panel. What the `>` buys is
   the opposite guarantee: the rule reaches this input's own backdrop and
   nothing else. */
.rinch-color-input--opened > .rinch-color-input__wrapper > .rinch-color-input__backdrop {
    display: block;
}

/* The field is lifted ABOVE the backdrop while the dropdown is open, and this
   is the one place `ColorInput` departs from `DropdownMenu`, whose trigger is a
   button: here the trigger *is* a text input, and clicking into it to place a
   caret has to keep working while the picker is open. Under the backdrop that
   click is swallowed — it would still dismiss (that is what a backdrop does)
   but the field would never take the keyboard.

   Only while open, so a closed `ColorInput` creates no stacking context and
   nothing about an existing layout moves. The three levels are
   999 < 1000 (`__dropdown`) < 1001, and the middle one is why a click on a
   swatch inside the picker picks a colour instead of dismissing. */
.rinch-color-input--opened .rinch-color-input__input-group {
    position: relative;
    z-index: 1001;
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
