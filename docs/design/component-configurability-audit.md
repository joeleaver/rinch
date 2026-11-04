# Component configurability audit (2026-11-04)

**Status:** audit only. No component code changed. One GitHub issue per component carries the proposals (see [Issues](#issues)).

## Why

Joe, 2026-11-04: *"I'm concerned about these decisions in a framework. Sensible defaults are nice, but some things should be left up to the implementer."* That came from sortable lists (#2093) turning off the drag ghost with no option, which blocked a kanban board in an app.

Two rules follow, and every proposal here applies both:

1. **A component may choose a default, but a UX choice must be an option, plus an escape hatch** (exposed state, a custom render function, a callback, a CSS variable or class).
2. **A sensible default means "what Chrome would do"** (Joe, same day). Where Chrome has a native equivalent (the HTML drag image, `<dialog>` focus steps, `<select>` popups and keys, `<input type=range>` events, `::selection`, caret blink, `<details>`, light dismiss of `popover`, `scrollIntoView`), the proposed default is Chrome's behaviour. Only where Chrome has no equivalent does the component keep its own default, which is then today's.

## Scope and method

- Every component in `crates/rinch-components/src` and its `styles/*.rs` stylesheet, plus the rich-text editor's built-in UI (`crates/rinch-editor-view`, and the defaults `rinch-editor-core` installs).
- Each finding was checked against the component's props, CSS variables, classes and callbacks, and `docs/src/guide/component-props.md`. A finding is listed when none of those changes it, or they change it only in part, or when the default differs from Chrome's.
- Each finding has a ***Chrome:*** line saying what Chrome does for the equivalent native feature, or "no equivalent". Where the auditor could not confirm Chrome's behaviour, the line says "unsure"; check those before implementing.
- **`Differs from Chrome`** marks a finding whose behaviour today differs from Chrome's. That is a bug-like gap, separate from configurability. **`Differs from Chrome (default only)`** marks one that is already configurable but whose default differs.
- **Inline styles count as "cannot change".** A `set_style`/`style=` write beats any class rule in an app stylesheet; only `!important` gets past it. A rule in a generated stylesheet can be overridden by a more specific selector, so those are listed only when the value should obviously be a variable or the rule uses `!important`.
- **Excluded:** SortableList's drag preview, ghost and gap reflow, which #2154 is already making configurable. (Under rule 2 its default should be a ghost that is on: Chrome shows a drag image.)
- Likelihood is how likely an app is to want the behaviour different: **High**, **Medium** or **Low**.
- File:line references are against `main` at `e03f79a25`.

**Moving a default to Chrome's changes behaviour for existing apps.** The findings say so where it matters: markdown shortcuts off and Tab moving focus in the editor (ui-zoo and Pimble rely on both), `onchange` meaning "end of gesture" on Slider/ColorPicker/ColorInput, `Accordion.multiple`, `Button` `type`, `Image` `fit`. Each of those needs a call from Joe (flip the default and note it, or keep today's as a documented exception) before it is implemented.

## Where rinch differs from Chrome

About 100 findings are marked. The most significant, roughly in order:

| Where | Today | Chrome |
|---|---|---|
| `crates/rinch-theme/src/css.rs:186` | **Reduced motion never applies on desktop.** Stylo's servo build (0.22, `servo/media_features.rs`) has no `prefers-reduced-motion`, so the theme's `@media` rule can never match, and nothing reads the OS setting. | Honours the OS setting (so does rinch-web). |
| Editor, `keys` / `contenteditable.md:1016-1022` | Tab stays in an editable editor, and outside a list it does nothing: a keyboard trap in a form. | Tab leaves a `contenteditable`. |
| Editor, `rinch-editor-core/src/lib.rs:79` | Markdown shortcuts always rewrite typed text. | Never rewrites typed text. |
| Editor, `view.rs:3558` | The selection disappears when the editor loses focus. | Keeps it, painted in the inactive colour. |
| Editor, `view.rs:3527`, `:3251`, `:3652` | Caret, selection, drop caret and IME text use a fixed `#1a73e8`; the IME box is always white (a white box in dark mode). | `caret-color: currentColor`, `::selection`/`Highlight`, IME underline in `currentColor`. |
| DropdownMenu, ContextMenu | No Escape, no arrow/Home/End/type-ahead navigation, focus not moved in. ContextMenu doesn't reopen on a second right-click and can't be opened with Shift+F10 or the Menu key. | Every native menu and `<select>` popup has all of these. |
| Popover, `popover.rs:327-366` | An invisible backdrop swallows the click that dismisses it. | `popover=auto` light dismiss closes it and the click still reaches the target. |
| Popover, `overlay_focus.rs:148` | With the default `trap_focus: false`, no `autofocus` on open and no focus restore on close. | `popover` honours `autofocus` and restores focus without trapping. |
| Modal / Drawer | No `role="dialog"`/`aria-modal`; close on backdrop click by default; scroll lock by default; 75% black backdrop; not centred. | `<dialog>.showModal()`: implicit roles, Escape only, no scroll lock, 10% backdrop, centred. |
| Tooltip, `tooltip.rs:150-167` | Shows instantly, never on focus, clipped at the window edge. | `title` tooltips appear after a delay. |
| Slider, `slider.rs:197-281` | No keyboard, focus or ARIA; `onchange` every frame. | `<input type=range>`: arrows/PageUp/Home/End, `input` per frame and `change` on release. |
| PasswordInput, `password_input.rs:266` | `autocomplete="off"` hard-coded: password managers stay away. | No `autocomplete` attribute: managers fill. |
| NumberInput | Arrow keys don't step on desktop; steppers inert when controlled; no hold-to-repeat or wheel. | Arrows step, spin buttons repeat while held. |
| Radio, `radio.rs:218-222` | Unnamed radios all share `name="radio-group"`; no arrow keys between radios. | An unnamed radio is in no group; arrows move within a group. |
| Select | Clipped by `overflow` ancestors; arrows wrap; closed Select ignores keys; 900 ms type-ahead. | Popup is a top-layer window; no wrap; arrows change a closed select; ~1 s type-ahead. |
| Checkbox, Switch, Radio | Labels can't hold links and never wrap; indeterminate half-wired. | `<label>` takes any content and wraps; `indeterminate` renders. |
| Accordion | `value`/`default_value`/`multiple` ignored; `onclick` dropped; no open state. | `<details open>`, `name` for exclusive groups, the `toggle` event. |
| Tree, `tree.rs:1427`, `:1136-1140`, `:1315-1331` | No drag ghost; every row a Tab stop and keyboard off by default; Ctrl-click toggles on macOS. | Drag image by default; a listbox is one Tab stop and works from the keyboard; platform modifiers. |
| BorderlessWindow, `borderless_window.rs:269-369` | Controls in Windows order on every platform; no restore glyph; no double-click to maximize. | Follows the platform's titlebar layout. |
| Anchor, Image, Progress | `href="#"` with no `onclick`; `fallback_src` inert on desktop; no indeterminate progress; stale `aria-valuenow`. | Placeholder links + click listeners; `onerror`/alt text; `<progress>` with no value is indeterminate. |
| Component stylesheet (`styles/mod.rs`) | Rules that need three classes to beat; no unstyled mode. | The UA stylesheet loses to any author rule. |

## Props that do nothing (bugs found along the way)

These are not missing options. The prop exists, and changing it has no effect. They are in the per-component issues, marked as bugs.

| Where | What happens |
|---|---|
| `accordion.rs:137-148`, `:180` | `value`, `default_value` and `multiple` are written to the root as data attributes that nothing reads. Every item starts closed, and items open independently. |
| `accordion.rs:282-290` vs `:198-203` | `AccordionControl.onclick` never fires: `AccordionItem` overwrites the control's `data-rid` with its toggle. |
| `styles/breadcrumbs.rs:34-36` | Any `separator` other than `/ > >> - •` renders as `/`: the `[data-separator]` fallback rule always matches. |
| `button.rs:143-146` | `loading` creates no spinner, so the button just stops responding (already #1334). |
| `image.rs:157` | `fallback_src` is a JavaScript `onerror` string, which does nothing on desktop. |
| `styles/stepper.rs:127-129` | A `StepperStep`'s children are `display: none` in every state. |
| `checkbox.rs:79-108` | `indeterminate` never adds the `rinch-checkbox--indeterminate` class its stylesheet targets, so an indeterminate unchecked box renders empty. |
| `slider.rs:222-231` | The value label is computed once at render and never updates. `show_label_on_hover: false` is also ignored by the CSS. |
| `password_input.rs:386-392` | The eye toggle is shown by default but does nothing unless the app wires both `ontoggle` and `visible_fn`. |
| `radio.rs:218-222` | Every unnamed radio shares `name="radio-group"`, so on the web, checking one unchecks unrelated radios. |
| `navlink.rs:246-251` | A nested NavLink never toggles open on click. A disabled `<a>` NavLink still fires `onclick` (`:314-320`). |
| `progress.rs:218-220` | `aria-valuenow` is written once and does not follow `value_fn`. |
| `crates/rinch-theme/src/css.rs:186` | `respect_reduced_motion` has no effect on desktop (see above). |
| `rinch-editor-core` `PlaceholderPlugin` doc | Points to an `Editor { placeholder }` prop that does not exist. The guide's tip to extend `markdown_input_rules()` cannot work: the function returns a fresh list the installed plugin never reads. |
| `borderless_window.rs:414-436` | Not a bug as such, but the inline-titlebar title is drawn as a hard-coded six-colour rainbow with inline colours. It looks like demo branding that leaked into the framework. |

## Patterns that recur

Each of these shows up in several components. Fixing the pattern once (a shared helper, a theme variable) is cheaper than fixing each component.

1. **Inline styles for presentation.** Placement (`floating::arm` writes `top/left/width/max-height` inline for Popover and DropdownMenu), the ContextMenu's `z-index: 200`, Accordion's panel `display`, Select's chevron rotation, ColorSwatch geometry, SimpleGrid's columns, Tree's indent, Sortable's reason box, BorderlessWindow's content padding, and all of the editor's caret, selection, node-outline, IME and block alignment/indent styles. **Fix shape:** write a CSS variable or a class inline, and keep the property in the stylesheet, e.g. `background-color: var(--rinch-editor-caret-color, currentColor)`.
2. **No open/close animation possible.** Modal and Notification hide with `display: none !important`; Accordion with inline `display`. Drawer's 300 ms slide is written in two rules that must agree. **Fix shape:** Drawer and Popover's `visibility` pattern, with a duration variable.
3. **State held privately.** Tabs' active tab, ContextMenu's, Select's and ColorInput's open state, Accordion items, NavLink children, Tooltip and HoverCard visibility. There is no `_fn` prop, no change callback and no way to open programmatically. **Fix shape:** the existing `_fn` + `on…change` convention.
4. **Per-frame `onchange`.** Slider, ColorPicker, ColorInput and FloatingPanel report every drag frame and never the end, so undo history and persistence get one entry per frame. **Fix shape:** Chrome's split, `oninput` per frame and `onchange` at the end of the gesture (or `onchange_end`, if renaming is too disruptive).
5. **Fixed English strings.** Sortable's announcements and item description, close/loader/breadcrumb/pagination aria labels, "Show password", "Increment", window-control labels, the required `*`, FloatingPanel's "Close panel". **Fix shape:** a string prop per component with today's text as the default; for Sortable, a callback.
6. **Hard-coded `color: white` contrast text** in about 11 places (tabs, pagination, action icon, stepper, avatar, navlink, alert, badge, button, tooltip). A light accent colour gives unreadable filled components. **Fix shape:** a theme `--rinch-primary-contrast` (and per-colour contrast variables), computed like Chrome's `accent-color` contrast.
7. **Focus moves tied to one switch.** In Modal, Drawer and Popover, moving focus in on open and restoring it on close both hang off `trap_focus` (`overlay_focus.rs:148`), and the first focus target is fixed per component. **Fix shape:** `initial_focus` and `return_focus` props, defaulting to `<dialog>`/`popover` focus steps (autofocus or first focusable in; restore on close).
8. **Inner fields unreachable.** Attributes on TextInput, Textarea, PasswordInput, NumberInput and ColorInput land on the root, not the `<input>`; there is no `name`, `autocomplete`, `maxlength`, `readonly`, `inputmode`, and no `onfocus`/`onblur`/`onkeydown`.
9. **Built-in chrome glyphs.** Close X, chevrons and checks are fixed SVGs (`icons.rs:157-272`) used across Accordion, Pagination, CloseButton, Alert, Stepper, Select, NavLink, Tree and NumberInput. **Fix shape:** an `IconSet` context read by the `*_dom` helpers.
10. **Stylesheets that are hard to beat.** The component sheet is injected whole, with no unstyled mode and no `@layer`; some active states need three-class selectors. The editor's sheet is appended to `<body>` after the app's styles, so an equal-specificity app rule loses, and none of its colours uses theme variables.

## Ranked: the High findings

Sorted roughly by how much an app is held back, most first. Defaults in the Proposal column follow Chrome where it has an equivalent. Detail is in the per-component sections.

| # | Component | Where | Decided for the app | Proposal (default) |
|---|---|---|---|---|
| 1 | Editor | `rinch-editor-core/src/lib.rs:79`; `rinch-editor-view/src/lib.rs:82` | Markdown shortcuts always on; schema always the starter kit; plugin set fixed | `create_editor_with(EditorConfig { schema, plugins, history })`; `Editor { markdown_shortcuts }` (Chrome: off; breaking) |
| 2 | Editor | `keys`, `contenteditable.md:1016-1022` | Tab kept in the editor, a no-op outside lists | Tab moves focus (Chrome); opt in to Tab-indents |
| 3 | Tree | `tree.rs:1427` | Drag ghost suppressed on every drag | `drag_ghost: bool` (Chrome: on) / a preview render fn matching #2154 |
| 4 | ContextMenu | `context_menu.rs:33-37`, `:182-350` | Open state private; no Escape; no keyboard nav | `opened_fn`, `on_open`/`on_close`, `open_at(x, y)`, Escape + keyboard nav on |
| 5 | DropdownMenu | `dropdown_menu.rs:325-512`, `:582-584` | No Escape, no keyboard nav; item sections are Tabler icons only | Escape + keyboard nav on; node slots; `shortcut` |
| 6 | Accordion | `accordion.rs:137-148`, `:282-290` | `value`/`default_value`/`multiple` ignored; control `onclick` dropped | Make them work; `value_fn` + `onchange`; independent items by default (`<details>`) |
| 7 | Tabs | `tabs.rs:218-224` | Active tab private; no `onchange` | `onchange`, `value_fn` |
| 8 | Editor | `view.rs:3527-3539`, `:3251-3260`, `:3558`; `styles.rs` | Caret/selection colours inline; selection hidden on blur; stylesheet without theme vars, injected after the app's | `var(--rinch-editor-…, <Chrome's>)`; keep an inactive selection; sheet first or in a layer |
| 9 | Editor | `component.rs:50-79` | No placeholder prop | `Editor { placeholder }`, `set_placeholder` |
| 10 | Popover / DropdownMenu | `floating.rs:226`, `:378-420`; `popover.rs:327-366` | Flip/shift/size always on and inline; dismissing click swallowed | `flip`, `shift`, `collision_padding`, `auto_size`; pass the click through (Chrome light dismiss) |
| 11 | Modal / Drawer / Popover | `overlay_focus.rs:148` | Focus-in and restore tied to `trap_focus` | `initial_focus`, `return_focus` (Chrome's dialog/popover focus steps) |
| 12 | Modal | `styles/modal.rs:26-28`; `modal.rs:171`, `:176` | Instant hide (`!important`); closes on backdrop; scroll lock | visibility-based hide + duration; Escape-only and no lock by default (Chrome) |
| 13 | Drawer | `drawer.rs:403-409` | Body scroll reset on every open | `keep_scroll: bool` |
| 14 | Select | `select.rs:130-158`, `:432-474` | No search, clear, disabled options, groups or option render | `searchable`, `clearable`, `SelectOption.disabled`, groups, `render_option` |
| 15 | Select | `select.rs:183-184`, `:371-379` | Up/down by a fixed 220 px guess; clipped by scroll ancestors | portal by default (Chrome's popup is top-layer); `position`, `max_dropdown_height` |
| 16 | Inputs (several) | `text_input.rs:164-222` etc. | Inner `<input>` attributes and focus/key callbacks unreachable | `input_attrs`, `onfocus`, `onblur`, `onkeydown` |
| 17 | TextInput | `text_input.rs:169-222` | No left/right section | `left_section`, `right_section` render fns |
| 18 | PasswordInput | `password_input.rs:266`, `:386-392` | `autocomplete="off"`; toggle inert uncontrolled | `autocomplete` (Chrome: unset); internal visibility signal |
| 19 | Slider | `slider.rs:222-231`, `:242-275`, `:197-281` | Label never updates; per-frame `onchange`; no keyboard/ARIA | `label_fn`; `oninput`/`onchange` split; keyboard on |
| 20 | ColorPicker / ColorInput | `color_picker.rs:241-248`; `styles/color_picker.rs:353-357` | Per-frame `onchange`; dropdown never flips | `oninput`/`onchange` split; `position` / `floating.rs` |
| 21 | Checkbox / Switch | `checkbox.rs:79-108`, `:187-191`; `switch.rs:185-190` | Indeterminate half-wired; label is plain text | the class + `indeterminate_fn`; children as the label |
| 22 | Radio | `radio.rs:218-222`, `:318-335` | Shared default name; RadioGroup not a controlled group | unique names; `RadioGroup { value_fn, onchange, name }`; arrow keys |
| 23 | NumberInput | `number_input.rs:401-404` | Steppers inert when controlled; no arrow keys on desktop | step in controlled mode; arrows, hold-to-repeat (Chrome) |
| 24 | Tooltip | `tooltip.rs:150-167` | Instant, never on focus, no controlled mode | `open_delay` (Chrome-like delay), `close_delay`, `open_on_focus`, `opened_fn` |
| 25 | HoverCard | `hover_card.rs:144-172` | Pure CSS `:hover`; no open callback | `opened_fn`, `on_open_change`, `open_on_focus` |
| 26 | Notification | `styles/notification.rs:14`, `:32-59` | Each toast fixed at a corner; two overlap | `NotificationStack { position, gap, limit }` |
| 27 | FloatingPanel | `floating_panel.rs:137-183` | Unbounded drag/resize; no gesture-end callbacks | `bounds`, `max_*`, `on_move_end`, `on_resize_end` |
| 28 | LoadingOverlay | `loading_overlay.rs:77`, `:145-150` | Children ignored; built-in loader only | loader slot + `message` |
| 29 | NavLink | `navlink.rs:246-251`, `:370-375` | Nested children never toggle | internal toggle, `opened_fn`, `on_opened_change` |
| 30 | Breadcrumbs | `styles/breadcrumbs.rs:34-36`; `breadcrumbs.rs:70-83` | Arbitrary separators show `/`; items can't route by callback | real separator span; `BreadcrumbsItem { onclick, active }` |
| 31 | Stepper | `styles/stepper.rs:127-129` | Step content never shown | show the active step's content below the steps |
| 32 | Button | `button.rs:143-146` | `loading` shows no spinner (#1334) | render label + loader; `loader` slot |
| 33 | Anchor | `anchor.rs:44-63` | No `onclick`; empty href becomes `#` | `onclick`; no `href` when none given (Chrome placeholder link); `rel` |
| 34 | BorderlessWindow | `borderless_window.rs:414-436`, `:269-369` | Rainbow title; Windows control order everywhere | plain title (`brand_renderer` for the rest); platform control layout |
| 35 | Global theme | `crates/rinch-theme/src/css.rs:186` | Reduced motion never applies on desktop | read the OS setting; teach Stylo or rinch the media feature |
| 36 | Sortable | `sortable.rs:1018`–`1244` | Keyboard announcements fixed English | `announce: Fn(&SortAnnouncement) -> String` |
| 37 | Tree | `tree.rs:1239`, `:1315-1331`, `:1136-1140` | Chevron fixed; click model fixed; every row a Tab stop | `render_chevron`; `handle_click`, platform toggle modifier; roving tabindex |
| 38 | Image / Avatar | `image.rs:155-158`; `avatar.rs:203-210` | No working load-failure path on desktop | `on_error`, `fallback` slot |

## Issues

One issue per component (small text primitives grouped). See the table at the end of this file, filled in when the issues were filed.

---

## Findings by component

The rest of this document is the full set of findings, grouped by area. Each finding gives `file:line`, what is decided, whether an app can change it today, what Chrome does, a proposed option, and the likelihood.


## Part A: Tree, Sortable, late_children

Read-only audit of `crates/rinch-components/src/{tree.rs, sortable.rs, late_children.rs}` and `styles/{tree.rs, sortable.rs}`, against `docs/design/sortable.md`. Each item below was checked against the props, CSS vars, classes and callbacks, and against the root-attribute routes (`class:`/`style:` and hyphenated or quoted attributes, which the caller writes last). An item is listed only when none of those routes changes it, or a route changes it only in part.

**Chrome rule (2026-11-04): "sensible defaults means 'like Chrome would do it'".** Each finding now has a `*Chrome:*` line, and its proposed default is Chrome's behaviour where Chrome has an equivalent. Chrome has no native tree and no sortable list. The closest equivalents used here are:
- HTML drag-and-drop, where Chrome draws a translucent drag image of the dragged element by default;
- the native listbox (`<select size=N>` / `<select multiple>`) for selection, keyboard and Tab-stop behaviour, since that is Chrome's only native selectable list;
- `<details>`/`<summary>` for disclosure;
- the UA stylesheet's nested-list indent for indentation;
- `focus()`, which scrolls minimally (nearest).

Where none of these applies, the finding says "no equivalent" and keeps today's default.

Excluded, as the brief says: SortableItem drag ghost, preview and gap-reflow (#2154). That covers `sortable.rs:1515` `suppress_drag_ghost` (inside the `data-ondragstart` handler at `sortable.rs:1514-1517`) and `data-drag-ghost="false"` at `sortable.rs:1493`. **Note for #2154 only (not counted here):** Chrome's HTML DnD shows a drag image by default, so by the revised rule #2154's default should be a ghost that is on.

### Tree (tree.rs, styles/tree.rs)

#### High

- **Differs from Chrome:** Chrome shows a drag image, and this tree shows none. **`crates/rinch-components/src/tree.rs:1427`: drag ghost suppressed unconditionally.** `wire_drag`'s `data-ondragstart` handler calls `rinch_core::suppress_drag_ghost()` on every drag, and the row also gets no `data-drag-ghost` attribute.
  - *Chrome:* HTML drag-and-drop shows a translucent snapshot of the dragged element under the pointer by default. `DataTransfer.setDragImage()` replaces it, and a transparent image hides it. With a multi-item drag, Chrome still shows only the pressed element unless the page sets a custom image.
  - *Can the app change it today?* No. No prop reaches it. A `data-ondragstart` the app adds would replace the tree's own handler id, which breaks the drag.
  - *Proposed option:* `drag_ghost: bool`, **default `true`** (Chrome: a snapshot of the pressed row). `false` gives today's behaviour. Better still, `render_drag_preview: Option<Rc<dyn Fn(&[String], &mut RenderScope) -> NodeHandle>>` (the `setDragImage` counterpart) for a "3 items" badge, matching whatever #2154 settles on for Sortable.
  - *Likelihood:* High. A drag out of the tree to another target (`ondragstart` is documented for exactly this) shows nothing under the pointer once it leaves the tree. Multi-row drags also want a count badge.

- **`tree.rs:1239` (and the static path, `tree.rs:1807`): the expand chevron is fixed.** The node is always `crate::icons::chevron_right_dom`. It always comes first in the row, before the icon and label. It is always added for a row with children.
  - *Chrome:* `<details>`/`<summary>` draws a disclosure triangle as the summary's `::marker` (`list-style-type: disclosure-closed` / `disclosure-open`), at the start of the line. Author CSS can replace it (`summary::marker { content: … }`, `list-style`) or remove it (`list-style: none`), and can then draw its own marker anywhere. The glyph itself (a triangle against a chevron) is a theme matter, so this is not marked as differing. What Chrome allows and rinch does not is replacing or moving the marker.
  - *Can the app change it today?* Partly. CSS can restyle or rotate `.rinch-tree__chevron` (`styles/tree.rs:55-74`). It cannot swap the glyph (for +/−, a caret or a triangle) or move the chevron to the right. `render_node` replaces only the label (`tree.rs:1294-1309`).
  - *Proposed option:* `render_chevron: Option<Rc<dyn Fn(&RenderTreeNodePayload, &mut RenderScope) -> NodeHandle>>`, which keeps the click wiring on the wrapper span. The simpler alternative is `chevron_icon: Option<TablerIcon>` plus `chevron_expanded_icon: Option<TablerIcon>`. The default stays today's themed chevron.
  - *Likelihood:* High. A file explorer, an outline editor and a settings tree each have their own disclosure glyph, and plus/minus is common on Windows.

- **Differs from Chrome:** on macOS a Ctrl-click toggles the selection here, where Chrome opens the context menu. On Windows and Linux, Meta-click toggles here as well. **`tree.rs:1315-1331`: row click semantics are fixed.** Ctrl **or** Meta toggles, Shift adds a run, a plain click replaces the selection, and the click also toggles expansion when `expand_on_click` is set.
  - *Chrome:* in a `<select multiple>` listbox, the toggle modifier follows the platform. On Windows and Linux it is Ctrl. On macOS it is Cmd, and Ctrl-click is the secondary click: it fires `contextmenu` and does not toggle. Shift-click selects the range from the anchor, and a plain click replaces the selection. Chrome: unsure whether a Win/Super-click does anything in a listbox on Windows or Linux; it is not a toggle there that I know of.
  - *Can the app change it today?* No. `handle_key` covers keys only. The row's `data-rid` is the tree's own handler, and there is no click intercept.
  - *Proposed option:* `toggle_modifier: ToggleModifier { Platform, CtrlOrMeta, Meta, Ctrl }`, **default `Platform`** (Cmd on macOS, Ctrl elsewhere, as Chrome does). On macOS a Ctrl-click then goes to the context-menu path instead of toggling. Also add `handle_click: Option<Rc<dyn Fn(&TreeClick) -> bool>>`, the click counterpart of `handle_key`. `TreeClick` would carry the value, the modifiers and the button; returning `true` consumes the click.
  - *Likelihood:* Medium-High. Mac-native apps, and any app with its own click model.

#### Medium

- **Differs from Chrome** (`multiple` trees only): a plain arrow here moves the cursor and selects nothing, where Chrome's `<select multiple>` moves the selection with it. **`tree.rs:2612-2616`: in a single-select tree the selection follows the keyboard cursor.** `move_cursor` calls `select_from_keyboard` on every arrow press and every type-ahead step. In a `multiple` tree it never does: Space toggles the row instead (doc comment at `tree.rs:850-866`).
  - *Chrome:* in a single-select `<select size=N>`, the arrow keys move the selection (selection follows focus), so the single-select default matches. In a `<select multiple>`, a plain arrow also replaces the selection with the new option, and Shift+arrow extends it. Ctrl+arrow (on Windows and Linux) moves the active option without changing the selection, and Ctrl+Space toggles it. Chrome: unsure about the exact macOS modifier for "move without selecting" in a multi-select listbox, and about what a plain Space does there.
  - *Can the app change it today?* Partly. With `select_on_click: false` nothing selects from a click either. The other route is to consume every navigation key in `handle_key` and re-implement navigation through `TreeController::set_cursor`.
  - *Proposed option:* `selection_follows_cursor: bool`, **default `true` in both modes** (Chrome's listbox). In a `multiple` tree a plain arrow then replaces the selection, and Ctrl+arrow moves the cursor alone. `false` gives arrows that only move the cursor, with Space or Enter to select: today's `multiple` behaviour, and the WAI-ARIA "selection does not follow focus" model.
  - *Likelihood:* Medium-High. Apps where selecting a row is expensive, such as opening a file, do not want every row the arrow passes to be selected. `oncursor` with its delay helps only part of the way, because `onselect` still fires on each row.

- **Rows have no context-menu or double-click hook (`tree.rs:1311-1331`).** The only row handler is click. There is no `oncontextmenu`, no `ondblclick` and no "activate" callback (Enter or double-click opens).
  - *Chrome:* `contextmenu` and `dblclick` fire on every element and bubble, so a page gets both for any row by listening on the container and reading `event.target`. A native listbox has no "activate" event. Chrome: unsure whether a right-click on a `<select multiple>` option moves the selection to it. This is about reach, not a different default, so it is not marked as differing.
  - *Can the app change it today?* Partly. The app can put handlers inside `render_node`, but that covers only the label area, not the chevron, the icon or the indent padding.
  - *Proposed option:* `onrowcontextmenu: Option<ValueCallback<String>>` (which also moves the cursor and, optionally, the selection to the row) and `onactivate: Option<ValueCallback<String>>` (double-click, or Enter in a single-select tree). A possible addition is `expand_on_double_click: bool` (default `false`).
  - *Likelihood:* Medium-High. A file tree with no right-click menu and no double-click to open is unusual.

- **Differs from Chrome:** Chrome indents nested lists from its UA stylesheet, which any author rule overrides. This indent is inline, so only `!important` beats it. **`tree.rs:1141-1144` (static path `tree.rs:1799-1802`): row indentation is an inline style.** The row gets `padding-left: calc(var(--tree-level-offset) * level)` inline.
  - *Chrome:* a nested `<ul>`/`<ol>` is indented by the UA stylesheet (`padding-inline-start: 40px`). Because that is a UA rule, any author selector replaces it with no `!important`.
  - *Can the app change it today?* Partly. The step can be changed through `level_offset`/`indent` or `--tree-level-offset`. Because the declaration is inline, a stylesheet cannot add a constant gutter (`8px + level * step`), indent with margin instead so the hover or selection highlight is not full-width, or use a different formula, unless it reaches for `!important`.
  - *Proposed option:* write only `--tree-level: {level}` inline and move the calc into the stylesheet: `padding-left: calc(var(--tree-row-inset, 0px) + var(--tree-level-offset) * var(--tree-level))`. This keeps today's rendering as the default and makes it overridable as Chrome's is. The guide's inline `left` at `tree.rs:1349-1354` should get the same change.
  - *Likelihood:* Medium. Apps routinely want a left gutter, or a highlight that starts at the indent.

- **`tree.rs:1286-1291` (static `tree.rs:1815-1820`): the node icon is fixed to an outline Tabler icon.** The style is always `TablerIconStyle::Outline`. There is one icon per node, with no expanded or collapsed variant (an open versus closed folder) and no custom node.
  - *Chrome:* no equivalent.
  - *Can the app change it today?* Partly. The app can leave `icon` as `None` and draw its own icon in `render_node`. That icon then sits in the label slot rather than `.rinch-tree__icon`, and it has to track expansion itself through the payload's `expanded_signal`.
  - *Proposed option:* `TreeNodeData::with_icon_expanded(TablerIcon)` and `icon_style: TablerIconStyle` (default `Outline`) on `Tree`, or `render_icon: Option<Rc<dyn Fn(&RenderTreeNodePayload, &mut RenderScope) -> NodeHandle>>`.
  - *Likelihood:* Medium. The open versus closed folder is the classic case, and filled icon sets are common.

- **`tree.rs:1294-1309`: `render_node` can replace only the label.** The row box, chevron or spacer, icon wrapper, aria wiring and click handler always come from the tree. There is no slot after the label for trailing actions or badges, outside the label's `flex: 1` span.
  - *Chrome:* no equivalent. The nearest, `<summary>`, takes arbitrary content.
  - *Can the app change it today?* Partly. The app can put everything into `render_node`'s output. It cannot reorder the chevron or icon, or add anything outside the label span.
  - *Proposed option:* `render_row: Option<…>`, a full row-content renderer handed the chevron node to place where it likes, or `render_trailing: Option<…>`, a slot after the label. Both default to `None` (today).
  - *Likelihood:* Medium. File trees often carry trailing counts or status dots, and hover actions such as "+" and "⋯".

- **`tree.rs:3349-3356` (and the click path, `tree.rs:3290-3292`): a collapse removes the hidden rows from a `multiple` tree's selection.** This happens on a collapse by the chevron, a click or the Left arrow.
  - *Chrome:* no equivalent. A native listbox has no collapsible groups (`<optgroup>` does not collapse), and closing a `<details>` does not touch any selection.
  - *Can the app change it today?* Only by turning `select_on_click` off, which hands over all of selection. Re-adding the rows after the collapse would also fire `onselectionchange` twice.
  - *Proposed option:* `unselect_on_collapse: bool` (default `true`, today's behaviour).
  - *Likelihood:* Medium-Low. Some apps, such as a bulk-operations view, want a selection that survives collapsing.

- **`tree.rs:1559-1588`: rows expanded by hovering during a drag stay expanded.** A row expanded this way is not collapsed again when the drag ends or moves on.
  - *Chrome:* no equivalent. HTML DnD has no spring-loaded expansion, and a `<details>` does not open on drag-over.
  - *Can the app change it today?* Partly. The app can record the `onexpand` calls between `ondragstart` and `ondragend` and collapse those rows itself. It cannot tell a hover expansion apart from other expansions.
  - *Proposed option:* `collapse_hover_expanded: bool` (default `false`, today's behaviour), which re-collapses rows that were opened by hovering and did not receive the drop. Another route is an `onexpand` payload saying why the row expanded.
  - *Likelihood:* Medium-Low. Finder-style spring-loaded folders close again; others keep them open.

- **`tree.rs:3192-3222`: in a `multiple` tree, a drag from a selected row carries the whole selection.** `drag_values` is called once per row of that selection, so it cannot tell which row was pressed and cannot narrow the drag to it.
  - *Chrome:* no direct equivalent. The options of a `<select multiple>` cannot be dragged. In HTML DnD, a drag from an element carries only what the page puts in `dataTransfer`, and the page knows which element was pressed (`event.target`).
  - *Can the app change it today?* Partly. `drag_values` can rewrite each value, but it cannot see the pressed row.
  - *Proposed option:* give `drag_values` the pressed row as well, `Rc<dyn Fn(&str /*row*/, &str /*pressed*/) -> Vec<String>>`, or add `drag_selection: bool` (default `true`, today's behaviour).
  - *Likelihood:* Low-Medium.

- **Differs from Chrome (default only):** here a click selects nothing by default; in Chrome's listbox a click selects. **`tree.rs:926`: `select_on_click` defaults to `false`.**
  - *Chrome:* clicking an option of a `<select size=N>` or `<select multiple>` selects it.
  - *Can the app change it today?* Yes, with `select_on_click: true`.
  - *Proposed option:* change the default to `true`.
  - *Likelihood:* Medium. Most trees that hold state want click-to-select, and today each one has to opt in.

- **Differs from Chrome (default only):** Chrome's listbox takes the keyboard with no opt-in; this tree does not. **`tree.rs:943`: `keyboard` defaults to `false`.** A default `Tree` with `tree` state has `role="tree"` but takes no arrow keys, and every row is a Tab stop.
  - *Chrome:* no native tree. Every native focusable composite control (`<select size>`, a radio group) is keyboard-operable by default with no opt-in.
  - *Can the app change it today?* Yes, with `keyboard: true`.
  - *Proposed option:* change the default to `true` whenever `tree` state is given.
  - *Likelihood:* Medium. Keyboard users get a `role="tree"` that does not behave as one unless the app knew to opt in.

#### Low

- **Differs from Chrome:** this tree pages by a fixed 10 rows, where Chrome's listbox pages by its visible height. **`tree.rs:2245`: PageUp and PageDown move a fixed 10 rows (`PAGE_ROWS`, used at `tree.rs:2323-2324`).** The step does not follow the viewport height.
  - *Chrome:* in a `<select size=N>` listbox, PageDown moves to the last visible option and then by a page, a page being the visible height in options.
  - *Can the app change it today?* Partly. `handle_key` can consume PageUp and PageDown and move the cursor with `TreeController::set_cursor`.
  - *Proposed option:* `page_rows: usize`, **default `0`, meaning "by the visible height"** (Chrome). `10` gives today's behaviour.
  - *Likelihood:* Low-Medium. In a tall tree, a 10-row page feels wrong.

- **Differs from Chrome:** the buffer here is 700 ms, where Chromium's is 1 s. **`tree.rs:2247`: type-ahead has a fixed 700 ms buffer (`TYPE_AHEAD_MS`, timer at `tree.rs:2826`) and cannot be switched off.**
  - *Chrome:* a native `<select>` / listbox has type-ahead find. Chromium's `TypeAhead` resets the buffer after 1 second without a key. That is from memory of the Chromium source; confirm before changing.
  - *Can the app change it today?* Only by consuming every printable key in `handle_key`, and that also takes away single-letter shortcuts the app wants to fall through.
  - *Proposed option:* `type_ahead_ms: u32`, **default `1000`** (Chrome), with `0` = off.
  - *Likelihood:* Low-Medium. Trees whose letter keys are shortcuts, as in vim-style apps.

- **`tree.rs:671-677`: the drop zones split a row 25% / 50% / 25% (before, inside, after).**
  - *Chrome:* no equivalent. HTML DnD computes no drop position; the page does.
  - *Can the app change it today?* No. `accepts_children` only switches between the three-zone and two-zone splits.
  - *Proposed option:* `drop_edge_fraction: f32` (default `0.25`, today's behaviour).
  - *Likelihood:* Low. Short rows sometimes want wider edges.

- **`styles/tree.rs:138-173`: the drop indicator and the dragged-row fade have no tree-specific vars.** The line is fixed at 2px. It uses `--rinch-primary-color` and `--rinch-color-red-6`, shared with everything else. The faded opacity is `0.4`.
  - *Chrome:* no equivalent. HTML DnD draws no drop indicator (only the drop-effect cursor), and it leaves the source element as it was.
  - *Can the app change it today?* Yes with an app stylesheet, since all of these are class selectors. Only partly through inherited custom properties, which matters for a plugin guest that cannot add a stylesheet. Sortable already exposes `--rinch-sortable-caret-color`, `--rinch-sortable-caret-size`, `--rinch-sortable-dragging-opacity` and `--rinch-sortable-refused-color`.
  - *Proposed option:* `--rinch-tree-drop-color`, `--rinch-tree-drop-size`, `--rinch-tree-drop-refused-color` and `--rinch-tree-dragging-opacity`, read with today's values as fallbacks.
  - *Likelihood:* Low-Medium. This is mostly for consistency with Sortable and for plugin guests.

- **`tree.rs:2626`, `2467` and `2936`: the cursor row is always scrolled into view.** It happens on every keyboard move, on `follow_focus` and on `TreeController::focus()`. `NodeHandle::scroll_into_view` is the minimal ("nearest") scroll (`crates/rinch-core/src/dom/mod.rs:1249-1264`).
  - *Chrome:* matches. Moving the active option in a listbox, and `focus()` with no options, scroll it into view minimally. (A bare `scrollIntoView()` defaults to `block: "start"`, but that is not what focus does.)
  - *Can the app change it today?* No.
  - *Proposed option:* none on its own. It could be part of a general `scroll_into_view` options type, if one appears; it would default to nearest.
  - *Likelihood:* Low.

- **Differs from Chrome:** every row here is a Tab stop, where Chrome's composite controls are one Tab stop. **`tree.rs:1803` (static tree, no `tree` state) and `tree.rs:1136-1140` (any tree with `keyboard: false`, the default): every row is `tabindex="0"`.** Every row is a Tab stop. Only a `keyboard` tree uses a roving tabindex.
  - *Chrome:* a `<select size>` listbox is one Tab stop, and so is a radio group (one stop, arrows inside).
  - *Can the app change it today?* Partly. `keyboard: true` gives one Tab stop for a tree with state. A static tree can change it only by patching the DOM.
  - *Proposed option:* one Tab stop by default: the roving tabindex whenever there is state, and on static rows no `tabindex`, or `-1` with the first row at `0`. This follows from the `keyboard` default change above.
  - *Likelihood:* Low-Medium. A tree of 200 rows takes 200 Tab presses to get past.

### SortableList / SortableItem / SortableHandle (sortable.rs, styles/sortable.rs)

Chrome has no sortable list, and HTML DnD has no keyboard drag, no announcements and no drop indicator. Most findings here are therefore "no equivalent" and keep today's default. The ghost, which does have an equivalent, is excluded (see the note at the top).

#### High

- **`sortable.rs:1018`, `1145`, `1161`, `1165`, `1186`, `1190` and `1244`: the keyboard announcements are fixed English text.** They read "Picked up {label}. Use the arrow keys…", "{label} and N more", "{label}: back where it was.", "…position N. Cannot drop here. {reason}", "Dropped in {list}, position N.", "Not moved." and "Cancelled. Not moved."
  - *Chrome:* no equivalent. HTML DnD announces nothing and has no keyboard drag.
  - *Can the app change it today?* No. Only the `label` values that get substituted in are the app's.
  - *Proposed option:* `announce: Option<Rc<dyn Fn(&SortAnnouncement) -> String>>` on `SortableList`. `SortAnnouncement` would be an enum: `Lifted { label, count }`, `Moved { label, count, list, position, verdict }`, `Back { label }`, `Dropped { list, position }`, `NotMoved` and `Cancelled`. Returning the default text keeps today's wording, and an empty string stays silent.
  - *Likelihood:* High. Any non-English app, and any app with its own wording (the design doc names Pimble's kanban as the first user).

#### Medium

- **`sortable.rs:1496-1501`: the ARIA strings on each item are fixed English, and the description can be wrong.** Every item gets `aria-roledescription="sortable item"` and `aria-description="Press Space to lift, the arrow keys to move, Space to drop, Escape to cancel."`. That description is wrong when `lift_keys` is not Space, for example `vec!["Enter"]`.
  - *Chrome:* no equivalent. Its native controls' accessible role names come from the platform, localized.
  - *Can the app change it today?* Partly. The app can override per item with hyphenated root attributes (`aria-description: "…"`, `aria-roledescription: "…"`), which the caller writes last (#433). Nothing does it list-wide, and the default does not follow `lift_keys`.
  - *Proposed option:* `item_description: String` and `item_role_description: String` on `SortableList`, read by its items. Short of that, build the default text from `lift_keys`.
  - *Likelihood:* Medium-High. i18n, plus the wrong instruction whenever `lift_keys` changes.

- **`sortable.rs:614` and `1400`: the built-in refusal reason box cannot be switched off.** It is shown with an inline `display: block` and hidden with an inline `display: none`.
  - *Chrome:* no equivalent. A refused HTML drop shows only the no-drop cursor, with no text.
  - *Can the app change it today?* Partly. The app can restyle it through `--rinch-sortable-reason-*` or the `.rinch-sortable__reason--shown` class. To hide it and draw its own reason from `ondropchange`, a stylesheet has to beat an inline `display` (`!important`), and a plugin guest has no way to do that.
  - *Proposed option:* `show_reason: bool` (default `true`, today's behaviour) on `SortableList`. Alternatively, a `--rinch-sortable-reason-display` var, with the inline style setting only the hidden state.
  - *Likelihood:* Medium. An app that already renders its own feedback from `ondropchange` (a toast or a status line) gets two messages.

- **`sortable.rs:1490`: every item is `tabindex="0"`.** Every card is a Tab stop, with no roving tabindex.
  - *Chrome:* no equivalent for a sortable list. A list of independently focusable elements (links, buttons) is one Tab stop per element, which is what this does. Only composite controls (a listbox, a radio group) take one stop. Not marked as differing, because a sortable list of cards is closer to the former.
  - *Can the app change it today?* Partly. The app can pass a quoted root attribute, `"tabindex": "-1"` (#1653). It then has to manage the arrow keys between items itself, and an item that cannot be focused cannot be lifted from the keyboard.
  - *Proposed option:* `roving_focus: bool` on `SortableList` (default `false`, today's behaviour): one Tab stop per list, and arrows move focus between items when nothing is lifted.
  - *Likelihood:* Medium. A kanban with hundreds of cards needs hundreds of Tab presses to get past it.

- **`sortable.rs:220-229` (`is_lift_key`) and `1275-1300`: keyboard lifting cannot be turned off.** An empty `lift_keys` means Space and Enter (`DEFAULT_LIFT_KEYS`, `sortable.rs:75`), so no setting frees both keys while keeping the pointer drag. `disabled` turns off both pointer and keyboard.
  - *Chrome:* no equivalent. HTML DnD has no keyboard drag at all.
  - *Can the app change it today?* Partly. `lift_keys: vec![" "]` frees Enter. Both keys cannot be freed.
  - *Proposed option:* `keyboard: bool` (default `true`, today's behaviour) on `SortableList`, or have `lift_keys: Some(vec![])` mean none, using `Option<Vec<String>>`.
  - *Likelihood:* Medium. A card whose Space selects and whose Enter opens, with moving done through a menu or a shortcut.

- **`sortable.rs:1285-1291` and `917-925`: the moving keys are fixed.** The arrows move the caret, Escape cancels, and Tab cancels and lets focus move on. Unlike `Tree::handle_key`, there is no intercept.
  - *Chrome:* no equivalent for the arrow moves. Escape does cancel a pointer drag in Chrome's HTML DnD, which matches the Escape-cancels default.
  - *Can the app change it today?* No. Only the lift and drop keys can be changed.
  - *Proposed option:* `handle_key: Option<Rc<dyn Fn(&KeyEventData, &SortKeyContext) -> bool>>`, run before the built-in keys. `SortKeyContext` would expose `lifted()`, the current `SortMove` and `cancel()`. With `None` the keys stay as they are today.
  - *Likelihood:* Low-Medium. Vim keys, Home/End to jump to the ends of a list, and Ctrl+arrow to jump lists.

- **No programmatic control of a drag.** `sortable.rs:1608` exposes only `sortable_drag_group()`. The app cannot cancel an active pointer drag or keyboard lift, or start a lift ("Move" in a card's menu).
  - *Chrome:* no equivalent. A page cannot cancel or start a native drag from script, apart from `preventDefault()` on `dragstart` before the drag begins.
  - *Can the app change it today?* No.
  - *Proposed option:* `pub fn sortable_cancel()` and `pub fn sortable_lift(group: &str, key: &str)`, the latter focusing and lifting the drawn item.
  - *Likelihood:* Medium-Low. A data reset or a modal opening mid-drag; a menu-driven move for touch or accessibility.

#### Low

- **`sortable.rs:1204` (with `1211-1233` and `1582`/`1591`): after a keyboard drop, focus always goes to the moved item.**
  - *Chrome:* no equivalent. There is no keyboard drop. After a pointer drop, focus stays where it was.
  - *Can the app change it today?* Partly. The app can move focus again after the item renders. Doing so races the `set_timeout(0)` re-focus.
  - *Proposed option:* `focus_after_drop: bool` (default `true`, today's behaviour).
  - *Likelihood:* Low-Medium.

- **`sortable.rs:1157`: each keyboard step scrolls the caret's item into view, unconditionally, with the minimal ("nearest") scroll.**
  - *Chrome:* no direct equivalent. It is the same minimal scroll that `focus()` does, so it is in line with Chrome.
  - *Can the app change it today?* No.
  - *Proposed option:* none on its own; fold it into a general scroll option if one appears.
  - *Likelihood:* Low.

- **`sortable.rs:1432`: the live region is `aria-live="assertive"`.**
  - *Chrome:* no equivalent.
  - *Can the app change it today?* No.
  - *Proposed option:* `live_politeness: String` (default `"assertive"`, today's behaviour).
  - *Likelihood:* Low.

- **`sortable.rs:846`, `867` and `984`: the place geometry is fixed.** Over an item, the place is before or after it by its midpoint. Keyboard moves pick the neighbouring list by the score `along + 2·across`.
  - *Chrome:* no equivalent. HTML DnD leaves the drop position to the page.
  - *Can the app change it today?* No.
  - *Proposed option:* not worth a prop yet. Note it for the wrapping-list work that `docs/design/sortable.md` lists under "Not built".
  - *Likelihood:* Low.

### late_children (late_children.rs)

No findings. It is internal infrastructure: it re-runs a container's own default-patch on late-inserted or removed children, for `List`, `RadioGroup` and `Stepper`. It makes no presentation or interaction decision of its own, and the choices it hands over belong to those containers' props.

### Summary

**9 findings are marked as differing from Chrome, all in Tree.** 7 are behaviour gaps and 2 are default-only. The excluded Sortable ghost would be a 10th; it is noted for #2154 and not counted.

The 5 most significant:
1. **No drag ghost** (`tree.rs:1427`). Chrome's HTML DnD shows a drag image by default, so `drag_ghost` should default to `true`.
2. **The toggle modifier ignores the platform** (`tree.rs:1315-1331`). On macOS, Ctrl-click toggles here where Chrome opens the context menu, and Meta toggles on Windows and Linux.
3. **Every row is a Tab stop by default** (`tree.rs:1136-1140`, `1803`) and **`keyboard` is off by default** (`tree.rs:943`). Chrome's listbox is one Tab stop and keyboard-operable with no opt-in.
4. **A plain arrow in a `multiple` tree selects nothing** (`tree.rs:2612-2616`). In Chrome's `<select multiple>` it replaces the selection.
5. **The indent is an inline style** (`tree.rs:1141-1144`). Chrome indents nested lists from its UA stylesheet, which any author rule overrides.

The other differences: PageUp and PageDown move 10 rows rather than the visible height; the type-ahead buffer is 700 ms against Chromium's 1 s, which needs confirming; and `select_on_click` defaults to `false` where Chrome's listbox selects on click.


## Part B: Overlays

Scope: `crates/rinch-components/src/` popover, floating, floating_panel, hover_card, tooltip, modal, drawer, overlay_dismiss, overlay_focus, overlay_scroll_lock, loading_overlay, notification, context_menu, dropdown_menu, plus their `styles/*.rs`. This was a read-only audit: I checked props against `docs/src/guide/component-props.md`.

Note on CSS: rules in the generated stylesheets are class-based, so an app can override them with a more specific selector. I list a stylesheet value only when it is `!important`, is duplicated in a way that makes an override fragile, or should obviously be a `--rinch-*` var. I always list inline `set_attribute("style")` / `set_style` writes of real properties (not custom properties), because an app stylesheet cannot override them.

**Chrome rule (2026-11-04, "sensible defaults means like Chrome"):** each finding now has a `*Chrome:*` line, and each proposed default is Chrome's behaviour where Chrome has an equivalent. Where it has none, the default stays today's. The equivalents used:

- Modal and Drawer map to `<dialog>` + `showModal()`.
- Popover maps to `popover="auto"`, with CSS anchor positioning for placement.
- DropdownMenu maps to a native menu or the `<select>` popup.
- ContextMenu maps to Chrome's native context menu.
- Tooltip maps to `title`.
- HoverCard maps to interest invokers (`interestfor`). These are recent, so I mark the details "unsure" where they are.

Findings marked **Differs from Chrome** are behaviour gaps, separate from whether the behaviour can be configured.

### Shared helpers (attributed below to the components that use them)

| Helper | Used by (in my set) | Also used by |
|---|---|---|
| `overlay_dismiss::arm_close_on_escape` (overlay_dismiss.rs:36) | Modal, Drawer, Popover | none |
| `overlay_dismiss::arm_close_on_escape_while_open` / `arm_keys_while_open` | none in my set | ColorInput, Select |
| `overlay_focus::arm_trap_focus` (:69) + `arm_overlay_focus` (:140) | Modal, Drawer, Popover | none |
| `overlay_scroll_lock::arm_lock_scroll` (:37) | Modal, Drawer | none |
| `floating::arm` (flip/shift/size) | Popover, DropdownMenu | none |
| `BACKDROP_ATTRIBUTE` any-button dismiss (#1093) | Modal, Drawer, Popover, DropdownMenu, ContextMenu | ColorInput, Select |
| `icons::close_icon_lines_dom` close button | Modal, Drawer, Notification | none |

---

### Cross-cutting (shared helpers)

- **overlay_focus.rs:148 (`arm_overlay_focus`, `if !trap_focus { return }`)**: the focus move-in on open and the focus restore-to-opener on close are tied to `trap_focus`. The helper's own doc admits that you cannot have containment without the move, or the move/restore without containment. The initial-focus target is fixed per component: `FirstFocusable` for Modal/Drawer (modal.rs:306, drawer.rs:309) and `AutofocusOnly` for Popover (popover.rs:404).
  - *Chrome:* **`showModal()`** focuses the first `autofocus` descendant. Failing that it focuses the first focusable descendant, and failing that the dialog itself. Everything outside the dialog becomes inert. On close, focus goes back to the element that had it before `showModal()`.
  - *Chrome:* **`popover="auto"`** moves focus only to an `autofocus` descendant. It traps nothing. On hide, it restores focus to the previously focused element, but only if focus was inside the popover.
  - *Chrome:* So Chrome **does** decouple restore from containment: a popover restores focus without trapping it. rinch's Popover, with its default `trap_focus: false`, neither moves focus to an `autofocus` child nor restores it. That is a behaviour gap; see Popover below.
  - *Chrome:* rinch's Modal/Drawer `FirstFocusable` skips the "prefer `autofocus`" step. Chrome: unsure whether rinch's `focus_into(FirstFocusable)` honours `autofocus` first. traits.rs:800 says FirstFocusable "falls back to the first stop", which suggests it does try autofocus first, so this probably matches.
  - Can the app change it? **Partially.** `trap_focus: false` turns off all three behaviours together. `autofocus` on a child picks the target only under `AutofocusOnly`.
  - Proposal: `return_focus: bool` (default true for Modal/Drawer/Popover, as Chrome does for both dialog and popover), `initial_focus: String` ("auto" = Chrome's dialog/popover rule for the component, "first", "none"), and optionally `initial_focus_fn: Option<Rc<dyn Fn() -> Option<NodeHandle>>>`.
  - Likelihood: **High**. "Focus the Cancel/primary button, not the first input", and "don't steal focus back to the opener after a destructive action" are common requests.
- **Differs from Chrome.** **Backdrop any-button dismiss (modal.rs:343, drawer.rs:345, popover.rs:363, dropdown_menu.rs:504, context_menu.rs:222)**: any mouse button outside dismisses, including a right or middle click, and the backdrop swallows the press (it does not pass through to what is underneath). For popovers this differs from Chrome: `popover="auto"` light dismiss has no backdrop, so the outside click **does** reach the element under the pointer. For modal dialogs, Chrome does not dismiss on a backdrop click at all by default (see Modal).
  - *Chrome:* `popover="auto"` closes on a pointer press outside it, and that press and its click are delivered normally to the target underneath. Nothing is swallowed, and hover keeps working because there is no covering layer. Chrome: unsure whether a right or middle press triggers light dismiss. The spec's light-dismiss hooks `pointerdown`/`pointerup` without filtering buttons, so I believe any button does.
  - *Chrome:* `<dialog>` + `showModal()`: a backdrop click does nothing by default. Pass-through is moot, because the page behind is inert. `closedby="any"` (recent Chrome) opts into light dismiss.
  - *Chrome:* For native menus and context menus, see ContextMenu.
  - Can the app change it? **No.** It only has the all-or-nothing `close_on_click_outside`; ContextMenu has no switch at all.
  - Proposal: `outside_press: String` ("any" | "primary" | "none", default "any") and `outside_press_passthrough: bool`. Its default is **true for Popover** (Chrome's light dismiss) and false for Modal/Drawer (inert page) and for the menus (see their findings).
  - Likelihood: **Medium**. Native menus re-open on a right-click elsewhere, and click-through to the next control is common for popovers.
- **Close button (modal.rs:354-360, drawer.rs:370-376, notification.rs:352-358)**: a fixed X glyph (`close_icon_lines_dom`) with **no `aria-label`**, so there is no accessible name and nothing to translate. There is no slot for a custom close control.
  - *Chrome:* no equivalent. `<dialog>` has no built-in close button.
  - Can the app change it? **No** (the app can hide it with `with_close_button: false` and build its own).
  - Proposal: `close_button_label: String` (default "Close"), `close_icon: Option<TablerIcon>`.
  - Likelihood: **Medium**. i18n and a11y.

### Modal (modal.rs)

- **styles/modal.rs:26-28**: the closed state is `.rinch-modal__root--hidden { display: none !important; }`. There is no open/close animation, and the `!important` stops an app from switching to a visibility/opacity transition (Drawer and Popover already use the `visibility` pattern).
  - *Chrome:* a closed `<dialog>` is `display: none` from the UA sheet with no animation, so today's default matches. Chrome lets an author animate it with `transition-behavior: allow-discrete` + `@starting-style` on `display`/`overlay`. rinch's `!important` forbids the equivalent.
  - Can the app change it? **No** (the class is toggled and the rule is `!important`).
  - Proposal: move to Drawer's `visibility` pattern and add `transition_duration: Option<u32>` → `--rinch-modal-transition` (default 0, instant, as Chrome does), plus a `rinch-modal--opened` class to hang app transitions on.
  - Likelihood: **High**. Fade/scale-in dialogs are the norm.
- **modal.rs:300-307**: focus moves to the first focusable element on open and returns to the opener on close. Same as the shared finding above.
  - *Chrome:* `showModal()` does the same: autofocus, else the first focusable element, else the dialog itself, then restore to the previously focused element on close. The default matches.
  - Can the app change it? **Partially** (`trap_focus: false` disables everything).
  - Proposal: see shared (`initial_focus`, `return_focus`, both defaulting to Chrome's dialog rule).
  - Likelihood: **High**.
- **Differs from Chrome (default only).** **modal.rs:171 `close_on_click_outside: true`**: a click on the backdrop closes the modal.
  - *Chrome:* a modal `<dialog>` ignores backdrop clicks by default (`closedby` defaults to "closerequest" for `showModal()`, meaning Escape only). `closedby="any"` opts in.
  - Can the app change it? **Yes** (`close_on_click_outside: false`). Only the default differs.
  - Proposal: default `false` (Chrome). Drawer is the same (drawer.rs:197); Drawer has no Chrome equivalent, but it is a modal dialog, so the same rule applies.
  - Likelihood: **Medium**. Accidental-dismiss of a half-filled form is the classic complaint.
- **Differs from Chrome (default only).** **modal.rs:176 `lock_scroll: true`** (and drawer.rs:202): the page behind cannot scroll while the modal is open.
  - *Chrome:* `showModal()` does not lock scrolling. The document behind is inert to clicks and focus, but the wheel over the `::backdrop` still scrolls the page, which is why authors add `body:has(dialog[open]) { overflow: hidden }` themselves.
  - Can the app change it? **Yes** (`lock_scroll: false`).
  - Proposal: default `false` would match Chrome. Note that most app designs want the lock, so this is a judgement call; at minimum, document that the default differs.
  - Likelihood: **Low**.
- **Differs from Chrome (default only).** **styles/modal.rs:43**: the backdrop is `rgba(0,0,0,var(--rinch-modal-overlay-opacity,0.75))`, a strong 75% black (the same at styles/drawer.rs:104).
  - *Chrome:* the UA sheet gives `dialog::backdrop` `background: rgba(0, 0, 0, 0.1)`, a faint 10% tint.
  - Can the app change it? **Yes** (`overlay_opacity` prop / the var; colour only via CSS).
  - Proposal: default opacity 0.1 to match Chrome, plus `--rinch-modal-overlay-color` (see Drawer).
  - Likelihood: **Low**.
- **Differs from Chrome (default only).** **modal.rs:170 `centered: false`; styles/modal.rs:16, :55**: the modal is pinned near the top (`align-items: flex-start` + `margin-top: xl`).
  - *Chrome:* a modal `<dialog>` is centred on both axes by the UA sheet (`position: fixed; inset: 0; margin: auto`).
  - Can the app change it? **Yes** (`centered: true`).
  - Proposal: default `true`.
  - Likelihood: **Low**.
- **Differs from Chrome.** **modal.rs:252-386**: no `role="dialog"`, no `aria-modal`, no `aria-labelledby` pointing to the title.
  - *Chrome:* `<dialog>` has an implicit `role="dialog"`, and `showModal()` exposes it as modal to accessibility (the `aria-modal` equivalent). Chrome does not infer a label automatically; the author adds `aria-labelledby`.
  - Can the app change it? **Partially.** The universal attribute props land on the root, not the panel.
  - Proposal: emit `role="dialog"` + `aria-modal="true"` by default, `aria-labelledby` when a title exists, and `aria_label: String` for title-less modals.
  - Likelihood: **Medium**. a11y.
- **modal.rs:364-372**: the title is a `String` in a fixed `<h2>`. There is no header slot (icon, subtitle, actions) and no footer slot, and the heading level is fixed.
  - *Chrome:* no equivalent (`<dialog>` has no title chrome).
  - Can the app change it? **Partially.** Leave `title` empty and put a custom header in the children, but then it renders inside `__body` padding, not the header bar.
  - Proposal: `header: Option<NodeHandle>`-style slot (or `title_node`), `footer` slot, and `title_order: u8` (default 2).
  - Likelihood: **Medium**.
- **modal.rs:209-213 / 332-346**: there are no `onopen` / `onopened` / `onclosed` (after transition) callbacks.
  - *Chrome:* `<dialog>` fires `close` and `cancel` (cancelable Escape), and in recent versions `beforetoggle`/`toggle`. There is no after-transition event; you use `transitionend`.
  - Can the app change it? **Partially** (the app can watch its own signal, but there is no after-transition hook and no cancelable "about to close by Escape").
  - Proposal: `on_open: Option<Callback>`, `on_exit_transition_end: Option<Callback>`, and `on_cancel: Option<Rc<dyn Fn() -> bool>>` (return false to keep open, like Chrome's `cancel`).
  - Likelihood: **Low**.

### Drawer (drawer.rs)

- **drawer.rs:403-409**: on every open, the effect unconditionally calls `body_clone.set_scroll_top(0.0)`, which resets the drawer body's scroll.
  - *Chrome:* no direct equivalent. Chrome: unsure whether a `<dialog>` reopened after `display: none` keeps its inner scroll offset, so the default stays today's.
  - Can the app change it? **No.**
  - Proposal: `keep_scroll: bool` (default false).
  - Likelihood: **High**. A nav or filter drawer that re-opens at the user's place is a common expectation.
- **drawer.rs:303-310**: focus moves to the first focusable element on open and returns to the opener on close. Same as the shared finding.
  - *Chrome:* matches the `showModal()` focus steps and restore.
  - Can the app change it? **Partially** (`trap_focus: false`).
  - Proposal: see shared.
  - Likelihood: **High**.
- **styles/drawer.rs:58-60 and :115**: the slide duration is 300ms. It is written twice: `transition: transform 300ms ease` on the panel, and a `visibility 0s linear 300ms` delay on `--hidden` (which carries an `!important` visibility). Changing only one makes the close animation get clipped or the hide lag behind.
  - *Chrome:* no equivalent (a native dialog does not slide). Keep 300ms.
  - Can the app change it? **Partially** (override both rules in its own CSS, and keep them in sync).
  - Proposal: `--rinch-drawer-duration` var (default 300ms) used in both places, plus a `transition_duration: Option<u32>` prop; also `--rinch-drawer-easing`.
  - Likelihood: **Medium**.
- **drawer.rs:143-146**: there is no `overlay_blur` (Modal has one) and no overlay colour. The sheet reads `rgba(0,0,0,var(--rinch-drawer-overlay-opacity,0.75))` (styles/drawer.rs:104), so it is always black.
  - *Chrome:* the `::backdrop` default is `rgba(0,0,0,0.1)`, freely restylable. See the Modal backdrop default-only finding; the 0.75 default differs in the same way.
  - Can the app change it? **Partially** (an app can override `.rinch-drawer__overlay` in its CSS).
  - Proposal: `overlay_blur: String` → `--rinch-drawer-overlay-blur`, and `--rinch-drawer-overlay-color` (and the same `--rinch-modal-overlay-color` for Modal, styles/modal.rs:43).
  - Likelihood: **Low**.
- **Differs from Chrome.** **drawer.rs:354-380**: the title is a `String` in a fixed `<h2>`, with no header or footer slot, and no `role="dialog"`/`aria-modal`. Same as Modal.
  - *Chrome:* `<dialog>` gives the role implicitly. There is no header chrome.
  - Can the app change it? **Partially.**
  - Proposal: same as Modal.
  - Likelihood: **Medium**.

### Popover (popover.rs)

- **Differs from Chrome.** **popover.rs:327-366**: outside-click dismissal works through a full-viewport invisible backdrop. While the popover is open, the first click anywhere only closes it and never reaches the clicked control, and hover effects under it stop.
  - *Chrome:* `popover="auto"` light dismiss closes on an outside press, and the press/click still reaches the element underneath. Hover is unaffected because there is no covering layer.
  - Can the app change it? **Partially** (`close_on_click_outside: false`, then the app implements outside-click itself, which rinch gives it no primitive for).
  - Proposal: see shared `outside_press_passthrough`, defaulting to **true** for Popover.
  - Likelihood: **High** (raised from Medium: this is the native behaviour, so users will expect it).
- **Differs from Chrome.** **popover.rs:172 `trap_focus: false` + overlay_focus.rs:148**: with the default `trap_focus: false`, Popover neither focuses an `autofocus` child on open nor restores focus on close. `AutofocusOnly` at popover.rs:404 only runs when `trap_focus` is true.
  - *Chrome:* a `popover` focuses an `autofocus` descendant on show and, on hide, returns focus to the previously focused element if focus was inside the popover. Neither needs a trap.
  - Can the app change it? **Partially** (`trap_focus: true` gets the move and restore, but also adds containment, which Chrome's popover does not have).
  - Proposal: decouple per the shared finding. Popover defaults become `initial_focus: "auto"` (autofocus only), `return_focus: true` (only when focus is inside), `trap_focus: false`.
  - Likelihood: **High**.
- **popover.rs:286-325 → floating.rs:226 (`floating::arm`)**: flip/shift/size collision handling is always on whenever `position` parses. It writes **inline** `top/bottom/left/right/width/box-sizing/max-height/overflow-y` on the dropdown (floating.rs:378-420), so app CSS cannot override placement. The edge margin is a fixed 8px (floating.rs:52), the viewport is the only boundary, a window resize does not re-place, and the measured width is pinned (content changes while open do not resize).
  - *Chrome:* CSS anchor positioning does **not** flip by default. Flipping is opt-in with `position-try-fallbacks: flip-block, flip-inline` (or named `@position-try` rules), and there is no automatic shift or size clamp. Chrome's own popups (`<select>`, autofill, date picker) do flip to stay on screen.
  - *Chrome:* anchor-positioned elements are **re-placed on scroll and resize** and track their content size. **Differs from Chrome** in that part: rinch does not re-place on resize and pins the width.
  - Can the app change it? **No.**
  - Proposal: `flip: bool`, `shift: bool`, `collision_padding: Option<f32>` (8.0), `auto_size: bool`, `on_position_change: Option<ValueCallback<String>>`, and re-place on resize/content change always. Defaults: Chrome's anchor-positioning default is no flip, but rinch's `position` prop is closer to an author writing a `position-try-fallbacks` list, and Chrome's native popups flip. So keep `flip: true`, `shift: true`, and state that choice in the doc.
  - Likelihood: **High**. Pinned/fixed-side popovers, popovers inside scroll panes, and dynamic content are everyday cases.
- **Differs from Chrome (partially configurable).** **styles/popover.rs:34, :74**: the fade/slide is a fixed 150ms `opacity` + `transform`.
  - *Chrome:* a popover appears and disappears instantly (`display: none` toggle). Animation is opt-in via `allow-discrete` + `@starting-style`.
  - Can the app change it? **Partially** (override in CSS; three places).
  - Proposal: `--rinch-popover-duration` var + `transition_duration` prop, default 0 (Chrome). Possibly keep 150ms if Joe prefers the polish; flag the choice.
  - Likelihood: **Low**.
- **popover.rs:96-132**: there is no trigger behaviour. The app must wire open/close itself (no `trigger: "click" | "hover" | "focus"`), and there is no `onopen`.
  - *Chrome:* `popovertarget` on a button gives click-to-toggle with no script, and `interestfor` (recent) gives hover/focus. With neither, a popover is manual.
  - Can the app change it? **Partially** (it wires it by hand).
  - Proposal: optional `trigger: String`, default "manual" (Chrome's default when no invoker is declared), plus a `Popover.Target`-style click toggle that matches `popovertarget`.
  - Likelihood: **Low**.

### DropdownMenu (dropdown_menu.rs)

- **Differs from Chrome.** **dropdown_menu.rs:325-512**: **Escape does not close it**. Unlike Popover/Modal/Drawer, it never calls `arm_close_on_escape`, and there is no `close_on_escape` prop. It also has **no arrow-key/Home/End/typeahead navigation** between items, and no focus move into the menu.
  - *Chrome:* the open `<select>` popup and native menus close on Escape and return focus to the opener. Up/Down move the highlight, Home/End jump, and typing jumps to a matching item (typeahead); in Chrome's `<select>` the arrows do not wrap. Focus or highlight moves into the list on open. A `popover="auto"` menu also closes on Escape.
  - Can the app change it? **No** (the app could add its own key handler, but has no access to the item list).
  - Proposal: `close_on_escape: bool` (true), `keyboard_nav: bool` (true), `loop_focus: bool` (**false**, as Chrome's select does not wrap), `typeahead: bool` (true), `on_key: Option<Rc<dyn Fn(&KeyEventData) -> bool>>` as an escape hatch.
  - Likelihood: **High**.
- **dropdown_menu.rs:582-584**: `DropdownMenuItem::left_section`/`right_section` are `Option<TablerIcon>` only. You cannot put a shortcut hint ("Ctrl+C"), a checkmark, a badge, an avatar or a submenu chevron of your choice there.
  - *Chrome:* no equivalent in HTML. Native OS menus do have a right-aligned accelerator column and check marks.
  - Can the app change it? **No** (children go into the label span only).
  - Proposal: `left_node` / `right_node` slots (or make `left_section`/`right_section` accept a `NodeHandle`), plus `shortcut: String`.
  - Likelihood: **High**.
- **dropdown_menu.rs:456-483 + :640-651**: close-on-item-click is menu-wide only. An item cannot opt out (for example a checkbox/toggle item that should keep the menu open), and an item without `onclick` never closes.
  - *Chrome:* native menus close on activating any item, checkable items included, and `<select>` closes on choosing an option. So the default `close_on_item_click: true` matches. Chrome has no per-item opt-out; a disabled item does not close.
  - Can the app change it? **Partially** (`close_on_item_click: false` applies to the whole menu).
  - Proposal: `DropdownMenuItem { close_menu_on_click: Option<bool> }` (None = inherit). Consider closing on activation of items without `onclick` too (Chrome closes on any enabled item).
  - Likelihood: **Medium**.
- **Differs from Chrome.** **dropdown_menu.rs:504 (backdrop)**: the outside press is swallowed by the backdrop.
  - *Chrome:* if the menu is built on `popover="auto"`, the outside click passes through. For a `<select>` popup, Chrome: unsure whether the dismissing click reaches the page (I believe the popup consumes it on desktop, as OS menus do).
  - Can the app change it? **No.**
  - Proposal: shared `outside_press_passthrough`, default false here until the native `<select>` behaviour is verified.
  - Likelihood: **Medium**.
- **dropdown_menu.rs:517-530 (`floating_side`, `_ => return None`)**: Left/Right positions get **no** flip/shift at all. Top/Bottom always get it (inline styles, as for Popover). `Bottom`/`Top` are silently start-aligned (`P::Bottom | P::BottomStart => Align::Start`).
  - *Chrome:* see the Popover placement finding (anchor positioning flips only with `position-try-fallbacks`, which works in both axes). Native submenus open right and flip left at the screen edge.
  - Can the app change it? **No.**
  - Proposal: same `flip`/`shift`/`collision_padding` props as Popover, and extend to side positions.
  - Likelihood: **Medium**. Submenus open right and need a left flip.
- **styles/dropdown_menu.rs:33**: `box-shadow: 0 4px 12px rgba(0,0,0,0.15)` is hard-coded even though a `shadow` prop exists. The prop adds a class, but the default is not a var.
  - *Chrome:* no equivalent.
  - Can the app change it? **Partially** (the app can use the `shadow` prop or CSS).
  - Proposal: `var(--rinch-dropdown-menu-shadow, …)`.
  - Likelihood: **Low**.

### ContextMenu (context_menu.rs)

- **Differs from Chrome.** **context_menu.rs:182-350**: **Escape does not close it** (no `arm_close_on_escape`), there is no keyboard navigation, and focus is not moved into the menu.
  - *Chrome:* the native context menu closes on Escape, Up/Down/Home/End navigate, mnemonic or first-letter keys select (platform-dependent), and Enter activates.
  - Can the app change it? **No.**
  - Proposal: `close_on_escape: bool` (true), with keyboard nav shared with DropdownMenu.
  - Likelihood: **High**.
- **Differs from Chrome.** **context_menu.rs:33-37**: the opened state is a `#[doc(hidden)]` internal `Signal`. There is no controlled mode, no `opened_fn`, no `on_open`/`on_close` callbacks, no way to open programmatically at a point (Shift+F10 / Menu key / long-press / toolbar "…" button), and no `disabled` to suppress opening conditionally.
  - *Chrome:* the Menu key and Shift+F10 fire `contextmenu` on the focused element, and so does a long-press on touch. The page can cancel it with `preventDefault()` (the analogue of `disabled` / returning false). So keyboard and touch opening is native behaviour that rinch lacks.
  - Can the app change it? **No** (the docs say "state is managed internally").
  - Proposal: open on Menu key / Shift+F10 when the target or a descendant is focused (default on, as Chrome does), plus `opened_fn: Option<ReactiveBool>` + `on_close: Option<Callback>` (controlled), `on_open: Option<Rc<dyn Fn(f32, f32) -> bool>>` (return false to cancel, like `preventDefault`), `disabled: bool`, and a public `open_at(x, y)` handle.
  - Likelihood: **High**.
- **Differs from Chrome.** **context_menu.rs:212-222**: a right-click (any button) on the overlay only closes the menu. It does not re-open the menu at the new point as native menus do, and a press outside never reaches the target underneath.
  - *Chrome:* on Windows and Linux, right-clicking elsewhere while a context menu is open closes it and opens a new one at the new point, firing `contextmenu` on the new target. Chrome: unsure whether a left click outside is also delivered to the page (on Windows I believe the dismissing click is consumed).
  - Can the app change it? **No.**
  - Proposal: `reopen_on_context_click: bool`, default **true** (Chrome). Left-click pass-through default false until verified.
  - Likelihood: **Medium**.
- **context_menu.rs:285-288**: the portal's `z-index: 200` is written as an **inline** style on every open, so app CSS cannot change it. There is no `z_index` prop (DropdownMenu, Popover and Modal all have one).
  - *Chrome:* no direct equivalent. The native menu is a separate OS window, always above all page content, and the top layer (`popover`/`<dialog>`) likewise sits above every z-index. rinch's 200 ties with Modal's, so a context menu can render under app overlays, which a native one never does.
  - Can the app change it? **No.**
  - Proposal: `z_index: Option<i32>` → `--rinch-context-menu-z-index` read from the stylesheet, defaulting above Modal/Notification (top-layer-like).
  - Likelihood: **Medium**. It conflicts with app overlays, modals (also 200) and notifications.
- **context_menu.rs:53-60**: placement constants are fixed: `EDGE_MARGIN` 4px, assumed `MENU_MIN_WIDTH` 180px (used for the left/right decision even if the app's menu is wider or narrower), and `PREFERRED_ROOM_BELOW` 320px. The frame/spacer/dropdown styles are inline (context_menu.rs:102-115, 297-302).
  - *Chrome:* the native menu opens with its top-left at the pointer and flips left/up by its **actual** measured size when it would leave the screen. rinch flips by an assumed 180px width, so a wider menu can still overflow. Partially **Differs from Chrome**.
  - Can the app change it? **No.**
  - Proposal: flip by measured size (default, Chrome), plus `collision_padding: Option<f32>`, `min_width: Option<f32>` (also feeding `--rinch-context-menu-min-width`), `prefer_room_below: Option<f32>`.
  - Likelihood: **Low**.

### Tooltip (tooltip.rs)

- **Differs from Chrome.** **tooltip.rs:150-167**: the tooltip shows instantly on pointer enter and hides instantly on leave. There is **no open/close delay** (HoverCard has props for this; Tooltip does not), it does **not open on keyboard focus**, and there is no controlled mode: `opened` only seeds an internal signal (:151) and there is no `opened_fn`.
  - *Chrome:* a `title` tooltip appears only after the pointer rests for a delay. It is roughly 0.5–1s and platform-dependent; Chrome: unsure of the exact value per OS. It disappears on pointer leave or move-away, and also after a timeout of several seconds. It does **not** appear on keyboard focus, and not on touch. So the instant show differs, while "no focus trigger" matches.
  - Can the app change it? **No.**
  - Proposal: `open_delay: Option<u32>` (default ~500ms, Chrome-like), `close_delay: Option<u32>` (default 0), `open_on_focus: bool` (default false, as Chrome does; note that WAI-ARIA recommends true), `opened_fn: Option<ReactiveBool>`.
  - Likelihood: **High**. Delayed tooltips are near-universal in toolbars.
- **Differs from Chrome.** **styles/tooltip.rs:56-82**: positioning is pure CSS. There is no flip/shift, so a tooltip near the window edge is clipped. It does not use `floating::arm`.
  - *Chrome:* a `title` tooltip is placed near the cursor (below it) and is always kept fully on screen. It is a native window, so it is never clipped by the window edge.
  - Can the app change it? **Partially** (pick a different `position` by hand).
  - Proposal: `auto_flip: bool` using `floating::arm`, default **true** (Chrome never clips).
  - Likelihood: **Medium**.
- **tooltip.rs:67, :178**: the content is `label: String` only. There is no rich content (a shortcut kbd, two lines with formatting, an icon).
  - *Chrome:* `title` is plain text only (newlines allowed), so today's default matches.
  - Can the app change it? **No** (use HoverCard instead, which has different behaviour).
  - Proposal: a `label_node` / `TooltipContent` slot.
  - Likelihood: **Medium**.
- **styles/tooltip.rs:29-35**: text colour is `color: white` (not a var), `z-index: 1000` and `white-space: nowrap` are fixed, and there is no fade.
  - *Chrome:* the tooltip uses the OS tooltip colours and font, and wraps long text at a maximum width. Chrome: unsure of the exact width. `white-space: nowrap` by default differs mildly, since long `title` text wraps.
  - Can the app change it? **Partially** (class CSS is overridable; `multiline` covers nowrap).
  - Proposal: `--rinch-tooltip-text-color`, `--rinch-tooltip-z-index`; consider `multiline` default true with a max width.
  - Likelihood: **Low**.
- **tooltip.rs:152-166**: `disabled` is captured once at render, and the content has no `aria-describedby` link to the target.
  - *Chrome:* `title` is exposed as the element's accessible description (or as its name, if there is none), so the description link is native. **Differs from Chrome** on the a11y side.
  - Can the app change it? **Partially** (reactive props re-render).
  - Proposal: `disabled_fn: Option<ReactiveBool>`, and emit `aria-describedby` by default.
  - Likelihood: **Low**.

### HoverCard (hover_card.rs)

- **Differs from Chrome.** **hover_card.rs:144-172 / styles/hover_card.rs:54**: open/close is pure CSS `:hover`. There is no controlled mode, no `on_open`/`on_close` (for example to lazy-load the card's data on first hover), it does not open on keyboard focus, and there is no touch path.
  - *Chrome:* the closest equivalent is interest invokers (`interestfor` + `popover="hint"`), which are recent and the details are still moving. They show on hover **and on keyboard focus**, and on touch via long-press. They fire `interest`/`loseinterest` events, and the page can drive them from script. Chrome: unsure of exact shipping status.
  - Can the app change it? **No.**
  - Proposal: `opened_fn: Option<ReactiveBool>`, `on_open_change: Option<ValueCallback<bool>>`, `open_on_focus: bool` (default **true**, matching interest invokers).
  - Likelihood: **High**. Lazy-loading profile data on hover is the canonical use.
- **Differs from Chrome (default only).** **styles/hover_card.rs:27, :54**: the default open delay is 0 and the default close delay is effectively the 150ms fade.
  - *Chrome:* interest invokers default to `interest-delay-start: 0.5s` and `interest-delay-end: 0.25s`. Chrome: unsure these defaults are final.
  - Can the app change it? **Yes** (`open_delay` / `close_delay`).
  - Proposal: defaults 500ms / 250ms.
  - Likelihood: **Low**.
- **styles/hover_card.rs:26-27**: `open_delay` is also used as the fade **duration** (`transition: opacity var(--rinch-hover-card-open-delay,150ms)`), so a 500ms delay also means a 500ms fade.
  - *Chrome:* no fade by default (a popover toggles `display`). The delay and any author transition are independent.
  - Can the app change it? **Partially** (override the transition in CSS).
  - Proposal: a separate `transition_duration: Option<u32>` → `--rinch-hover-card-duration` (default 0 per Chrome, or keep 150ms as polish; flag the choice).
  - Likelihood: **Medium**.
- **styles/hover_card.rs:22-23, 99-140**: there is no flip/shift (unlike Popover), and `z-index: 100` is not a var and has no prop.
  - *Chrome:* anchor positioning without `position-try-fallbacks` does not flip either; `popover="hint"` is in the top layer, above all z-index. See the Popover placement finding for the default choice.
  - Can the app change it? **Partially** (CSS).
  - Proposal: `z_index: Option<i32>` → `--rinch-hover-card-z-index`, and opt-in `floating::arm`.
  - Likelihood: **Medium**.

### Notification (notification.rs)

- **styles/notification.rs:14, :32-59**: each Notification is `position: fixed` at a corner on its own. Two open at the same `position` paint on top of each other. There is no stack/queue container, no limit, and no gap.
  - *Chrome:* no equivalent (Web Notifications are OS-level).
  - Can the app change it? **Partially.** The app can override `position` via `class:` + CSS and stack them itself.
  - Proposal: a `NotificationStack { position, gap, limit }` container (Notifications inside it become static), or `--rinch-notification-offset` for manual stacking.
  - Likelihood: **High**. More than one toast at a time is the normal case.
- **notification.rs:269-317**: the `auto_close` timer runs straight through. It does not pause on hover or focus, and does not pause while the window is unfocused.
  - *Chrome:* no equivalent.
  - Can the app change it? **No.**
  - Proposal: `pause_on_hover: bool` (default false today; true is the common UX).
  - Likelihood: **Medium**.
- **styles/notification.rs:21-23**: hidden is `display: none !important`. There is no enter/exit animation and an app cannot add one (same issue as Modal).
  - *Chrome:* no equivalent.
  - Can the app change it? **No.**
  - Proposal: visibility-based hide + `--rinch-notification-transition` / `transition_duration` prop (default 0).
  - Likelihood: **Medium**.
- **notification.rs:116, :325-330**: the icon is `Option<TablerIcon>` only (no custom node, image or avatar). There is no `role="status"`/`"alert"` and no `aria-live`, and the loading spinner is fixed.
  - *Chrome:* no equivalent element (`<output>` has implicit `role="status"`, the closest native live region).
  - Can the app change it? **No.**
  - Proposal: `icon_node` slot; emit `role="status"` by default with `role: String` override ("alert").
  - Likelihood: **Medium**.

### FloatingPanel (floating_panel.rs)

- **floating_panel.rs:137-151, 171-183**: drag and resize are unconstrained. The panel can be dragged fully off-screen (no viewport/parent clamp), there is no max size, no snap, and only one bottom-right resize handle. Signals are written every move frame, with **no `on_drag_end`/`on_resize_end`** commit callback (for persisting layout).
  - *Chrome:* no equivalent for dragging. CSS `resize: both` is the nearest equivalent for resizing; it also has a single bottom-right handle (matching), has no max beyond `max-width`/`max-height`, and has no end event.
  - Can the app change it? **Partially.** The app can clamp by watching the signals, which causes a 1-frame jump. It cannot get a gesture-end event.
  - Proposal: `bounds: String` ("viewport" | "parent" | "none", default "none"), `max_width`/`max_height: Option<f32>`, `on_move_end: Option<Rc<dyn Fn(f32, f32)>>`, `on_resize_end: Option<Rc<dyn Fn(f32, f32)>>`, `resize_handles: String`.
  - Likelihood: **High**. Persisting panel layout and keeping it on-screen are the first things a tool-panel app needs.
- **floating_panel.rs:113-135**: the header is fixed: a title `String` and a `×` text glyph close button with a hard-coded `aria-label="Close panel"` (English, :125-126). There is no slot for header actions (pin/collapse/menu), and the whole header is the drag handle.
  - *Chrome:* no equivalent.
  - Can the app change it? **No.**
  - Proposal: `header_actions` slot, `close_label: String` (default "Close panel"), `close_icon: Option<TablerIcon>`.
  - Likelihood: **Medium**.
- **floating_panel.rs:90-101**: fallbacks for missing signals are hard-coded at 50/50/300/200, with minimums of 200×100. These are props, so this is fine apart from the internal signal not being exposed when not passed. styles/floating_panel.rs:6 `z-index: 150` has no prop and no bring-to-front on click.
  - *Chrome:* no equivalent.
  - Can the app change it? **Partially** (CSS z-index).
  - Proposal: `z_index: Option<i32>` and `bring_to_front_on_focus: bool`.
  - Likelihood: **Low**.

### LoadingOverlay (loading_overlay.rs)

- **loading_overlay.rs:77 (`_children`), :145-150**: children are ignored and the loader is always the built-in `rinch-loader` (oval/bars/dots). There is no custom loader, no message text ("Saving…"), and no progress.
  - *Chrome:* no equivalent.
  - Can the app change it? **No.**
  - Proposal: render children in place of (or under) the loader, e.g. `loader: Option<NodeHandle>`-style slot + `message: String`.
  - Likelihood: **High**.
- **loading_overlay.rs:40-42**: there is no `visible_fn` (`_fn` surgical prop). A `{|| }` reactive `visible` re-renders the whole overlay. The overlay also blocks the pointer but not the keyboard (covered controls stay Tab-reachable, with no `inert`).
  - *Chrome:* no equivalent overlay. The `inert` attribute is the native way to block both pointer and focus; an overlay alone blocks only the pointer, as here.
  - Can the app change it? **Partially.**
  - Proposal: `visible_fn: Option<ReactiveBool>`, `block_focus: bool` (default false today).
  - Likelihood: **Medium**.

### Scroll lock (overlay_scroll_lock.rs, used by Modal and Drawer)

- No configurability finding beyond the prop. `lock_scroll: bool` exists on both and fully disables the lock. (Popover/DropdownMenu/ContextMenu never lock; that matches Chrome's `popover`.) The default-only difference from `showModal()`, which does not lock, is recorded under Modal.

---

### Summary

**19 findings are tagged Differs from Chrome** at the start of their first line. 14 are behaviour gaps. 5 are default-only: Modal outside-click, Modal/Drawer lock_scroll, backdrop 0.75 vs 0.1, Modal not centred, and HoverCard delays. The Popover 150ms fade is partly configurable and is counted among the 14.

4 further differences are flagged inside a `*Chrome:*` line, not tagged at the start:
- Popover does not re-place on resize.
- ContextMenu flips by an assumed 180px width.
- Tooltip is `nowrap`.
- Tooltip has no `aria-describedby`.

The 5 most significant:
1. **DropdownMenu and ContextMenu ignore Escape and have no arrow/Home/End/typeahead keyboard navigation.** Every native menu and `<select>` popup has all of these.
2. **Popover's outside click is swallowed by an invisible backdrop.** `popover="auto"` light dismiss lets the click reach the target underneath and leaves hover working.
3. **Popover (default `trap_focus: false`) neither honours `autofocus` on open nor restores focus on close.** Chrome's popover does both without trapping, because rinch ties move/restore to the trap.
4. **Modal/Drawer close on a backdrop click by default.** Chrome's `showModal()` does not (`closedby` defaults to Escape-only). Modal/Drawer also lack `role="dialog"`/`aria-modal`, which `<dialog>` provides implicitly.
5. **Tooltip shows instantly and can be clipped at the window edge.** A `title` tooltip waits about 0.5–1s and is always kept on screen. ContextMenu is the same kind of gap: it does not re-open at the new point on a second right-click, and it cannot be opened by the Menu key or Shift+F10.


## Part C: Form inputs

Read-only audit of `crates/rinch-components/src/{select,number_input,color_input,color_picker,color_swatch,password_input,text_input,textarea,slider,switch,checkbox,radio,value_binding,field_ids,fieldset}.rs` and their `styles/*.rs` (ColorInput/ColorSwatch CSS is in `styles/color_picker.rs`). Cross-checked against `docs/src/guide/component-props.md`. All paths are relative to `crates/rinch-components/src/` unless stated.

**Chrome rule (2026-11-04): "sensible defaults means 'like Chrome would do it'".** Every finding now has a `*Chrome:*` line. Each proposed default is Chrome's behaviour wherever Chrome has an equivalent, and today's behaviour where it has none. `**Differs from Chrome**` marks a finding whose current behaviour departs from Chrome's. That is a bug-like gap, separate from configurability. `**Differs from Chrome (default only)**` marks a choice that is already configurable but has a non-Chrome default. Chrome facts I am not sure of are marked "unsure".

A note on how far an app can reach today: rsx attributes on a component (`class:`, `style:`, `data-*`, `"aria-label"`) land on the component's **root**, not on the inner `<input>`/`<textarea>` (component-props.md:11). Inline `set_style`/`style=` writes on inner nodes beat any app stylesheet. CSS in `styles/*.rs` is class-based and can be overridden.

Two things cut across several components:
- **X1. Inner field attributes can't be passed through** (TextInput, Textarea, PasswordInput, NumberInput, ColorInput). Nothing reaches the `<input>`/`<textarea>`: `name`, `autocomplete`, `inputmode`, `maxlength`, `readonly`, `spellcheck`, `autofocus` (PasswordInput is the only one with `autofocus`), or an `aria-label` for an unlabelled field. There are also no `onfocus`/`onblur`/`onkeydown` callbacks.
  - *Chrome:* every one of these is an attribute or event on the native element, and the page sets it. The defaults are: `autocomplete` absent (autofill on), `spellcheck` on for textarea and text inputs, and no `maxlength`.
  - Proposal: an `input_attrs: Vec<(String, String)>` prop (or rsx forwarding of `input:`-prefixed attributes), plus `onfocus: Option<Callback>`, `onblur: Option<Callback>` and `onkeydown: Option<Rc<dyn Fn(&KeyEventData) -> bool>>`. Defaults follow Chrome: nothing set. PasswordInput's `autocomplete="off"` is a separate finding below.
  - Likelihood: **High**. Login forms, search boxes and validation-on-blur all need these.
- **X2. Fixed English strings.** "Increment"/"Decrement" (NumberInput), "Show password"/"Hide password" (PasswordInput), and the required mark `*` / ` *` (all labelled fields).
  - *Chrome:* the browser's own control strings follow the browser UI locale. A native `required` field shows no mark at all. The mark here is a component-label convention with no Chrome equivalent.
  - Proposal: string props with today's text as the default (no Chrome equivalent to adopt).
  - Likelihood: **Medium**. Any localised app needs this.

### Select (select.rs)

- **Differs from Chrome** — **select.rs:183-184, 371-379; styles/select.rs:86-112 (placement, clipping, stacking).** Today the list can be clipped and covered, which Chrome's popup never is. Down vs up is decided once at open, from `get_click_context()` against a hard-coded `DROPDOWN_RESERVE_PX = 220`. The dropdown is `position:absolute` inside the trigger, so an `overflow` ancestor clips it, and the fixed z 300 (styles/select.rs:100) can sit under a higher layer. There is no horizontal collision handling and no portal.
  - *Chrome:* the popup is a separate widget above everything. It is never clipped by `overflow` ancestors, and on desktop it can extend past the window. It opens below, flips above when there is more room there, is at least as wide as the select, and is shifted horizontally to stay on screen. Its height comes from the item count up to a cap (unsure of the exact cap, roughly 20 rows) and the available space.
  - Can the app change it? Partially. `max-height:200px` can be overridden in CSS, but the 220px reserve does not follow it, and the flip is not exposed.
  - Proposal: `position: String` (`"auto"` default = Chrome's flip, `"bottom"`, `"top"`). `max_dropdown_height: Option<f32>`, feeding both the CSS and the reserve. `within_portal: bool` defaulting to **true** (Chrome's popup is never clipped), or reuse `floating.rs`. `z_index: Option<i32>` → `--rinch-select-z-index`.
  - Likelihood: **High**. A Select in a scrolling panel or table cell gets clipped.
  - Unverified: when Enter/Space opens the list, the flip reads the last *click*'s context.
- **Differs from Chrome** — **select.rs:103-108, 130-158, 432-474 (no disabled options or groups; no search, clear or custom render).** `SelectOption` is `{value, label}` only. Options render as plain text nodes (437). There is no per-option `disabled` and no groups, both of which native `<select>` has. There is also no searchable input, no clear button, and no way to render an option's content.
  - *Chrome:* `<option disabled>` is skipped by arrows and type-ahead and cannot be picked. `<optgroup label>` gives non-selectable headings. There is no search or clear. Rich option content arrived with customizable select (`appearance: base-select`, Chrome 135+).
  - Can the app change it? No. The library has no Autocomplete, Combobox or MultiSelect to fall back on.
  - Proposal: `SelectOption.disabled: bool` and groups (Chrome parity, default false/none). `render_option: Option<Rc<dyn Fn(&SelectOption, bool /*selected*/) -> NodeHandle>>`. `searchable: bool` (default false = Chrome), `clearable: bool` (default false = Chrome), `nothing_found: String`.
  - Likelihood: **High**. Long lists (countries, fonts) and option rows with icons are routine.
- **Differs from Chrome** — **select.rs:15-31, 485-560 (closed-select keys).** A closed Select consumes no keys except Enter/Space, which open it. Arrow keys and typed letters on a focused, closed Select do nothing.
  - *Chrome:* on Windows/Linux, ArrowUp/ArrowDown/Home/End and type-ahead on a focused, closed `<select>` change the value directly, firing `input`/`change` without opening. On macOS, the arrows open the popup. Alt+ArrowDown, Alt+ArrowUp and F4 open it, and Space opens it. Unsure whether Enter opens it on every platform.
  - Can the app change it? No.
  - Proposal: implement Chrome's closed-select keys as the default, with `keyboard_closed: String` (`"platform"` default, `"open_only"` = today).
  - Likelihood: **Medium**. Keyboard-heavy forms.
- **Differs from Chrome** — **select.rs:58, 65-77 (keyboard in the open list).** ArrowUp/ArrowDown always wrap (the code cites Mantine's `loop`). PageUp/PageDown are not handled. The type-ahead reset is a fixed 900ms.
  - *Chrome:* the open popup does **not** wrap: ArrowDown on the last item stays there. PageUp/PageDown move a page. Type-ahead resets after 1 second (Blink's `TypeAhead` timeout). Space and Enter commit. Disabled options are skipped. Unsure what Tab does in the open popup (commit or close).
  - Can the app change it? No.
  - Proposal: `wrap_around: bool` defaulting to **false** (Chrome), PageUp/PageDown handling, and the type-ahead reset at 1000ms (a `typeahead_timeout_ms: u32` prop is optional). Rinch's own `select_widget.rs` `TYPEAHEAD_RESET_MS = 900` has the same gap.
  - Likelihood: **Low/Medium**.
- **Differs from Chrome** — **select.rs:353-365; styles/select.rs:79 (chevron rotates).** The icon is always `TablerIcon::ChevronDown`. Its open rotation is written **inline** (`set_style("transform", "rotate(180deg)")`), so CSS cannot change or remove it. The 200ms transition is CSS and can be overridden.
  - *Chrome:* the native drop-down arrow does not rotate or animate when the popup opens. An app restyles it via `appearance: none` / `::picker-icon` (base-select).
  - Can the app change it? No for the icon or the rotation.
  - Proposal: `chevron_icon: Option<TablerIcon>` / a `right_section` render fn. Move the rotation to a `.rinch-select--opened` class, gated by `rotate_chevron: bool` defaulting to **false** (Chrome).
  - Likelihood: **Medium**. Custom icons, and designs without rotation.
- **select.rs:176, 380 (open state is internal).** `opened` is a private `Signal`.
  - *Chrome:* `select.showPicker()` (Chrome 121+) opens it programmatically. There is no open/close event on a classic select, and nothing closes it from script except blur.
  - Can the app change it? No. There is no `opened_fn`, `on_dropdown_open` or `on_dropdown_close`.
  - Proposal: `opened_fn: Option<ReactiveBool>`, `on_dropdown_open: Option<Callback>`, `on_dropdown_close: Option<Callback>`. Default uncontrolled, as Chrome.
  - Likelihood: **Medium**. Lazy-loading options on open, and closing on programmatic events.
- **select.rs:565-584 (always closes on outside click).** The backdrop is always mounted.
  - *Chrome:* an outside press closes the popup, so this matches. Unsure whether that press also reaches the page underneath. Rinch's backdrop swallows it.
  - Can the app change it? No. ColorInput has `close_on_click_outside`, Select does not.
  - Proposal: `close_on_click_outside: bool` (default true = Chrome), for parity.
  - Likelihood: **Low/Medium**.
- **select.rs:425 (always closes on pick).** `pick` always calls `opened.set(false)`.
  - *Chrome:* a single `<select>` closes on pick, so this matches.
  - Can the app change it? No.
  - Proposal: `close_on_select: bool` (default true = Chrome).
  - Likelihood: **Low** for a single Select.

### NumberInput (number_input.rs)

- **Differs from Chrome** — **number_input.rs:401-404 (controlled steppers are inert).** With `value_fn` set, the steppers only fire `onincrement`/`ondecrement`, which carry no value. The app must re-implement step and clamp itself.
  - *Chrome:* the spin buttons always step (`stepUp`/`stepDown`: snap to the step base, clamp to min/max) and fire `input` then `change`. A framework's controlled value then re-renders from those events.
  - Can the app change it? Partially, via those callbacks.
  - Proposal: by default, run `apply_step` in controlled mode as well and report the stepped text through `oninput`/`onchange`, so the app's signal stays the single writer. Alternatively, add `onstep: Option<ValueCallback<f64>>`.
  - Likelihood: **High**. Controlled is the documented pattern for forms.
- **Differs from Chrome** — **number_input.rs (no ArrowUp/ArrowDown stepping on desktop); rinch/src/app/mod.rs:4027-4050.** The component registers no key handler. The desktop runtime deliberately leaves ArrowUp/ArrowDown alone on `type="number"` ("a browser steps the value there"; test `a_number_input_is_left_alone` in `rinch/src/app/textarea_vertical_arrow_tests.rs:350`). So on desktop the arrows do nothing. On rinch-web, the browser's own `<input type=number>` steps.
  - *Chrome:* ArrowUp/ArrowDown step by `step`, clamped to min/max, and fire `input`/`change`. Holding a key auto-repeats.
  - Can the app change it? No.
  - Proposal: step on ArrowUp/ArrowDown by default (`keyboard_step: bool`, default true), through the same path as the steppers.
  - Likelihood: **High**. Keyboard users expect it, and it differs between backends today.
- **Differs from Chrome** — **number_input.rs:461-513 (no hold-to-repeat; fixed icons).** There is no press-and-hold auto-repeat on the steppers. The icons are fixed (`chevron_up_dom`/`chevron_down_small_dom`). The buttons are `tabindex=-1`.
  - *Chrome:* holding a spin button repeats the step after a short delay (unsure of the exact delay and interval). The spin buttons are not in the Tab order, so `tabindex=-1` matches. Their look is restyled only via `::-webkit-inner-spin-button`.
  - Can the app change it? Only `hide_controls`, which removes them entirely.
  - Proposal: `step_hold_delay: Option<u32>` / `step_hold_interval: Option<u32>`, defaulting **on** with Chrome-like timings. `increment_icon`/`decrement_icon` render fns, with today's icons as the default.
  - Likelihood: **Medium**.
- **Differs from Chrome** — **number_input.rs (no wheel stepping).** No wheel handler.
  - *Chrome:* a mouse wheel over a *focused* `<input type=number>` steps the value. Fairly sure, though it varies by version.
  - Proposal: `wheel_step: bool` (default true = Chrome while focused).
  - Likelihood: **Low/Medium**. Some apps want it off. It is a common complaint about the native control.
- **number_input.rs:440 (clamping only on stepper clicks; no invalid state).** `clamp_to` runs only in `apply_step`. A typed value outside `[min,max]` is never clamped. There is also no out-of-range or invalid state reported to the app or styled.
  - *Chrome:* it does not clamp typed values either, so this matches. The field matches `:invalid` (`validity.rangeOverflow`/`rangeUnderflow`/`stepMismatch`), and form submission is blocked.
  - Can the app change it? Only by clamping in `onchange` itself, and by computing validity by hand.
  - Proposal: `clamp_behavior: String` (`"step"` default = Chrome, `"blur"`, `"strict"`). Also add a `rinch-number-input--invalid` class plus `aria-invalid` when out of range or off-step, mirroring Chrome's `:invalid`.
  - Likelihood: **Medium**.
- **number_input.rs:211-221 (display formatting).** `format_shown` rounds to 1e-10 or `decimal_scale`. There are no thousands separators, no decimal separator option and no locale.
  - *Chrome:* no grouping separators. Fairly sure it localises the displayed decimal separator to the browser locale. There is no formatting hook.
  - Proposal: `thousand_separator: String` (default empty = Chrome), `decimal_separator: String`, or `format_fn: Option<Rc<dyn Fn(f64) -> String>>`.
  - Likelihood: **Medium** (prices, i18n).
- **number_input.rs:468, 500.** The aria-labels are fixed (see X2).
  - *Chrome:* the field is a `spinbutton` and its spin buttons are not separately labelled controls.

### ColorInput (color_input.rs)

- **Differs from Chrome** — **styles/color_picker.rs:353-357 (placement).** The dropdown is always `top:100%; left:0`, with no flip and no collision handling. Unlike Select, it doesn't even flip near the window bottom.
  - *Chrome:* `<input type=color>` opens Chrome's own popup (Windows/Linux/ChromeOS) or the OS colour panel (macOS). The popup is placed like a select popup: below, flipping above when there is no room, never clipped by the page.
  - Can the app change it? Partially. An app could override `.rinch-color-input__dropdown` CSS for *all* instances to open upward, but nothing is decided per instance or at runtime.
  - Proposal: `position: String` (`"auto"` default = Chrome's flip, `"bottom-start"`), or use `floating.rs`.
  - Likelihood: **High**. The picker is about 260px tall and gets cut off near the window bottom.
- **Differs from Chrome** — **color_input.rs:72, 440-457 (onchange fires every drag frame).** `onchange` fires on every frame of a panel or slider drag. There is no commit-at-end callback.
  - *Chrome:* `input` fires continuously while the picker changes. `change` fires once, when the choice is committed (popup closed / OK).
  - Can the app change it? No.
  - Proposal: Chrome's split as the default. Add `oninput` for the per-frame stream and have `onchange` fire at the end of the gesture. This is a behaviour change for existing `onchange` users. Without renaming, the fallback is `onchange_end: Option<InputCallback>`.
  - Likelihood: **High** (undo history, persistence).
- **color_input.rs:226-230 (empty means black).** An empty `value` is replaced by `"#000000"`, and the field is written with it (615). So the field is never empty, and `placeholder` is never visible on mount.
  - *Chrome:* `<input type=color>` has no empty state either. Its value defaults to `#000000`, and an invalid value is sanitised to `#000000`. Today matches Chrome.
  - Can the app change it? No.
  - Proposal: keep `#000000` as the default (Chrome). Add `allow_empty: bool` (default false) so the swatch shows the checkerboard and the field shows the placeholder.
  - Likelihood: **High**. "No colour chosen" is a normal form state, and the placeholder prop is effectively dead on mount.
- **color_input.rs:392-405 (every press on the field toggles the dropdown).** The handler sits on the whole input group, including the text field. A second click into the text (to place the caret) closes the picker. There is no open-on-focus mode and no way to have no dropdown (Mantine's `withPicker={false}`).
  - *Chrome:* no equivalent. The native control is a button with no text field, and a click opens the picker.
  - Can the app change it? No.
  - Proposal: `with_picker: bool` (default true), `open_on: String` (`"click"` toggle default, `"focus"`), or toggle only from the swatch.
  - Likelihood: **Medium**.
- **color_input.rs:440-457 (swatch pick never closes).** A swatch pick never closes the dropdown.
  - *Chrome:* unsure. With a `list` attribute, Chrome's popup shows suggestion swatches, and I don't know whether picking one closes the popup.
  - Can the app change it? No.
  - Proposal: `close_on_swatch_click: bool` (default false, today's).
  - Likelihood: **Medium**. Swatch-only palettes want pick-and-close.
- **color_input.rs:225 (open state is internal).** No `opened_fn` or open/close callbacks.
  - *Chrome:* `showPicker()` opens it. There is no close API or open/close event.
  - Can the app change it? No.
  - Proposal: `opened_fn: Option<ReactiveBool>`, `on_dropdown_open`/`on_dropdown_close: Option<Callback>`.
  - Likelihood: **Medium**.
- **color_input.rs:265-283, 584-590 (preview swatch).** The swatch is always shown. Its size comes from a fixed table (158-166). Its overlay's `background-color` and `border-radius: var(--rinch-radius-sm)` are written **inline** on every value change.
  - *Chrome:* no equivalent for hiding the swatch. The native control *is* a swatch, styled via `::-webkit-color-swatch`, which CSS can restyle.
  - Can the app change it? No to hiding it. No to its radius (inline beats CSS). Size only via `size`.
  - Proposal: `with_preview: bool` (default true). Move the radius to the swatch class (`--rinch-color-input-swatch-radius`).
  - Likelihood: **Low/Medium**.
- **color_input.rs:563-572 (closes when focus leaves).** The list closes when the focus leaves the input, with no opt-out (`close_on_click_outside` covers only the backdrop).
  - *Chrome:* the colour popup closes when it loses focus, so this matches.
  - Likelihood: **Low**.

### ColorPicker (color_picker.rs)

- **Differs from Chrome** — **color_picker.rs:241-248, 290-294, 335-339, 551-605 (no commit-at-end callback).** `onchange` fires on every drag frame. There is no `onchange_end` (Mantine `onChangeEnd`), and Drag `.on_end` is never hooked.
  - *Chrome:* as for ColorInput, `input` fires continuously and `change` fires once on commit.
  - Can the app change it? No.
  - Proposal: Chrome's split as the default. `oninput` per frame. `onchange` fired from `Drag::on_end`, a swatch click and a hex commit. The fallback name is `onchange_end: Option<InputCallback>`.
  - Likelihood: **High**. Undo stacks and saving to a store want one event per gesture.
- **Differs from Chrome** — **color_picker.rs:196-251, 277-297, 313-342 (no keyboard or ARIA).** The saturation panel, hue and alpha sliders have no `tabindex`, no `role="slider"`/`aria-value*`, and no arrow keys. They are pointer-only.
  - *Chrome:* its redesigned popup picker (Windows/Linux/ChromeOS) is keyboard-operable: Tab between parts, with arrows moving the colour-well point and the hue. I'm unsure of the exact keys and step sizes. On macOS the OS panel's own keyboard support applies.
  - Can the app change it? No.
  - Proposal: make each part focusable, with Arrow keys stepping (`saturation_step`/`hue_step: f64` props) and aria labels as string props. On by default.
  - Likelihood: **Medium**.
- **Differs from Chrome** — **color_picker.rs:135-149 (empty means red).** An empty or unparseable `value` starts at pure red (h0 s1 v1).
  - *Chrome:* an empty or invalid colour value is `#000000`. ColorInput already follows that, so the two rinch components disagree.
  - Proposal: `default_value: String` defaulting to `"#000000"` (Chrome).
  - Likelihood: **Low**.
- **color_picker.rs:502-505 (swatch size).** The swatch size is computed from a hard-coded `200`px picker width, so `size: "lg"/"xl"` (240/280px, styles/color_picker.rs:46-47) leaves the grid short. The size is passed into `ColorSwatch`, which writes it **inline**.
  - *Chrome:* no equivalent.
  - Can the app change it? No (inline).
  - Proposal: compute it from the size step, or use CSS grid with `--rinch-color-picker-swatches-per-row`. Optionally add a `swatch_size: String` prop.
  - Likelihood: **Medium**.
- **color_picker.rs:507-532 (swatches).** Swatches are always ColorSwatch squares with no tooltip or label. There is no `render_swatch` and no swatch-click callback separate from `onchange`.
  - *Chrome:* `<datalist>` suggestions show as plain swatches, so there's no richer equivalent.
  - Proposal: `on_swatch_click: Option<InputCallback>`.
  - Likelihood: **Low/Medium**.
- **color_picker.rs:378-383, 457-463 (preview swatch in `with_input`).** The swatch is fixed at 28px with an inline radius.
  - *Chrome:* no equivalent.
  - Likelihood: **Low**.

### ColorSwatch (color_swatch.rs)

- **color_swatch.rs:62-68 (inline geometry).** `width`, `height` and `border-radius` are written **inline**.
  - *Chrome:* no equivalent.
  - Can the app change it? Only through the `size`/`radius` props. An app class or stylesheet cannot resize it (e.g. responsive sizing).
  - Proposal: publish `--rinch-color-swatch-size` / `--rinch-color-swatch-radius` on the root and size from them in CSS.
  - Likelihood: **Medium**.
- **color_swatch.rs:70-74 (clickable swatch isn't a button).** A clickable swatch gets `data-rid` only: no `tabindex`, no `role="button"`, no `aria-label`. ColorPicker swatches are therefore unreachable by keyboard.
  - *Chrome:* no direct equivalent. The nearest is a `<button>`, which is focusable and activates on Enter/Space.
  - Proposal: when `onclick` is set, add `tabindex=0` and `role=button`, plus an `aria_label: String` prop (defaulting to the colour text).
  - Likelihood: **Medium**.
- **styles/color_picker.rs:10-16 (checkerboard).** The checkerboard `#ccc` is a literal.
  - *Chrome:* no equivalent.
  - Can the app change it? Yes, by overriding the class background-image. Not a finding beyond the missing variable.
  - Likelihood: **Low**.

### PasswordInput (password_input.rs)

- **Differs from Chrome** — **password_input.rs:266 (`autocomplete="off"`).** `autocomplete="off"` is hard-coded on the field.
  - *Chrome:* a password field has no `autocomplete` by default, and the password manager offers to fill and save. Chrome's password manager in fact ignores `autocomplete="off"` on password fields. `current-password`/`new-password` refine its behaviour, and `new-password` triggers password generation.
  - Can the app change it? No (X1). Password managers are told not to fill.
  - Proposal: `autocomplete: String` defaulting to **unset** (Chrome), documenting `"current-password"` and `"new-password"`.
  - Likelihood: **High**. Every login form wants password-manager support.
- **password_input.rs:386-392, 367-384 (toggle does nothing uncontrolled).** The toggle (shown by default, `toggle_visibility: true`, 180) only calls `ontoggle`. No internal visibility state exists. Without `ontoggle` + `visible_fn` the eye button is dead: it never registers a handler and never swaps the icon or aria-label.
  - *Chrome:* no reveal button (Edge has `::-ms-reveal`; Chrome has none). The default is kept as today's.
  - Can the app change it? Yes, by wiring both, but uncontrolled use is broken by default.
  - Proposal: keep an internal `Signal<bool>` when `visible_fn` is `None` (Mantine `defaultVisible`/`onVisibilityChange`). Keep `ontoggle` as a notification.
  - Likelihood: **High**.
- **password_input.rs:349, 354 (toggle out of the Tab order).** The toggle is `tabindex=-1` + `data-nofocus`, so keyboard users cannot reach it.
  - *Chrome:* no equivalent. Unsure whether Edge's reveal button takes focus.
  - Can the app change it? No.
  - Proposal: `toggle_tabbable: bool` (default false, today's).
  - Likelihood: **Medium** (accessibility requirements).
- **password_input.rs:335-343 (eye icons).** The eye/eye-off icons are fixed, with no override.
  - *Chrome:* no equivalent.
  - Proposal: `visibility_toggle_icon: Option<Rc<dyn Fn(bool) -> NodeHandle>>`.
  - Likelihood: **Medium**.
- **password_input.rs:248-257 vs text_input.rs:225-232 (description position differs).** The description renders *above* the field here and in NumberInput (254-261), but *below* in TextInput, Textarea, Select and ColorInput. None of them exposes an order.
  - *Chrome:* no equivalent (the page lays out its own description).
  - Proposal: `input_wrapper_order: Vec<String>` (Mantine `inputWrapperOrder`), or one shared default.
  - Likelihood: **Medium** (visual consistency across a form).
- **password_input.rs:356-361, 377, 381.** The aria-labels "Show password"/"Hide password" are fixed (see X2).
  - *Chrome:* no equivalent.

### TextInput (text_input.rs)

- **text_input.rs:169-222 (no left/right sections).** The field is a bare `<input>`, with no left/right section slot for a search icon, a unit or a clear button.
  - *Chrome:* no general equivalent. The nearest is `<input type=search>`, which shows a built-in clear (×) button while non-empty, and Escape clears it.
  - Can the app change it? No. Wrapping it doesn't place content inside the border.
  - Proposal: `left_section` / `right_section: Option<Rc<dyn Fn(&mut RenderScope) -> NodeHandle>>`, plus `left_section_pointer_events`.
  - Likelihood: **High**.
- **text_input.rs:164-222.** See X1: no `name`, `autocomplete`, `maxlength`, `readonly`, `autofocus` or focus/blur/key callbacks on the inner input.
  - *Chrome:* all native attributes, set by the page.
  - Likelihood: **High**.
- **text_input.rs:153-157.** The required mark is always `*` when `required`. There is no `with_asterisk` toggle (Mantine separates `required` and `withAsterisk`).
  - *Chrome:* no equivalent. A native `required` shows no mark, and the label is the page's.
  - Proposal: `with_asterisk: Option<bool>` (default follows `required`, today's).
  - Likelihood: **Low/Medium**.

### Textarea (textarea.rs)

- **Differs from Chrome (default only)** — **styles/textarea.rs:24 (`resize: vertical`).** Today the user can only resize vertically.
  - *Chrome:* the UA stylesheet gives `<textarea>` `resize: auto`, which resizes in **both** directions.
  - Can the app change it? Yes, via CSS. There is no prop.
  - Proposal: `resize: String` prop defaulting to `"both"` (Chrome). Autosize keeps `none`.
  - Likelihood: **Low/Medium**.
- **textarea.rs:177-181 (`max_rows` cap).** The cap is written **inline** as `calc(1.2em * rows + 2*var(--rinch-spacing-sm) + 2px)`. It assumes the default line-height and padding. If an app restyles either, the cap is wrong and CSS cannot fix it.
  - *Chrome:* no equivalent. `field-sizing: content` (Chrome 123+) grows a textarea, and an app caps it with its own `max-height`.
  - Proposal: publish `--rinch-textarea-max-rows` and compute `max-height` in the stylesheet from `--rinch-textarea-line-height` / padding variables.
  - Likelihood: **Low/Medium**.
- **textarea.rs:13-81.** No `radius` prop (TextInput, PasswordInput and NumberInput have one). X1 applies (no `maxlength`, `readonly`, `spellcheck`, focus/blur). No Ctrl/Cmd+Enter submit.
  - *Chrome:* no equivalent for submit-on-modifier+Enter (a textarea never submits its form). `spellcheck` is on by default for a textarea.
  - Proposal: `radius: String`, `onsubmit: Option<Callback>` (modifier+Enter, default unset).
  - Likelihood: **Medium** (chat and comment boxes want submit-on-Ctrl+Enter).
- **styles/textarea.rs:25, 59-63 (min-height floor).** The `min-height` 60–120px floor overrides `min_rows` below about 3 lines (acknowledged at 64-69).
  - *Chrome:* no floor. A `rows` value is honoured exactly (the default is `rows=2`).
  - Can the app change it? Yes, through CSS. It's still a surprising default for an explicit prop.
  - Proposal: drop the floor when `min_rows` is set.
  - Likelihood: **Low**.

### Slider (slider.rs)

- **Differs from Chrome** — **slider.rs:197-281 (no keyboard or ARIA).** No `tabindex`, no `role="slider"`, no `aria-valuemin/max/now`, and no key handling. The slider is pointer-only and not in the Tab order.
  - *Chrome:* `<input type=range>` is focusable and in the Tab order. ArrowRight/ArrowUp increase and ArrowLeft/ArrowDown decrease by `step` (horizontal direction flips in RTL). PageUp/PageDown move by a larger step: max(step, range/10), fairly sure. Home/End go to min/max. Each key fires `input` then `change`.
  - Can the app change it? No.
  - Proposal: a focusable thumb with Chrome's key map on by default, `large_step: Option<f64>` (default range/10), and `aria_label: String`.
  - Likelihood: **High** (accessibility).
- **Differs from Chrome** — **slider.rs:242-275 (onchange fires every frame; no commit-at-end).** `onchange` fires on the press and on every drag frame, which is `input` semantics. There is no event at release.
  - *Chrome:* `input` fires on every value change during a drag. `change` fires once at release, and after each key step.
  - Can the app change it? No.
  - Proposal: Chrome's split as the default. `oninput: Option<ValueCallback<f64>>` per frame, and `onchange` at release and per key step. This changes behaviour for existing `onchange` users. The fallback is `onchange_end: Option<ValueCallback<f64>>` from `Drag::on_end`.
  - Likelihood: **High** (expensive recompute, persistence, undo).
- **Differs from Chrome** — **slider.rs:242, 283 (no internal state).** Uncontrolled use doesn't work. With only `value` (static), the thumb never moves. Without `onchange`, nothing is interactive.
  - *Chrome:* a range input is uncontrolled by default. It moves on drag and keys with no script attached.
  - Can the app change it? Only by supplying `value_signal` + `onchange`.
  - Proposal: an internal signal when `value_signal` is `None` (Chrome's default).
  - Likelihood: **Medium**.
- **Differs from Chrome (default only)** — **slider.rs:164-172, 91, 249-250 (defaults for value and step).** With no `value`, the slider starts at `min`. With no `step`, values are continuous (no snapping).
  - *Chrome:* with no `value`, a range starts at the midpoint, `min + (max-min)/2`. The default `step` is `1`. min/max default to 0/100, which matches.
  - Can the app change it? Yes, by passing `value`/`step`.
  - Proposal: default to the midpoint and `step: 1` (Chrome), and allow `step: "any"` (as a sentinel) for continuous values.
  - Likelihood: **Low/Medium**.
- **slider.rs:222-231 (label never updates).** The label text is computed **once at render** from the initial value. With `value_signal` the thumb moves (283-298) but the label never updates. The format is a `{value}` string template only, with the raw `f64` `Display` (e.g. `33.33333333333333`).
  - *Chrome:* no equivalent (a range has no value bubble).
  - Can the app change it? No.
  - Proposal: `label_fn: Option<Rc<dyn Fn(f64) -> String>>`, reactive to the value signal. Keep `label` as the template.
  - Likelihood: **High** (it's also a bug).
- **styles/slider.rs:101-108 vs slider.rs:222 (`show_label_on_hover` ignored).** The CSS shows the label on `:hover` whenever a label exists. So `label: "{value}%"` with `show_label_on_hover: false` still shows it on hover. The flag only decides whether the label node exists.
  - *Chrome:* no equivalent.
  - Proposal: add a `rinch-slider--label-on-hover` class and gate the hover rule on it.
  - Likelihood: **Medium**.
- **slider.rs (no marks, inverted mode or custom thumb).** No marks, `inverted`, `thumb_children`/thumb render, or `scale` fn.
  - *Chrome:* `<input type=range list=…>` with a `<datalist>` draws tick marks, and the thumb snaps near them. The thumb is restyled via `::-webkit-slider-thumb`. There is no inverted mode (`direction: rtl` / vertical writing modes flip it).
  - Proposal: `marks: Vec<(f64, String)>` (default none = Chrome without a list).
  - Likelihood: **Low/Medium**.

### Checkbox (checkbox.rs)

- **Differs from Chrome** — **checkbox.rs:79-108, 168-172; styles/checkbox.rs:125-133 (indeterminate is half-wired).** `indeterminate` only swaps the icon. The `rinch-checkbox--indeterminate` class the stylesheet styles is **never added**, so an indeterminate, unchecked box renders empty with a hidden icon. It is also static only: no `indeterminate_fn`, and the icon is chosen once at render.
  - *Chrome:* `indeterminate` is a script-set property that draws a dash in the filled box, whatever `checked` is, and matches `:indeterminate`. A user click clears it (sets `indeterminate = false`) and toggles `checked`.
  - Can the app change it? No.
  - Proposal: add the class and `indeterminate_fn: Option<ReactiveBool>` toggling both class and icon. Clear the visual indeterminate state on click, as Chrome does, and report it through `onchange`.
  - Likelihood: **High** ("select all" rows; it's also a bug).
- **Differs from Chrome** — **checkbox.rs:187-191 (label is plain text).** The label is a `String` text node.
  - *Chrome:* a `<label>` holds arbitrary phrasing content. Clicking a link inside it follows the link and does **not** toggle the checkbox, because interactive descendants keep their activation. Clicking plain label text toggles it.
  - Can the app change it? No. "I agree to the <a>Terms</a>" needs a link inside the label.
  - Proposal: let children render as the label (`children` → `.rinch-checkbox__label`), with Chrome's rule that interactive content inside doesn't toggle. Or `label_node: Option<render fn>`.
  - Likelihood: **High**.
- **Differs from Chrome** — **checkbox.rs:145-151 (no internal state).** No internal checked state: without `checked_fn`, clicking never changes the visual.
  - *Chrome:* a checkbox toggles itself on click and Space with no script.
  - Proposal: an internal signal when `checked_fn` is `None` (Chrome's default).
  - Likelihood: **Medium**.
- **Differs from Chrome (default only)** — **styles/checkbox.rs:83 (label `white-space: nowrap`).** Long labels never wrap. The same rule is in styles/switch.rs:81 and styles/radio.rs:83.
  - *Chrome:* label text wraps normally (`white-space: normal`).
  - Can the app change it? Yes, through CSS.
  - Proposal: default `white-space: normal` for all three.
  - Likelihood: **Medium**. Consent text and long option labels overflow narrow layouts.
- **checkbox.rs:33-56 (no colour or label position).** No `color` prop (Radio and Slider have one). No `label_position` (Switch has one).
  - *Chrome:* `accent-color` sets the checked fill, and an app sets it in CSS. Label position is just DOM order.
  - Can the app change it? Partially, by overriding `--rinch-primary-color` via `style:` on the root.
  - Proposal: `color: String` → `--rinch-checkbox-color` (the analogue of `accent-color`), and `label_position: String` (default `"right"`, today's).
  - Likelihood: **Medium**.
- **checkbox.rs:168-172 (check icon).** The checkmark/indeterminate icons are fixed.
  - *Chrome:* no equivalent short of `appearance: none` and drawing it yourself.
  - Proposal: `icon: Option<Rc<dyn Fn(bool /*indeterminate*/) -> NodeHandle>>`.
  - Likelihood: **Medium**.
- **styles/checkbox.rs:60.** The icon colour `white` is a CSS literal.
  - *Chrome:* the check is white (or dark, picked for contrast) on the accent colour, so this broadly matches.
  - Can the app change it? Yes, through CSS.
  - Proposal: `--rinch-checkbox-icon-color`.
  - Likelihood: **Low**.

### Switch (switch.rs)

- **Differs from Chrome** — **switch.rs:185-190.** The label is plain text, as for Checkbox.
  - *Chrome:* the switch equivalent is a checkbox. Chrome has no native switch (Safari has `<input type=checkbox switch>`). Its `<label>` takes arbitrary content.
  - Proposal: children as the label.
  - Likelihood: **High/Medium**.
- **Differs from Chrome** — **switch.rs:151-157.** No internal checked state, as for Checkbox.
  - *Chrome:* the underlying checkbox toggles itself.
  - Likelihood: **Medium**.
- **switch.rs:33-57 (no colour, track labels or thumb icon).** No `color`, no `on_label`/`off_label` inside the track, no `thumb_icon`. Sizes stop at `lg` (83-90).
  - *Chrome:* no equivalent (beyond `accent-color` for colour).
  - Can the app change it? Colour partially, via `--rinch-primary-color` on the root.
  - Proposal: `color: String` → `--rinch-switch-color`, `on_label`/`off_label: String`, `thumb_icon` render fn.
  - Likelihood: **Medium**.

### Radio / RadioGroup (radio.rs)

- **Differs from Chrome** — **radio.rs:218-222 (shared default name).** A radio with no `name` gets `name="radio-group"` (the comment says "the DOM crashes without it"). Every unnamed radio in the app is one group, so checking one unchecks unrelated radios elsewhere (on the web certainly).
  - *Chrome:* a radio with **no** `name` belongs to no group. It is its own group of one and never unchecks any other radio. Named groups are scoped to their form owner (or the tree when there is no form).
  - Can the app change it? Yes, by always passing `name`. The default is a trap, though.
  - Proposal: an unnamed Radio gets a per-instance unique name (Chrome's no-group semantics), and RadioGroup generates one shared unique name for its children. Separately, fix whatever crashes on an absent `name`.
  - Likelihood: **Medium/High**.
- **Differs from Chrome** — **radio.rs:318-335 (RadioGroup isn't a controlled group; no arrow keys).** `RadioGroup` has no `value`, `value_fn` or `onchange`. Every `Radio` must be wired by hand with its own `checked_fn` and `onchange`. There is no group-level arrow-key roving (nothing in the components or the desktop runtime handles arrows on radios).
  - *Chrome:* checking one radio unchecks the rest of its group automatically. ArrowDown/ArrowRight move to the next radio and **check** it, ArrowUp/ArrowLeft the previous, wrapping at the ends and skipping disabled radios. Tab stops once on the group, at the checked radio (or the first when none is checked). `RadioNodeList.value` reads the group's value.
  - Can the app change it? Only by per-radio wiring.
  - Proposal: `RadioGroup { value_fn: Option<ReactiveString>, onchange: Option<InputCallback>, name: String }` that wires its radios by their `value`, with Chrome's arrow keys and single Tab stop on by default.
  - Likelihood: **High**.
- **radio.rs:361-369 (group label isn't linked).** The group label is a `div`, not linked to the `role=radiogroup` via `aria-labelledby`. The group's `error` doesn't reach the radios' `error` state.
  - *Chrome:* the native pattern is `<fieldset><legend>`, where the legend names the group in the accessibility tree.
  - Proposal: wire `aria-labelledby`/`aria-describedby` as `field_ids` does, and propagate the error class.
  - Likelihood: **Medium**.
- **radio.rs:81-105 (label).** No `label_position`, and the label is a String only (as for Checkbox).
  - *Chrome:* a `<label>` takes arbitrary content (see Checkbox).
  - Likelihood: **Medium**.
- **radio.rs:197-205 (focus ring ignores `color`).** `color` sets `--rinch-radio-color`, but the focus ring stays `--rinch-primary-color-2` (styles/radio.rs:50).
  - *Chrome:* the focus ring is the browser's own two-tone ring and is unaffected by `accent-color`. Today's independent ring is in that spirit.
  - Likelihood: **Low**.

### Fieldset (fieldset.rs)

- **Differs from Chrome** — **fieldset.rs:184-186 (`disabled` doesn't reach rinch components).** `disabled` is static and only sets the attribute. rinch-dom itself does apply HTML's rule: `node_is_disabled` in `rinch-dom/src/node.rs:4477-4500` disables descendant native controls, exempting the first legend. But the rinch components inside don't read it, so their `--disabled` classes and styling don't follow. Unverified: whether handlers registered on component roots via `data-rid` (e.g. Checkbox's label `onchange`) still fire. A reactive prop re-renders the whole fieldset.
  - *Chrome:* `<fieldset disabled>` disables every descendant form control (except those inside its first `<legend>`), live as the attribute toggles. They match `:disabled`, can't be focused or activated, and fire no events.
  - Proposal: `disabled_fn: Option<ReactiveBool>`, plus a context the input components read, so styling and handlers follow, as in Chrome.
  - Likelihood: **Low/Medium**.
- **Differs from Chrome** — **fieldset.rs:188-193 (legend is plain text).** The legend is a `String` text node, so it can't hold an icon, badge or a "toggle all" checkbox.
  - *Chrome:* `<legend>` takes arbitrary content, and controls inside the first legend stay enabled when the fieldset is disabled.
  - Proposal: a `legend_node` render fn, or the first child as the legend, keeping the first-legend exemption.
  - Likelihood: **Low/Medium**.

### value_binding.rs, field_ids.rs

No app-facing choices beyond what each component exposes (`id` is already a prop on every field). The generated id prefix `rinch-field-{n}` is fine. *Chrome:* no equivalent.

---

**Differs from Chrome: 30 findings** (27 `**Differs from Chrome**` + 3 `**Differs from Chrome (default only)**`: Textarea resize, Slider value/step defaults, label `nowrap` across Checkbox/Switch/Radio). Most significant:
1. **Slider has no keyboard, focus or ARIA, and `onchange` has `input` semantics.** Chrome's range is in the Tab order with Arrow/PageUp/Home/End keys, and fires `change` once at release.
2. **PasswordInput hard-codes `autocomplete="off"`.** Chrome leaves it unset, and its password manager fills and saves.
3. **NumberInput: arrows don't step on desktop, controlled steppers are inert, no hold-to-repeat.** Chrome always steps and fires input/change. The web and desktop backends behave differently today.
4. **Radios: unnamed radios share one `name="radio-group"`, and there is no group value or arrow-key roving.** In Chrome an unnamed radio is its own group, and arrows move and check within a group.
5. **Select is clipped by `overflow` ancestors and can be covered, wraps arrows, and ignores keys while closed.** Chrome's popup is never clipped, doesn't wrap, and steps the value on arrows/typing while closed (Windows/Linux). Close behind it: Checkbox `indeterminate` half-wired, and checkbox/switch/radio labels can't hold links (Chrome's `<label>` can).


## Part D: Navigation and display

Scope: `crates/rinch-components/src/` tabs, accordion, stepper, pagination, navlink, breadcrumbs, list,
borderless_window, styles/app_menu_bar, button, action_icon, close_button, avatar, image, progress,
loader, skeleton, alert, badge, card, paper, text, title, highlight, mark, kbd, code, blockquote,
divider, anchor, icons, class_utils, layout primitives (briefly), styles/mod.rs.

Paths below are relative to `crates/rinch-components/src/` unless they start with `crates/`.
Checked against props in each struct and `docs/src/guide/component-props.md`.

Several items are **bugs where a prop exists but has no effect**. They are listed because to an app they
look the same as "cannot change it": the knob is there and does nothing.

**Chrome rule (2026-11-04): "sensible defaults means like Chrome would do it".** Each finding now has a
`- *Chrome:*` line. Proposed defaults are Chrome's behaviour where an equivalent exists, and today's where
it does not. `**Differs from Chrome**` marks today's behaviour departing from Chrome's;
`**Differs from Chrome (default only)**` marks a choice an app can already change whose default departs.
Most components here (Tabs, Stepper, Pagination, Badge, Alert, Loader, Skeleton) have **no native
equivalent**, so their defaults stay as they are.

---

### Global (cross-cutting)

- **Differs from Chrome** — **`crates/rinch-theme/src/css.rs:186-194`: reduced motion is a no-op on
  desktop.** The theme emits `@media (prefers-reduced-motion: reduce) { * { animation-duration: 0.01ms
  !important; transition-duration: 0.01ms !important } }` (on by default, `theme.rs:90`), but rinch-dom
  runs Stylo 0.22 in servo mode, whose media-feature table
  (`~/.cargo/registry/.../stylo-0.22.0/servo/media_features.rs`, ending at line 254) has
  `prefers-color-scheme` and no `prefers-reduced-motion`. That feature exists only in
  `gecko/media_features.rs:783`. Nothing in `crates/rinch-dom` or `crates/rinch` reads the OS setting
  either (a grep for `reduced` finds nothing). So every transition and animation in this audit (spinners,
  pulse, stripes, chevrons, collapses) runs at full motion on desktop even when the OS asks for less.
  The earlier note "reduced motion *is* handled globally" was wrong for desktop. It is right on
  rinch-web, where the browser evaluates the query.
  Can the app change it? **No** (the query never matches; an app can only drop animations itself).
  Proposal: read the OS preference (GNOME `org.gnome.desktop.interface enable-animations`, Windows
  `SPI_GETCLIENTAREAANIMATION`, macOS `accessibilityDisplayShouldReduceMotion`) and evaluate
  `prefers-reduced-motion` from it (a Stylo patch or a rinch-side rewrite of that one query), plus
  `App::reduced_motion(Option<bool>)` to force it. Default: follow the OS.
  - *Chrome:* evaluates `prefers-reduced-motion: reduce` from the OS setting. Chrome does not reduce
    motion by itself; a page that writes the query, as the rinch theme does, gets reduced motion.
  Likelihood: **High** (accessibility; affects every animated component).

---

### Accordion (accordion.rs)

- **Differs from Chrome** — **`accordion.rs:137-148` + `accordion.rs:179`: `value`, `default_value` and
  `multiple` are written as `data-active-item` / `data-multiple` on the root, but nothing reads them.**
  Every `AccordionItem` starts with `Signal::new(false)` (line 179), so no item is ever open initially.
  Each item also toggles on its own, so behaviour is always "multiple", whatever `multiple` says.
  Chrome honours `<details open>` at first render; rinch ignores the equivalent prop.
  Can the app change it? **No.** Every one of these props does nothing.
  Proposal: have the items read their open state from the root (context, or a pass after render as
  Stepper does). Add `value_fn: Option<Rc<dyn Fn() -> Vec<String>>>` and
  `onchange: Option<ValueCallback<Vec<String>>>` for controlled mode, and make `multiple: false` close
  the other items. **Default (revised to Chrome): items are independent** (Chrome's `<details>`
  without `name`), so the prop's documented default `multiple: false` (component-props.md:1331)
  should become `true`, or the prop be replaced by `exclusive: bool` (default `false`, Chrome's `name`).
  `default_value` is honoured.
  - *Chrome:* `<details open>` starts open; `<details>` elements are independent unless they share a
    `name` attribute (Chrome 120+), which makes the group exclusive (opening one closes the others).
  Likelihood: **High** (an FAQ that opens the first item, or a single-open accordion, is the common case).
- **Differs from Chrome** — **`accordion.rs:279-289` vs `accordion.rs:196-202`: `AccordionControl.onclick`
  is silently dropped.** The control writes its handler to `data-rid`. `AccordionItem` renders after its
  children and overwrites `data-rid` with the toggle, so the app's callback never fires. Tabs avoids this
  with `data-user-rid` (tabs.rs:265-271); Accordion does not.
  Can the app change it? **No.**
  Proposal: use the Tabs pattern (`data-user-rid`, dispatched after the toggle), and add
  `on_toggle: Option<ValueCallback<bool>>` on `AccordionItem`.
  - *Chrome:* a `click` listener on `<summary>` fires and the toggle still happens (unless it calls
    `preventDefault`); `<details>` then fires a `toggle` event.
  Likelihood: **High** (lazy-loading panel content, analytics).
- **Differs from Chrome** — **`accordion.rs:204-214`: the panel opens and closes with an inline
  `display: none` / `display: ""`.** Inline beats the sheet, so an app cannot animate the open/close
  (height or opacity transitions), and cannot keep a closed panel laid out.
  Can the app change it? **No** (only with `!important`).
  Proposal: use the `hidden` attribute or a `rinch-accordion__panel--open` class, as Tabs does since
  #760 (`[hidden]{display:none}` in the sheet). Default: no animation (Chrome). Optionally add
  `keep_mounted: bool` / `transition_duration`.
  - *Chrome:* hides closed content through the UA rule on `::details-content` (`content-visibility:
    hidden`), which an author rule overrides; since Chrome 131 an author can animate the open/close
    with `::details-content` + `interpolate-size`. No animation by default. Find-in-page auto-opens a
    closed `<details>` that holds a match.
  Likelihood: **Medium**.
- **Differs from Chrome** — **`accordion.rs:179`: open state is internal and per item, with no exposed
  signal and no `opened_fn`.** An app cannot open an item from outside, for example "expand all" or
  deep-linking.
  Can the app change it? **No.** Covered by the controlled-mode proposal above.
  - *Chrome:* `details.open` is a readable and writable property, reflected to the `open` attribute.
  Likelihood: **Medium**.
- **Differs from Chrome (default only)** — **`styles/accordion.rs:101`: the chevron transition is a fixed
  `transform 200ms ease`.**
  Can the app change it? Partially: it is a class rule an app can override. There is no var.
  Proposal: `--rinch-accordion-transition-duration`, **default `0ms`** (Chrome).
  - *Chrome:* the `<summary>` disclosure marker (`::marker`, `disclosure-open`/`disclosure-closed`)
    switches instantly, with no transition.
  Likelihood: **Low**.

### Tabs (tabs.rs)

- **`tabs.rs:218-224`: the active tab is an internal `Signal`.** There is no `onchange` on `Tabs`, no
  `value_fn`, and no exposed signal. `value` is read once at render. To follow a switch, an app must put
  an `onclick` on every `Tab`. To switch programmatically, it must pass `value: {|| sig.get()}`, which
  re-renders the whole Tabs and all its panels.
  Can the app change it? **Partially** (per-tab `onclick`, plus a full re-render).
  Proposal: `onchange: Option<ValueCallback<String>>` on `Tabs`, and `value_fn: Option<Rc<dyn Fn() -> String>>`
  driving `active_tab` through an effect (the `_fn` pattern).
  - *Chrome:* no equivalent (there is no native tabs element).
  Likelihood: **High** (a URL- or state-driven tab is the common app case).
- **`tabs.rs:340-347`: every panel stays mounted and is only hidden.**
  Can the app change it? **No.**
  Proposal: `keep_mounted: bool` (default `true`, today's behaviour). With `false`, a panel is built
  only while it is active.
  - *Chrome:* no equivalent.
  Likelihood: **Medium** (heavy panels, such as editors or `RenderSurface`).
- **`tabs.rs:248-274`: there is no keyboard model**, and no `role="tablist"`/`tab`/`tabpanel` or
  `aria-selected`. Clicks are the only binding, with no arrow, Home or End roving, and no
  `activate_on_focus`. This is mainly a gap rather than a forced choice, but an app cannot add it either,
  because the selection signal is not exposed.
  Proposal: arrow-key roving (default on), `loop_keys: bool`, `activate_tab_with_keyboard: bool`.
  - *Chrome:* no native equivalent. The convention a web app follows is the WAI-ARIA tabs pattern
    (roving tabindex, arrows, Home/End, automatic activation); that is a convention, not Chrome behaviour.
  Likelihood: **Medium**.
- **`styles/tabs.rs:173`: the `pills` active label is `color: white`.**
  Can the app change it? Partially: CSS with a three-class selector. There is no var.
  Proposal: `--rinch-tabs-active-text, white`. See the global contrast finding under styles/mod.rs.
  - *Chrome:* no equivalent for tabs (see the `accent-color` note under styles/mod.rs).
  Likelihood: **Low/Medium** (a light-coloured accent needs dark text).

### NavLink (navlink.rs)

- **Differs from Chrome** — **`navlink.rs:246-251`, `navlink.rs:370-375`: a nested NavLink never
  toggles.** `opened` and `default_opened` only choose the initial class. Clicking the parent does not
  expand it, there is no `opened_fn`, and there is no `onchange`. A collapsible nav section does not work
  unless the app re-renders the whole NavLink from a signal.
  Can the app change it? **Partially** (`opened: {|| sig.get()}` re-renders, and the app has to wire
  the toggle itself through `onclick`).
  Proposal: an internal toggle that uses `default_opened`, plus `opened_fn: Option<ReactiveBool>` and
  `on_opened_change: Option<ValueCallback<bool>>` (the Collapse pattern). Default: clicking toggles.
  - *Chrome:* the nearest equivalent is `<details>`/`<summary>`: clicking (or Enter/Space on) the
    summary toggles it, with a `toggle` event and a script-settable `open`.
  Likelihood: **High**.
- **`navlink.rs:246-269`: with children, the fixed hand-drawn chevron replaces the app's
  `right_section` silently.**
  Can the app change it? **No.**
  Proposal: `chevron_icon: Option<TablerIcon>` and `disable_right_section_rotation: bool`, and keep
  `right_section` when it is set.
  - *Chrome:* no direct equivalent (a `<summary>` marker is restyleable through `::marker` or
    `list-style`, so an author can replace it).
  Likelihood: **Medium**.
- **Differs from Chrome** — **`styles/navlink.rs:135-137`: the collapse is `max-height: 1000px` with a
  fixed `200ms` transition.** Children taller than 1000px are clipped.
  Can the app change it? Partially (class CSS).
  Proposal: `--rinch-navlink-children-max-height` and `--rinch-navlink-transition-duration`, or the same
  mechanism Collapse uses. Default: no clip, and no transition (Chrome's `<details>`).
  - *Chrome:* an open `<details>` is as tall as its content and opens instantly.
  Likelihood: **Medium** (long nested lists get clipped).
- **Differs from Chrome** — **`navlink.rs:314-320`: a disabled `<a>` NavLink still fires `onclick`.**
  The handler is registered whatever `disabled` is (the button branch at 350-361 at least sets
  `disabled`). The CSS `pointer-events: none` covers mouse input but not keyboard activation.
  Can the app change it? **No**.
  Proposal: skip registration when disabled (bug fix), and set `aria-disabled="true"`.
  - *Chrome:* `<a>` has no `disabled` attribute; with `pointer-events: none` a link still activates from
    Enter, exactly as here. But `<button disabled>` (the NavLink's other branch) gets no click from mouse
    or keyboard, so the same prop behaves differently in the two branches; `disabled` should mean what
    it means on `<button>`.
  Likelihood: **Medium**.
- **`navlink.rs:225-243`: `label` and `description` are `String` only**, with no node slot for a
  badge or a count.
  Can the app change it? Partially (children are taken as nested links, so they cannot be used).
  Proposal: `label_fn: Option<SectionRenderer>` (the BorderlessWindow `SectionRenderer` type).
  - *Chrome:* no equivalent.
  Likelihood: **Medium** (unread counts in a sidebar).

### Breadcrumbs (breadcrumbs.rs)

- **`styles/breadcrumbs.rs:34-35`: any `separator` outside the five whitelisted ones renders as `/`.**
  `.rinch-breadcrumbs__list[data-separator] … ::after { content: '/' }` outranks the generic
  `attr(data-separator)` rule (`styles/breadcrumbs.rs:28-31`), and only `/ > >> - •` have a rule of
  their own. So `separator: "›"` or `"|"` shows `/`.
  `breadcrumbs.rs:48` also HTML-escapes into an attribute, so `&` would show as `&amp;`.
  Can the app change it? **No** (the prop is effectively a fixed list).
  Proposal: drop the `[data-separator]` fallback rule and the escaping. Better: render the separator as
  a real `<span class="rinch-breadcrumbs__separator">` element, which also allows
  `separator_icon: Option<TablerIcon>`.
  - *Chrome:* no equivalent (a breadcrumb is author markup).
  Likelihood: **High** (`›` and chevrons are the common choices).
- **`breadcrumbs.rs:38`: `aria-label="Breadcrumb"` is fixed English.**
  Can the app change it? **No.**
  Proposal: `aria_label: String`, defaulting to `"Breadcrumb"`.
  - *Chrome:* no equivalent (Chrome localizes its own UA strings, such as `<details>`' default
    "Details" summary, to the browser language; it has nothing for author ARIA).
  Likelihood: **Medium** (i18n).
- **`breadcrumbs.rs:70-83`: an item with no `href` is always the "active / current" item.** An item
  cannot have an `onclick` (in-app routing in a desktop app), and there is no `aria-current`.
  Can the app change it? **No.**
  Proposal: `BreadcrumbsItem { onclick: Option<Callback>, active: Option<bool> }`.
  - *Chrome:* an `<a>` without `href` is a placeholder link: not focusable, not styled as a link, but a
    `click` listener on it still fires. Rendering a `<span>` for it is close to that; the gap is only the
    missing `onclick`.
  Likelihood: **High** (desktop apps route with callbacks, not hrefs).

### Pagination (pagination.rs)

- **`pagination.rs:242-260, 272-291, 346-365, 377-396`: the prev/next/first/last buttons have no
  `aria-label` at all.** The icons are fixed (`icons::chevron*_dom`), and the dots at line 330 are a
  literal `"..."`.
  Can the app change it? **No.**
  Proposal: `labels: PaginationLabels { first, prev, next, last }` (i18n strings),
  `prev_icon`/`next_icon`/`first_icon`/`last_icon: Option<TablerIcon>`, `dots: String`.
  - *Chrome:* no equivalent.
  Likelihood: **Medium**.
- **`pagination.rs:298-334`: page items render only their number.** There is no custom item render,
  and no `aria-current="page"`.
  Proposal: `get_item_props` / `render_item: Option<Rc<dyn Fn(&mut RenderScope, u32, bool) -> NodeHandle>>`.
  - *Chrome:* no equivalent.
  Likelihood: **Low**.
- **`styles/pagination.rs:47`: the active page is `color: white`.**
  Can the app change it? Partially (class CSS).
  Proposal: `--rinch-pagination-active-text`.
  - *Chrome:* no equivalent.
  Likelihood: **Low**.

### Stepper (stepper.rs)

Stepper is the most configurable component here: `on_step_click`, the icon defaults, the per-step
`state`, and `disabled`. It has no Chrome equivalent, so none of its defaults change.

- **`styles/stepper.rs:127-129`: `.rinch-stepper__step-content { display: none }` with no rule that
  ever shows it.** The children of a `StepperStep` (`stepper.rs:1392-1398`) are never visible, in any
  state. Mantine shows the active step's content.
  Can the app change it? **Partially**: an app rule
  `.rinch-stepper__step--progress .rinch-stepper__step-content { display: block }` works, but the
  content then sits inside the step row's layout and not below the stepper.
  Proposal: show the progress step's content in a `rinch-stepper__content` region below the steps, or
  document the children as unused. Add `content_for_active_only: bool` (default `true`).
  - *Chrome:* no equivalent.
  Likelihood: **High** (a wizard needs its step content).
- **`stepper.rs:1416-1428`: `StepperCompleted` always renders.** It is not tied to
  `active >= step count`, so the app must wrap it in an `if`.
  Can the app change it? **Partially** (an rsx `if`).
  Proposal: have Stepper hide it unless `active` ≥ number of steps (the Mantine behaviour).
  - *Chrome:* no equivalent.
  Likelihood: **Medium**.
- **`stepper.rs:1376-1389`: `label` and `description` are `String` only, and icons are `TablerIcon`
  only (`stepper.rs:1314-1334`).** There is no node slot for a custom icon, such as an avatar or a
  number style.
  Proposal: `icon_fn` / `label_fn: Option<SectionRenderer>`.
  - *Chrome:* no equivalent.
  Likelihood: **Low/Medium**.
- **`styles/stepper.rs:84,90`: the completed and progress icon text is `color: white`.** The progress
  ring is a fixed `4px` at `20%` (`styles/stepper.rs:91`).
  Can the app change it? Partially (class CSS).
  Proposal: `--rinch-stepper-icon-text`, `--rinch-stepper-ring`.
  - *Chrome:* no equivalent.
  Likelihood: **Low**.

### BorderlessWindow (borderless_window.rs) + styles/app_menu_bar.rs

The Chrome equivalent here is Chrome's own client-side-decorated window (its tab strip titlebar),
which follows the platform: on Windows minimize, maximize/restore, close on the right; on macOS the
traffic lights (close, minimize, zoom) on the left; on Linux with GTK it reads the desktop's
`gtk-decoration-layout` / GNOME `button-layout` (GNOME's own default is `appmenu:close`, close only;
Ubuntu ships `appmenu:minimize,maximize,close`).

- **Differs from Chrome** — **`borderless_window.rs:414-436`: in `InlineTitlebar` menu mode the title
  is drawn as a hard-coded rainbow.** Each character gets an inline `style="color: #ff6b6b"` and so on
  from a fixed six-colour palette (`borderless_window.rs:417-419`). Inline styles mean no stylesheet can
  override it. It looks like demo branding that leaked into the framework.
  Can the app change it? **No** (only with `!important` on every span).
  Proposal: render the title plainly in `.rinch-borderlesswindow__brand`. Keep the rainbow only behind
  `brand_renderer: Option<SectionRenderer>` (or in ui-zoo). Default: plain text.
  - *Chrome:* a CSD titlebar (Chrome's, or a GTK headerbar) draws the title as plain text in the
    titlebar colour.
  Likelihood: **High** (every real app using the inline menu bar).
- **Differs from Chrome** — **`borderless_window.rs:269-369`: the window controls are fixed.** The order
  is minimize, maximize, close, on the right (Windows layout), on every platform. The glyphs are
  hand-drawn SVG paths with no override, and the aria labels `"Minimize"`, `"Maximize"`, `"Close"`
  (lines 275, 308, 345) are fixed English. Maximize does not switch to a "restore" glyph or label.
  Can the app change it? **Partially**: `show_*` can hide a control, and `left_section`/`right_section`
  can render replacements. Placement and labels cannot change.
  Proposal: `controls_layout: Option<WindowControlsLayout>` whose **default (`None`) follows the
  platform** (Windows order on Windows; traffic-light order on the left on macOS; the GTK/GNOME
  `button-layout` setting on Linux, falling back to the Windows order when it cannot be read), with an
  explicit `left`/`right` + order override; `control_labels` (i18n); a `render_controls:
  Option<SectionRenderer>` escape hatch; and a restore glyph and label while maximized
  (`maximized_fn`, or the runtime's maximized state).
  - *Chrome:* follows the platform layout as above, and on Windows swaps maximize to a restore glyph
    while maximized.
  Likelihood: **Medium** (any app shipped on macOS, or to GNOME users with a custom layout).
- **Differs from Chrome** — **new: double-clicking the titlebar does nothing.** `borderless_window.rs:218`
  marks the titlebar `data-drag-window`, and neither this file nor the `data-drag-window` handling in
  `crates/rinch/src/app/click_handling.rs` maximizes on a double click (`toggle_maximize_current_window`
  exists, `crates/rinch/src/windows.rs:391`, but nothing calls it from the titlebar).
  Can the app change it? Partially (an app can put its own `ondblclick`-style handling in a section,
  not on the drag region itself).
  Proposal: double-click toggles maximize by default, following the platform action where it can be
  read (GNOME `action-double-click-titlebar`, macOS "double-click a window's title bar to"), with
  `on_titlebar_double_click: Option<Callback>` / `titlebar_double_click: TitlebarAction` to change it.
  - *Chrome:* double-clicking its titlebar maximizes/restores on Windows and Linux (following the
    desktop's setting where it has one) and zooms or minimizes on macOS per the system preference.
  Likelihood: **Medium** (users expect it).
- **`borderless_window.rs:34`, `styles/borderless_window.rs:48-49,146`: the titlebar height is a fixed
  36px.** It is duplicated between the Rust constant (used for `--rinch-window-top-inset` at
  `borderless_window.rs:198-214`) and the CSS. `MENU_BAR_HEIGHT = 28`
  (`crates/rinch/src/menu/app_menu_bar.rs:30`) is pinned the same way through `line-height: 19px`
  (`styles/app_menu_bar.rs:31-34`). An app that restyles the titlebar or menu bar height desyncs the
  inset, and content slides under the bar.
  Can the app change it? **No** (CSS can change the box, but not the inset the Rust code computes).
  Proposal: `titlebar_height: u32` prop (default 36), written as `--rinch-titlebar-height` and used by
  both the CSS and the inset. Do the same for the menu bar (`MenuBarContext.bar_height` already exists;
  make it configurable on the builder).
  - *Chrome:* unsure of exact heights; Chrome's titlebar height is its own per-platform constant, not
    configurable by a page. Keep 36.
  Likelihood: **Medium** (compact or macOS-style chrome).
- **`borderless_window.rs:218`: the whole titlebar is always `data-drag-window="true"`.**
  Can the app change it? Partially (a section's own elements can presumably opt out with interactive
  children). There is no prop.
  Proposal: `draggable: bool` (default true).
  - *Chrome:* its whole titlebar outside the tabs and buttons drags the window; same as today.
  Likelihood: **Low**.
- **`borderless_window.rs:375-381`: content gets an inline `padding-top: {bar_height}px`.**
  The app cannot override it from a stylesheet.
  Proposal: write it as a var (`--rinch-menu-bar-height`) that a class rule consumes.
  - *Chrome:* no equivalent.
  Likelihood: **Low/Medium** (a full-bleed content area under a translucent bar).
- **`styles/app_menu_bar.rs:51,57,59,199,203,217,223`: fixed menu min-widths (220px and 200px), shadow
  `rgba(0,0,0,0.15)` and `z-index: 200`.**
  Can the app change it? Partially (class CSS; the shadow is not the theme's `--rinch-shadow-*`).
  Proposal: use `var(--rinch-shadow-md)` and `--rinch-menu-min-width`.
  - *Chrome:* no web equivalent; Chrome's own menus use platform menu styling.
  Likelihood: **Low**.

### Button (button.rs)

- **`button.rs:143-146` + `styles/button.rs:148-172`: `loading: true` only adds a class.** No
  `.rinch-button__label` or `.rinch-button__loader` element is ever created (confirmed by
  `crates/rinch/src/app/decorative_pseudo_773_tests.rs:331`). A loading button shows no spinner and
  stays fully visible; it only becomes unclickable.
  Can the app change it? **No.**
  Proposal: render the label and loader elements (a fix), plus
  `loader: Option<SectionRenderer>` / `loader_type: String` (re-using `Loader`).
  - *Chrome:* no equivalent.
  Likelihood: **High** (async submit buttons).
- **`button.rs:177-200`: no `type` attribute is set.** Inside a `<form>` that means `submit`, and there
  is no prop to change it. ActionIcon and CloseButton set `type="button"`.
  Can the app change it? Partially (a root attribute `type: "button"`, if the rsx root-attribute
  passthrough covers it).
  Proposal (revised to Chrome): `r#type: String`, **defaulting to `"submit"`**, Chrome's default and
  what the button already is on rinch-web; the prop lets an app say `"button"` or `"reset"`. On desktop
  the form submit path is Enter in a field firing the nearest `data-onsubmit`
  (`crates/rinch/src/app/mod.rs:3951-3973`); whether a click on a submit button submits on desktop:
  unsure, not traced.
  - *Chrome:* `<button>` with no `type` (or an invalid one) is `type=submit`; inside a form a click or
    Enter/Space submits it.
  Likelihood: **Medium**.
- **`button.rs:160-173`: `color` accepts only theme palette names.** A hex or rgb value produces
  `var(--rinch-color-#ff0000-6)`, which is invalid. ActionIcon, Avatar, Tabs and others accept a raw
  colour.
  Can the app change it? Partially (set `--rinch-button-color*` through `style:`).
  Proposal: the same `#`/`rgb`/`hsl` branch the other components use.
  - *Chrome:* no equivalent.
  Likelihood: **Medium**.
- **`button.rs:93-110`: no `left_section`/`right_section`.**
  Can the app change it? Partially (put the icon in children).
  Proposal: `left_section`/`right_section: Option<TablerIcon>`, as Tab and NavLink have.
  - *Chrome:* no equivalent.
  Likelihood: **Low/Medium**.
- **Differs from Chrome (default only)** — **`styles/button.rs:17-18`: a fixed `:active { transform:
  translateY(1px) }` press nudge**, and the same in `styles/action_icon.rs:46…105`.
  Can the app change it? Partially (class CSS).
  Proposal: `--rinch-button-active-offset`, **default `0px`** (Chrome).
  - *Chrome:* a pressed `<button>` changes colour (`:active` UA styling / the platform theme) but does
    not move.
  Likelihood: **Low**.

### ActionIcon (action_icon.rs)

- **`action_icon.rs:180-240`: no `aria-label` prop.** Only `title` exists, which becomes a tooltip.
  Can the app change it? Partially (a root attribute, if passthrough works; `title` itself names it, below).
  Proposal: `aria_label: String`.
  - *Chrome:* accessible-name computation falls back to `title` when nothing else names the button, so
    a titled icon button does have a name; an `aria-label` replaces it without showing a tooltip.
    Whether rinch's AccessKit tree uses `title` the same way: unsure.
  Likelihood: **Medium**.
- **Differs from Chrome** — **`action_icon.rs:184` and `close_button.rs:158`: `type="button"` is fixed.**
  There is no `type` prop, so neither can be a form's submit button.
  Can the app change it? **No** (the component writes the attribute itself).
  Proposal: `r#type: String`. The Chrome rule would make the default `"submit"`; for an icon or close
  button inside a form that is almost never wanted, so this is the one place I would keep `"button"` as
  the default and say so in the docs.
  - *Chrome:* `<button>` defaults to `type=submit`.
  Likelihood: **Low**.
- **`action_icon.rs:221-232`: the icon size comes from a fixed table** (`ActionIconSize::icon_size`),
  with a fixed outline style.
  Can the app change it? Partially (pass children instead of `icon`).
  Proposal: `icon_size: Option<u32>`, `icon_style: TablerIconStyle`.
  - *Chrome:* no equivalent.
  Likelihood: **Low**.

### CloseButton (close_button.rs) and the close buttons in Alert

- **`close_button.rs:161`, `alert.rs:258`: `aria-label="Close"` is fixed English.**
  Can the app change it? **No.**
  Proposal: `aria_label: String`, defaulting to `"Close"`, on both.
  - *Chrome:* no equivalent (Chrome localizes only its own UA strings).
  Likelihood: **Medium** (i18n).
- **`close_button.rs:151-156`: the glyph is fixed, `children` is ignored, and its size is written as
  an inline `style`, so a stylesheet cannot resize it.** `icon_size` exists as a prop.
  Can the app change it? Partially (`icon_size`).
  Proposal: `icon: Option<TablerIcon>`, and honour children.
  - *Chrome:* no equivalent.
  Likelihood: **Low**.

### Alert (alert.rs)

- **`alert.rs:194-199` (`enum AlertColor`, `alert.rs:47`): `color` is limited to blue, green, yellow, red,
  gray and primary.** Any other theme colour (violet, teal, …) or a hex value silently falls back to
  the default.
  Can the app change it? **No** (beyond class CSS).
  Proposal: accept any palette name or raw colour through `--rinch-alert-color` (the pattern Blockquote
  uses).
  - *Chrome:* no equivalent.
  Likelihood: **Medium**.
- **`alert.rs:238-243`: `title` is `String` only.** There is no `role="alert"`, and no node slot for
  the title.
  Proposal: `role: String` (default `"alert"`), `title_fn: Option<SectionRenderer>`.
  - *Chrome:* no equivalent.
  Likelihood: **Low/Medium**.
- **`styles/alert.rs:70,116-120`: hard-coded `rgba(0,0,0,0.05)` close-button hover, and `white` text
  on the filled variant.**
  Can the app change it? Partially (class CSS).
  - *Chrome:* no equivalent.
  Likelihood: **Low**.

### Badge (badge.rs)

- **`badge.rs:142-144` + `styles/badge.rs:89-102…`: colour works only through `[data-color="<name>"]`
  rules for the 14 built-in palette names.** A hex value or a custom theme colour does nothing, and
  there is no `--rinch-badge-color` var. `lime` and `yellow` are forced to `#000` text, and the filled
  variant to `white` text (`styles/badge.rs:50`).
  Can the app change it? **Partially** (an app rule per colour).
  Proposal: `--rinch-badge-color` / `--rinch-badge-text`, set from `color` like the other components.
  - *Chrome:* no equivalent.
  Likelihood: **Medium**.
- **`badge.rs:89-100`: no `left_section`/`right_section`** (a status dot, or a remove "x").
  - *Chrome:* no equivalent.
  Likelihood: **Low/Medium**.

### Avatar (avatar.rs)

- **Differs from Chrome** — **`avatar.rs:203-210`: an image that fails to load gives the app no
  signal, and does not fall back to initials or the placeholder.** `src` simply wins. There is no
  `on_error`.
  Can the app change it? **No.**
  Proposal: `on_error: Option<Callback>` (Chrome's `error` event), driven by an image load-error hook
  (the same gap as Image below). Falling back to initials has no Chrome equivalent, so it stays off by
  default: `fallback_on_error: bool` (default `false`, today's behaviour).
  - *Chrome:* a failed `<img>` fires `error` and renders its `alt` text (with a broken-image icon when
    it has a size and no alt fits). rinch-dom reserves the attributes' box for a failed image with `alt`
    (`crates/rinch-dom/src/replaced.rs:72-73`); whether desktop paints the alt text: unsure.
  Likelihood: **Medium**.
- **`avatar.rs:172-185`: the initials rule is fixed** (first letter of the first two words,
  uppercased), and `children` is ignored, so there is no custom placeholder content.
  `.rinch-avatar__placeholder-icon` (`avatar.rs:216`) is an empty span with no glyph and no CSS rule.
  Can the app change it? **No.**
  Proposal: `initials: String` override, honour children as the placeholder, `placeholder_icon:
  Option<TablerIcon>`.
  - *Chrome:* no equivalent.
  Likelihood: **Medium**.
- **`styles/avatar.rs:29`: filled text is `white`.** Low. *Chrome:* no equivalent.

### Image (image.rs)

- **Differs from Chrome** — **`image.rs:155-158`: `fallback_src` is implemented as a JavaScript
  `onerror="this.src='…'"` string attribute.** It has no effect on desktop: nothing in rinch-dom or the
  shell reads `onerror`. There is no `on_error` callback either.
  Can the app change it? **No** (on desktop the prop is inert).
  Proposal: a real load-failure path, `on_error: Option<Callback>`, and `fallback: Option<SectionRenderer>`.
  Default: with no fallback, show the `alt` text as Chrome does.
  - *Chrome:* runs the inline `onerror` handler (so the prop works on rinch-web), fires `error`, and
    without a handler renders the `alt` text.
  Likelihood: **Medium/High** (remote images fail).
- **Differs from Chrome (default only)** — **`image.rs:9-12, 132-142`: the default `fit` is
  `ImageFit::Cover`, written as an inline `object-fit: cover`**, so a stylesheet cannot set it either.
  The `fit` prop does cover it.
  Proposal: default `fit` to `Fill` (Chrome's initial `object-fit`), and write it as a class or var
  rather than inline. An image given both a width and a height then stretches, as in Chrome.
  - *Chrome:* `object-fit` initial value is `fill`.
  Likelihood: **Low/Medium** (the change is visible on any sized image that relies on the default).

### Progress (progress.rs)

- **Differs from Chrome** — **`progress.rs:216-219`: `aria-valuenow` is written once.** The `value_fn`
  effect (`progress.rs:234-242`) updates the width but not `aria-valuenow`. The 0–100 range is fixed
  (no `max`). There is no `aria-label` or label prop.
  Can the app change it? **No.**
  Proposal: update `aria-valuenow` in the effect (a fix); add `max: f32`, `label: String`,
  `aria_label: String`. Chrome's default `max` is `1.0`, but defaulting to it would silently rescale
  every existing `value`; keep `100` and document it (a deliberate departure).
  - *Chrome:* `<progress>` exposes its current `value`/`max` to assistive technology on every change;
    `max` defaults to `1.0`.
  Likelihood: **Medium**.
- **Differs from Chrome** — **`progress.rs:194-200`: `value: None` renders 0%; there is no
  indeterminate mode.** `value` is already `Option<f32>` (`progress.rs:120`), but `None` becomes
  `unwrap_or(0.0)`.
  Can the app change it? **No.**
  Proposal: `value: None` (and no `value_fn`) renders an indeterminate bar (an animated class,
  `--rinch-progress-indeterminate-*` vars) without `aria-valuenow`; or `indeterminate: bool`.
  - *Chrome:* `<progress>` without a `value` attribute is indeterminate (animated, matches
    `:indeterminate`).
  Likelihood: **Medium** (unknown-length work is common).
- **Differs from Chrome (default only)** — **`styles/progress.rs:23,30-34,43`: fixed `width 200ms`
  transition, stripe colour `rgba(255,255,255,0.15)`, and a `1s` stripe animation.**
  Can the app change it? Partially (class CSS).
  Proposal: `--rinch-progress-transition`, **default none** (Chrome); the stripes are opt-in props and
  have no Chrome equivalent, so they keep their look.
  - *Chrome:* a `<progress>` value change jumps, with no transition. There is no `<meter>`-like
    component in rinch to compare (`<meter>`'s low/high/optimum colouring has no counterpart here).
  Likelihood: **Low/Medium**.

### Loader (loader.rs)

- **`loader.rs:127`: `aria-label="Loading"` is fixed English.**
  Can the app change it? **No.**
  Proposal: `aria_label: String`.
  - *Chrome:* no equivalent.
  Likelihood: **Medium** (i18n).
- **`loader.rs:141-160`: the loader kinds are fixed to oval, bars and dots.** There is no app-wide
  default type and no custom loader. Button, ActionIcon and Stepper each draw their own spinner and
  ignore `Loader`.
  Proposal: a theme-level default loader type and `Loader { custom: Option<SectionRenderer> }`, re-used
  by Button, ActionIcon and Stepper loading.
  - *Chrome:* no equivalent (the nearest is an indeterminate `<progress>`).
  Likelihood: **Low/Medium**.

### Skeleton (skeleton.rs)

- **`skeleton.rs:105-111`: the loaded state wraps children in a `<span>`.** Block children end up
  inside an inline wrapper, and the app cannot choose the wrapper.
  Proposal: `display: contents` on the wrapper, or a `wrapper: String` tag.
  - *Chrome:* no equivalent.
  Likelihood: **Low**.
- **`styles/skeleton.rs:19`: fixed `1.5s` pulse.** Low. *Chrome:* no equivalent (and see the global
  reduced-motion finding: on desktop the pulse never stops for reduced motion).

### Anchor (anchor.rs)

- **Differs from Chrome** — **`anchor.rs:44-63`: there is no `onclick`, and an empty `href` becomes
  `"#"`.** A desktop app cannot route in-app from a link styled as an Anchor. `target` is passed through
  with no `rel`.
  Can the app change it? **No.**
  Proposal: `onclick: Option<Callback>` (when set, no default navigation), and `rel: String`. With no
  `href`, write no `href` (Chrome's placeholder link), not `"#"`.
  - *Chrome:* an `<a>` with no `href` is a placeholder: not focusable, not a link, no navigation, but
    click listeners fire. `href="#"` instead scrolls to the top and adds a history entry. A click
    listener can `preventDefault()` the navigation. `target="_blank"` implies `rel=noopener` (Chrome
    88+), so `rel` is only needed for `noreferrer` and the like.
  Likelihood: **High** (in-app links in desktop apps).

### Text / Title / Highlight / Blockquote / Divider (display text)

- **`text.rs:168-188`: `color` takes only 17 named values.** A hex value or a custom theme colour is
  silently ignored. `styles/text.rs:63-75` define `--truncate` and `--line-clamp` classes, but `Text`
  has no prop for either.
  Can the app change it? Partially (add the class through a root `class:`; colour through `style:`).
  Proposal: a raw-colour branch through `--rinch-text-color`; `truncate: bool`, `line_clamp: Option<u32>`.
  - *Chrome:* no equivalent.
  Likelihood: **Medium**.
- **`title.rs:26-30`: `size` must be `"1"`–`"6"`.** Any other value falls back to `order`.
  Low. *Chrome:* no equivalent.
- **`highlight.rs:23-29`: only one highlight term.** There is no `Vec<String>`, no class or render
  for the match, and the default `ignore_case: true` uses `to_lowercase` byte offsets
  (`highlight.rs:82-91`). That is a separate bug: a string whose lowercase form changes byte length
  (for example `İ`) gives wrong or panicking slices.
  Proposal: `highlight: Vec<String>`, `match_class: String`.
  - *Chrome:* no component equivalent; Chrome's find-in-page is case-insensitive by default, matching
    `ignore_case: true`.
  Likelihood: **Low/Medium**.
- **`blockquote.rs:72`: the body is a `<p>`**, so block children are invalid HTML on web. `cite` is
  String only.
  - *Chrome:* the HTML parser would close the `<p>` before a block child, but a tree built through the
    DOM API, as rinch-web builds it, keeps the nesting and renders it. No behavioural equivalent.
  Likelihood: **Low**.
- **`divider.rs:79-81`: `label` is String only** (no icon or node). Low. *Chrome:* no equivalent
  (`<hr>` has no label).

### List (list.rs)

- **`list.rs:178-194`: the default icon is a `TablerIcon` only**, always outline style, and `ListItem`
  is the same (`list.rs:327`). There is no custom marker node.
  - *Chrome:* a list marker is replaceable with `list-style-image` or `::marker { content: … }`; the
    default is a disc. No change to the default.
  Likelihood: **Low**.

### Card / Paper / Mark / Kbd / Code / layout primitives

Nothing material. Props map to classes and vars.
- `simple_grid.rs:92`: the grid template and gap are always written **inline**, including the
  defaults. An app stylesheet (for example a breakpoint `@media` rule) can never change the column
  count, and there is no breakpoint prop.
  Proposal: emit vars (`--rinch-simple-grid-cols`, `--rinch-simple-grid-gap`) consumed by the class
  rule, plus an optional `breakpoints`.
  - *Chrome:* no equivalent.
  Likelihood: **Medium** (resizable desktop windows).

### icons.rs / class_utils.rs (cross-cutting)

- **`icons.rs:157-272`: the built-in chrome glyphs (close X, chevrons, check) are fixed SVGs.** They
  are used by Accordion, Pagination, CloseButton, Alert and Stepper. There is no app-wide icon override
  (for example a context naming the TablerIcon to use for "close", "chevron" and "check").
  Proposal: an `IconSet` context with defaults, read by the `*_dom` helpers.
  - *Chrome:* no equivalent (its own control glyphs are not author-replaceable, apart from
    `<summary>`'s `::marker`).
  Likelihood: **Medium** (apps with their own icon language).
- `class_utils.rs:51`: `radius_class` only knows `xs`–`xl`. A raw value (`"10px"`) is dropped
  silently. Low. *Chrome:* no equivalent.

### styles/mod.rs (global)

- **Differs from Chrome** — **`styles/mod.rs` (`generate_all_component_styles`): one monolithic sheet,
  always injected** (`crates/rinch/src/lib.rs:531-532`, concatenated with the theme). There is no
  per-component opt-out, no "unstyled" mode, and no `@layer`. Several active-state rules need
  three-class selectors to beat (`.rinch-tabs--pills > .rinch-tabs__list > .rinch-tabs__tab--active`).
  Real `!important` is used only in modal, notification and drawer visibility (`styles/modal.rs:27`,
  `styles/notification.rs:22`, `styles/drawer.rs:59`), outside this audit's components.
  Can the app change it? Partially (out-specify each rule).
  Proposal: wrap the component sheet in `@layer rinch-components` (whether rinch's Stylo build has
  cascade layers enabled: unsure, to check), so any unlayered app rule wins, or add
  `App::component_styles(false)` / per-component exclusion. Default: app rules win.
  - *Chrome:* the built-in look of native controls comes from the UA stylesheet, which loses to **any**
    author rule regardless of specificity. A framework's defaults behaving the same way is the Chrome
    model; today they behave like author CSS of high specificity.
  Likelihood: **Medium**.
- **Differs from Chrome** — **The `color: white` contrast text is hard-coded in 11 places** (tabs 173,
  pagination 47, action_icon 39, stepper 84/90, avatar 29, navlink 33/53, alert 116-119, badge 50, button
  65). None is behind a var. A light accent colour (yellow, lime) then gives unreadable filled
  components; Badge alone special-cases yellow and lime.
  Proposal: a theme var `--rinch-primary-contrast` (and `--rinch-color-<name>-contrast`), used
  everywhere, **computed by default from the colour's contrast** (white or black, whichever reads),
  with the app able to set it.
  - *Chrome:* with `accent-color`, Chrome picks the checkmark/thumb foreground automatically (light or
    dark) for contrast against the accent.
  Likelihood: **Medium**.
- **Differs from Chrome (default only)** — **Transition and animation durations (150ms, 200ms, 0.6s,
  0.8s, 1s, 1.5s) are literal in every sheet.** Proposal: `--rinch-transition-duration` in the theme.
  Where a native equivalent exists (the accordion chevron, the NavLink collapse, the progress width,
  the button nudge, all above) Chrome has no transition, so those defaults become `0`; spinners,
  skeleton pulse and stripes have no equivalent and keep theirs. Reduced motion: see the Global
  finding (a no-op on desktop today).
  - *Chrome:* native controls (`<details>`, `<progress>`, `<button>`) change state without transitions.
  Likelihood: **Low**.

---

### Summary

Findings marked **Differs from Chrome**: 24 (19 `Differs from Chrome`, 5 `Differs from Chrome (default only)`).

Most significant five:
1. **Reduced motion is a no-op on desktop** — the theme's `prefers-reduced-motion` block can never match
   in servo-mode Stylo 0.22 and nothing reads the OS setting (Global).
2. **Accordion ignores `default_value`/`value`/`multiple`, drops `AccordionControl.onclick`, and exposes
   no open state** — `<details open>`, `name`, `toggle` and `details.open` all work in Chrome.
3. **Window controls are Windows-ordered everywhere, with no restore glyph and no double-click to
   maximize** — Chrome follows the platform's button layout and titlebar action.
4. **Anchor writes `href="#"` and has no `onclick`; Image's `fallback_src` is inert on desktop** — Chrome
   has placeholder links, click listeners, `onerror`, and alt-text rendering.
5. **Progress: `None` renders 0% instead of indeterminate, and `aria-valuenow` goes stale** — Chrome's
   `<progress>` without `value` is indeterminate and always reports its current value.


## Part E: Rich-text editor view

Scope: `crates/rinch-editor-view/src` (component, view, keys, blink, styles, links,
images, margins, node_views, handle, collab) plus the editor-core defaults
(`default_plugins`, keymap, input rules, history). Checked against
`docs/src/guide/contenteditable.md`, `docs/internals/editor.md`,
`docs/src/guide/focus.md`. Read-only; nothing built.

**Chrome rule (2026-11-04, "sensible defaults means 'like Chrome would do it'").**
The Chrome reference throughout is a plain `contenteditable` element (with
`<textarea>` where noted). Every finding now has a `*Chrome:*` line. Proposed
defaults are Chrome's where Chrome has an equivalent, and today's where it has
none. **Differs from Chrome** marks a gap between today's behaviour and Chrome's.
Where I am not sure what Chrome does, the line says "Chrome: unsure".

**Cross-cutting note.** `view.rs` is renderer-agnostic, so every inline-style
finding below applies to desktop **and** rinch-web. An inline `set_style(s)`
declaration beats any class or attribute rule an app writes. Per the CSS
cascade, an author `!important` rule (e.g.
`[data-pm-caret] { background-color: red !important }`) should still beat a
non-important inline declaration, but that is undocumented for the editor and
unverified on rinch-dom. Where it applies, the finding says "partially". The
cheapest fix throughout is the same: keep the inline declaration but write it
as `var(--rinch-editor-…, <default>)`. An app can then set the variable on
`[data-pm-editor]`.

**Already configurable (not reported):** link click and hover (`on_link_click`,
`on_link_hover`, web navigation table; the editable default of never
navigating matches Chrome, where a click in a `contenteditable` link places the
caret); image input, hover and source (`on_image_input`, `on_image_hover`,
`set_image_source`); paste rewriting (`Plugin::handle_paste`); key pre-emption
(`on_key`); read-only (`set_read_only`); overlay slot and margin anchors;
explicit scroll (`scroll_into_view_aligned`); the right-click menu
(`data-oncontextmenu` replaces it); node views; and decorations (app CSS
class). There are no image resize handles or collaborator cursors built in.
That matches Chrome, which shows no resize handles in a `contenteditable`. Both
are left to the app, which has the seams it needs (`on_image_hover` + overlay;
`collab_sticky_*` + `caret_rect` + overlay slot), so neither is an imposed choice.

---

### Editor construction / default plugin set (rinch-editor-view `lib.rs`, rinch-editor-core `lib.rs`)

- **Differs from Chrome:** Chrome never autoformats. **`crates/rinch-editor-core/src/lib.rs:79` (via `crates/rinch-editor-view/src/lib.rs:82-84`)** —
  Markdown input rules (`MarkdownInputRulesPlugin`) are always on. `create_editor()`
  always installs `default_plugins()` = BaseCommands + MarkdownInputRules + History,
  over `Schema::starter_kit()`. Typing `- `, `1. `, `# `, `> `, `*x*`, `` `x` ``,
  `==x==`, or `---` rewrites the text.
  - *Chrome:* a plain `contenteditable` does no autoformatting. What you type
    is inserted literally. (The OS can still apply smart quotes or
    autocorrect, e.g. on macOS, but that is not Markdown.)
  - Can the app change it? **No.** There is no `remove_plugin`, `add_plugin` can
    only append, and input rules are additive across plugins. `on_key` cannot
    stop them, because rules fire on inserted text and with an IME the text
    arrives as a commit, not as keys. `EditorHandle::new(container, doc_ref, schema, doc, plugins)`
    is `pub`, but it needs a container and document up front and is documented as
    the eager/test path. It does not register with the runtime. The guide's tip
    to "append … to `markdown_input_rules()`" (contenteditable.md §Markdown
    shortcuts) can't work, because that function returns a fresh `Vec` that the
    installed plugin never reads. I also found no Backspace-undoes-the-rule
    command (ProseMirror's `undoInputRule`).
  - Proposed: `create_editor_with(EditorConfig { schema: Rc<Schema>, plugins: Vec<Rc<dyn Plugin>> })`
    plus `Editor { markdown_shortcuts: bool }`, or
    `EditorHandle::remove_plugin(key: PluginKey) -> bool` (construction-time,
    like `add_plugin`). **Chrome default: `markdown_shortcuts: false`.** Turning
    them off by default changes behaviour today's apps rely on (ui-zoo,
    Pimble), so it needs a migration note, or an explicit decision to keep
    `true` as a documented exception. Also add an `undoInputRule` command bound
    to Backspace right after a rule fires.
  - Likelihood: **High.** Chat boxes, comment fields, and code or notes apps
    that want a literal `*`, `-`, or `1.` are common, and this is the
    most-requested switch in every PM-style editor.

- **`crates/rinch-editor-view/src/lib.rs:82`** — The schema is always `Schema::starter_kit()`
  (tables, task lists, headings 1–6, images, highlight, colour marks, …).
  - *Chrome:* a `contenteditable` accepts any HTML (paste keeps most
    formatting), and `contenteditable="plaintext-only"` is the only built-in
    restriction. A broad schema by default is therefore close to Chrome. Keep
    today's default.
  - Can the app change it? **Partially.** The app can only do it through the
    awkward `EditorHandle::new` path above. It can't restrict a "comment"
    editor to bold/italic/link through the normal `create_editor` / `Editor {}`
    path. Paste then admits every starter-kit node.
  - Proposed: the same `create_editor_with(EditorConfig { schema, plugins })`,
    defaulting to `starter_kit()`. A `Schema::plain_text()` preset would be the
    analogue of `plaintext-only`.
  - Likelihood: **High.** Constrained editors (comment, title, chat) are a
    standard need.

- **`crates/rinch-editor-core/src/lib.rs:80`** — History is fixed at `HistoryPlugin::new()`
  (500 ms typing-merge window, depth 200). `HistoryPlugin::with(delay, depth)`
  exists, but `add_plugin` refuses an already-installed key, so it can't replace
  the default.
  - *Chrome:* a `contenteditable` has native undo and redo (Ctrl+Z,
    Ctrl+Shift+Z / Ctrl+Y) that coalesces consecutive typing. Nothing about it
    is configurable. The exact coalescing rule (time window vs. a typing run
    broken by a caret move) and the depth: Chrome: unsure. Keep today's
    default.
  - Can the app change it? **No** (except through the `EditorHandle::new` path).
  - Proposed: `EditorConfig::history: Option<HistoryPlugin>` (`None` = no undo,
    e.g. when an outer app-level undo owns it), or the config above.
  - Likelihood: **Medium.** Apps with their own undo stack, or collaboration
    hosts, want to turn it off or tune it.

### Default keymap (rinch-editor-core `commands/mod.rs`, `keymap.rs`)

- **Differs from Chrome:** in an editable editor, Tab and Shift+Tab are kept
  inside it. **`crates/rinch-editor-core/src/commands/mod.rs:1702-1703`**
  (`Tab` → `sinkListItem`, `Shift-Tab` → `liftListItem`), plus the view's table
  cell navigation. `docs/src/guide/contenteditable.md:1016-1022` says an
  editable editor "keeps them for table cells and list indentation". Outside a
  list, `sink_list_item_any` returns `None` (Tab is a no-op, line 520-521).
  The guide's wording and the old audit note ("consumed always") say focus
  still does not move on. I have not re-verified that at runtime.
  - *Chrome:* in a `contenteditable`, Tab and Shift+Tab move focus to the
    next and previous focusable element, including inside a list. Chrome does
    not indent on Tab and does not insert a tab character.
  - Can the app change it? **Partially.** `on_key` can claim Tab, but the
    app's code then has to move focus itself.
  - Proposed: `Editor { tab_behaviour: TabBehaviour }` with
    `enum TabBehaviour { MoveFocus, IndentInListsAndTables, IndentAlways }`.
    The **Chrome default would be `MoveFocus`**. In a document editor Tab
    indenting is the expected convention (Google Docs), so if today's default
    is kept, keep it as a deliberate, documented exception. Either way, when
    Tab is a no-op outside lists, let it fall through to focus traversal
    instead of being swallowed. Today that is a keyboard trap for a keyboard
    user in a plain paragraph (WCAG 2.1.2).
  - Likelihood: **High.** Form-embedded editors (comment box, issue body) need
    Tab to reach the Submit button.

- **Differs from Chrome:** many chords beyond Chrome's three.
  **`crates/rinch-editor-core/src/commands/mod.rs:1692-1751`** — Fixed chords:
  Mod+B/I/U/E, Mod+Shift+S/7/8/9/B/0, Mod+Shift+L/E/R/J (alignment), Mod+Alt+1–6,
  and Mod/Alt+Backspace/Delete.
  - *Chrome:* a `contenteditable` binds Ctrl/Cmd+B, I and U to bold, italic and
    underline (the `execCommand` editing commands), plus Ctrl+Backspace/Delete
    word delete (Alt on Mac) and undo/redo. Every other chord goes to the
    browser: Ctrl+E focuses the omnibox search, and Ctrl+Shift+J/R are
    DevTools console and hard reload. Mod+B/I/U and word delete match Chrome.
    The rest are additions, and some take a browser chord away on the web.
  - Can the app change it? **Partially.** A plugin added with `add_plugin` can
    rebind a chord, because the later plugin wins in `Keymap::add` (`keymap.rs:168-169`,
    `HashMap::insert`). `on_key` returning `true` can swallow a chord. There is
    no way to unbind a chord so that it falls through to the app or the
    browser (the guide notes Ctrl+Shift+J/R clash with Chrome). There is also no
    way to read or replace the whole keymap, and swallowing in `on_key` means
    the app re-implements the dispatch. This rebinding works but is undocumented
    as a rebinding mechanism.
  - Proposed: `EditorHandle::unbind_key(binding: &str)` / `bind_key(binding: &str, command: &str)`
    (construction-time, like `add_plugin`), or `EditorConfig::keymap: Option<Keymap>`
    that replaces the BaseCommands bindings. Document the "later plugin wins"
    rule. **Chrome default:** Mod+B/I/U, word delete and undo/redo only. The
    others go in an opt-in `Keymap::rich_text_extras()`, or at minimum are not
    bound on wasm where they collide with browser chords (Mod+E,
    Mod+Shift+J/R).
  - Likelihood: **Medium.** The alignment chords clash with browsers, apps
    have their own Mod+Shift+… shortcuts, and some apps want no heading or
    list chords in a constrained editor.

### Caret (view.rs, blink.rs, rinch-web editor_input.rs)

- **Differs from Chrome:** the caret is a fixed blue, where Chrome uses the
  text colour. **`crates/rinch-editor-view/src/view.rs:3527-3539`** — Caret
  colour `#1a73e8` inline (`background-color`, line 3535) and width
  `CARET_WIDTH` = 2px (`crates/rinch-core/src/dom/traits.rs:136`) inline. The
  same blue applies in `set_dark_mode(true)`.
  - *Chrome:* `caret-color: auto`, which is effectively `currentColor` (the
    text colour, so it follows dark themes for free). The app can set
    `caret-color` per element. The caret is about 1 CSS px wide and not
    settable from CSS. Chrome: unsure whether it honours the Windows
    caret-width accessibility setting.
  - Can the app change it? **Partially:** only an undocumented `!important` rule on
    `[data-pm-caret]` (see note). There is no prop or variable, and the width is
    baked into caret geometry (#1041).
  - Proposed: honour `caret-color` on the editor root, i.e. inline
    `background-color: var(--rinch-editor-caret-color, currentColor)`, resolved
    from the root's `caret-color` when set. The default becomes the text colour.
    For width, `EditorHandle::set_caret_width(f32)` if the geometry can take it.
    The Chrome default would be 1px, but 2px reads better at 1× DPI, so keep 2px
    and note it as a deliberate difference.
  - Likelihood: **High.** Brand colours and themed or dark editors are the
    norm, and the caret is the most visible piece of editor chrome.

- **Differs from Chrome:** the blink rate is fixed and ignores the OS setting.
  **`crates/rinch-editor-view/src/blink.rs:28`** (desktop) **and
  `crates/rinch-web/src/editor_input.rs:3807-3811`** (web, a literal `530`) —
  The caret always blinks with a 530 ms half-period. It never reads the OS
  caret-blink setting, including "don't blink". The phase does reset to solid
  on every caret move or edit (`blink.rs:11-13`, `view.rs:3508-3521`), which
  matches Chrome.
  - *Chrome:* the blink interval comes from the OS: Win32 `GetCaretBlinkTime`
    (530 ms by default, and "none" means a solid caret), GTK
    `gtk-cursor-blink-time` on Linux, and the macOS default. The caret is solid
    while typing, then blinks again. On Linux, Chrome also follows GTK's
    blink timeout and stops blinking after a while. Chrome: unsure whether
    that applies on every platform. Chrome does not tie the blink to
    `prefers-reduced-motion`. On the web, rinch's hidden-field caret is not
    Chrome's, so the OS rate is not reached through the browser either.
  - Can the app change it? **No.** `set_caret_blink(visible)` exists on the
    handle, but the runtime's `caret_blink_tick` and the web `setInterval`
    re-toggle it every half-period.
  - Proposed: `rinch::editor::set_caret_blink(CaretBlink)` with
    `enum CaretBlink { System, Interval(Duration), Solid }`, **default `System`**
    (read the OS rate on desktop; on the web there is no API for it, so fall
    back to 530 ms). Also add a per-handle override.
  - Likelihood: **Medium-High.** It's an accessibility setting, and some users
    and apps (terminal-like UIs, kiosk displays) want a solid caret.

- **Differs from Chrome:** the drop caret is fixed blue.
  **`crates/rinch-editor-view/src/view.rs:3017-3027`** — The drag-and-drop drop
  caret is the same inline `#1a73e8`, 2px.
  - *Chrome:* while you drag over a `contenteditable`, the drop point is shown
    with an ordinary caret, in the element's caret colour.
  - Can the app change it? **Partially** (`!important` on `[data-pm-drop-caret]`
    only).
  - Proposed: `var(--rinch-editor-drop-caret-color, var(--rinch-editor-caret-color, currentColor))`.
  - Likelihood: **Medium.**

### Selection and node selection (view.rs)

- **Differs from Chrome:** the selection is a fixed translucent blue that
  ignores `::selection` and the system colour.
  **`crates/rinch-editor-view/src/view.rs:3251-3260`** — The selection
  highlight is inline `rgba(26,115,232,0.3)`, behind the text
  (`z-index: -1`). It is identical in dark mode, and the text colour does not
  change.
  - *Chrome:* the selection is painted with the system highlight colour
    (`Highlight` / `HighlightText`), and `::selection { background; color }`
    on the element overrides it. On Linux, Chromium's built-in default is
    roughly dodger blue (`#1E90FF`). Chrome: unsure of the exact value and
    alpha per platform and GTK theme. Chrome may also change the text
    colour of selected text.
  - Can the app change it? **Partially** (undocumented `!important` on
    `[data-pm-selection]`).
  - Proposed: honour `::selection` on the editor if rinch-dom can resolve it.
    Otherwise use `background-color: var(--rinch-editor-selection-bg, Highlight)`,
    with `Highlight` resolved to the platform colour, or to today's rgba where
    the platform colour is unknown. Use the same token as rinch's other text
    fields.
  - Likelihood: **High.** It has to match the app theme and the rest of rinch's
    text fields.

- **`crates/rinch-editor-view/src/view.rs:3418-3428`** — The node-selection
  outline (an image, hr, embed, or node view that is selected) is inline
  `border: 2px solid #1a73e8; background-color: rgba(26,115,232,0.15)`.
  - *Chrome:* an image selected in a `contenteditable` is painted with the
    selection highlight tint over it, and has no outline. There is no
    equivalent for an `<hr>` or embed. Chrome: fairly sure for images, unsure
    for other replaced elements. There is a partial difference: Chrome draws no
    outline.
  - Can the app change it? **Partially** (`!important` on `[data-pm-selected]`).
    A node view can't tell the outline not to draw, for example when it shows
    its own selected state.
  - Proposed: CSS vars `--rinch-editor-node-outline` (border shorthand) and
    `--rinch-editor-node-selection-bg`, with the bg defaulting to the
    selection colour. Keep today's outline as the default, since a
    tint-only selection is hard to see on a photo. Add
    `NodeViewSpec::draws_own_selection(bool)` to suppress the outline for that
    view.
  - Likelihood: **Medium-High.** Apps with their own image or embed chrome
    usually want their own selected look.

- **Differs from Chrome:** a blurred editor's selection disappears.
  **`crates/rinch-editor-view/src/view.rs:3558`** (`hide_overlays`, called from
  `crates/rinch/src/app/focus.rs:433` on blur) — A blurred editor shows no
  selection at all, and no caret.
  - *Chrome:* when the window loses focus, the selection stays and is painted
    in the inactive colour (Chromium's default inactive background is a light
    grey, about `#C8C8C8`; Chrome: unsure per platform). When focus moves to a
    button or another non-text element in the page, the
    `contenteditable`'s DOM selection is kept and is still painted. When focus
    moves into another text field, that field takes the selection, and the
    editor's highlight goes. Chrome: unsure whether a later `focus()` back
    restores the old range in every case. The caret hides on blur, which
    matches rinch.
  - Can the app change it? **No.** `data-nofocus` on a toolbar avoids the blur
    for presses, but a real focus move (a colour picker input, a link-URL
    `TextInput`, a find box) erases the visible selection while the app still
    acts on it.
  - Proposed: `EditorHandle::set_show_selection_when_blurred(bool)`, **default
    `true`** (Chrome-like), drawing with
    `var(--rinch-editor-selection-inactive-bg, <inactive Highlight>)` and a
    `data-pm-inactive` attribute on the rects. Strictly, Chrome drops it when
    another text field takes focus, but keeping it there too is what "insert
    link" dialogs want, so the option covers both.
  - Likelihood: **Medium-High.** "Insert link" and "find" dialogs are the
    standard case.

### IME composition overlay (view.rs)

- **Differs from Chrome:** the preedit is boxed in white with a blue
  underline. **`crates/rinch-editor-view/src/view.rs:3652-3662`** — The preedit
  span is inline `background-color: #ffffff; border-bottom: 1px solid #1a73e8`.
  It stays white in dark mode, so a CJK user typing in a dark editor sees a
  white box.
  - *Chrome:* composition text is drawn inline with no background, underlined
    in the text colour. The IME's spans (`ImeTextSpan`) make the active clause
    thick and the others thin. The IME can also ask for a background on the
    target clause. Chrome: unsure about per-IME colour overrides.
  - Can the app change it? **Partially** (`!important` on `[data-pm-preedit]`).
  - Proposed: **default transparent background and a `currentColor`
    underline**, i.e. `var(--rinch-editor-preedit-bg, transparent)` and
    `var(--rinch-editor-preedit-underline, currentColor)`. Later, honour the
    IME's clause segmentation (thick underline under the active clause) if
    winit exposes it.
  - Likelihood: **Medium.** It's a visible defect for IME users in dark mode,
    and an app can't fix it cleanly.

### Built-in stylesheet (styles.rs)

- **Differs from Chrome:** text colour and font are hard-coded where a bare
  `contenteditable` inherits them.
  **`crates/rinch-editor-view/src/styles.rs:36-41, 74, 93, 206` and the whole sheet** —
  Every colour is a hard-coded hex (text `#1f2328`, bg `#fff`, border `#d0d7de`,
  link `#0969da`, mark `#fff3a3`, code bg, placeholder `#8c959f`, the dark-mode
  set…). The font is a system stack at 16px / 1.65, with padding 16px 20px, a
  1px border and a 10px radius. None of it uses `--rinch-*` theme vars, so a
  rinch-theme palette or a dark theme has no effect on the editor.
  - *Chrome:* a bare `contenteditable` `<div>` has no styling of its own. It
    inherits font, size, colour and background, and has no border, padding or
    radius. Focus shows the `:focus-visible` outline (`outline: auto`). Links
    and `<mark>` get the UA defaults (`<mark>` is yellow `#ff0`). There is no
    equivalent for the editor's frame (border, radius, padding).
  - Can the app change it? **Partially.** These are stylesheet rules, so an
    app can override them, but it has to out-specify them. The sheet is
    appended to `<body>` on first mount (`styles.rs:276`), so it comes after the
    app's own stylesheet, and an equal-specificity app rule like
    `[data-pm-editor] { padding: 0 }` loses. The dark rules are
    `[data-pm-editor][data-pm-theme="dark"] …` (higher still).
  - Proposed: put the values behind `--rinch-editor-*` vars. Text colour,
    background and font default to **`inherit`** (Chrome-like), or to
    rinch-theme's `--rinch-color-*` vars where the theme is present. The frame
    (border, radius, padding) keeps today's values, since Chrome has no
    equivalent. Insert the sheet at the start of `<head>`/`<body>` (or wrap it
    in `@layer rinch-editor`) so equal-specificity app rules win.
  - Likelihood: **High.** Every themed app restyles the editor, and today that
    takes a specificity fight across ~60 rules.

- **`crates/rinch-editor-view/src/styles.rs:276`** (`ensure_default_styles`) — The
  stylesheet is always injected, with no "unstyled/headless" mode.
  - *Chrome:* no equivalent (a `contenteditable` is unstyled to begin with).
    Keep today's default.
  - Can the app change it? **No.** The app can only override it rule by rule.
    Some rules are load-bearing (tables as grid, list-marker hanging, empty-block
    `min-height`, placeholder position), so the sheet should be split, not dropped.
  - Proposed: `rinch::editor::set_default_theme(false)` (or
    `EditorConfig::default_theme: bool`, default `true`) that injects only the
    structural rules.
  - Likelihood: **Medium.** Design-system apps want a blank slate.

- **Differs from Chrome:** dark mode is a manual switch that does not follow
  the page's colour scheme. **`crates/rinch-editor-view/src/view.rs:2145-2151`**
  (`set_dark_mode`) — Dark mode is only a manual per-handle switch
  (`data-pm-theme`). It doesn't follow the rinch theme's colour scheme or
  `prefers-color-scheme`, and there is no `Editor` prop.
  - *Chrome:* a `contenteditable` inherits the page's colours, and form-like
    UA parts follow the CSS `color-scheme` property. Nothing per-element needs
    switching.
  - Can the app change it? **Partially.** The app calls `set_dark_mode` from
    its own effect.
  - Proposed: once the colours inherit (finding above), the switch becomes
    moot. Until then, **default to following the rinch theme's scheme**, with
    `Editor { dark_mode_fn: Fn() -> bool }` as an override.
  - Likelihood: **Medium.**

### Placeholder (view.rs, rinch-editor-core `plugins/placeholder.rs`)

- **`crates/rinch-editor-view/src/component.rs:50-79`** /
  **`crates/rinch-editor-core/src/plugins/placeholder.rs`** — `Editor {}` has no
  `placeholder` prop. Its doc comment says the view builds it "from the
  `Editor { placeholder: … }` prop", which doesn't exist. The plugin works through
  `editor.add_plugin(Rc::new(PlaceholderPlugin::new("…")))`, but that isn't in
  the guide. The text is fixed once added: `add_plugin` refuses the same key,
  and `view.rs:1749` keeps the existing node. The overlay's class is
  `rinch-editor-placeholder` (`view.rs:1729-1730`), and its colour comes from
  the stylesheet.
  - *Chrome:* a `contenteditable` has no native placeholder. Apps use
    `:empty::before { content: attr(data-placeholder) }`. `<textarea placeholder>`
    is the nearest equivalent. It shows while the field is empty, in a grey
    `::placeholder` colour (about `#757575` in light mode; Chrome: unsure of the
    exact value), and the attribute is live, so changing it updates the text.
    So: no placeholder by default (today's), and a reactive one when set.
  - Can the app change it? **Partially:** an undocumented plugin, with no
    reactive text (i18n or context-dependent prompts).
  - Proposed: `Editor { placeholder: String }` (default empty = none, as
    today), plus `EditorHandle::set_placeholder(Option<String>)` (reactive,
    like the textarea attribute), and `--rinch-editor-placeholder-color`.
    Document it in the guide.
  - Likelihood: **High.** Almost every embedded editor (comment, chat, note)
    shows a prompt.

### Block formatting written inline (view.rs `apply_type_attrs`)

- **Differs from Chrome:** inline `text-align` and `margin-left` are written
  even at the defaults. **`crates/rinch-editor-view/src/view.rs:132-146`** —
  For every paragraph and heading the view always writes inline
  `margin-left: 0` (or `{indent*2}em`) and `text-align: left` (or
  center/right/justify), even at the default. An app stylesheet rule such as
  `[data-pm-editor] p { text-align: justify }`, `h1 { text-align: center }`,
  or an RTL `text-align: start` is defeated. The indent step is fixed at 2em
  per level.
  - *Chrome:* a block gets no inline style until a command is used.
    `execCommand('justifyCenter')` and the others write an inline `text-align`
    on that block only. `execCommand('indent')` wraps the block in a
    `<blockquote style="margin: 0 0 0 40px; border: none; padding: 0px;">`,
    i.e. 40px per level. The default alignment is `start`, so RTL works.
  - Can the app change it? **No** (`!important` only).
  - Proposed: **write nothing at the defaults** (indent 0, align unset: remove
    the property instead of writing `0`/`left`). That makes `start` the
    effective default, as in Chrome. Write the indent as
    `calc(var(--rinch-editor-indent-step, 2em) * N)`, or as a
    `data-pm-indent="N"` / `data-pm-align` attribute styled by the sheet. Keep
    2em rather than Chrome's 40px: it scales with font size, and 40px is an
    `execCommand` artefact rather than a design choice.
  - Likelihood: **Medium-High.** Any centred-title or justified-body house
    style, and any RTL app, hits it.

- **Differs from Chrome:** table columns are equal widths instead of
  content-sized. **`crates/rinch-editor-view/src/view.rs:116-121`** — Tables
  always get inline `grid-template-columns: repeat(N, minmax(0, 1fr))`, so all
  columns are the same width.
  - *Chrome:* a `<table>` in a `contenteditable` uses `table-layout: auto`.
    Columns size to their content, and a `width` on a cell or `<col>` is
    honoured. Chrome itself offers no column-resize UI.
  - Can the app change it? **No** (`!important` only). There's no column-width
    attribute in the schema.
  - Proposed: a `colwidth` cell attr (ProseMirror tables' model) that the view
    writes when present. Without it, the Chrome-like default is content
    sizing, roughly `repeat(N, auto)` (Chrome: unsure how closely grid `auto`
    matches the table auto-layout algorithm for wide content).
  - Likelihood: **Medium.** Column widths and resizing are a common table
    request.

### Scrolling the caret into view (view.rs, handle.rs)

- **Differs from Chrome:** a 16px margin, where Chrome reveals the caret to
  the edge. **`crates/rinch-editor-view/src/view.rs:1585, 2727`** — The
  automatic caret reveal is always on for user moves. It is "nearest" and
  instant, with a fixed `REVEAL_MARGIN` = 16px.
  - *Chrome:* when typing or moving the caret, it scrolls the minimum amount,
    aligning to the edge only if needed (no extra margin), instantly, through
    every scrollable ancestor. Chrome: unsure whether the caret reveal honours
    the scroller's `scroll-padding`. `scrollIntoView` and focus scrolling do.
  - Can the app change it? **Partially.** `scroll_into_view_aligned` lets the
    app *add* a scroll, but it can't change the margin, ask for smooth
    scrolling or "typewriter" (centred caret) mode, or turn the automatic
    reveal off (for example when the app manages its own scroll sync, like a
    split preview).
  - Proposed: `EditorHandle::set_caret_reveal(CaretReveal { margin: f32, align: ScrollAlign, enabled: bool })`,
    **default `{0.0, Nearest, true}`** (Chrome-like), plus honouring
    `scroll-padding` on the scroller. That solves the sticky-header case
    better than a constant does. If a nonzero margin is kept as rinch's taste,
    note it as a deliberate difference.
  - Likelihood: **Medium.** Writing apps want typewriter scrolling, and apps
    with sticky toolbars need a larger margin, since a 16px margin under a 48px
    sticky header hides the caret.

- **`crates/rinch/src/app/event_dispatch.rs:4492-4538`**,
  **`crates/rinch-web/src/editor_input.rs:2825`** — Page Up and Page Down move
  0.875 of the scrollport.
  - *Chrome:* paging scrolls by 87.5% of the visible height (Chromium's
    `kFractionToStepWhenPaging`), so this matches. Chrome: unsure whether
    its overlap cap also applies in an editable.
  - Can the app change it? **No** (`on_key` can take the key over entirely).
  - Proposed: leave it.
  - Likelihood: **Low.**

### Task-list checkbox (rinch-web `editor_input.rs`, rinch `event_dispatch.rs`)

- **`crates/rinch/src/app/event_dispatch.rs:6292-6296`**,
  **`crates/rinch-web/src/editor_input.rs:2291-2301`** — A press in a task
  item's gutter always toggles `checked` (`toggle_task_checked_at`). There's no
  callback to observe or veto it beyond `on_change`, which fires afterwards.
  The read-only editor refuses it, which is good. The glyph is a CSS
  `::before` (overridable), and the checked style is dim plus strike-through
  (overridable CSS).
  - *Chrome:* no direct equivalent (no task-list element). Chrome: unsure
    whether an `<input type=checkbox>` inside a `contenteditable` toggles on
    click. Keep today's default.
  - Can the app change it? **No** for the click behaviour, which can't be
    intercepted or vetoed. **Yes** for the look.
  - Proposed: `EditorHandle::on_task_toggle(Fn(Pos, bool) -> bool)` where
    returning `true` claims the click (mirrors `on_link_click`).
  - Likelihood: **Low-Medium.** Mostly for apps that sync tasks elsewhere or
    confirm completion.

### Spellcheck

- **Differs from Chrome:** there is no spellcheck. No file (absence). I found
  no spellcheck in `rinch-editor-view`. The guide's `Spellcheck` example
  (`contenteditable.md:376-396`) is an app-written decoration plugin.
  - *Chrome:* a `contenteditable` is spellchecked by default (red squiggles,
    and suggestions in the context menu), and `spellcheck="false"` turns it
    off.
  - Can the app change it? **Partially.** An app can write a decoration plugin
    with its own dictionary. Nothing connects to the OS spellchecker.
  - Proposed: `Editor { spellcheck: bool }`, **default `true`**, backed by the
    OS checker where one exists (NSSpellChecker, Windows Spell Checking API,
    hunspell/enchant on Linux; on the web the hidden field cannot surface
    the browser's). This is a feature gap more than a configurability one,
    and it is large. Record it rather than schedule it.
  - Likelihood: **Medium.** Prose apps expect it, and chat or code apps turn
    it off.

### Already configurable, but the default differs from Chrome

- **Differs from Chrome (default only):** the read-only editor's Tab
  (`docs/src/guide/contenteditable.md:1016-1020`) wraps round past the last Tab
  stop on desktop. Chrome sends focus to the browser UI after the last stop.
  The desktop has no browser chrome, so wrapping is the reasonable analogue.
  Note it only. The web already matches.
- **Differs from Chrome (default only):** the right-click menu
  (`data-oncontextmenu` replaces it). rinch's built-in text menu on desktop is
  Cut/Copy/Paste/Select all. Chrome's `contenteditable` menu adds spelling
  suggestions, "Paste as plain text" (also Ctrl+Shift+V, which rinch binds),
  Emoji, and "Writing direction"/Inspect. The difference is mostly the missing
  spelling entries (see Spellcheck). A right-click inside the existing
  selection keeps it, as in Chrome. Chrome: unsure whether rinch's menu
  reopens at the new point on a second right-click, so verify.

### Collaboration (collab.rs)

- No built-in remote cursors or presence rendering, so nothing is imposed.
  The seams exist (`collab_sticky_index`/`collab_resolve_sticky`, `caret_rect`,
  the overlay slot). Not a finding. *Chrome:* no equivalent. If cursors are
  ever added built-in, they need colour and label render fns from day one.

---

**Tally.** 17 findings are marked Differs from Chrome: 15 behaviour
gaps and 2 "default only". The node-selection outline is a partial
difference and is not counted.

The 5 most significant:
1. **Tab is kept inside an editable editor** (and swallowed as a no-op outside
   lists). Chrome moves focus. This is a keyboard trap in form-embedded
   editors.
2. **Markdown autoformat is on by default.** Chrome never rewrites typed text.
3. **The blurred editor hides its selection.** Chrome keeps it and paints it
   in the inactive colour, which breaks "insert link" and "find" flows.
4. **Caret, selection, drop caret and IME preedit use a fixed `#1a73e8` /
   white**, where Chrome uses `caret-color: currentColor`, `::selection` /
   `Highlight`, and an un-backgrounded `currentColor` underline. The white
   preedit box in dark mode is the worst of these.
5. **Inline `text-align: left` / `margin-left: 0` are written at the
   defaults.** Chrome writes nothing, so app CSS and RTL `start` alignment
   are defeated.

