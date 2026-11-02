# Keyboard focus

Read this page when your task touches: the focus arbiter, Tab order, traps, dismiss stack, key handlers, selection ranges, scroll lock, IME, text context menu.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Keyboard Focus

Exactly one widget owns the keyboard at a time — the **focus arbiter**
(`FocusTarget` in `crates/rinch/src/app/mod.rs`, design A10): an `<input>`, an
open `<select>`, the rich-text editor, a render surface, or a generic focusable
DOM node (`FocusTarget::Node`). Every transition goes through
`RinchApp::set_focus_target`, which tears the previous owner down first.

**An element's own `onkeydown` / `onkeyup`** (#1678): `Fn(&KeyEventData) ->
bool` (registered with `register_key_handler`, written to `data-onkeydown` /
`data-onkeyup`), `true` consuming the key — `preventDefault` +
`stopPropagation`: no insert, no `onsubmit`, no activation, no Tab, nothing
after it hears the press. Offered to the arbiter's `FocusTarget::Input` /
`Node` node (web: the event target) and its ancestors, nearest first, stopping
at the first that consumes (`dispatch_key_handler_chain`; ids read before any
runs). Order, both backends: menu chords → the editor's / a node view's
`on_key` → **element handlers** → `set_keyboard_interceptor` → key entries →
dismiss stack → (desktop) a focused registered widget's `FocusEntry::on_key`
→ a focused render surface's claim → **app key listeners**
(#1784) → the element's own handling (`RinchApp::offer_element_key`,
`event_delegation::offer_element_key`). Focus requests a handler made are
applied before the key's default, so an unconsumed key lands in the field a
handler focused, as in Chrome. A raw `<input>`/`<textarea>` with only a key
handler gets the desktop text-field shim (it focuses and takes text, as on
the web). Not offered: keys the editor holds,
a focused render surface's, an open native `<select>` list's (desktop), while
the text context menu is open (desktop), or during an IME composition
(desktop: the focused field shows a `data-preedit`; web: `isComposing`,
or `key == "Process"`; known gap: Safari's composition-confirming Enter —
`key: "Enter"`, `keyCode` 229, `isComposing` false — is offered, since
refusing every 229 Enter would refuse an Android soft keyboard's plain
Enter). `onkeyup`'s consumption stops only its ancestors: the
interceptor hears every release. `KeyEventData` and `KeyEventKind` are in the
prelude. Pins: `app/element_key_1678_tests.rs`,
`rinch-web/tests/element_key_1678.rs`.

**App-wide key listeners** (#1784, `rinch_core::events::app_key`):
`on_app_key(Fn(&KeyEventData) -> bool) -> AppKeyListener`, a list separate from
`set_keyboard_interceptor` (whose one slot per document, second call replaces,
stays as it was). Offered every key press the stages above left, in
registration order, the first `true` consuming it (no later listener, no
default; web `preventDefault` + `stopPropagation`); every release goes to every
listener, answer ignored — except a release whose press a focused render
surface claimed (`withhold_app_key_release`, cleared by that release or the
next delivered press of the code). Entries are keyed by
`current_dispatching_doc` at registration (`doc_matches`), carry their owner
(dead owner skipped and pruned, `on_cleanup` removes at disposal), run
untracked inside it; the handle's drop releases nothing (`remove()` does).
Desktop: `RinchApp::offer_app_key`, called in `event_dispatch.rs`'s KeyDown arm
for `FocusTarget::Input/None/Node` after `offer_registered_node_key` (the
#147 registry's first refusal, moved out of `handle_unclaimed_key_fallback`)
and before the fallback, in
the `Surface` arm for an unclaimed key, and after the interceptor on every
KeyUp; not for `Editor` or `Select`, nor while the text context menu is open or
the focused field shows a `data-preedit`. Web: the document bubble keydown
delegate (a key with nothing focused targets `<body>` and bubbles there) after
the surface block and before the Tab trap, skipping `isComposing`/`Process`
and a key whose target is an editor's (`key_target_is_editor`); the keyup
delegate after the interceptor. rinch-web marks no document, so a web listener
hears every island's keys and page inputs' outside any root; a desktop one
registered from `main` hears the DevTools window's. Pins: `app/app_key_1784_tests.rs`,
`rinch-web/tests/app_key_1784.rs`, `rinch-core` `events::app_key::tests`.

**`NodeHandle::selection_range()` / `caret_rect()` / `caret_rect_at(offset)`**
(#1678), the read half of the paragraph below: `(start, end,
SelectionDirection)` in UTF-16 units, `start <= end`
(`DomDocument::text_selection_range`; web: the element's properties; desktop:
a posted request (`peek_pending_text_selection_request`), else the focused
field's `data-selection-start`/`data-cursor-pos` bytes, else
`NodeTree::text_selections` — written at blur by `clear_input_focus_attrs` and
when a request is stashed, set to the end by a `set_attribute("value")`
that changes an unfocused control's value (a browser's value setter; a
parsed `value` does not), dropped in `remove_subtree` — else `(0, 0,
Forward)`, Chrome 155's answer for a parsed or default value; a `None`
direction reads `Forward` (Chrome on Linux/Windows; macOS unmeasured); a
programmatic or Tab focus shows exactly this selection (`try_focus_input`);
a posted request is cancelled by such a value write; `None` for a non-text control and, as Chrome, for
`email`/`number`/date types). The caret rect (`DomDocument::text_caret_rect`,
`None` unless rendered) is width 0 in logical window px on desktop
(`form_control::painted_caret_rect`, shaped by the builder paint draws with,
`form_control::field_text_layout`, at the origin paint uses,
`form_control::field_text_origin`, through the field's scroll, #827; caret
height 1.2 × font-size) and client px on the web (a hidden mirror
`<div>` with the control's computed text styles at `clientWidth` less padding,
its in-box offset scaled by painted/layout size for a transform; pinned
against `caretPositionFromPoint`; height is the font's content area).

**`NodeHandle::select()` / `set_selection_range(start, end, direction)`**
(issue #552) are `focus()`'s selection counterparts — for a rename box that
opens with its name filled in **and selected**, so the first keystroke
replaces it. `set_selection_range` matches `HTMLInputElement
.setSelectionRange(start, end, direction)` exactly, **UTF-16 code units
included**, so one offset pair means the same thing on desktop and on
`rinch-web`; `select()` is `set_selection_range(0, <UTF-16 length>, Forward)`.
Applied to the control directly if it already holds the keyboard; otherwise
stashed for the next time it gains it, the same thing a browser's own
`setSelectionRange()`/`select()` does on an unfocused control — so
`focus()` and `select()` posted together from one effect do not need to run
in either particular order. Desktop posts both through a channel keyed
**per node** (`DomDocument::set_selection_range`,
`rinch_core::post_text_selection_request`/`take_pending_text_selection_requests`) —
**separate** from `FocusRequest`'s own single slot, since folding the two
together would let a `select()` posted right after a `focus()` silently
discard the focus request, and per-node because two *different* nodes'
`set_selection_range` calls posted in the same tick (no `focus()` between
them — a form restoring two independent selections) must not clobber each
other the way `FocusRequest`'s slot correctly does for focus itself (only
one node can hold the keyboard; any node can hold a pending selection) —
review of #1325, finding 2. Drained in full at the same points
`FocusRequest` is, applied live to `RinchApp::focused_input_state`'s
`EditableState` (converting UTF-16 to the byte offsets it keeps) if the
node is the focused input, or stashed in `RinchApp::pending_text_selection`
for `try_focus_input` to consult the next time that node is focused —
**but only for a node `node_takes_text_focus` would accept**: a checkbox,
a `<select>`, or a generic `tabindex` node never reaches that
stash-consuming branch, so stashing for one would never be consumed and the
request is dropped instead (review of #1325, finding 3 — the desktop
equivalent of a browser's `InvalidStateError` on such a control).
A **mouse-click** focus (`click_handling.rs`'s own `EditableState`
construction, not `try_focus_input`'s) places its own caret and must clear
any stash for that node too, not only consult-and-clear it — otherwise a
stale pre-click stash reappears on the *next* programmatic/Tab focus
(review of #1325, finding 1). A stash entry for a node that is removed
before ever being focused is pruned by connectivity
(`DomDocument::is_connected`, not slab presence — `discard()`/`remove()`
are both a detach on desktop today, #723) on every drain, which does not
close the #304 recycled-slab-id hazard for a node-keyed registry: a freed
id handed to an unrelated new node before the next prune would inherit a
stale entry, accepted as the same pre-existing hazard class every other
node-keyed registry here carries. `rinch-web` calls the element's own
`setSelectionRange` directly — already UTF-16 units, so nothing to
convert, and the browser's own refusal on a non-text control (a thrown,
caught `Err`) stands in for desktop's explicit drop.

Focusability on desktop comes from the **tag** or an explicit `tabindex` (or a
`data-oninput` on a custom control), and an explicit `tabindex` always wins —
the browser rule (issue #252). Focusable by tag: `<button>`, `<select>`,
`<textarea>`, `<input>`, and `<a>` with a non-empty `href`. **Not** `<summary>`
(rinch has no `<details>` behaviour) and **not** `data-rid` (the DropdownMenu
backdrop carries one). `tabindex="-1"` is focusable by click and
programmatically but not tabbable. An HTML integer attribute — `tabindex`,
`<textarea rows>`, `<ol start>`, `<li value>` — is read by HTML's rules for
parsing integers (`rinch_core::dom::parse_html_integer` /
`parse_html_non_negative_integer`, #1138/#1153), never by Rust's `parse`:
`" 3"`, `"2.5"` and `"3abc"` are 3, 2 and 3, and a value past `i32` is an error,
as in Chrome 153 (the editor's HTML import, which depends on no rinch crate,
reads `<ol start>`, `colspan` and `rowspan` through a private copy,
`rinch-editor-core/src/serialize/html_integer.rs`, cross-checked against it in a
test — #1164);
`disabled` and `data-disabled` are both honoured (issue #315) as **boolean
attributes**, take no focus by any route, and are re-checked at edit time: a
field that goes disabled *while focused* stops accepting keys **and releases the
keyboard** — with its `data-onchange` commit suppressed, since going disabled is
not the user committing an edit (`release_focus_for_disabled`, the only
transition that suppresses it). `readonly` focuses and selects but refuses every
text-changing command, on a `<textarea>` as well as an `<input>`.

**Presence is the whole value, and `"false"` is not an escape for the HTML
pair** (issue #612). `disabled` and `readonly` are read by presence alone, the
way a browser reads them — `<button disabled="false">` is disabled and
`<input readonly="false">` is read-only, measured in Chrome 150 as both the IDL
property and a `:disabled` / `:read-only` match — so **the value rule** is now
one rule across desktop and `rinch-web`. Only that clause: desktop is still
broader than a browser elsewhere in the family (`node_is_disabled` is
tag-agnostic where HTML ignores `disabled` on a `<div>`, and rinch has no
`:read-only` pseudo-class at all). Desktop used to honour a `"false"` escape the
browser has no notion of, which meant one markup and opposite behaviour. To say
*enabled*, **remove** the attribute, which is what a falsey reactive `bool` does
for you (`NodeHandle::write_attribute`, #551).
rinch's **own** `data-disabled`, `data-nofocus`, `data-trap-focus`,
`data-backdrop` (#1093) and `data-scroll-lock-exempt` (#701) keep the escape,
and are the only five that have it — a
rinch convention rather than a desktop quirk, which `data-nofocus`,
`data-trap-focus` and `data-backdrop` are what
show: the web reads them the same way, through
`[data-nofocus]:not([data-nofocus="false" i])`,
`[data-trap-focus]:not([data-trap-focus="false" i])` and `data_attr_is_on`
itself for `data-backdrop`. (`data-disabled` has no web reader; the browser does
not know the attribute. Nor has `data-scroll-lock-exempt`: the web's scroll
lock stops the page and no scroll container inside it.) The rules are one function each —
`rinch_core::dom::data_attr_is_on` for the `data-` family, a bare
`contains_key` for the HTML pair — and `"0"` is the only value that can tell
which one a reader uses, so every desktop reader pins it
(`data-backdrop`'s on both backends).

A disabled `<fieldset>` disables its
subtree (except its first `<legend>`); every other tag's `disabled` removes
only the node from the Tab order, not its subtree. A **mouse press claims
the nearest focusable ancestor** of the hit node, browser-style, so a clicked
`tabindex` div owns Enter/Space immediately — with **any** button
(`RinchApp::claim_press_focus`, issue #452): a right or middle press focuses
before `data-oncontextmenu` or the text menu runs, as a browser's `mousedown`
does, and only `:active` is left to the primary button. A press on an editor
(any button) is the editor's to place and an open `<select>` popup is modal,
so the claim stays out of both. The right/middle path
used to run no claim, so a right press on a nested focusable released the
outer claim and nothing took the keyboard.

**On `rinch-web` a press on a `data-rid` element resolves focus the same way**
(#1146). `dispatch_click_at` `preventDefault`s the `pointerdown` of a press
whose handler ran, and that cancels the browser's own focus change, so before
#1146 no such press moved focus: a `Button` took none, the field that had it
was not blurred (its `change` uncommitted), and a text field inside a
clickable (`ColorInput`'s, in its input group) could not be focused by a click
at all. Now (`PressFocus`, `event_delegation.rs`): when the nearest focusable
ancestor-or-self (`PRESS_FOCUSABLE_SELECTOR`) is a form control or editable
content the default is not prevented, so the browser focuses it and places
the caret; any other focusable is focused by rinch (`preventScroll`) **before**
the handler runs, so the old field blurs first and a focus move the handler
makes wins; a press already prevented (`data-nofocus`) or with nothing
focusable on its chain moves no focus (desktop releases for the latter). The
selector is a superset (a `disabled` button, a `contenteditable="false"`
island match it and refuse `focus()`), so `focus_press_target` asks the
browser and walks on to the next match, as Chrome focuses the `tabindex` row
around a disabled button. A **touch/pen** press holds its focus move until the
owning pointer's `pointerup` (`PendingPressFocus`): a `pointercancel` (Chrome's
pan), a move past the scroll slop or the next primary press drops it, and a
focus move made since the press landed (the handler's own) wins — a pan that
starts on a button used to blur the field and drop a phone's soft keyboard.
(The click itself still fires on contact outside a scroller, #1582.) In a
`data-nofocus` region a press whose nearest focusable is a form control or
editable content strictly inside the region is not prevented (#1579), as on
desktop. Behaviour changes for web apps: an editor toolbar needs
`data-nofocus`, as on desktop; any focused field blurs and commits `onchange`
on a press of a focusable `data-rid`; a `trap_focus` overlay opened from a
`Button` restores focus to that `Button`; a native `<select>`/range inside a
clickable is no longer prevented, so it works natively beside the handler.
Safari and macOS Firefox do not focus a `<button>` on a click; rinch-web does
(Chrome's and desktop's rule). Measured with trusted CDP input in Chrome 155;
pins: `rinch-web/tests/press_focus_1146.rs`, `press_focus_1580_review.rs`,
`press_focus_1580_round2.rs`.

A focused **`<select>` is closed**, like a browser's — Enter/Space/Alt+Down
opens its popup, which then owns the keyboard (issue #314). A pointer opens and
picks with the primary button only (issue #1111): a right or middle press on the
closed control falls through to the ordinary click phases, one on an open list's
option is consumed and picks nothing, and a press of any button outside the list
still dismisses it. It is never handed
to the text engine, whatever handlers it carries: branching on `data-oninput`
without a tag guard used to install an `EditableState` over a select's `value`
and make it a typable text field (issue #424). `Select`'s trigger `<div>`
carries `tabindex="0"` + combobox ARIA (issue #251), and its **open** option
list answers the keyboard on both backends (issue #434): ArrowUp/Down (wrapping),
Home/End, Enter to commit (Space too, unless a type-ahead prefix is live, which
it extends), Escape and Tab to close without committing, and type-ahead, with
`aria-activedescendant` naming the highlighted option. The trigger keeps the
focus; Escape arrives through `arm_close_on_escape_while_open`, the other keys
through a **key entry** on the dismiss stack. A closed `Select` consumes no key.

**The dismiss stack holds two kinds of entry.** `push_dismiss_handler(doc_key,
|| bool)` hears Escape only; `push_key_handler(owner: &NodeHandle, on_key:
|&KeyEventData| -> bool, on_focus_leave: || ())` (in `rinch_core` and the
`rinch` prelude, #434) hears **every key press** (never a release), ahead of the
Escape scan, from `dispatch_keyboard_event` — so both backends, no backend
branch; a consumed key is `preventDefault`ed on the web. **Focus is part of the
contract** (the review of #1165, where an open `Select` ate every key in the
document — a field focused by a "/" shortcut got none, Enter there committed the
`Select`, and on the web a page `<input>` outside the mount was prevented): an
entry is offered a key only while `owner.active_element()` is the owner or
inside it, and an unknown focus (`None` — nothing focused, a backend that does
not model focus, or on the web an element outside the mounted root) is inside
nothing. `rinch_core::notify_focus_moved(doc_key)` calls `on_focus_leave` for
each entry whose owner lost the focus: desktop calls it from
`RinchApp::fire_focus_work` after every arbiter transition (callers fire that
work after installing the new owner, so `focused_node` is already the new one),
rinch-web from a document `focusin` and from a `focusout` with no
`relatedTarget` (with key 0, which reaches every island). A window blur moves
no focus — desktop keeps the claim, and on the web the `focusout` it fires has
no `relatedTarget` but `activeElement` is still the trigger while it runs
(measured, Chrome 153) — so it is told apart (#1171):
`rinch_core::notify_window_blurred(doc_key)` calls **every** key entry's
`on_focus_leave` of that document, focus or not, from desktop's
`WindowFocus(false)` arm and from a rinch-web `window` capture `blur` listener
(at-target phase only: element blurs pass through it too). So an open `Select`
list closes on window blur, as a native `<select>` and Mantine's do, and the
trigger keeps the focus; desktop's native `<select>` popup closes there too
(`close_select_popup_returning_focus`). `ColorInput`'s dropdown holds a key
entry too while open (#1572), owned by the input's root, taking no key, whose
`on_focus_leave` closes it — so Tab away, a programmatic focus elsewhere, a
press outside that moves the focus, and a window blur close it, as Mantine's
input `onBlur` does; its panel and preview swatch are `data-nofocus`, so a
press in the picker keeps the keyboard on the field, and opening focuses the
field when it does not already hold it. With `close_on_click_outside: false`
an outside press reaches the page under it and still closes the dropdown by
moving the focus. `DropdownMenu`'s dismiss-only entry is not told. Push a key entry at **open** and release it at close
and unmount (`overlay_dismiss::arm_keys_while_open`). `MockDomDocument` now
records `focus_element` and answers it from `active_element`, so a component
test has to focus the owner before its keys reach an entry.

**Tab is contained by an open overlay** (`trap_focus`, #474). `Modal`, `Drawer`
and `Popover` stamp **`data-trap-focus`** on their root while open and **remove**
it when closed, and both backends read it: desktop's
`RinchApp::tab_trap_root` starts `collect_focusable_nodes_from` at the trap so
`handle_tab`'s existing wrap becomes a wrap *inside* it, and `rinch-web`'s
keydown listener collects the trap's focusable descendants and calls `focus()`
itself. Which trap: the nearest one the current claim sits inside, else the
**last** visible one in DOM pre-order — so nesting resolves inside-out and no
`z-index` is read, matching the dismiss stack's LIFO. Two guards, and a fixture
each, because a closed `Modal` satisfies both: the attribute is removed (hence
`is_boolean_attribute`, so `write_attribute` removes rather than writing
`"false"`), **and** a trap with no box is skipped. **Only Tab is contained** — a
click or a scripted `focus()` outside still moves focus out, which is a
*non-modal* `<dialog>`'s behaviour; `showModal()` inerts the page and refuses
both, and rinch models neither. On the web the **browser** is the
authority on focusability, not `trap_focusables`' selector: `handle_trapped_tab`
checks `activeElement` after each `focus()` and steps on when the browser
declines, because a filter cannot be closed over `<fieldset disabled>`, a
`tabindex` the browser parsed differently, or `inert`. A trap whose every
control the browser refuses therefore takes the **empty-trap** path — key
consumed, focus unmoved — not an endless retry. `register_focus_target`'s
`on_key` is the obvious-looking route and silently does nothing: the arbiter
offers a key to a registered target only while it holds `FocusTarget::Node`, and
the focused element inside a dialog is normally an `<input>`.
`crates/rinch/src/app/trap_focus_tests.rs` and
`crates/rinch-web/tests/trap_focus.rs` are the pins — twins, not shared code,
since the focusable set is a tree walk on one backend and a CSS selector on the
other.

**`trap_focus` also moves focus in and gives it back** (issue #695) — the half
#474 deferred, and the same prop, because `trap_focus` is rinch's spelling of
"this overlay is modal". Opening remembers whatever holds the keyboard and
focuses inside: an `autofocus` descendant wherever it sits, else the first
focusable (`Modal`/`Drawer`) or nothing at all (`Popover`, matching the HTML
popover API rather than the dialog). Closing — and unmounting while still open,
which `if show { Modal { … } }` does — hands it back if that element is **still
connected**, and otherwise releases the claim rather than focusing into a
detached subtree; focus the user moved outside the overlay is left alone. The
memory is per-overlay, which is the whole of nesting: the inner remembers what
the outer focused. `rinch-components`' `overlay_focus::arm_overlay_focus` is the
policy and it is the *only* copy; what it rests on is three `DomDocument`
methods — `active_element`, `focus_into`, `restore_focus` — reachable on any
`NodeHandle` and defaulted so `MockDomDocument` keeps compiling. There is no
bare `blur()`: releasing the keyboard is only ever the fallback half of a
restore, and the decision belongs where the facts are. **Desktop resolves both
overlay calls a layout later**, parked as a `rinch_core::FocusRequest` in the
same single slot `request_focus` has always used, because an overlay opening or
closing is a class change in the same effect flush and every node inside it
still carries a zero-size box. `FocusRequest::needs_layout` says which kinds
that applies to, and **a consumer that has not run a layout re-parks them** —
the two synchronous drains after `dispatch_event` (`click_handling` and
`activate_focused_node`) otherwise consumed the slot and lost the move for good,
which is every overlay opened by a click or by Enter. `rinch-web` answers on the
spot and lets the browser refuse what it should refuse — after `NodeHandle`
has released its document borrow and before the call returns
(`DomDocument::take_after_borrow`), because a browser `focus()`/`blur()` runs
every focus listener synchronously and rinch's own listeners touch the document
(an editor's caret on `blur`, the key entries on `focusin`); under the borrow
they panicked "RefCell already borrowed" inside the listener, which aborted it.
**A `NodeHandle` verb that takes the focused element out of place lets the
focus go first** (#1478): Chrome fires `focusout`/`blur`/`change` inside
`removeChild`, a move, `innerHTML =` and `textContent =` over a focused node,
so `remove`, `discard`, `remove_child`, `replace_with`, `set_inner_html`,
`set_text` and the insertion verbs (for a child that is already somewhere) ask
`DomDocument::release_focus_within(node, children_only)` with no borrow held
(`rinch_core::dom::release_focus_within`, up to `FOCUS_RELEASE_ROUNDS` times,
since a listener may focus something else in there) and `rinch-web` answers a
`blur()` of the focused element inside the part that goes; then the verb
borrows and mutates as before. The last round's blur is made with the
departing node held `inert`, so a listener that refocuses in there every time
(a field that refocuses itself on blur) cannot: run under the removal's borrow
it was aborted with the runtime's flush guard set, and later signal writes
stopped reaching the DOM. The verb does not ask for a mutation the backend
will refuse (`can_take`: a reference that is not the parent's child, a node
into its own subtree), which moves nothing and keeps the focus. The ask is
free while nothing is focused: `FOCUS_MAY_BE_HELD` (`web_document.rs`, raised
by `focusin`, lowered by a `focusout` that leaves `activeElement` at `<body>`)
answers with no JS call, and a text write to a text node asks only `nodeType`.
A modified field's `onchange` runs from that blur only while its handler is
alive: a field the hidden branch built has had it freed first (#141 order). The listeners see the node still in place and
not yet retired, the mutation and the document's maps stay one synchronous
step, and Firefox, which fires nothing for a removed node, gets the same
`focusout`. **A move inside the document keeps the focus where the browser has
`Node.moveBefore`** (#1483), as desktop always has (its focus is the arbiter's
own state, keyed by node): `WebDocument::append_child` / `insert_before` /
`insert_child` call it through `insert_or_move` when `moves_in_place` holds
(the method exists, looked up once per page; child and parent both connected,
one document; the parent an element, the child an element or character data),
and the `NodeHandle` insertion verbs ask the same predicate through
`DomDocument::moves_keeping_focus` (default `false`) and release nothing — the
two must stay one predicate, or the old call runs listeners under the borrow.
Measured in Chrome 153, and in CI's Firefox 157: no `blur`/`focusout`,
selection kept, an open `Select`'s list and key entry live, an `<iframe>` not
reloaded; a moved scroller's `scrollTop` is kept in Chrome and reset in
Firefox 157. A browser without the method, a move into or out of a detached subtree,
`replace_with`'s replacement and a refused `moveBefore` take the blur-first
`insertBefore`, where the moved node loses the focus. The looked-up method is
accepted only if it throws for two detached scratch elements
(`refuses_a_disconnected_node`): a page's polyfill over `insertBefore` does
not, and used as a move it fired `blur` under the borrow with nothing
released. Also kept across the move in Chrome 153: a running CSS animation or
transition, and an IME composition. Not kept: a node moved under a parent
where it cannot be focused (`display: none`, `hidden`, `inert`, …) is blurred
by the browser after the call, outside the borrow. Cost: one `isConnected`
read per insertion of a node that is in no document (a mount); a move reads 7
(`isConnected`, `nodeType` and `ownerDocument` twice each, `isSameNode`), and
7 more from the verb's ask when the focus is inside the moved node. Pins: `rinch-web/tests/move_keeps_focus_1483.rs` (branches
on the feature, not the engine), `review_1486_polyfill.rs`, and
`review_1486_no_move_before.rs`, which deletes the method before the first
lookup: CI's Chrome and Firefox both have it, so no other file runs the old
path. Chrome also blurs inside three attribute writes on the focused
element itself — `hidden`, and `tabindex` / `contenteditable` removed from an
element focusable only through it (measured in 153; `disabled`, `inert`,
`display: none` and any of them on an ancestor fire nothing inside the write)
— and only the browser knows which write is one, so for those names
(`is_focus_revoking_attribute`) `NodeHandle::set_attribute` /
`remove_attribute` first run `DomDocument::before_attribute_write`, which on
the web is the literal browser write made outside the borrow; the write under
the borrow repeats it and changes nothing. `RootHandle::unmount` releases the
focus itself (`NodeHandle::release_focus_before_detach`). **A direct
`DomDocument` caller gets none of this** (`UpdateBatch::apply` is one), and a
new `WebDocument` method whose browser call can unfocus an element needs one of
the two hooks. Pins: `rinch-web/tests/focused_node_displaced_1478.rs`.
"Still there" means
**can still take focus**, not merely attached: a `disabled` opener, or one
inside an outer overlay closed first, releases the keyboard instead. Identity is
not checked, so a recycled node id (issue #304, live on desktop through
`NodeTree::remove_subtree`) would still be focused.
`crates/rinch/src/app/overlay_focus_tests.rs` and
`crates/rinch-web/tests/overlay_focus.rs` are the pins, twins like the
containment pair. **A focused rich-text editor is where its container is**
(#1126): the web editor holds the keyboard through the capture
`<textarea data-pm-capture>` in `<body>`, so every rinch-web focus question
reads it through `editor_input::keyboard_editor` / `effective_active_element`
— `WebDocument::active_element` answers the container, `restore_focus`
releases an editor inside the closing overlay (`release_editor`, the
`EditorHandle::blur` path) before restoring, and `release_focus_within`
releases one inside the part that goes. Desktop's arbiter mirrors an
`FocusTarget::Editor` claim into `NodeTree::keyboard_editor`
(`RinchApp::sync_keyboard_editor`, at every `focus_target` write), which its
`active_element` answers. So an overlay opened from an editor gives the
keyboard back to it on close, on both backends (`try_focus_input` /
`give_editor_keyboard`). Tab containment never sees a Tab from an editor (its
keymap consumes Tab) and reads the raw `activeElement`. Pins:
`rinch-web/tests/overlay_editor_focus_1126.rs`, `overlay_focus_tests.rs`'s
section 7.

**Tab walks the HTML sequential order** (issue #435): positive `tabindex`
ascending, ties in tree order, then every `0` / tag-focusable stop in tree
order — `RinchApp::tab_sequence`, a stable sort over
`collect_tab_stops_from`'s tree-ordered stops, applied inside a trap as well
(rinch-web's `handle_trapped_tab` sorts its ring the same way, by
`tabIndex`). The collection itself (`collect_focusable_nodes_from`) stays in
**tree order**, because `focus_into_subtree` picks a dialog's first control in
tree order, as HTML and Chrome do. A start that is not a stop — a focused
`tabindex="-1"` node (even one nested inside a stop), or a claim whose box just went — resumes at its **tree**
neighbour (next stop after it for Tab, previous for Shift+Tab; with nothing
after it, the first stop at `0`), all measured in Chrome 153 and pinned by
`app/tab_order_tests.rs` and the #435 fixtures in `rinch-web/tests/trap_focus.rs`.

**Scroll into view moves every scroll container, the page last** (#842).
Tab focus, the editor's caret and reveal probes, `<select>`'s and `Tree`'s
keyboard highlight and any app all go through `NodeHandle::scroll_into_view`
(and `scroll_to_fraction`); the web answers with the browser's
`scrollIntoView({block: "nearest", inline: "nearest"})`, desktop queues the
request and `RinchApp::apply_scroll_into_view` applies it after the layout
through `app/scroll_into_view.rs`, which does what CSSOM View and Chrome do.
It walks the target's DOM ancestors, and each one that `scrolls_x()` or
`scrolls_y()` (so not a `display: contents` box, #1038, and not
`overflow: hidden` — Chrome does scroll a hidden one, rinch does not, #2013) brings the
target's **painted** box into its scrollport, the **padding** box
(`clientTop`/`clientHeight`), each axis `nearest`: inside or covering it moves
nothing, otherwise the near edge is aligned (the start edge for a box larger
than the scrollport sticking out past the end). Each scroller further out
brings the target *as the inner ones have just placed it*, **clipped to their
scrollports** (Chrome's `ScrollRectToVisible` intersects as it climbs): not
the scroller around it, which can be taller than the window — measured,
Chrome scrolls the page to the target there, not to the scroller's top. The
page scroller is `<body>` (the UA sheet's `overflow-y: auto`). The walk stops
above a `position: fixed` box (`box_position()`): nothing outside it moves it.
A placed request (`scroll_to_fraction`) is placed in the first ancestor that
scrolls vertically and is `nearest` everywhere else, as the web backend's
`request_scroll_to_fraction` does. A scroller is marked scrolled (paint-dirty,
hit cache) only when its offset changed. Until #842 desktop moved the nearest
`overflow-y` scroller only, vertically only, from its border-box top against
its content-box height (#1367): a caret in a scroller below the fold of a
scrolling page was scrolled into its scroller and left off screen. Not
modelled: the containing-block chain (an `absolute` box is walked through its
DOM ancestors, so a scroller between it and its containing block is moved
although it cannot move the box), and smooth scrolling (neither backend
animates it). Pins: `app/scroll_into_view_nested_842_tests.rs` (Chrome 156,
two and three levels of padded, bordered scrollers in a scrolling page, both
axes, a placed target, a scroller taller than the window, a fixed boundary,
#1367's padded rows), `app/mod.rs` `scroll_into_view_tests` (transforms), and
`editor_scroll_gate_tests::nested_scrollers_the_outer_one_follows_too`.

**`data-nofocus` takes the click without the keyboard** (issue #312) — the
`preventDefault()`-on-mousedown mechanism browsers converged on, which an editor
toolbar needs so Bold does not blur the editor it acts on. Same boolean rule as
`data-disabled` — including the `"false"` escape, which the rinch-owned
attributes keep and the HTML pair does not — read **anywhere on the pressed
node's ancestor chain** so a toolbar carries it once; it protects whatever holds
the keyboard (editor, input, surface, node), the `data-rid` click still fires,
and a form control *inside* the
region (any `<input>`, `<textarea>`, `<select>`, editable content — a checkbox
too) still focuses normally (the web since #1579). Both backends — on web it
becomes `preventDefault()` on the `pointerdown`, skipped when the press's
nearest focusable is a form control strictly inside the region.

A custom component that takes keyboard input registers for the lifecycle
(issue #147, `crates/rinch/src/focus_registry.rs`, in the prelude):

```rust
register_focus_target(
    &node,
    FocusEntry::new()
        .on_focus_gained(move || focused.set(true))
        .on_focus_lost(move || focused.set(false))
        .on_key(move |k| k.key == "ArrowDown"), // true = consumed
);
```

- Keyed `(doc_key, node_id)` like the editor registry (#134); **deregistered by
  the ambient scope's `on_cleanup`**, so unmounting is **silent** —
  `on_focus_lost` never fires after disposal (that would read freed signals and
  panic, #141 PR4). The registry is the arbiter's liveness authority for
  registered nodes, which closes the recycled-slot window (#304) for them:
  a node `set_inner_html` frees while the registering scope is alive takes its
  entry with it before the write, so a node minted on that id is not a
  registered target. If the freed target held the claim (whatever callbacks it
  registered), the arbiter releases it at the next `AboutToWait` (or an
  earlier key or IME dispatch), even when the markup re-minted a focusable
  node on the id, and its `on_focus_lost` still fires, deferred —
  unless its component was disposed meanwhile, which stays silent (#1509 — the focus registry registers a
  `rinch_core::dom::FreedNodesListener`, which `set_inner_html` calls with the
  replaced subtree's ids, walked only while some listener holds an entry; a
  new node-keyed registry outside `rinch-core` can do the same). The scope's
  cleanup itself releases by token, so it never drops a later registration on
  a re-issued id (#1490).
- Both focus callbacks run **after** the transition completes (deferred through
  the same `PendingFocusWork` mechanism as a blurred input's `data-onchange`),
  so they may re-enter the runtime freely.
- `on_key` is offered before the runtime's own handling; `true` consumes. It
  gets `k.key` spelled as `KeyboardEvent.key` on every backend — the spacebar is
  `k.key == " "`, `k.code == "Space"`, and `k.is_space()` answers it (#1161;
  desktop said `"Space"` until then, so match `code` or `is_space()`). It
  sees **releases too** (#337): `k.kind`/`k.is_up()` tell the phases apart
  (auto-repeat is a `Down`, and `KeyEventData` still does not tell it from a
  fresh press — the *platform* event does, see **Key auto-repeat** below).
  A press and its release are spelled
  by the same rule from the same fields, so pairing them by `k.key` works by
  construction — a release carries no text and resolves through `logical_key`,
  which `PlatformEvent::KeyUp` now carries for exactly that reason.
  `logical_key` is `Option<String>` holding the full **case-accurate**
  `KeyboardEvent.key` value (`"A"` under Shift, `"!"`, `"Enter"`, `"Dead"`;
  it was a lowercased `Option<char>` letter, which made a shifted release
  disagree with its own press) — lowercase at the comparison site, as
  `editor_key_binding` does, never at the source. A release's
  return value is ignored (nothing downstream to suppress, and the activation
  latch must clear regardless). `KeyEventData` is `#[non_exhaustive]` — build
  one with `KeyEventData::new(key, code)` plus `with_modifiers`/`with_kind`.
  `set_keyboard_interceptor` is unrelated — a document-level capture-phase hook
  dispatched *before* the arbiter. **It is the wrong registry for Escape**
  (#474): it is one slot per document, so a second overlay registering there
  disables the first and its unmount clears the slot rather than restoring what
  it displaced. Escape goes through the **dismiss stack** instead —
  `rinch_core::push_dismiss_handler(doc_key, || bool) -> DismissHandle`,
  LIFO, per document (`doc_matches`: only two differing `Some` keys are
  refused, so a backend that marks none — rinch-web — reaches every handler),
  owner-checked at dispatch, dispatched from inside
  `dispatch_keyboard_event` for an Escape *press* after the interceptor (so
  both backends get it with no edit). `Modal`/`Drawer`/`Popover`'s
  `close_on_escape` rides it, and so does the **DOM menu bar**, which is the
  stack's first non-component member and registers on both backends from one
  `build_overlay`; a custom overlay should too — **and there are two
  registration policies, not one** (#465). Those three register at *mount* and
  answer `opened_fn` at dispatch, because `render` runs once and a closed
  overlay stays mounted. That is wrong for an overlay **statically nested inside
  another one**: a component renders *after* its children, so
  `Modal { ColorInput { … } }` pushes the input's entry first and the modal's on
  top, and the modal answers Escape while the picker is what is on screen. An
  overlay that owns its open state pushes at *open* time instead and releases on
  close — the `<select>` popup's shape (#671), spelled for components as
  `rinch_components::overlay_dismiss::arm_close_on_escape_while_open` and used by
  `ColorInput`'s dropdown (whose outside-click backdrop is `DropdownMenu`'s,
  `position: fixed` and all). Such a handler has no closed state to decline
  in, since it exists only while the overlay is open, but it declines while its
  root is not rendered (`NodeHandle::is_rendered`: connected, no `display: none`
  on it or an ancestor, not `visibility: hidden`/`collapse`; #802) — a picker
  left open inside a `Modal` the app closed (`display: none`, still mounted)
  used to swallow the next Escape. It is declined, not closed. Desktop answers
  from the last resolve's styles, the web from `getComputedStyle`; the mock
  answers `true`. Its release has to cover unmount-while-open as well as close. It shares the
  *lifetime* rule though (#183):
  registering it during a render releases it on unmount, ownerless registration
  keeps app lifetime, and an earlier unmount never clobbers a later
  registration. Same for `set_paste_interceptor`, `set_selection_callback` and
  `set_selection_sync_callback`; the discipline is
  `rinch_core::reactive::install_doc_scoped_slot` / `clear_doc_scoped_slot`, and
  any new **document-level** callback registry should go through those rather
  than paraphrase them — all four of the callbacks named above do. The
  thread-scoped pair `install_scoped_slot` / `clear_scoped_slot` is the same
  discipline over a plain `RefCell<Option<Rc<T>>>`, for a registry that is
  genuinely per-thread rather than per-document; reaching for it when you wanted
  the doc-scoped one gives two `RinchContext`s on one thread a single shared
  slot, last-registration-wins for both, which is the #134 class of bug.
  A registry written *repeatedly* from a live component (or one that wants the
  callback attributed to its component when it runs) takes the other template
  instead — owner beside the callback, `is_alive()` at dispatch, invoked inside
  `owner.run(...)` — as `main_thread::park_main_callback`, `rinch-ws`'s
  `HANDLERS`, the menu registry and `rinch-android`'s sensor / location /
  lifecycle / activity-result registries all do.
- **Window blur notifies but retains**: `PlatformEvent::WindowFocus(false)`
  fires `on_focus_lost` and keeps the claim (releasing would fire
  `data-onchange` on every alt-tab, #226); refocus re-fires `on_focus_gained`.
  `ime_state()` reports disabled while blurred.
- **IME rides the same claim** (#176): adding `.on_ime(|e: &ImeEvent| …)`
  declares the target a *text* target, so `ime_state()` switches the platform
  input method on for it and every `ImeEvent` is routed to it, exactly like the
  editor and `<input>`. Without `on_ime` a focusable node drives no IME (a card
  must not pop a candidate window). `.caret_rect(|| Some((x, y, w, h)))` places
  the OS candidate box in **logical window pixels** (not physical — the shell
  passes it to winit as a `LogicalPosition`), and is re-polled every event-loop
  iteration so it follows the caret. The runtime fabricates no events: a focus
  change is *not* an `ImeEvent::Disabled`, so clear your preedit in
  `on_focus_lost`. `ImeEvent::DeleteSurrounding` stays inert on desktop
  (`sync_ime` requests only `with_cursor_area()`).
- Not yet: the Android soft keyboard for a registered target (the shell still
  watches for a focused `<input>`/editor). **Tab containment does work** — see
  `data-trap-focus` above — as do *dismissal* (the dismiss stack) and the
  focus move/restore (#695). What remains unmatched is **modality**: a click
  still reaches a control the backdrop does not cover, and a click — or a
  scripted `focus()` — outside an overlay moves focus out of it. That matches a *non-modal* `<dialog>`; a browser's
  `showModal()` inerts the rest of the page and refuses even a scripted focus
  behind it (measured, Chrome 150), which rinch does not model.
- **`lock_scroll` gates the gesture on desktop and the style on web** (#474).
  `Modal`/`Drawer`'s prop reaches
  `DomDocument::set_scroll_locked(locked, root)` through
  `NodeHandle::set_scroll_locked`; it takes the overlay's **root node**, not
  just a bool, because desktop has to know which subtree may still scroll.
  Desktop records the locking roots on `NodeTree` and refuses a container
  outside every one of them (`NodeTree::scroll_locked_out`) at two input sites:
  the **wheel arm** in `event_dispatch.rs` (both axes, both the ancestor walk
  and the geometric fallback — an open overlay is exactly the shape that
  fallback exists for) and **`find_scrollbar_hit`**, so the page's #178 thumb
  cannot be dragged either. Nothing is restyled: the page does not reflow and
  keeps its `scroll_offset`. Web sets `overflow: hidden` on the real `<html>`
  (page-global and counted in `web_document.rs`, previous inline value saved and
  restored), because rinch cannot gate the browser's own wheel. **The
  divergence to know:** web removes the page's *scrollbar* too, so a classic
  (non-overlay) scrollbar there shifts the layout on open; on desktop a locked
  page's bar stays painted and is simply inert. Locks are **counted** in both
  backends, so an inner modal closing over an outer one leaves the page locked,
  and `overlay_scroll_lock::arm_lock_scroll` releases on close *and* in
  `on_cleanup` — an unmount while open would otherwise wedge the page for the
  session. A scrollbar drag already **in flight** when the lock arrives is
  ended rather than left scrolling (the #189 shape, re-checked in `MouseMove`),
  and the native `<select>` popup is **exempt** through a second list,
  `NodeTree::scroll_lock_exempt` — its option panel is a `<body>` portal, so it
  is inside no overlay's root and a long list in a dialog was unscrollable.
  Exempt is a separate list because `scroll_lock_roots` is also the count:
  putting the popup there would freeze the page whenever a `<select>` was open.
  `ContextMenu` is the other body portal: since #1761 its
  `.rinch-context-menu__frame` (the window less a 4px margin, which scrolls a
  menu taller than the window) carries the attribute below. **The portable spelling of the
  exemption is `data-scroll-lock-exempt`** (#701,
  `rinch_core::events::SCROLL_LOCK_EXEMPT_ATTRIBUTE`): `push_scroll_lock_exempt`
  is a `NodeTree` method, reachable from the runtime and not through
  `NodeHandle`, so a scroll container built by a *component* or an app and
  sitting outside the locking overlay carries the attribute instead — on itself
  or on an ancestor, since `scroll_locked_out` reads it on the same ancestor
  walk (and only there, so only while a lock is held). A mark on a node that
  **contains a locking root** — `<body>`, a wrapper around the overlay — is
  ignored (`NodeTree::holds_scroll_lock_root`, one parent walk per root when a
  mark is met), or it would reach the page behind the overlay and switch the
  lock off (review of #1443: the page moved 700). The mark is not checked
  against paint order, so a marked scroller *behind* a modal's backdrop still
  scrolls through the wheel's geometric fallback
  (`a_marked_panel_behind_the_backdrop_still_scrolls` pins it). It goes with its node, so
  there is nothing to release. It is the fifth rinch-owned `data-` boolean
  (`data_attr_is_on`: on unless `"false"`; a falsey reactive `bool` removes it)
  and has no web reader, because the web's lock stops no inner scroller. The DOM
  menu bar stamps it on each dropdown and each submenu flyout (a flyout is the
  dropdown's sibling, not its descendant): they are window chrome, in no
  overlay's root, and paint over an open `Modal` in the wrapped layout (bar 201
  against 200) and in the borderless titlebar layout (both 200, the layer later
  in the document), so a menu past `95vh` could not be scrolled while a
  `lock_scroll` overlay was open. Pins: section 8 of
  `app/overlay_scroll_lock_tests.rs`. That is the wheel; the dropdown's
  scrollbar *thumb* cannot be grabbed with or without a lock (#1441:
  `find_scrollbar_hit_node` prunes a subtree at an ancestor's own box, and the
  dropdown hangs below its 28px bar). What the lock does **not** gate:
  programmatic scrolling (`set_scroll_top`), and keyboard page-scrolling, which
  desktop does not have at all. A touch scroll and the MCP `scroll` tool both arrive as
  `PlatformEvent::MouseWheel`, so they are gated. `RenderScope::body_handle()`
  is the trap the design avoids — on web it is `<div id="rinch-body">`, not the
  page.
- **An open `<select>` popup joins the dismiss stack** (#671). It is handled by
  the arbiter, which is step 2, while the stack is inside step 1 — so once
  `close_on_escape` started working, a `<select>` inside a `Modal` lost Escape
  to the modal. It now pushes its own entry when it opens (`open_select_popup`,
  released at `remove_select_popup_nodes`, the one place `open_select` becomes
  `None`), and since the popup opens *after* the modal mounted, LIFO puts it on
  top with no precedence special case anywhere. A dismiss handler is an
  `Fn() -> bool` and closing the popup needs `&mut RinchApp`, so the handler
  only sets a flag and consumes; `handle_event` drains it the moment
  `dispatch_keyboard_event` returns — the `PendingFocusWork` shape. On that path
  the flag cannot be left set: only a handler that returns `true` sets it, and
  `true` is exactly when the drain site runs. That holds for the Escape path,
  not for the flag as such — `dispatch_dismiss` is public and a shell calling it
  for another gesture (Android Back) would set the flag with nothing to drain
  it, so a new caller has to drain it the way `handle_event` does.
- **Web has no arbiter** — `register_focus_target` is desktop/Android/embed
  only; use a real `tabindex` and the DOM's own `focus`/`blur` there.

**A right press on a text target opens a built-in Cut / Copy / Paste / Select
all menu** (#813) — an `<input>` of a text-like type, a `<textarea>`, or the
rich-text `Editor`; not a checkbox, a plain focusable node or a render surface.
It is a runtime-built DOM overlay in the `<select>` popup's shape
(`app/text_context_menu.rs`): body-portal nodes, a dismiss-stack entry pushed at
*open* and released on close, closed by any outside press (swallowed), a wheel,
a window blur, a focus move, or the field leaving the document (checked on
every event). A live `data-oncontextmenu` on the field or an ancestor wins and
the menu stays shut. Each item runs **the chord's code** — `handle_cut` /
`handle_copy` / `handle_paste` / `handle_select_all` for the editable fields,
`editor_cut` / `editor_copy` / `dispatch_editor_paste` / `selectAll` for the
editor — and `text_context_menu_tests` pins item and chord to identical value,
selection and clipboard. Enabled states: Cut = selection && writable &&
not `password`; Copy = selection && not `password`; Paste = writable, **never
decided by reading the clipboard** (#149 — so it stays enabled over an empty
clipboard); Select all = content. **All three clipboard rows — Cut, Copy and
Paste — are greyed when `rinch` is built without the `clipboard` feature**:
there is no clipboard for Copy or Paste to reach, and an enabled Cut would
delete text it never copied — and the Ctrl+X chord and `perform_text_edit`
do nothing on a plain control then too (`handle_input_edit_command`, #823;
the editor's Cut arms were already compiled out). The press applies the native caret rule:
outside the selection it moves the caret, inside it keeps the selection. The
field keeps the keyboard throughout. The platform-neutral half is a seam for a
shell with its own toolbar: `RinchApp::text_edit_state()`,
`perform_text_edit(TextEditAction)`, `prepare_text_context_target(..)`, and
`set_text_context_menu_presentation(Shell)`, under which the gesture emits
`AppAction::ShowTextContextMenu` instead of opening the DOM menu — what the
Android half of #813 builds on.

**Key auto-repeat: the press says so** (#463). Enter/Space on a focused
`FocusTarget::Node` activates **once per physical press**, and the OS delivers a
held key as a stream of `KeyDown`s indistinguishable from real ones — so
`PlatformEvent::KeyDown` carries `repeat: KeyRepeat`, three-state
(`Fresh` / `Repeat` / `Unknown`) for the same reason `PrimaryButton` is
(#189): *a backend that cannot see is a real case and must not be made to
guess.* winit fills it on desktop, `KeyEvent::repeat_count()` on Android, the
debug/MCP channel and the `game-embed` host fill it themselves;
`Unknown` is the `Default` and falls back to `RinchApp::node_activation_held`,
the press/release latch rinch has always used.

**The latch alone was the #189 shape**: armed on the way down, cleared only by
the matching `KeyUp` — and a release that never reaches us (alt-tab while held,
a WM grab, a native menu or modal taking the keyboard, an embed host or the MCP
channel that sends no releases at all) stranded it, killing that key on that
node **for the rest of the session**, silently. A `WindowFocus(false)` clear
(landed with #147, pinned by nothing until #463) bounds it, and on Android it was
the *only* clear until #479, since that backend translated no `KeyAction::Up`.
It does now: `shell/android_key.rs` (`KeyTranslator`, host-compiled and
unit-tested, since no host test compiles `android_runtime`) turns `ACTION_UP`
into a `KeyUp` whose `logical_key` is the string **its press was reported
with** — remembered per `(device, key code)`, so Shift+A with Shift released
first comes up as `"A"` and a dead-key `"é"` comes up as `"é"`, where desktop
and the web spell a release from the modifiers held at the release; a release
whose press was never seen is spelled from the key character map. A release
never reads or clears the pending dead-key accent, which waits for the next
*press*. Text the soft keyboard commits goes through `InputConnection`
(`android_ime`) and has no key events; a raw key event an IME sends
(`sendKeyEvent`, typically Backspace and Enter, which
`RinchInputConnection.java` forwards to the activity's input queue) comes
through the same translation as a hardware key, release included — so its
release is now a `KeyUp` too, unmeasured on a device. An auto-repeat is spelled
from what it types, so after a dead key (`"é"` down, `"e"` repeats, `"é"` up)
or a Shift released mid-hold a repeat is a down no release names (#1418):
track held keys by `k.code`. But a second event can be swallowed too; a fact carried **by the
press being judged** cannot. `RinchApp::press_is_fresh` is the one place that
decides, and `Fresh` is authoritative *over* the latch — that is the repair, not
a tie-break.

Not carried to `KeyEventData`, so a registered node's `on_key` still cannot tell
a repeat from a press: the honest field is the same three-state one, and
`rinch-core` depends on neither `rinch-platform` nor the browser, so it needs a
home for the type first (**#797**).

Guide: `docs/src/guide/focus.md`.
