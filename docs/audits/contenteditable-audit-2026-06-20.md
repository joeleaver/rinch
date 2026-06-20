# Rinch ContentEditable: Architectural & Adversarial Audit

*Generated 2026-06-20 via a 127-agent multi-phase audit (9 subsystem maps → 7 adversarial auditors → 2-lens verification of every finding → synthesis). 55 findings raised; 36 confirmed, 8 disputed, 11 rejected.*

## 1. Verdict

**It does not deliver.** The CE system is architected around a clean promise — "CRDT-first, single mutation path, EditorDocument is the source of truth, DOM is a pure view" — and that promise is honored for exactly one slice of behavior (typed text insert/delete/split into a focused, ASCII, paste-free, IME-free element). Outside that slice, the system has **multiple mutation paths writing to different sources of truth with no reconciliation back to consistency.** That is the single root cause: the model is authoritative *by convention only*, kept in lockstep by every method manually dual-writing in the right order, with **no `reconcile()` function** (the documented `sync_editor_doc_from_dom` does not exist — confirmed across multiple findings at `ce_ops.rs:288`). The moment any DOM-direct path runs (paste, cursor-only formatting) or any position-math bug fires (byte/char mismatch), DOM and CRDT diverge silently and **stay** diverged.

The two most damaging concrete manifestations are: (a) **paste injects raw DOM, never touches the CRDT, and reports zero created nodes** — the next keystroke re-renders the block from the stale model and erases the paste (confirmed critical, multiple lenses); and (b) **clicking Bold then typing produces unformatted text** because cursor-only `toggle_wrap` writes formatting only into transient DOM nodes the CRDT-first re-render destroys (confirmed critical/high, multiple lenses). On top of these correctness failures sit table-stakes gaps — **no IME** (CJK/dead-key input impossible on desktop), **no accessibility** — and a documented "editor framework" (`Editor`/`StarterKit`/24 extensions) that is **dead at runtime**, so users who follow the discoverable API are editing code that never runs. This is precisely why "every attempt to use it dead-ends."

---

## 2. How it's supposed to work vs how it actually works

### Intended architecture (per CLAUDE.md, ce.rs, ce_ops.rs docstrings)

```
keystroke / toolbar / paste
        │
        ▼
ContentEditableApi (single mutation bottleneck)
        │  every method mutates EditorDocument (CRDT) FIRST
        ▼
EditorDocument (Automerge) ── authoritative ──► re-render affected DOM blocks (DOM = view)
        │
        └── dispatch CeEvent ──► bridge observes ──► keeps model in sync
```

One trait, one implementor-shaped contract, one position space, events that tell observers exactly what changed so "observers know without diffing the DOM."

### Actual data/control flow

```
TEXT insert/delete/split ──► CeOps (CRDT-first) ──► EditorDocument ──► re-render block   ✅ works (ASCII only)

CURSOR movement/selection ──► move_dom_cursor (reads DOM/Parley) ──► writes ce.cursor/anchor
                                                                    └─ one-way sync ─► CeOps.cursor   ⚠️ second cursor model

PASTE (Ctrl+V, HTML) ──► paste_html_into_ce ──► raw rinch-dom node creation   ❌ NEVER touches CRDT
                                              └─ HtmlPasted{created_node_ids: []}  ❌ observers get nothing

CURSOR-ONLY FORMATTING ──► toggle_wrap !has_selection ──► creates DOM <strong>+ZWS   ❌ NEVER touches CRDT

COPY/CUT serialization ──► walks live DOM (lossy tag/attr view)   ❌ disagrees with the CRDT mark model

EVENT SYNC ──► the one bridge subscriber is a no-op (`let _ = remote_flag.get();`)   ❌ rich CeEvent enum feeds nothing
              actual "sync" = polling flush() that does full extract-and-diff           ❌ contradicts "no diffing" promise
```

**Where intent and reality diverge, named:**

