# Drag and drop, pointer capture, title tooltips

Read this page when your task touches: DnD attributes, Drag builder, missed releases, right/middle press, file drop, the element wheel event (`onwheel`), title tooltips.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Drag and Drop

Rinch has two drag systems: **DOM drag attributes** for element-to-element DnD, and the **`Drag` builder** for pointer capture (sliders, panel dragging, resize handles).

> **Picking the right one:** if you want **continuous per-frame tracking** (slider value, panel position, timeline scrub), use **`Drag::absolute()` / `Drag::percent()`** from inside an `onclick` handler — that's the pointer-capture system. The HTML5-style `draggable: true` + `ondragstart` + `ondragend` attributes only fire at the **endpoints** of the drag; for per-frame events on that path use `data-ondragmove` (source) and `data-ondragover` (target).

### DOM Drag Attributes

Set these attributes on elements to participate in element-to-element drag-and-drop:

| Attribute | Fires on | When |
|-----------|----------|------|
| `data-ondragstart` | Source | Drag begins |
| `data-ondragmove` | Source | Pointer moves during drag |
| `data-ondragenter` | Target | Drag enters a drop target |
| `data-ondragover` | Target | Pointer moves over drop target (every coalesced move) |
| `data-ondragleave` | Target | Drag leaves a drop target |
| `data-ondrop` | Target | Drop on target |
| `data-ondragend` | Source | Drag finishes — dropped or cancelled |

A drag is **cancelled** — `data-ondragleave` on the target it is over, then
`data-ondragend` on the source, and no `data-ondrop` — by Escape, a
`PointerCancel`, and on desktop by a left press or a window blur while it is
still live, which prove its release was missed (#1028, see the Drag Builder
section).

**Input & activation — and the two backends do NOT agree.**
- **rinch-web** drives this suite from Pointer Events and branches on the pointer kind, so activation differs by input and touch does not hijack scrolling. **Mouse:** past `WEB_DRAG_THRESHOLD`, 5 CSS px. **Touch / pen:** a short **long-press** hold, `TOUCH_LONG_PRESS_MS` = 350ms, while the contact stays within `TOUCH_MOVE_SLOP`; moving before the hold completes is a scroll/pan and the drag is abandoned. That is the standard mobile reorder gesture, and it lives in `rinch-web/src/event_delegation.rs`.
- **Desktop has no such branch.** `event_dispatch.rs` applies one flat `DRAG_THRESHOLD` (5px, `app/mod.rs`) to every pointer alike — the shell folds touch into a plain left-click and discards winit's device kind. Desktop's own touch translation is a *different* machine (`shell/touch_gesture.rs`): a finger that moves past `SCROLL_THRESHOLD` becomes a **scroll** and emits `PointerCancel` first, a finger that lifts while still is a **tap**, and one held past `LONG_PRESS_TIMEOUT` (**500ms**, not 350) becomes a **context menu** — a right-button press/release. So the mobile reorder gesture described above is a web behaviour; do not assume a touch drag of this suite behaves the same way on desktop, and treat its reachability there as unestablished rather than as working.

