pub fn styles() -> String {
    r#"
/* Modal root.
   `top` clears any window chrome rinch draws itself (the in-app menu bar, the
   BorderlessWindow titlebar) — those reserve space with in-document padding,
   which a fixed element correctly ignores. Without it the modal centres against
   the whole window and, once the window is short enough that the modal fills
   it, the top of the dialog is hidden behind the chrome. Fallback 0px.

   #2508: the root also *sizes* the panel. It is a flex column filling the
   area below the inset, and the panel is a flex item that shrinks to fit its
   padding box, so a tall modal ends one `xl` above the window bottom whatever
   the inset. The panel used to cap itself with a `100vh` calc that knew
   neither the inset nor its own margin, and ran off the window by both. */
.rinch-modal__root {
    position: fixed;
    top: var(--rinch-window-top-inset, 0px);
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    padding: var(--rinch-spacing-xl);
    /* #474: `Modal { z_index }` publishes --rinch-modal-z-index on this
       root; the panel below derives its own level from the same number, so
       one prop moves both and the panel stays one above the overlay. */
    z-index: var(--rinch-modal-z-index, 200);
}

/* Hidden state */
.rinch-modal__root--hidden {
    display: none !important;
}

/* Modal overlay — absolute within the fixed root */
.rinch-modal__overlay {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    /* #474: `overlay_opacity` publishes --rinch-modal-overlay-opacity on this
       element; the alpha channel is where it belongs, not an `opacity`
       declaration — the backdrop is already 75% transparent, so an `opacity`
       would compose with that alpha instead of replacing it (and would make
       the overlay a stacking context). The fallback is the value that was
       hard-coded here, so an unset prop paints exactly as before. */
    background-color: rgba(0, 0, 0, var(--rinch-modal-overlay-opacity, 0.75));
    backdrop-filter: blur(var(--rinch-modal-overlay-blur, 0));
}

/* Modal container.
   A flex item of the root's column (#2508): `min-height: 0` lets it shrink
   below its content to the root's padding box, and `max-width: 100%` keeps a
   fixed-width size inside a narrow window. It is a flex column itself, so the
   header keeps its height and the body takes what is left and scrolls. */
.rinch-modal {
    position: relative;
    display: flex;
    flex-direction: column;
    flex-shrink: 1;
    min-height: 0;
    max-width: 100%;
    background-color: var(--rinch-color-body);
    border-radius: var(--rinch-radius-default);
    box-shadow: var(--rinch-shadow-xl);
    overflow: hidden;
    margin-top: var(--rinch-spacing-xl);
    z-index: calc(var(--rinch-modal-z-index, 200) + 1);
}

/* Centered modal: auto margins on the column's main axis. When the panel is
   taller than the area they resolve to 0 and it fills it, top to bottom. */
.rinch-modal--centered {
    margin-top: auto;
    margin-bottom: auto;
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
    flex-shrink: 0;
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

/* Modal body: the scroller (#2508), so the header stays in view. */
.rinch-modal__body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--rinch-spacing-lg);
}

/* Modal footer (`Modal { footer }`): the panel's last row. `flex-shrink: 0`
   keeps it whole while the body (`flex: 1 1 auto; min-height: 0`) shrinks,
   so the body gives way first, down to zero, in a short window. It pads like
   the body. Its children sit at the end with a `sm` gap, and a top border
   separates it from the body. The border is always drawn: "only while the body
   is scrolled away from its bottom" needs the body's overflow at open, after a
   resize and after a content change, and rinch has no post-layout signal to
   set that from (a scroll listener alone is wrong until the first scroll). An
   app that wants it gone writes `.rinch-modal__footer { border-top: none; }`. */
.rinch-modal__footer {
    flex-shrink: 0;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: flex-end;
    gap: var(--rinch-spacing-sm);
    padding: var(--rinch-spacing-lg);
    border-top: 1px solid var(--rinch-color-border);
}

/* An rsx `Fragment { … }` is a `<div data-fragment>`: let its children be the
   row's items, so `footer` returning a Fragment of buttons gets the gap. */
.rinch-modal__footer > [data-fragment] {
    display: contents;
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