- **"Single mutation path" is false.** Cursor movement, paste, copy/cut, and cursor-only formatting all bypass `CeOps`/`EditorDocument`. The "single bottleneck" handles text-structure mutations only.
- **"DOM is a pure view" is false.** Paste and cursor-only formatting make the **DOM the temporary source of truth**, and there is no path to fold those edits back into the model.
- **"Event-driven sync" is dead.** The rich `CeEvent` enum (~20 dispatch sites, block split/join, table insert/delete, indent/outdent) has **no functional consumer**; the bridge subscriber is empty and sync happens by `CeOps` dual-writing inline plus a polling full-rewrite `flush()`.
- **"Backend-agnostic trait" is aspirational.** The only implementor (`CeOps`) and the entire render/position layer (`ce_render.rs`) index `RinchDocument.tree.nodes` concretely, under the `DomDocument` abstraction the project claims is THE backend boundary.
- **The stale docstrings actively mislead.** `ce_ops.rs:8-11` claims mutations are "handled by app.rs directly; these stubs dispatch CeEvents" — the inverse of reality. `editor.md` describes a "Hidden Textarea + Virtual DOM" architecture that does not exist. `skip_next_sync` documents a `sync_editor_doc_from_dom` that does not exist.

---

## 3. Confirmed bugs & defects

Ranked within each theme by severity. Locations and consequences are drawn from confirmed findings.

### 3a. Data-integrity / round-trip loss (the "save eats my content" class)

| # | Severity | Location | Defect & consequence |
|---|---|---|---|
| D1 | **Critical** | `ce_render.rs:43` (`other => other`), `:27` (`_ => None`), `:149` (`attrs: HashMap::new()`) | **All attributed marks except `<a>` are destroyed on DOM round-trip.** `mark_type_to_tag` emits the mark name as a literal tag (`text_color` → `<text_color>`); `tag_to_mark_type` returns `None` for it; extraction drops the mark *and* its color attr. The CRDT preserves it (`ce_ops.rs:585-587`) but `extract_content`/`load_html` wipe it. **This is issue #59 generalized** — it is structural, not one case. Concrete: `InlineRunData{marks:[{mark_type:"text_color", attrs:{color:"#ff0000"}}]}` round-trips to `marks: []`. |
| D2 | **High→Med** | `ce_render.rs:113-156` | **`<br>` / hard breaks are never extracted** — `extract_inline_runs` has no `<br>` branch; the interchange `InlineRunData` type (`text`+`marks` only) cannot even *represent* a break. `load_html("<p>line one<br>line two</p>")` collapses to one run, permanently, on the first sync. (The framework never *emits* `<br>` itself, so blast radius is HTML import, not in-app editing — hence Med on the runtime lens.) |
| D3 | **Med** | `ce_render.rs:80` (extract verbatim) vs `:104` (render forces `<p>`) | **Unknown block tags round-trip non-idempotently.** `<section>` extracts as `block_type:"section"`, renders back as `<p>`, re-extracts as `"paragraph"`; the CRDT is polluted with stringly-typed block types no `Schema` rejects. Reachable via `load_html` of arbitrary clipboard HTML (no tag whitelist). |

**On #59 and #50:** #59 is **confirmed and broader than reported** (D1). The #50 link-href fix **holds** — but the verification exposes that it holds *only because `<a>` is a hardcoded special case* (`ce_render.rs:139`), and the **only non-gated round-trip test (`ce_link_roundtrip.rs`) covers solely that one special-cased mark.** So #50 fixed the one path that is now tested, while every other attributed mark/block remains lossy and untested. The fix is correct but non-general, and the test gives false confidence.

### 3b. State-sync / model divergence (the "paste then everything breaks" class)

| # | Severity | Location | Defect & consequence |
|---|---|---|---|
| S1 | **Critical** | `app/contenteditable/mod.rs:399,422-424`; `ce_paste.rs:12-59` | **HTML paste bypasses the CRDT entirely**, mutates only `RinchDocument`, and dispatches `HtmlPasted{created_node_ids: Vec::new()}` despite creating real nodes. The CRDT never learns of the paste; the next CRDT-first edit re-renders the block from the stale model and **deletes the pasted content**. The collaboration save path (`save_incremental`) loses it unconditionally. No observer can reconcile (empty node list + the only subscriber is a no-op). |
| S2 | **Critical** | `ce_ops.rs:1644-1802` | **Cursor-only `toggle_wrap` writes formatting to DOM only** (wrapper element + ZWS node), never to `editor_doc`, no undo entry, no BlockMap rebuild. The next `insert_text` writes unmarked text to the CRDT and re-renders the block from it, destroying the wrapper. **"Click Bold, then type" yields unformatted text** — the canonical "formatting never sticks" report, on the most common formatting gesture. |
| S3 | **High** | `ce_ops.rs:1363-1368` (and `1228-1231`) | **`delete_selection` records undo text by char index over byte-indexed positions.** The flat position space is byte-based throughout; `deleted_text.chars().skip(start).take(end-start)` mis-slices for any multibyte content before/within the selection. The live delete is correct, but **undo re-inserts wrong/empty text** — silent corruption for accented/CJK/emoji documents. (`delete_backward`/`delete_forward` byte-slice correctly, making the bug easy to miss.) |
| S4 | **High** | `ce_paste.rs:20-23`, `mod.rs:399-453` | **Paste deletes the existing selection via the raw-DOM path and never calls `sync_ce_ops_cursor`.** After "select then paste," `CeOps.cursor`/`editor_doc` reflect pre-paste state; the next edit computes its position against a stale BlockMap → off-by-N edits or position-0 fallback. |