**A touch `data-rid` click waits for the release where the finger could pan** (rinch-web, #1582, #1588): a touch or pen press dispatches on `pointerup`, and is dropped by a move past the slop or a `pointercancel`, when the element or an ancestor scrolls **or the page does** (`has_scrollable_ancestor` asks the scrolling element under the viewport's `overflow`, `<html>`'s or the propagated `<body>`'s; `<html>` itself computes `visible`) — unless `touch-action: none` sits on the element or between it and that scroller, where the click stays on the press so a handler can arm a `Drag` (`Slider`, `ColorPicker`'s areas, `FloatingPanel`'s header and resize handle carry it; a control that drags needs it). A `Drag` armed by a deferred click is ended at the release. The deferred `pointerdown` is `preventDefault`ed like the eager one (not for a form control, which the browser focuses), or Chrome sends a compatibility `mousedown` after the tap to whatever the handler put under the finger, which blurred a `Modal`'s field. Real touches are pinned by `rinch-web/tests/real_touch_1582_1588.rs` through `tests/support/run_real_keys.sh` (CDP `Input.dispatchTouchEvent`); synthetic ones by `touch_deferral_1582_1588.rs`.

Because there's no built-in drag ghost (the app renders its own from `data-ondragmove`), **set `pointer-events: none` on your ghost element** — on touch the drop target is resolved via `elementFromPoint`, so a ghost under the finger would otherwise intercept the hit and drops would silently fail.

**A drag auto-scrolls the container it is near the edge of** (both backends). While the drag is active and its pointer is within `DRAG_AUTO_SCROLL_EDGE` (32px; a third of the box for one shorter than 96px) of the top or bottom of a scroll container under it, that container scrolls, at up to `DRAG_AUTO_SCROLL_MAX_SPEED` (1200 px/s) on the edge, falling off with the square of the distance into the band (`rinch_core::events::drag_auto_scroll_velocity`, one curve for both). It keeps scrolling while the pointer rests there, a frame at a time: desktop from the `AboutToWait` frame clock (`RinchApp::tick_drag_auto_scroll`, which asks for the next frame while it is in a band), rinch-web from `requestAnimationFrame` (a loop that a move of an active drag starts and that stops itself out of every band and on the drag's end). The container is the nearest ancestor of what is under the pointer (or the body / scrolling element) that overflows vertically, has the pointer in a band and can still move toward that edge, so a list at its end lets the page around it take over — "its end" is where a write moves the offset no further: on desktop the range's end on the device pixel grid (#1543), so at 125% a range ending off the grid is at its end when the offset cannot move any further, and the clock rests there; on the web a frame whose `scrollTop` write moves nothing marks that container at its end in that direction (`AutoScrollLoop::stuck`; a pointer move clears it), because `scrollHeight` and `clientHeight` are rounded and at DPR 1.5 a 200.4px list stops at 599.33 where they say 600 (CI runs `drag_auto_scroll` and `review2_1592` a second time in Chrome at `--force-device-scale-factor=1.5`). While a `lock_scroll` overlay holds the page, the page and every container outside the overlay's root are skipped on both backends, as desktop's `NodeTree::scroll_locked_out` decides (`data-scroll-lock-exempt` included); rinch-web records the locking roots for this (`web_document::scroll_locked_out`), since a `scrollTop` write ignores `<html>`'s `overflow: hidden`. The web lock is page-wide, so with two islands on one page an overlay open in one locks out the other's lists as well, where desktop's lock is per document. Closing the overlay during a drag scrolls a list under the still pointer from the next frame on both backends (rinch-web restarts its loop from the unlock). Bands are measured on the part of the box inside the viewport, not on the part an outer scroller clips: a scroller partly scrolled out of another keeps its band at its own hidden edge. A drag frame whose pointer is in no band costs no hit test on desktop: the tick reuses the hit the drag's last resolve (or the tick's own last fresh hit test) made while the hit cache's generation and the pointer are unchanged and nothing animates. A drag whose release went missing scrolls while its pointer rests in a band until it is ended — on desktop by the missed-release heal (the next left press or a window blur cancels it, #381/#1028), on the web by its next move reporting no button. After a frame that scrolled, the drag is resolved again at the still pointer, so the drop target scrolled under it hears `data-ondragenter`/`data-ondragover` (and the container its `data-onscroll`). Vertical only. The wheel and the auto-scroll share one vertical scroll write (`RinchApp::scroll_vertically_by`). Pins: `rinch/src/app/drag_auto_scroll_tests.rs`, `rinch-web/tests/drag_auto_scroll.rs`.

Handlers can read `get_click_context()` for cursor position and element bounds. Use `DragContext<T>` to pass typed data between source and target:

```rust
let drag = DragContext::<MyItem>::new();

// In source's ondragstart:
drag.set(item.clone());

// In target's ondrop:
if let Some(item) = drag.take() {
    target_list.update(|list| list.push(item));
}
```

### Pointer Capture Drag (Drag Builder)

For tracking mouse movement from a click handler until mouseup (sliders, panels, resize):

```rust
// Absolute pixel coordinates (panel dragging)
let ctx = get_click_context();
let offset_x = ctx.mouse_x - panel_x.get();
Drag::absolute()
    .on_move(move |x, y| panel_x.set(x - offset_x))
    .on_end(move |x, y| save_position(x, y))
    .on_cancel(move |_x, _y| restore_position())  // teardown; on_end does NOT fire
    .start();

// Percentage 0.0–1.0 (sliders) — reads element bounds from ClickContext automatically
Drag::percent()
    .on_move(move |px, _| slider_value.set(px * 100.0))
    .start();

// Cancel: fires on_cancel (with the last on_move coords) but NOT on_end —
// a cancelled drag must not commit. The web backend calls this on pointercancel
// (touch-scroll takeover, system gesture, pointer-capture loss).
Drag::cancel();

// Check if active
Drag::is_active();
```

**Pointer moves are coalesced, and a drag lays out once per frame.** The
desktop shell keeps only the newest `PointerMoved` winit reports and hands it
to the app before the next other event (so a press is judged where the pointer
last was) or in `about_to_wait` — the browser's one-`pointermove`-per-frame
model. `on_move`, `data-ondragmove`/`data-ondragover`, `onmousemove` and hover
all see the latest position once per batch, not every raw report. The drag arm
of `MouseMove` no longer calls `resolve_and_repaint`: `on_move`'s signal writes
patch the DOM, and the layout runs once at `AboutToWait` or in the paint
preamble. `perf_stats_tests::queued_drag_moves_lay_out_once` pins it (5 moves,
1 layout; it was 5). The layout is **owed**, not skipped
(`RinchApp::drag_layout_owed`): `handle_event` settles it before any event
other than a move, so a release, press or wheel delivered in the same batch as
the last move — the coalescer flushes the move right before it, and so do an
MCP `mouse_move` + `mouse_up` in one wake and an embed `update(&[move, up])` —
is hit-tested against the box where that move put the dragged element.
`a_release_right_after_a_drag_move_sees_the_moved_box` and
`tests/embed_drag_release.rs` pin it. A fast sweep also skips the drop targets
it crossed between two batches, so an intermediate `data-ondragenter` /
`data-ondragleave` pair never fires, as in a browser. Embed and Android feed moves themselves and are not
coalesced, but get the once-per-frame drag layout.

**A drag belongs to the document that armed it (issue #139).** `Drag` state is a
single thread-local, but a thread can pump several documents' pointer streams
through it — a desktop window and its DevTools panel are two `RinchApp`s on one
thread, and so are two embedded `RinchContext`s. Only the document whose events
armed the drag drives it: another document's `MouseMove` does not reach
`on_move`, its `MouseUp` does not fire `on_end`, and `Drag::is_active()` answers
`false` there (so a drag in one window never freezes hover in the other). A drag
armed **outside** any document — from a timer armed in `main`, a menu
callback, or on rinch-web, which has one page-wide pointer stream — belongs to
no document in particular and stays drivable by anybody. Nothing changes for a single-window
app. **An effect counts as its document's code** (issue #295): the effect queue
is thread-global, so a write in document A's handler flushes document B's
effects inside A's `handle_event` — but every `Effect` and `Memo` records the
document current at its creation (`RinchApp::mount_component` marks the mount,
`handle_event` the dispatch) and re-enters it around every run, so a drag armed,
or an interceptor registered, from B's effect belongs to B. **So does a timer,
a parked continuation (an `rinch-http` completion), a `run_on_main_thread`
closure and a `rinch-ws` callback** (issue #963), **and a menu, tray,
hotkey, notification, focus-target or `rinch-android` callback** (issue
#1032): each records a `rinch_core::CallbackFrame` — the document *and the
context root* current where it was armed, queued or registered (beside its
owner, where it has one; a focus target takes its node's document) — and runs
inside `CallbackFrame::enter`, as an effect re-enters both (#136), so its
interceptor install or clear is that document's rather than reaching every
document on the thread, and a `use_store` in a timer armed in an embedded
`RinchContext` resolves that context's store; one armed from `main` or queued
from a worker thread runs under no document at the thread-global root,
wherever it is fired — a menu item made in `main` included, even when the DOM
menu bar fires it inside a document's click; in an embedded `RinchContext`
such an item resolves the thread-global store namespace, not the hosting
context's, so `use_store` there panics for a store only that context created
(build the item inside the context's component instead). One that outlives
its context — an `unowned` timer of a dropped `RinchContext` — runs in that
dead root: it reads none of the context's stores (they were cleared with it),
and a `create_store` there leaves an entry nothing clears. The dismiss stack
and the interceptors are not re-entered this way: they run at the dispatch's
root, which in embed is the thread-global one (#1665). (Two desktop *windows* do not cross-feed a plain mouse drag on their own —
the pointer is grabbed to the pressing window while a button is held — so this
matters for an embed host pumping several contexts from one event stream, and
for a drag left live past a missed `MouseUp`.)

**Arming a drag ends the live one through its `on_cancel` (issue #293)** —
whichever document armed it, the same one or another. There is one slot, and a
press that arms a drag while another is live usually means the old drag's
release was never delivered: swallowed in its own document (#189), or in
another document on the thread, since the pointer is grabbed to the pressing
window while a button is held. So `Drag::start()` ends it the #189 way —
`on_cancel` with the last coordinates it delivered, never `on_end` — before
arming the new one. It used to drop the old drag with neither callback run,
which #139 made reachable: a second document's `is_active()` answers `false`,
so it no longer held off arming. A superseded drag whose arming scope was
disposed is dropped silently (#141), and a drag armed from inside the
superseded one's `on_cancel` is superseded in turn (at most 8 cancels per
`start()`, then the leftover is dropped with a warning). Every `on_cancel` —
this one, a heal, a pointer-cancel — runs under its own drag's owner and
arming document, not the caller's. A second finger's press on rinch-web is a
press like any other, so its drag cancels the first finger's. Two *simultaneous*
pointer-capture drags — one per finger, or one per context — are not possible:
the slot holds one.

**A drag whose release was swallowed heals itself (issue #189).** A native
context menu, a modal dialog, a window-manager grab, or the pointer leaving a
non-capturing surface can eat the `pointerup` that should have ended a drag,
leaving it armed for the rest of the session and following the cursor with no
button held. On a move that reports the primary button/contact released, the
drag ends through **`on_cancel`** — not `on_end`: a release nobody saw has no
trustworthy commit position, so the honest ending is the teardown one, with the
last coordinates actually delivered to `on_move`.

The backend says which by calling `update_drag_with_button(x, y,
PrimaryButton::{Down,Up,Unknown})`; `update_drag(x, y)` is exactly the `Unknown`
form. Three states rather than a bool because a backend that cannot see the
button state is a real case: **rinch-web reports `Down`/`Up` from `buttons & 1`
and heals on the move; desktop reports `Unknown` and does not.**
`PlatformEvent::MouseMove` carries no button mask, and neither does winit's
`PointerMoved` behind it, so desktop has no source of truth on a move — and a
flag the runtime kept itself would be no help, since the missed `MouseUp` that
strands the drag is the same event that would have cleared the flag. Nothing is
ever ended on a guess: `Unknown` behaves exactly like `Down`.

**Desktop heals on the next event that proves the release was missed** (issue
#381), through `rinch_core::heal_missed_release` — the same `on_cancel` ending,
at the last move. Two events count: a **left `MouseDown`** while the drag is
live (a button cannot be pressed twice without a release in between), handled
*before* the press is dispatched so its handlers never see the stranded drag
and a drag the press arms is not the one ended — and before the built-in text
context menu can swallow it, so the press that closes that menu heals too (its
release used to commit the drag through `on_end`); and **`WindowFocus(false)`**.
The blur rule is a trade-off, not a proof: on Windows deactivation takes the
pointer capture and the release does go to another window, but on X11/Wayland
the press's implicit pointer grab still delivers it, and a blur mid-drag can
come from a transient keyboard grab (a global hotkey, a WM holding Alt+Tab) —
that healthy drag is cancelled too, and its release then commits nothing.
Accepted because it fails safe: a cancel, never a wrong commit. Before #381 the next unrelated
click's `MouseUp` ran `finish_drag` and **committed** the stranded drag's
`on_end` at that click's position. A right or middle press proves nothing (a
chord can be real) and ends nothing. The left-press proof does not ask which
button armed the drag, so a middle- or right-button drag is ended by a left
chord press (it used to be committed by that press's release), and a "grab mode" drag armed with no
button held (a shortcut, a timer) and placed with a click is cancelled by that
click, as rinch-web cancels it on its first move: arm a `Drag` from a press. Both events also release the editor's
drag-select (`registry::DRAG`, the same shape — #294). What is left: between the
swallowed release and that next press or blur, the drag still follows a pointer
with no button held — a native context menu that takes a grab without blurring
the window leaves exactly that — and only an OS query for the button state
(`XQueryPointer`, `GetAsyncKeyState`, `pressedMouseButtons`; Wayland has none)
could close it. Because the shell folds a desktop touch contact into a left
press, a second finger's press ends a drag the first finger is making, as a
second finger's drag already does on rinch-web. Every heal is document-scoped
like the rest of the drag — another document's idle pointer, press or blur
cannot tear down this one's live drag. Pins: `app/missed_release_381_tests.rs`.

**The same two events end every other press-armed desktop gesture** (#1028),
through one `RinchApp::heal_missed_release`: an element drag of the DOM DnD
suite (below) is **cancelled** — `data-ondragleave` on the target and
`data-ondragend` on the source, **no** `data-ondrop`, which is HTML's cancelled
drag and exactly what Escape and `PointerCancel` do (`cancel_active_dnd`); a
press on a `draggable` whose release went missing before the threshold is
**discarded**, not clicked; and a scrollbar-thumb drag and a read-only
text-selection drag are released, the selection kept. Before #1028 the next
unrelated click's release **dropped** a stranded element drag on whatever
target it was last over, and a stranded thumb or selection went on scrolling or
extending until some release arrived. The press heal and a `PointerCancel` share
`release_press_gestures`; they differ in the pointer-capture drag (the heal is
document-scoped, `PointerCancel` calls the unscoped `Drag::cancel`) and in
`:active` (the heal does not clear it: after a press the new press sets its
own, but after a blur nothing does, so `:active` stays on the element the
stranded press was on until the next left release — pre-existing, as before #1028).
The blur trade-off above applies to
all of them. Pins: `app/missed_release_1028_tests.rs`.

**A gesture ends on the release of the button that started it** (#1087). The
`MouseUp` arm ends the left-armed gestures — an element drag (dropped, or
clicked while still pending), a scrollbar-thumb drag, the read-only text and
editor drag-selects, `:active` — on a **left** release only; a right or middle
release while the left button is held is a chord, and it used to drop a live
element drag wherever the pointer was. `data-onmouseup` and a focused render
surface still get every release with its button. A pointer-capture `Drag` ends
through `rinch_core::finish_drag_for_button`, on the release of the button it
belongs to: the button of the last press when `Drag::start` ran, which desktop
reports for **every** press, handler or not, through
`rinch_core::note_pointer_press` before dispatching it. So a drag armed by a
right press ends on the right release, and one armed outside any press (a
timer) belongs to the most recently pressed button. rinch-web keeps `finish_drag`: a browser's `pointerup` fires only
once every button is up. A right or middle press also no longer starts,
restarts or clears a read-only text selection. Pins:
`app/mouse_up_button_1087_tests.rs`.

**A right or middle press does not click** (#1093, a behaviour change on
**both** backends). A `data-rid` (`onclick`) runs for a **left** press only, as
a browser's `click` does: a right press is a `contextmenu` (a live
`data-oncontextmenu`, else the #813 text menu on a text target, else nothing —
except on a `data-drag-window` region, which a right press still drags, since
the window drag is not a click and was left out of scope) and a middle press an
`auxclick`, which rinch has no attribute for. Desktop used to click the
`data-rid` under either — a right-click on a `Button` ran its `onclick`, and so
did an Android long press, which the touch translation delivers as a right
press — and so did rinch-web whenever no `data-oncontextmenu` was in the
ancestry (measured in Chrome 153). The press still does everything a browser's
`mousedown` does: `data-onmousedown` / `data-onmouseup` fire for every button
with it in `get_click_context().button`, focus moves as before, and the
`data-rid` still stops `handle_click_with_button`'s claim walk, so a right press
on a titlebar button does not drag the window. **An app that handled a
right-click in `onclick` must move it to `oncontextmenu`, or to `onmousedown`
reading `get_click_context().button`.**

**The one exception is a backdrop.** Dismissing an overlay on a press outside
it is a `mousedown` of any button (native menus; Mantine's `useClickOutside`),
and every dismiss backdrop is a `data-rid`, so a `data-rid` element that also
carries `rinch_core::events::BACKDROP_ATTRIBUTE` (`data-backdrop`: on unless
`"false"`, like `data-nofocus`, and removed by a falsey reactive `bool`)
runs for every button on both backends. `ContextMenu`, `DropdownMenu`,
`Select`, `Popover`, `ColorInput`, `Modal`, `Drawer` and the DOM menu bar's
overlay carry it; **a custom overlay's backdrop needs it too**, or a right
press outside it is swallowed by the backdrop and does nothing. Pins:
`app/right_press_click_1093_tests.rs`, `app/backdrop_any_button_1093_tests.rs`
(an Android long press outside a `ContextMenu` included),
`rinch-components/tests/backdrop_any_button_1093.rs`,
`rinch-web/tests/right_press_click_1093.rs`.

**A press on a `<label>` activates its control** (#866; guide: `rsx-syntax.md#labels`).
Desktop's half of HTML's label activation, which rinch-web gets from the
browser. In `handle_click_with_button`'s phase 3, a **left** press resolves
`pressed_label` (the nearest `<label>` up the box tree from the hit, `None` if
interactive content — `button`, `a[href]`, non-hidden `input`, `select`,
`textarea`, `details`, `embed`, `iframe`, `audio`/`video[controls]`,
`img[usemap]` — comes first; a `tabindex` is not interactive content, as in
Chrome 156) and its `labeled_control` (`for`: first element in tree order with
that `id` in the label's tree, kept only if labelable — no fallback; else the
first labelable descendant). After the press's own `data-rid` (if any) runs,
`activate_label_control` re-resolves the control on the tree as it is now
(Chrome resolves `label.control` after the label's listeners; a handler that
removed it forwards nothing), returns for a disabled control
(`node_is_disabled_in_tree`, fieldset included), calls `try_focus_input` (the
`focus()` path: kept selection, a `<select>` closed) and then dispatches the
nearest live `data-rid` above the control through `dispatch_rid_click` —
unless it is the node the press already dispatched. Chrome 156's sequence for
a press on a label is `click@label | focus@control | click@control`; a browser
would run a handler above both twice (the second click bubbles back), and
rinch runs it once, as rinch-web's #272 duplicate rule does. A press that
claims `data-drag-window` activates no label. rinch-web applies the same rule
(#2033): a gesture records the `data-rid`s its press ran (`GESTURE_RAN`, in
`dispatch_click_at`) and, when it suppresses the trailing click on or inside a
`<label>`, that label's `control` (`GESTURE_LABEL_CONTROL`; not when that
click was `preventDefault`ed, since the label then forwards nothing). The record
is consumed by the forward, which the browser dispatches synchronously in the
same task; an unconsumed one is cleared by a `setTimeout(0)`, one task later
(on a real click a microtask checkpoint runs after each listener, before the
forward, so a microtask clear would be too early). The forwarded click at that control dispatches its nearest live
`data-rid` if the press did not run it, and is suppressed if it did or if the
press ran nothing (an activated drag); a trailing click outside any label stays
suppressed. Web pins: `crates/rinch-web/tests/label_onclick_2033.rs`. Desktop
pins:
`app/label_activation_866_tests.rs`.

### File Drop (OS → App)

File drops from the OS use `data-onfiledragenter`, `data-onfiledragleave` attributes, and `register_file_drop_handler` for the actual drop. See the File Drop section of UI Zoo for an example. The drop handler reads the drop point and its element's painted box through `get_click_context()` (`dispatch_file_drop` sets it, #1650; logical window px, as a click's), and so do the files an editor hands back from a mixed drop. rinch-web dispatches no `data-onfiledrop` (a browser drop has no paths); its `ondropfiles` handler reads the same context (below).

**From winit to the `File*` events (#1962, `shell/os_file_drag.rs`).** Since
winit 0.31.0-beta.3/4 an OS drag is a data transfer on every platform:
`DragEntered { id, position }` (X11 gives no position until the first
`XdndPosition`), `DragPosition`, `DragDropped { id, .. }` (no position, no
paths), `DragLeft`, and the data fetched asynchronously
(`fetch_data_transfer` → `DataTransferReceived`). A drag is rejected unless
the app accepts an action. `OsFileDrag` is the state machine, with no event
loop in it: a drag counts only if it offers a URI list (`TypeHint::UriList`);
it is accepted as a copy, or a move for a source offering only that
(`set_valid_dnd_actions(id, [Copy, Move])`, the pair the beta.2 fork's Wayland
DnD accepted), its URI list is fetched once a position is known, every position
is a `FileDragMoved`, the paths arrive as one `FileHoverEnter` each at the last
position, a drop is one `FileDropped` at the last position, and a leave naming
the hovering transfer is `FileHoverCancelled`. A drop whose paths have not
arrived waits for them (fetched then if the hover's fetch failed; a fetch that
fails at the drop drops no paths); one whose paths do not come within
`DROP_TIMEOUT` (3 s: X11 logs a failed selection read and answers nothing)
ends as `FileHoverCancelled`, which takes the drop caret and the hover target
away. A new drag entering (a second seat, a quick second drag) first ends,
with one `FileHoverCancelled`, both a drag still hovering (whose end never
came) and any drop still waiting (discarded, with a warning): the app holds
one drag's state (`RinchApp::file_drag`, its caret, `file_hover_target`), so a
late drop handed over while a new drag hovered would land at the new drag's
caret and end it, and the new drag would start with the old one's state. A read that
would block or deadlock (`WouldBlock`, `Deadlock`) waits for the next
`DataTransferReceived`; any other error reads as no paths. A failure is logged
once per drag (`DragStep::Warn`); a failed hover fetch is not retried per move.
The runtime's wake (`tick_caret_blink`) includes the next drop deadline. `file:` URIs become paths with `url::Url::to_file_path`;
other URIs are left out. Positions are winit's physical px, made logical
before `OsFileDrag` sees them. So the app sees the beta.2 shape, except that
the paths now follow the entry by a moment on every platform (on Wayland they
used to come only with the drop), and a drag that hovers before they arrive
is path-less, as Wayland's whole hover used to be (`FileDrag::paths_known`).
Pins: `shell::os_file_drag::tests`, which drive `run_steps` through a fake `DragHost` as well.

**`ondropfiles` (`data-ondropfiles`, #1771)** is the cross-platform form: the
files as `rinch_core::file::LocalFile`s (`file.rs`: a name, a size, an optional
path and an `Rc<dyn FileSource>` read on demand — a path on a spawned thread
woken through its `Send` waker, a browser `File` with a `FileReader`'s single
`loadend`, bytes in memory). Registry `FILES_DROP_REGISTRY` in
`events/handlers.rs` (owner and context root re-entered, untracked, one batch).
Desktop: `dispatch_file_drop` walks to the first node carrying a **live**
`data-onfiledrop` or `data-ondropfiles` and fires each it carries, paths
first; the files are `LocalFile::from_path` (one `stat` each; a path that
fails it is left out of the files, not the paths), and the hover walk
(`find_file_drop_target`) counts either attribute. The editor's leftovers
(`editor_file_drop`'s `rest`) reach both kinds. Web
(`event_delegation::install_file_drop`): document **bubble** `dragover`/`drop`
listeners that prevent the default over the nearest element with a live
handler of **either** kind (`nearest_drop_zone`, the desktop walk's rule, so
nested zones agree) when the drag carries `Files`, dispatch when that element
has `ondropfiles`, and skip an event already prevented (so a host page's own
earlier document listener that prevents the drop silences them) — the editor's capture `on_drop` prevents every drop it
is involved in and hands the files it did not take (non-images, everything on a
read-only editor or a drop with no picture) to `dispatch_dropped_files` itself.
**The drop point** (#1822): both backends set the `ClickContext` before an
`ondropfiles` handler runs — the drop point (desktop logical window px, web
the `drop` event's client px, each the frame its clicks use), the box of the
element carrying the handler (not the child the drop landed on), the viewport,
the modifiers held and `MouseButton::Left` — on the editor-leftover path too
(web `dispatch_dropped_files` calls `set_click_context_for` with the zone).
Pins: `rinch/src/app/files_drop_1771_tests.rs`, `rinch-web/tests/files_1771.rs`;
the context: `rinch/src/app/file_drop_click_context_1650_tests.rs`,
`rinch-web/tests/drop_point_1822.rs`.

**The picker** (`rinch::file::FilePicker`, web always, native with
`file-dialogs`): `pick()` starts the picker when called (a browser needs the
user gesture) and returns a future; `open(cb)` is `pick()` in `spawn_local`.
Native: the blocking `rfd` dialog, the future already resolved. Web: a hidden
but rendered `<input type=file data-rinch-file-picker>` in `<body>` (an input
that is not rendered makes `showPicker()` throw `NotSupportedError`), one
closure on `change` and `cancel` that removes the input, takes itself off
it and drops itself (#1855: the input outlives the answer, and a later
`change`/`cancel` at a listener whose closure was freed threw "closure invoked
recursively or after being dropped"; pinned by `files_1771.rs`
`an_answered_picker_input_takes_later_events_without_throwing`), and
`showPicker()` — which throws `NotAllowedError` without user activation where
`click()` is ignored silently and no event would ever come; then the future
resolves empty and a warning is logged once — **unless** the click is being
dispatched from a touch/pen `pointerdown` (which grants no activation; #1582
runs such a click on contact when nothing can scroll): rinch-web marks that
dispatch with `rinch_core::events::set_press_without_activation(Some(pointer
id))`, and the picker (`Session::show_on_release`) opens on that pointer's
`pointerup` (activation-triggering for touch and pen), resolving empty on its
`pointercancel`. `click()` where `showPicker` is missing or refuses for another
reason, refused when `navigator.userActivation.isActive` says no. Where the
input has no `oncancel`, a `focus` listener on the window resolves empty
`FOCUS_RETURN_MS` (1 s) after the focus returns with no `change`. The future
holds the `Session` (listeners hold it weakly; its `Drop` takes the input away and every
listener off its target before freeing it). A desktop read is `std::fs::read` of a regular file only (a
FIFO or device is refused, `metadata().is_file()`), whole. Pins add
`rinch-web/tests/real_touch_picker_1771.rs` (trusted CDP touch, run by
`tests/support/run_real_keys.sh`). Playwright's Firefox grants every page
activation, so the test's no-gesture phase runs only when
`navigator.userActivation.isActive` is false.

## The element wheel event (`onwheel`)

`onwheel` is written to `data-onwheel` and registered with
`register_wheel_handler` (`Fn(&WheelEvent) -> bool`, its own `WHEEL_REGISTRY`
in `rinch-core/src/events/handlers.rs`, freed with its scope like every
handler). Both backends collect the `data-onwheel` ids from the element under
the pointer up to the root **before** calling any (a handler may change the
tree), then `dispatch_wheel_handler_chain` offers them nearest first and stops
at the first `true`, each call in its own `batch`.

- **Desktop** (`app/event_dispatch.rs`, the `PlatformEvent::MouseWheel` arm):
  one shared hit test; a render surface under the pointer takes the wheel
  first, unchanged; otherwise `offer_element_wheel` runs the chain, and a
  consumed wheel skips both scroll-container halves (vertical and horizontal)
  and requests a redraw. `PlatformEvent::MouseWheel` has winit's sign (positive
  `delta_y` = wheel away from the user), so the `WheelEvent` is built with
  both deltas negated into the DOM's sign. Lines are already 40 px in the
  runtime, pixel deltas already logical (#299). A `WindowEvent::PinchGesture`
  no surface takes goes to `RinchApp::pinch_as_wheel`: a Ctrl wheel with
  `delta_y = -100 ln(1 + delta)`, so `WheelEvent::zoom_factor()` gives back
  winit's `1 + delta` — the same e-per-100-px curve as
  `render_surface::wheel_zoom_scale`. Touch pans from `shell/touch_gesture.rs`
  are `MouseWheel`s too and reach `onwheel` (a browser sends no wheel for a
  touch pan); `PlatformEvent::MouseWheel` carries no source to tell them apart.
- **Web** (`rinch-web/src/event_delegation.rs`, `install_wheel`): one
  document-level `wheel` listener, bubble phase, **`passive: false`** — Chrome
  treats a document-level wheel listener as passive by default, and a passive
  one cannot `preventDefault()`. A consumed wheel is `preventDefault()`ed (no
  scroll, no page zoom for a pinch, which browsers send as `ctrlKey` wheel).
  `deltaMode` lines are ×40, pages × the viewport height; `x`/`y` are client
  px. A render surface's canvas listener `stopPropagation()`s, so a surface
  still takes the wheel first, as on desktop. Not passed through
  `add_capture`: that helper registers with a `bool` and cannot ask for a
  non-passive listener.

Tests: `rinch/src/app/element_wheel_tests.rs` (order, consumption stopping the
scroll, modifiers and sign, pinch) and `rinch-web/tests/element_wheel.rs`
(order, `defaultPrevented`, line deltas, Ctrl).

## `title` tooltips (desktop)

An element with a non-empty `title` (the nearest one up the tree; a `title=""`
or white-space-only one on a nearer element means none) shows a tooltip after
the pointer rests 500 ms over it (#1646, `app/title_tooltip.rs`,
`rinch_dom::paint::tooltip`). It is **not a DOM node**: a
`rinch_dom::paint::tooltip::Tooltip` is shaped once (Parley, the theme's
`--rinch-font-family` else `sans-serif`, 12px) and painted after the document
by `build_pixels` / `build_scene`, its framebuffer rect added to the frame's
damage by hand like the inspect highlight's (`TitleTooltipState::last_rect`),
so showing or hiding it runs no style, layout or IFC pass (pinned by
`title_tooltip_tests::showing_it_repaints_its_rect_and_lays_out_nothing`).
Colours: `--rinch-color-surface` / `-text` / `-border` read off `<body>`'s
custom properties, light fallbacks without the theme. The hover arm calls
`note_title_tooltip_move` with the move's shared hit (restarting the rest timer
on every move); a document with no `title` pays one read of
`NodeTree::title_sources` (counted by `set_attribute`/`remove_attribute`,
like `mousemove_handlers`). `AboutToWait` runs `tick_title_tooltip`, which
shows a due one and follows a shown one's `title` (changed: re-shaped; removed
or detached: hidden). The deadline is the second timed wake:
`RinchApp::title_tooltip_deadline` is folded into the desktop shell's
`WaitUntil` (with the caret blink) and into `RinchContext::next_wake`. Placed
below-right of the pointer (+10, +20), flipped left / above to stay in the
window. Hidden by a move onto another source, `MouseDown`, `MouseWheel`,
`KeyDown` (these three also suppress it until the pointer reaches another
`title`), `PointerCancel`, `Resized`, `WindowFocus(false)`, a file hover, the
mouse leaving the window (the shell's `PointerLeft`), and any drag. Off on
Android; `App::title_tooltips(bool)` (a thread-local default read when the
window's `RinchApp` is made) and `RinchApp::set_title_tooltips`. Embed:
`RinchContext::pointer_left()` is the host's half of the window-leave hide.
A hide pushes `RequestRedraw` itself (a lone Shift press must repaint). SVG
follows Chrome 155: an SVG element's `title` attribute is ignored and its first
`<title>` child's text is its title (`paint::tooltip::own_title`); only the
`<svg>` root is reached, since hit testing does not enter shapes.
`title_sources` also counts every `<title>` element created. Not on
`rinch-web` (the browser draws its own). Components declare no `title` prop, so
it goes on an HTML element.
