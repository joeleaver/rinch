# Collab Engine Migration — Automerge → yrs

**Status:** Implemented (issue #190, PR1 = `d963ac4`).
**Scope:** `rinch-editor-collab`'s CRDT engine only. The M9 collaboration
architecture — the crate boundary, the `EditorHandle` seam, the model↔CRDT
invariant — is unchanged and stays documented in
[`editor-rearchitecture.md`](./editor-rearchitecture.md); this is an addendum
recording *why* the engine swapped and what a future reader needs to know that
isn't visible from the code alone.

## Decision

`rinch-editor-collab` dropped Automerge for **yrs 0.27**, as a rip-and-replace
(not a parallel adapter) — the whole crate now speaks one CRDT engine, same as
before. Decided by Joe, 2026-08-10.

**Evidence (issue #169):** Automerge 0.5.12 performed fine at the time (~660µs to
apply a remote change to a 90KB document), but Automerge 0.10 was a ~22× regression
on the same benchmark — so staying on Automerge meant either eating that regression
on upgrade or freezing on an old version indefinitely. Neither is viable long-term.
yrs 0.27 does the same remote-apply work in ~0.9µs, and — the more important number —
that cost is **flat at every document size** (O(update), not O(document)), because
yrs garbage-collects tombstoned content instead of accumulating it forever the way
Automerge's document-wide history does. yrs also builds for `wasm32-unknown-unknown`
with zero feature shims, where Automerge's transitive `uuid` dependency needed the
app to wire up a `getrandom`/`uuid` `js` feature just to compile.

**Precedent:** two sibling projects had already made this exact swap and their
lessons are folded into this migration: rinch-ce (commit `503fa78`) and Playweft
(commit `2b5973f`, plus the "wipe don't convert" persistence lesson in `a734e11`).

## What stayed: the seam

M9 already concentrated the CRDT behind a seam, and the point of that seam was
exactly this — the engine should be swappable without touching the model or the
editor. It held. Unchanged by this migration:

- The invariant **`model ≡ project(model)`**: every local step is projected onto
  the CRDT, every remote CRDT change is rebuilt into the model, and convergence
  follows from the CRDT's own convergence rather than from any hand-written merge
  logic.
- The **A22 fail-loud staged scope**: the adapter supports flat text-blocks + marks
  and the list containers (`bullet_list`/`ordered_list`/`list_item`, nested to any
  depth); everything else is `CollabError::Unsupported`, never a silent drop. The
  all-or-nothing validation pre-pass — validate every touched node before the first
  CRDT write — is unchanged in intent (see the load-bearing gotcha below).
- **`build_remote_transaction`** (`remote.rs`) — the convergence-critical path that
  turns a rebuilt `Node` into a minimal block-level `ReplaceStep`. It contains no
  engine type at all; it only ever needed "give me the converged document as a
  `Node`", which is why the engine swap left it untouched.
- `CollabSession`'s lifecycle shape, `CollabPlugin` (pure ProseMirror-style
  version/unconfirmed-steps bookkeeping), `rebase_steps`, `ORIGIN_REMOTE`, the
  `NodeData`/`BlockData`/`SpanMark` intermediate representation, and the block-list
  diff in `project_change`.
- The **opaque `Vec<u8>` byte surfaces** — `snapshot()`, a broadcast delta, an
  incremental update. Callers already treated these as blobs; only the encoding
  inside them changed.
- **The tests are the contract.** Every existing assertion carried over: the
  integration suite, the seeded N-peer fuzz suites (re-checking
  `model ≡ project(model)` every round), the projection unit guards, the plugin
  tests, and the handle-level collaboration tests in `rinch-editor-view` — including
  the three sync-protocol tests from #182 (see the engine mapping table below).

## Engine mapping