### 3c. Editing edge cases

| # | Severity | Location | Defect & consequence |
|---|---|---|---|
| E1 | **High** | `ce_ops.rs:1213-1222` | **Typing over a selection is not atomic for undo.** Nested `begin_undo_group` calls `pending_undo_ops.clear()`; the insert's group ends up with ≤1 op and is *discarded entirely* by the length guard. One Ctrl+Z re-inserts the deleted selection without removing the typed text, landing in a state the user never saw (`"abc"`→type`"X"`→`"X"`→undo→`"Xabc"`). |
| E2 | **Med** | `ce_ops.rs:1413-1424` | **Undo of empty ordered-list-item exit converts it to a bullet list.** The list-exit detection accepts both list types, but the undo op hardcodes `old_type = "bullet_list"` (`// approximate`) and drops attrs (indent). Numbered list → undo → bulleted, nesting lost. |

### 3d. Selection / clipboard

| # | Severity | Location | Defect & consequence |
|---|---|---|---|
| C1 | **High** | `ce_paste.rs:194-223` | **Block paste split logic is unimplemented.** Despite the doc comment "Splits the current block at the cursor," `insert_parsed_blocks` appends blocks *after* the cursor's block, leaving the paragraph intact and unsplit. The block/inline decision is all-or-nothing, so mixed fragments are mangled. Real clipboard HTML is almost always block-wrapped, so this is the *common* paste path. |
| C2 | **Med→Low** | `ce_selection.rs:489` | **`serialize_html_range` emit_tag logic is incoherent** (reassigned 3×, comments admit confusion). Copied HTML serializes from the lossy DOM view, not the CRDT — **link `href` is dropped on copy** (`emit_html_attributes` emits only `style`/`class`), so copy→paste within the same editor loses the link target. |
| C3 | **Med→Low** | `ce_selection.rs:336` | **`extract_selection_text` panics with usize underflow** (`all_text.len() - 1`) when the flattened leaf list is empty and a cross-node selection is requested. Reachable via degenerate subtrees / stale cursors; one-line fix. |

> **Note on UTF-8 panics in `mutations.rs:259,679-681`:** the unguarded byte slices are real but **not reachable** through the live cursor machinery (callers snap to char/cluster boundaries; tests confirm marking "résumé" works, only synthetic mid-codepoint offsets panic). Treat as a latent robustness gap, not an active crash.

---

## 4. Architectural problems

### 4a. No single source of truth → no recovery path (the keystone)

The CRDT is authoritative *by convention*. Every method must dual-write in the correct order, and there is **no reconciliation function** to repair drift. The disputed-but-substantiated finding at `ce_ops.rs:288` is correct in its load-bearing claim: `sync_editor_doc_from_dom` does not exist, and `skip_next_sync` gates nothing real. The refutation correctly notes a *primitive* exists (`extract_content()` + `EditorDocument::from_block_data`, used by `load_html`) — but it is **not invoked after the DOM-direct seams** (paste, cursor-only formatting), so the recovery capability exists yet is never wired where it's needed. **Every DOM-direct seam therefore produces permanent rather than transient divergence.** This is the structural reason individual bugs (S1, S2, S4) compound into "the editor is unusable after I touch it."

### 4b. Two cursor models kept in lockstep by hand

Content mutations flow through `CeOps`/`EditorDocument`; **cursor movement is a separate app-level model** (`move_dom_cursor` reads DOM/Parley, writes `ce.cursor`, then one-way-syncs into `CeOps`). One lens disputed the *severity* (mutating paths do sync bidirectionally; re-renders rebuild the BlockMap), and that refutation is largely right — but the **architectural smell is real**: the caret value lives in two fields reconciled by an enforced call-order convention. It is a latent class of bug, not a guaranteed live one. Fix by making the model own the caret, or by validating the synced `DomCursor` resolves in the current BlockMap before any edit (today a miss silently maps to position 0, `ce_render.rs:712`).

### 4c. Three overlapping editing engines; the documented one is dead

Confirmed (multiple lenses): **three non-interoperating engines with three undo stacks.**
- `rinch-editable` — generic flat-buffer engine; **live only for single-line `<input>`**; it *no-ops every block/format command by design*.
- `rinch-editor` — the TipTap-style `Editor`/`History`/`CommandDispatcher`/`ExtensionRegistry`/24-extension `StarterKit` with shortcuts and markdown input-rules. **`Editor::new` is constructed only in tests** (zero non-test callers). `build_shortcuts`/`build_input_rules` have zero live consumers. **This is the discoverable, documented API — and it does nothing at runtime.**
- `CeOps` — the actual live engine, with its **own bespoke undo stack**, reusing `rinch-editor` only as the `EditorDocument` data type and `rinch-editable` only as a key→`EditCommand` lookup.

There are *two complete inverse-op interpreters over the same `EditorDocument` API* (`Editor::apply_inverse` vs `CeOps::apply_undo_op`) — one live, one dead. **Anyone who reads the docs and reaches for `StarterKit`/`RichTextEditor`/`Editor` commands is editing unreachable code.** This is a primary dead-end mechanism.

### 4d. The keyboard-bypass / render-surface preemption path

The render-surface keyboard interceptor is checked **before** CE routing and consumes *all* keys when a render surface has focus. CE is reached only via the `focused_contenteditable` fallback. There is an **implied, unenforced contract** that a CE element and a focused render surface are never focused simultaneously — violate it and the surface silently eats every CE keystroke with no diagnostic. A plausible "CE never receives input" root cause in any app with a `RenderSurface`/`GameViewport`.

### 4e. rinch-dom coupling vs #51 (web backend)

The entire live editing/render path — `CeOps` (sole `ContentEditableApi` impl) and `ce_render.rs` (BlockMap + position mapping) — is **statically welded to `RinchDocument`/`NodeTree`** via `Rc<RefCell<RinchDocument>>` fields and raw `d.tree.nodes[...]` indexing, *under* the `DomDocument` abstraction. None of ~3000 lines ports to the web backend (`web_document.rs` implements `DomDocument` but has no `NodeTree`). Several portability findings were downgraded to "real but forward-looking" — correctly: there is no live web CE bug today, but #51 in **either** direction (compile rinch-dom to wasm, or rewrite against web_sys) inherits 100% of this. Notably, the **caret/selection paint stack and the global-offset producer are pure dead weight on web** (the browser owns native caret/selection) — a strong argument that the web path should *not* port this subsystem at all.