| Automerge (before) | yrs (after) |
|---|---|
| `AutoCommit` + `ObjId` object graph | `Doc::with_options(Options { offset_kind: OffsetKind::Utf16, .. })` |
| `content: List<Block{type, attrs, text: Text}>` + marks over `Text` | `content: Array<Map{type, attrs, text: Text}>`; marks are the `Text`'s own native formatting attributes; plus a `meta` root map carrying the projection-format marker (see `load()` below) |
| JSON-string-encoded mark values (`encode_attrs`/`decode_mark_value`) | **Deleted.** yrs format attributes carry structured `Any` values natively — the JSON indirection existed only because Automerge marks carried a scalar |
| `get_heads()` / `ChangeHash` compare | `state_vector()` bytes — but see "never a convergence test" below; the comparison semantics are not equivalent |
| `save_incremental()` broadcast delta | an `observe_update_v1` outbox drained by `CollabSession::save_incremental` (see below — this is *not* a straight rename) |
| `sync.rs` re-exporting `ChangeHash`/`SyncMessage`/`SyncState` | **Deleted.** Replaced by `state_vector()` + `encode_diff_v1(&peer_sv)` (`CollabDoc::diff_since`) + `apply_update` |
| `EditorHandle::collab_generate_sync_message`/`collab_receive_sync_message`/`collab_heads` (#182) | **Deleted**, replaced by `collab_state_vector()` + `collab_sync_diff(remote_sv)`; `collab_receive` is unchanged and now serves both broadcast and reconciliation, since in yrs they're the same wire format |
| `CollabDoc::patches_to_remote_ops` / `remote_ops_since` (consuming `automerge::Patch`) | **Deleted, not ported.** Was already documented as non-convergence-critical with zero session consumers; can be rebuilt on yrs observer deltas (`TextRef::observe` → `TextEvent`) if a future cursor-preserving refinement wants it |
| `CollabError::Automerge(String)` | `CollabError::Engine(String)`, with `From` impls for yrs's decode/update error types |
| `uuid`/`getrandom` `js` shims (Automerge's transitive actor-id dependency) | **Deleted.** yrs carries `fastrand/js` itself; `collab-editor-web`'s shim is gone and the crate joined CI's `clippy-wasm` job |

## Two hard-won design points

**Broadcast is an `observe_update_v1` outbox, not a diff against the last-sent state
vector.** The obvious design — remember the state vector you last broadcast from,
and diff against it each time — does not work with yrs: `encode_diff_v1` writes the
*complete* delete set regardless of the state vector it's given. Once a document has
seen any deletion, every subsequent "diff since last broadcast" re-carries the
document's entire deletion history, growing without bound (measured 46→292
bytes/keystroke over 200 edits during review). The adapter instead subscribes to
`observe_update_v1` on the yrs document and parks each committed transaction's own
update in an outbox (`Arc<Mutex<Vec<Vec<u8>>>>`) for `save_incremental` to drain.
That update is constant-size per edit and genuinely empty when nothing happened.
Updates applied from a peer are tagged with an origin the observer checks for and
skips — otherwise a re-broadcast would echo a change straight back to whoever sent
it.

**State vectors are never used as a change or convergence test — anywhere in this
crate.** A yrs state vector summarizes *insertions* only: a delete-only change, and a
mark *removal* (yrs implements un-formatting by deleting the format marker), leave
the state vector completely unchanged. Two replicas can hold different documents
behind equal state vectors. This shows up in two places that both had to get it
right independently:
- `CollabSession::integrate_incremental` decides "did the document change" by
  rebuilding from the converged CRDT and comparing the resulting `Node`s
  (`build_remote_transaction` returns `None` for an identical rebuild) — never by
  comparing state vectors before and after the merge.
- The reconciliation protocol (`state_vector()` / `sync_diff(remote_sv)`) always
  requests-and-applies a diff rather than short-circuiting when two state vectors
  compare equal; the diff reply carries the full delete set regardless, which is
  what actually repairs a peer that missed a deletion.

The review process caught an SV-equality short-circuit that had crept into an early
draft of the reconciliation path; the regression tests at both the session and
handle layer now pin the trap directly — they assert the state vector is unchanged
as a *precondition*, then assert the deletion still propagates.

## `load()` validates a format marker and the content, not a type tag

Joining from a peer's snapshot (`CollabDoc::load`) has to reject bytes that aren't
actually a rinch projection — otherwise a version mismatch or a stray CRDT document
would silently "join" an empty collaboration with no shared content. Automerge could
check this by object type. yrs cannot: a root type carries **no wire type tag** — a
foreign root arrives as `Out::UndefinedRef`, and asking for it as a typed ref (e.g.
`get_or_insert_array`) *reinterprets* whatever is actually there rather than failing
(a foreign `Map` root named `content` reads back as a zero-length array; a `Text`
root reads back as an array of single characters).

So the projection tags itself. A `meta` root map carries
`format = "rinch-editor-collab/yrs-1"`, written at creation in the same transaction as
the initial content, and `load` requires it **first**: missing or mismatched is a loud
`Schema` error. A marker is ordinary map *content*, which is exactly why it survives the
hazard above — content is what the wire carries, and a non-empty root map is transmitted
like any other data. The per-entry content check still runs afterwards, so a `content`
array full of junk is refused even if it somehow carried the marker. This is Playweft's
"wipe, don't convert" precedent (`a734e11`): tag every blob, refuse an untagged one
rather than guessing. Bump the trailing version if the wire shape ever changes
incompatibly. One consequence is inherent and intended: a snapshot produced between the
engine swap and this change carries no `meta` root at all, so it is refused like any other
untagged blob — re-project such a document from the model rather than trying to load it.
Nothing downstream had shipped on yrs, so that costs nobody.

The marker **replaced emptiness as the discriminator**, and that was the point of adding
it (issue #192). The original guard rejected a zero-block `content` array as "not a rinch
projection" — a heuristic that happened to catch the foreign shapes above because they all
reinterpret as empty. But zero blocks is also a *legitimate* converged state: two peers
concurrently deleting different blocks deletes every block, so the emptiness heuristic
locked a late joiner out of a real session. With the marker doing the discriminating,
`load` accepts any block count including zero. `to_doc` then supplies the starter
paragraph the editor schema requires, `project_change` knows that paragraph is not in the
CRDT and inserts it on the next local edit, and the invariant reads:

> `model ≡ project(model)`, **except** that a CRDT holding zero content blocks projects
> to the starter-paragraph model — an equality of documents that is not backed by CRDT
> content, scoped to that one state, and cured by the next local edit.

Eleven foreign-bytes shapes are pinned by unit tests — the four original foreign roots,
undecodable bytes, a missing marker on an otherwise-valid projection, a wrong marker
version, a marker of the wrong value kind, a `Text` and an `Array` root named `meta` (the
reinterpretation hazard aimed at the marker itself), and a correct marker over junk
content. All fail loud; none panic. Note that without the marker the shapes that
reinterpret as *empty* would now sail through, so the two halves of this design are
load-bearing together — which the tests pin from both directions.

## The `Send` contract, pinned

`CollabSession`/`CollabDoc` must stay `Send` — a server holds a session across an
`.await` (plotweb-crdt does this), so losing the bound breaks a downstream consumer
at their next dependency bump, which is the worst place to find out. It was lost
once already during this migration: moving the broadcast delta to an
observer-fed outbox introduced both a shared queue and yrs's `Subscription` type,
and `Subscription` is only `Send` with yrs's `sync` feature enabled. The fix is the
`sync` feature (a bare marker — it adds no dependencies, so it cannot affect the
wasm build) plus making the outbox an `Arc<Mutex<_>>` rather than the more natural
single-threaded `Rc<RefCell<_>>`. A `const` assertion in `lib.rs` (`assert_send::<CollabDoc>()`,
`assert_send::<CollabSession>()`) turns any future regression into a compile error
instead of a downstream surprise.

## Offsets

The projection is built with `OffsetKind::Utf16` — **never `Doc::new()`**, whose
default is `OffsetKind::Bytes`. `rinch-editor-core` positions are Unicode scalars
(chars); yrs offers only byte or UTF-16 offsets, so every index crossing a `Text`
boundary is converted at the projection boundary. Getting this wrong is silent: yrs
snaps an index that lands mid-surrogate-pair to the nearest boundary instead of
erroring, which is why the offset tests specifically use **two** astral characters —
a single astral character (or none) cannot distinguish a correct conversion from a
silently-snapped one. This is Playweft's lesson, carried over rather than
rediscovered the hard way.

## Undo: yrs `UndoManager` was deliberately rejected

The editor's undo history stays exactly what it already was — the model-layer Step
history — rather than adopting yrs's own `UndoManager`. Both prior-art projects
(rinch-ce, Playweft) independently rejected it for the same two reasons: its default
clock is compiled out on `wasm32` (no `Instant`), and origin-scoped undo has the
wrong semantics under concurrent editing (undoing "my last change" when a peer's
edit landed in between does not mean what a user expects). Remote transactions stay
`ORIGIN_REMOTE` and non-undoable, exactly as before the migration.

## Follow-up issues filed during review

Adversarial review of the PR1 branch found four **pre-existing** bugs — verified
code-identical on `main` before this migration, not introduced by it, and
deliberately not folded into PR1's scope:

- **#192** — concurrent deletion of two different blocks by two peers converges to
  an empty content list and permanently wedges the session. **Fixed** since (in its own
  change, not PR1): see the `load()` section above for the marker and the scoped
  invariant exception.
- **#193** — the mark/attr resync clears and re-applies *every* mark (or replaces
  the whole attrs map) on a changed block instead of diffing per mark/key, silently
  discarding a peer's concurrent unrelated mark or attr edit on the same block.
- **#194** — the A22 "all-or-nothing" guarantee only covers the validation pre-pass;
  an error partway through the write phase itself can commit a partial change and
  still broadcast it. PR1 narrowed an overclaiming comment describing this guarantee
  to match what it actually covers, without fixing the underlying gap.
- **#196** — mid-session (not join-time) foreign bytes whose `content` root was
  created as a `Text` type elsewhere leave the session in a one-way partition:
  inbound integration fails forever, but local edits keep committing and
  broadcasting into the void.

## Downstream: PlotWeb

PlotWeb pins rinch by git revision and upgrades deliberately, so nothing breaks
silently — but the exposure is real: `plotweb-crdt` consumes
`rinch-editor-collab`'s projection directly, and `plotweb-web` builds with
`collaboration` on both wasm and desktop. Two migration paths, depending on what
PlotWeb's persisted CRDT blobs turn out to be:

- If PlotWeb's durable truth is git-committed `DocNode`s (`plotweb-git`) and the CRDT
  is only ever a live projection of that, migration is re-projection from
  `DocNode` — construct a fresh session from the model, discard the old CRDT bytes
  entirely. Playweft's "wipe, not convert" precedent (`a734e11`) applies: tag every
  persisted blob with its engine, and refuse to load an untagged or wrongly-tagged
  one rather than guessing.
- If any stored CRDT blob is itself authoritative (no `DocNode` to re-derive from),
  the lossless path is: old-crate `CollabDoc::to_doc()` → `Node` → new-crate
  `CollabDoc::from_doc()`. This round-trips through the model rather than trying to
  transcode Automerge bytes into yrs bytes directly (the two engines share no wire
  format).

PlotWeb's own separate `automerge = "0.5"` usage (its structure docs, `local_book.rs`)
is unaffected by this migration and is PlotWeb's own call to make.