**Incremental-fix vs reframe:** The correctness bugs in §3 are individually fixable in a week. But the **architecture cannot be incrementally trusted** while DOM-direct seams exist with no reconciliation and three engines compete. My opinion: **fix the critical correctness bugs incrementally now (they're contained), and reframe the architecture deliberately** — collapse to one authoritative model, eliminate every DOM-direct mutation path, and delete the dead `Editor` framework. Do not keep paying interest on three undo stacks and a dead extension system.

---

## 5. Gaps vs a good editor

Ranked by how badly each blocks "a real authoring tool."

1. **IME / composition (Critical gap).** No `WindowEvent::Ime` arm, no `set_ime_allowed`, no preedit state on desktop (`rinch_runtime.rs:1473-1965`). **CJK/Japanese/Korean and dead-key accent input are impossible** on the primary platform. Android has IME; desktop does not. This alone makes the editor unusable for a large fraction of users.
2. **Accessibility / ARIA (Critical/table-stakes gap).** No AccessKit, no accessibility tree, no role/state export; the custom Vello/tiny-skia surface is invisible to NVDA/JAWS/VoiceOver/Orca. WCAG/ADA/508 blocker for production. (Framework-wide, not CE-specific, but CE is where it bites hardest.)
3. **Paste fidelity (broken, not just missing).** Raw-DOM injection bypasses the model (S1), no block-split (C1), no tag whitelist (`<script>`/`<iframe>` would be materialized — only attributes are sanitized), block/inline decision all-or-nothing.
4. **Tables (advertised but inert).** `TableExtension`, `TableModel`, and `CeEvent::TableInserted/TableDeleted` all exist; **`CeOps` has zero table code** and never dispatches the events. Documented in `contenteditable.md` as a feature; completely dead in the live path.
5. **Collaboration not end-to-end / untested in CI.** The 52-test DOM↔CRDT sync + multi-peer suite (`ce_crdt_sync.rs`) is gated behind `#![cfg(feature="collaboration")]`; CI runs `cargo test --workspace` with default features, so **it compiles to zero tests.** The in-repo bridge subscriber is a no-op, and `flush()` deletes-and-rebuilds the whole doc (defeating real CRDT merge). No automated test drives the real winit keyboard/IME path at all.
6. **Schema validation unused.** A full ProseMirror-style `Schema` exists with ~30 passing tests but is **never consulted** by any mutation/serialization/bridge — block/mark types are written to the CRDT unvalidated.
7. **Placeholder / empty-state.** `<input>`/`<textarea>` paint placeholders; CE has none (no `data-placeholder`, no `:empty` affordance). (CSS `:empty::before` partially works but `attr()` content is unsupported.) Low, but every production editor has it.
8. **Spellcheck and find/replace.** Entirely absent. Low for a framework primitive, but expected for "a good editor."

---

## 6. How to make it good

### Strategic options

**Option A — One authoritative model, DOM is a pure *render output* (commit to the existing promise).**
Make `EditorDocument` the *only* source of truth. Eliminate every DOM-direct mutation path: paste, cursor-only formatting, copy/cut serialization, and selection-delete all go through `ContentEditableApi` and re-render from the CRDT. Cursor/selection become *queries on the model*, not a second DOM-side store.
- *Pro:* This is what the docs already claim; it makes the divergence bugs (S1–S4, D1) *structurally impossible* rather than individually patched. Single undo stack. Portable model.
- *Con:* Requires reworking paste (parse HTML → BlockData → insert) and stored-marks (pending format in the model). Real work, but bounded.

**Option B — Thin web_sys-native engine for #51, keep desktop as-is.**
For web, write a second `ContentEditableApi` implementor that delegates structure/layout/caret/selection to the browser's native contenteditable and syncs *only* the `EditorDocument`. Don't port `CeOps`/`ce_render`/paint.
- *Pro:* Avoids shipping rinch-dom (parley/taffy/vello) to wasm; the browser eliminates an entire class of Parley-geometry/ZWS/float-equality caret bugs and gives IME + a11y *for free*.
- *Con:* Two implementors to keep behavior-compatible; the trait must first be made attr-aware (see #59 below) and the no-op default bodies removed so a partial web impl fails to compile instead of silently losing content.

**Option C — Delete the editor framework, document `CeOps` as the real API.**
Orthogonal to A/B: cut the dead `Editor`/`CommandDispatcher`/`StarterKit`/`RichTextEditor`/second undo interpreter. Make `with_active_ce_api → CeOps` the documented, primary surface.
- *Pro:* Removes the single largest "documented thing that does nothing" dead-end; halves undo maintenance.
- *Con:* Loses the aspirational extension story (which doesn't run anyway).

### Recommended path (opinionated, sequenced)

Do **Option C now**, **Option A next**, and **set up Option B as the web target** — in that order.

1. **Stop the silent data loss (week 1).** Route paste through the model: parse clipboard HTML → `BlockData`/`InlineRunData` → CRDT-first insert at cursor, then re-render. Kill the raw-DOM `paste_html_into_ce` path or, as a stopgop, immediately call `extract_content()` + `from_block_data()` + `rebuild_block_map()` after injection and populate `HtmlPasted.created_node_ids`. → fixes **S1, S4, C1**, and adds a tag whitelist while you're there. *(This is the #1 user-visible dead-end.)*
2. **Make formatting stick (week 1).** Replace cursor-only `toggle_wrap`'s DOM-only ZWS wrappers with **stored marks on the cursor in the model** (ProseMirror-style); have `insert_text` apply stored marks via `insert_text_with_marks`. → fixes **S2**. *(This is the #2 dead-end.)*
3. **Make the position space sound (week 1–2).** Pick ONE unit — byte offsets (the space is already byte-based) — and fix the lone outlier `delete_selection` undo capture to byte-slice; add `BytePos`/`CharPos` newtypes so the two can't be mixed. → fixes **S3**; un-breaks undo for non-ASCII.
4. **Fix undo atomicity (week 1).** Add an undo-group nesting depth counter so nested `begin/commit` are no-ops and don't clear `pending_undo_ops` mid-group. → fixes **E1**; while here, fix **E2** (read the real block type/attrs before list-exit).
5. **Resolve #59 properly (week 2).** Make the mark bridge *total and attr-aware*: render attributed marks as `<span style=...>`/`data-mark=...`, recognize them on extract, copy `node.attributes` for **every** recognized mark (not just `<a>`), remove the `other => other` fallthrough, add a `wrap_selection_with_attrs(mark_type, attrs)` **on the trait in rinch-core** (so both desktop and a future web impl inherit it). Add `<br>` extract/render. Unify the divergent tag tables (`ce_render` vs `fragment.rs`) into one canonical table hoisted to rinch-core. → fixes **D1, D2, D3** and the trait-level half of #51's styled-text gap.
6. **Make the contract fail loud (week 2).** Remove the six no-op default trait bodies in `ce.rs:406-447` (or split into a supertrait) so a partial web implementor can't silently return an empty document.
7. **Test the thing that ships (week 2).** Add a CI job `cargo test -p rinch --features collaboration`, move the non-collaboration sync assertions out from behind the gate, and add per-mark/per-block non-gated round-trip tests (color, highlight, sub/sup, `<br>`, mixed nested lists, unknown-block normalization) plus a smoke test that drives `handle_contenteditable_key` end-to-end on a headless document. → addresses the false-confidence gap and would have caught D1/D2/D3/S3.
8. **Delete the dead framework (week 2–3, Option C).** Remove `Editor`/`History`/`CommandDispatcher`/`ExtensionRegistry`/`StarterKit`/`RichTextEditor`/`Editor::apply_inverse`. Document `CeOps` + `with_active_ce_api` as primary. Fix the stale docstrings (`ce_ops.rs:8-11`, `editor.md` hidden-textarea claim, `skip_next_sync`).
9. **IME (week 3).** Add the `WindowEvent::Ime` arm + `set_ime_allowed`/`set_ime_cursor_area` on CE focus, commit composed text on `Ime::Commit`. Mirror the working Android path.
10. **Then commit to the model fully (Option A) and stand up web (Option B).** With paste, formatting, and copy/cut all CRDT-first, and the trait attr-aware, the web implementor delegates caret/selection/layout/IME/a11y to the browser and syncs only `EditorDocument` — closing #51 *and* the IME/a11y gaps in one move.

---

## 7. Quick wins (small, high-leverage — do these first)

- **`extract_selection_text` underflow guard** (`ce_selection.rs:336`): `if all_text.is_empty() { return String::new(); }` + `saturating_sub(1)`. One line, removes a crash (**C3**).
- **`delete_selection` undo slice** (`ce_ops.rs:1364`): change `deleted_text.chars().skip(start).take(end-start)` to byte-slice `deleted_text[start..end]`. One line, stops silent undo corruption on non-ASCII (**S3**).
- **Populate `HtmlPasted.created_node_ids`** (`mod.rs:422`): pass the real IDs `create_parsed_nodes` already computes. Lets any future reconciler work; trivial today.
- **Undo-group depth counter** (`ce_ops.rs:1213-1222`): make nested `begin/commit_undo_group` no-op while a group is open. Fixes select-then-type undo (**E1**).
- **List-exit undo type** (`ce_ops.rs:1419`): read the real `block_type`/attrs before `set_block_type` instead of hardcoding `"bullet_list"`. Fixes ordered→bullet on undo (**E2**).
- **Copy `href`** (`ce_selection.rs:553-570`): add `href`/`title`/`target` to `emit_html_attributes`. Stops links losing their target on copy→paste (**C2**).
- **CI flag**: add `cargo test -p rinch --features collaboration` to the workflow. Zero code; turns 52 dead tests live and would surface D1/D3/S3 immediately.
- **Delete the dead `if/else`** at `ce_render.rs:309-314` (both arms identical) and the misleading comment — and fix the stale `ce_ops.rs:8-11` and `editor.md` docstrings so the next contributor isn't lied to about the data flow.

These eight are a single focused day, remove three crashes/corruptions and several "wait, the docs are wrong" dead-ends, and unblock the test suite that would have caught the rest.