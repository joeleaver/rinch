//! The rich-text editor's spellchecker (#2862, stage 2).
//!
//! [`SpellcheckPlugin`] keeps the misspelled words as document ranges and
//! hands them out as inline decorations of class [`SPELL_ERROR_CLASS`]
//! (styled by `styles.rs`: a wavy underline in `--rinch-spellcheck-color`).
//! The ranges map through every transaction, so an edit elsewhere never
//! re-checks or moves them wrongly, and a decoration-only result skips the
//! view's document diff. The words a change touches are dropped at once and
//! put on the plugin's dirty list.
//!
//! The checking itself is the handle's ([`EditorHandle`]): once the user has
//! interacted with the editor (a press, a Tab, or typing; Chrome's rule, as
//! for text fields), it checks the whole document, and after each change
//! waits for the configured pause, then sends the dirty words to the
//! `rinch-spellcheck` thread. The result comes back as a meta-only
//! transaction, which the plugin applies only if the document has not changed
//! since (its generation), so a stale result is never drawn.
//!
//! **On the web** there is no thread: the check runs on the page's own thread
//! in slices (`Slicer`), each one a few milliseconds of words in its own
//! browser task, and each slice's result is applied as it comes (the plugin
//! trims its dirty list to the slice's end, so an edit between slices loses
//! nothing already checked). The first whole-document check of a long
//! document is sliced the same way.
//!
//! The plugin is installed by [`create_editor`](crate::create_editor) only
//! when the thread has a spellcheck configuration (an app run through
//! `App::run`, an embedded `RinchContext`), never in a test that configured
//! none, and on the web only when the configuration is on and holds the
//! app's own dictionaries (the browser lends rinch none of its own): an editor
//! made before they arrive gets the plugin, its undo history kept, when they
//! do (`watch_for_dictionaries`).

use std::any::Any;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use rinch_editor_core::pos::Pos;
use rinch_editor_core::state::{EditorState, Transaction};
use rinch_editor_core::transform::Mapping;
use rinch_editor_core::{Attrs, Decoration, DecorationSet, Node, Plugin, PluginKey};
use rinch_spell::runtime as spell;

use crate::handle::EditorHandle;

/// The plugin's key.
pub const SPELLCHECK_KEY: PluginKey = PluginKey("spellcheck");

/// The class of a misspelled word's decoration (`<span data-pm-deco
/// class="pm-spell-error">`).
pub const SPELL_ERROR_CLASS: &str = "pm-spell-error";

/// The transaction meta key the handle talks to the plugin through.
const META: &str = "spellcheck";

/// The next [`SpellState::generation`]: one counter for the process, so a
/// generation never repeats, not even across a `load_html` (which builds a
/// fresh state) or between two editors.
fn fresh_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What the handle tells the plugin.
#[derive(Debug)]
pub(crate) enum SpellMeta {
    /// The user interacted with the editor: check all of it.
    Activate,
    /// The configuration changed, or the editor is to be checked again:
    /// drop every mark, check everything again.
    Reset,
    /// The editor is not to be checked now (read-only, `spellcheck` off):
    /// drop every mark and forget what is pending.
    Clear,
    /// These marks' words are correct now (added to the dictionary,
    /// ignored): drop them. The document did not change, so nothing else is
    /// stale.
    Drop(Vec<(usize, usize)>),
    /// A check came back: `found` are the misspelled words in `checked`, for
    /// the document at `generation`. `upto` is `None` when the check covered
    /// every dirty word (the dirty list is cleared), or the position a sliced
    /// check stopped at (only the dirty words before it are done).
    Result {
        generation: u64,
        checked: Vec<(usize, usize)>,
        found: Vec<(usize, usize)>,
        upto: Option<usize>,
    },
}

/// The plugin's state.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpellState {
    /// The user has interacted with the editor.
    pub(crate) activated: bool,
    /// The misspelled words, sorted document ranges.
    pub(crate) marks: Vec<(usize, usize)>,
    /// What is waiting to be checked, document ranges.
    pub(crate) dirty: Vec<(usize, usize)>,
    /// A fresh value ([`fresh_generation`]) at every document change and
    /// every widening of `dirty`: a result for an older one is stale.
    pub(crate) generation: u64,
    /// The plugin's settings, for the handle building a check.
    pub(crate) skip: Rc<Skip>,
}

/// What a [`SpellcheckPlugin`] leaves out.
#[derive(Clone, Debug, Default)]
pub(crate) struct Skip {
    nodes: Vec<String>,
    marks: Vec<String>,
}

/// Marks misspelled words (see the module docs). Its state lives in the
/// editor state.
///
/// `create_editor` installs one with the defaults when the app has spellcheck
/// configured; an editor built with [`EditorHandle::new`] and its own plugin
/// list adds one itself (`SpellcheckPlugin::new()`), and the handle drives it
/// the same way. Everything is checked by default, as in Chrome;
/// [`skip_nodes`](Self::skip_nodes) and [`skip_marks`](Self::skip_marks)
/// leave text out by node type (`code_block`) or mark (`code`), and a
/// textblock whose host element, or an element between it and the editor's
/// container, says `spellcheck="false"` is not checked either.
#[derive(Debug, Default, Clone)]
pub struct SpellcheckPlugin {
    skip: Skip,
}

impl SpellcheckPlugin {
    /// Everything checked.
    pub fn new() -> Self {
        Self::default()
    }

    /// Never check text inside nodes of these types (a textblock or any
    /// ancestor of it), such as `"code_block"`.
    pub fn skip_nodes<I, S>(mut self, types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.skip.nodes = types.into_iter().map(Into::into).collect();
        self
    }

    /// Never mark text that carries one of these marks, such as `"code"`.
    pub fn skip_marks<I, S>(mut self, marks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.skip.marks = marks.into_iter().map(Into::into).collect();
        self
    }
}

impl Plugin for SpellcheckPlugin {
    fn key(&self) -> PluginKey {
        SPELLCHECK_KEY
    }

    fn init_state(&self, _doc: &Node) -> Option<Rc<dyn Any>> {
        Some(Rc::new(SpellState {
            generation: fresh_generation(),
            skip: Rc::new(self.skip.clone()),
            ..SpellState::default()
        }))
    }

    fn apply(
        &self,
        tr: &Transaction,
        _old: &EditorState,
        new_doc: &Node,
        prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        let prev = prev.and_then(|p| p.downcast_ref::<SpellState>());
        let meta = tr
            .get_meta(META)
            .and_then(|m| m.downcast_ref::<SpellMeta>());
        if !tr.doc_changed() && meta.is_none() {
            // Nothing for the spellchecker: keep the same state.
            return Some(Rc::new(prev.cloned().unwrap_or_default()));
        }
        let mut st = prev.cloned().unwrap_or_default();
        let whole = (0, new_doc.content_size());
        if tr.doc_changed() {
            st.generation = fresh_generation();
            let mapping = tr.mapping();
            let changed = changed_ranges(mapping);
            st.marks = st
                .marks
                .iter()
                .filter_map(|&(a, b)| {
                    let (a, b) = (mapping.map(a, 1), mapping.map(b, -1));
                    (a < b && !changed.iter().any(|&c| touches(c, (a, b), new_doc)))
                        .then_some((a, b))
                })
                .collect();
            if st.activated {
                let mut dirty: Vec<(usize, usize)> = st
                    .dirty
                    .iter()
                    .map(|&(a, b)| (mapping.map(a, -1), mapping.map(b, 1)))
                    .collect();
                dirty.extend(changed);
                st.dirty = merge(dirty);
            }
        }
        match meta {
            Some(SpellMeta::Activate) if !st.activated => {
                st.activated = true;
                st.generation = fresh_generation();
                st.dirty = vec![whole];
            }
            Some(SpellMeta::Clear) => {
                st.marks.clear();
                st.generation = fresh_generation();
                st.dirty.clear();
            }
            Some(SpellMeta::Reset) => {
                st.marks.clear();
                st.generation = fresh_generation();
                st.dirty = if st.activated {
                    vec![whole]
                } else {
                    Vec::new()
                };
            }
            Some(SpellMeta::Drop(gone)) => {
                st.marks.retain(|m| !gone.contains(m));
            }
            Some(SpellMeta::Result {
                generation,
                checked,
                found,
                upto,
            }) if st.activated && *generation == st.generation => {
                st.marks
                    .retain(|&(a, b)| !checked.iter().any(|&(c, d)| a < d && b > c));
                st.marks.extend(found.iter().copied());
                st.marks.sort_unstable();
                match *upto {
                    None => st.dirty.clear(),
                    Some(p) => {
                        st.dirty = st
                            .dirty
                            .iter()
                            .filter(|&&(_, b)| b > p)
                            .map(|&(a, b)| (a.max(p), b))
                            .collect();
                    }
                }
            }
            _ => {}
        }
        Some(Rc::new(st))
    }

    fn decorations(&self, state: &EditorState) -> DecorationSet {
        let Some(st) = state.plugin_state::<SpellState>(SPELLCHECK_KEY) else {
            return DecorationSet::empty();
        };
        if st.marks.is_empty() {
            return DecorationSet::empty();
        }
        let attrs = Attrs::new().with("class", SPELL_ERROR_CLASS);
        DecorationSet::new(
            st.marks
                .iter()
                .map(|&(a, b)| Decoration::inline(Pos(a), Pos(b), attrs.clone()))
                .collect(),
        )
    }
}

/// The ranges of the new document a transaction changed: each step's
/// replaced range, mapped through the steps after it.
fn changed_ranges(mapping: &Mapping) -> Vec<(usize, usize)> {
    let maps = mapping.maps();
    let mut out = Vec::new();
    for (i, map) in maps.iter().enumerate() {
        let rest = mapping.slice(i + 1, maps.len());
        for r in map.ranges().as_chunks::<3>().0 {
            let new_size = if map.is_inverted() { r[1] } else { r[2] };
            out.push((rest.map(r[0], -1), rest.map(r[0] + new_size, 1)));
        }
    }
    out
}

/// Whether change `c` (new-document range) touches mark `m`: overlaps it, or
/// abuts it with anything but inserted white space (a space typed after a
/// word leaves the word as it was).
fn touches(c: (usize, usize), m: (usize, usize), doc: &Node) -> bool {
    if c.0 < m.1 && c.1 > m.0 {
        return true;
    }
    if c.1 == m.0 {
        return c.0 == c.1 || !char_at(doc, c.1 - 1).is_some_and(char::is_whitespace);
    }
    if c.0 == m.1 {
        return c.0 == c.1 || !char_at(doc, c.0).is_some_and(char::is_whitespace);
    }
    false
}

/// The character of a text node at document position `pos`.
fn char_at(doc: &Node, pos: usize) -> Option<char> {
    let mut found = None;
    doc.nodes_between(pos, pos + 1, &mut |node, at, _| {
        if let Some(text) = node.text()
            && at <= pos
            && pos < at + node.text_len()
        {
            found = text.chars().nth(pos - at);
        }
        found.is_none()
    });
    found
}

/// The text of document range `a..b` (an inline leaf contributes nothing).
fn text_of(doc: &Node, a: usize, b: usize) -> String {
    let mut out = String::new();
    doc.nodes_between(a, b, &mut |node, at, _| {
        if let Some(t) = node.text() {
            let from = a.max(at) - at;
            let to = b.min(at + node.text_len()).saturating_sub(at);
            out.extend(t.chars().skip(from).take(to.saturating_sub(from)));
        }
        true
    });
    out
}

/// A word the spellchecker marked, as [`EditorHandle::misspelling_at`]
/// answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// Where the word starts.
    pub from: Pos,
    /// Where it ends.
    pub to: Pos,
    /// The word.
    pub word: String,
}

/// Sorted, merged ranges.
fn merge(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (a, b) in ranges {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// One textblock's text to check: `text` holds one char per document
/// position from `start` (an inline leaf is U+FFFC, a word boundary), and
/// `ranges` are the byte ranges of it to check.
pub(crate) struct BlockJob {
    start: usize,
    text: String,
    ranges: Vec<Range<usize>>,
}

/// The dirty words of `doc`: every textblock a dirty range reaches, with the
/// dirty part of its text widened to white space.
pub(crate) fn jobs(
    doc: &Node,
    dirty: &[(usize, usize)],
    skip: &Skip,
    block_off: &dyn Fn(usize) -> bool,
) -> Vec<BlockJob> {
    dirty_blocks(doc, dirty, skip)
        .into_iter()
        .filter(|(pos, _)| !block_off(*pos))
        .filter_map(|(pos, block)| block_job(pos, &block, dirty, skip))
        .collect()
}

/// Every textblock of `doc` a dirty range reaches (not inside a skipped
/// node), with its position, in document order.
fn dirty_blocks(doc: &Node, dirty: &[(usize, usize)], skip: &Skip) -> Vec<(usize, Node)> {
    let mut blocks: Vec<(usize, Node)> = Vec::new();
    for &(a, b) in dirty {
        let (a, b) = (a.min(doc.content_size()), b.min(doc.content_size()));
        doc.nodes_between(
            a,
            b.max(a + 1).min(doc.content_size()),
            &mut |node, pos, _| {
                if skip.nodes.iter().any(|t| t == node.type_name()) {
                    return false;
                }
                if node.is_textblock() {
                    // `dirty` is sorted and the walk is in document order, so
                    // a block already taken is the last one taken.
                    if blocks.last().is_none_or(|(p, _)| *p < pos) {
                        blocks.push((pos, node.clone()));
                    }
                    false
                } else {
                    true
                }
            },
        );
    }
    blocks
}

/// The check of the textblock `block` at `pos`: its text, and the dirty part
/// of it widened to white space. `None` when nothing in it is dirty.
fn block_job(pos: usize, block: &Node, dirty: &[(usize, usize)], skip: &Skip) -> Option<BlockJob> {
    let start = pos + 1;
    let mut text = String::new();
    // Text under a skipped mark is U+FFFC, one per position, so it is
    // never a word and the positions stay aligned.
    for i in 0..block.child_count() {
        let child = block.child(i);
        let skipped = child
            .marks()
            .iter()
            .any(|m| skip.marks.iter().any(|n| n == m.type_name()));
        match child.text() {
            Some(t) if !skipped => text.push_str(t),
            _ => (0..child.node_size()).for_each(|_| text.push('\u{FFFC}')),
        }
    }
    let chars = text.chars().count();
    let byte_of = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for &(a, b) in dirty {
        let (a, b) = (a.max(start), b.min(start + chars));
        if a > b {
            continue;
        }
        let r = spell::widen_to_tokens(&text, byte_of(a - start)..byte_of(b - start));
        spell::add_range(&mut ranges, r);
    }
    (!ranges.is_empty()).then_some(BlockJob {
        start,
        text,
        ranges,
    })
}

/// The document ranges a check covered, and the misspelled words in them.
pub(crate) type CheckResult = (Vec<(usize, usize)>, Vec<(usize, usize)>);

/// Check `jobs` (on the checking thread): the document ranges checked, and
/// the misspelled words found in them.
pub(crate) fn run_jobs(session: &spell::Session, jobs: &[BlockJob]) -> CheckResult {
    let mut checked = Vec::new();
    let mut found = Vec::new();
    for job in jobs {
        let pos = |byte: usize| job.start + job.text[..byte].chars().count();
        checked.extend(job.ranges.iter().map(|r| (pos(r.start), pos(r.end))));
        found.extend(
            session
                .check_ranges(&job.text, &job.ranges)
                .into_iter()
                .map(|r| (pos(r.start), pos(r.end))),
        );
    }
    (checked, found)
}

/// Whether this build checks the editor at all: always off the web; on the
/// web with the `spellcheck` feature (`rinch-web`'s, on by default). Off,
/// nothing reaches the checking code and the linker drops it.
const ON: bool = cfg!(any(feature = "spellcheck", not(target_family = "wasm")));

// The web's slices are configured per app: `SpellcheckConfig::slice_budget_ms`
// (how long one slice may take, applying its marks included),
// `slice_max_found` (applying a result builds a `<span>` per new mark, and in
// a browser that, not the checking, is most of a long slice: 6,400 new marks
// took 25 ms to apply on #2862's long document) and `slice_chunk_bytes` (the
// piece checked between two looks at the clock).

thread_local! {
    /// Native tests take the web's path (slices, the plugin only with the
    /// app's dictionaries).
    #[cfg(test)]
    static WEB_PATH: Cell<bool> = const { Cell::new(false) };
    /// Native tests check one chunk per slice.
    #[cfg(test)]
    static ONE_CHUNK_SLICES: Cell<bool> = const { Cell::new(false) };
    static STATS: Cell<SliceStats> = const { Cell::new(SliceStats { slices: 0, max_us: 0, total_us: 0, max_apply_us: 0, words: 0 }) };
}

/// Whether checking runs on the UI thread in slices (the web) rather than on
/// the checking thread.
fn web_path() -> bool {
    #[cfg(test)]
    if WEB_PATH.with(Cell::get) {
        return true;
    }
    cfg!(target_family = "wasm")
}

/// Whether a new editor gets the plugin: the thread has a configuration and,
/// on the web, it is on and has the app's own dictionaries.
pub(crate) fn wanted() -> bool {
    if !ON {
        false
    } else if web_path() {
        spell::session().is_some()
    } else {
        spell::config().is_some()
    }
}

/// An editor made on the web before the app's dictionaries arrived (or
/// while checking was off, or before the app configured anything at all: a
/// handle made before `App::run`, an island whose app configures after its
/// fetch): when a configuration change makes checking possible, the plugin
/// is added then, keeping the editor's state. One listener, nothing else.
pub(crate) fn watch_for_dictionaries(handle: &EditorHandle) {
    if ON && web_path() && handle.spell_peek(|_| ()).is_none() {
        handle.spell_listen();
    }
}

/// What the web's slices cost, for measuring ([`slice_stats`]): each slice
/// timed from its start to its result applied.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SliceStats {
    /// Slices run.
    pub slices: u32,
    /// The longest one, microseconds.
    pub max_us: u64,
    /// All of them, microseconds.
    pub total_us: u64,
    /// The longest time one slice spent applying its result (the plugin,
    /// the decorations, the view), microseconds.
    pub max_apply_us: u64,
    /// Words checked.
    pub words: u64,
}

/// The cost of the slices run on this thread since [`reset_slice_stats`].
#[doc(hidden)]
pub fn slice_stats() -> SliceStats {
    STATS.with(Cell::get)
}

/// Start [`slice_stats`] again.
#[doc(hidden)]
pub fn reset_slice_stats() {
    STATS.with(|s| s.set(SliceStats::default()));
}

/// A check on the web, worked through a slice at a time ([`EditorHandle::spell_slice`]).
pub(crate) struct Slicer {
    /// The plugin generation it was taken at: a slice for another is dropped.
    generation: u64,
    /// The textblocks to check, in document order, and the next one.
    blocks: Vec<(usize, Node)>,
    next: usize,
    /// The dirty ranges it was taken for.
    dirty: Vec<(usize, usize)>,
    skip: Rc<Skip>,
    /// The block being checked.
    current: Option<Current>,
    /// The piece size (`slice_chunk_bytes`).
    chunk_bytes: usize,
    /// How long the last slice took to apply its result: the next one stops
    /// checking that much sooner, so the whole slice keeps to the budget.
    last_apply: std::time::Duration,
}

/// The block a [`Slicer`] is in: its text, its remaining pieces, and how far
/// into its text positions have been counted (`(byte, char)`).
type Current = (
    BlockJob,
    std::collections::VecDeque<Range<usize>>,
    (usize, usize),
);

impl Slicer {
    /// The next piece to check, moving on to the next block (skipping the
    /// ones `block_off` turns off) when this one is done.
    fn next_piece(&mut self, block_off: &dyn Fn(usize) -> bool) -> Option<Range<usize>> {
        loop {
            if let Some((_, pieces, _)) = &mut self.current
                && let Some(piece) = pieces.pop_front()
            {
                return Some(piece);
            }
            let (pos, block) = self.blocks.get(self.next)?.clone();
            self.next += 1;
            if block_off(pos) {
                continue;
            }
            self.current = block_job(pos, &block, &self.dirty, &self.skip).map(|job| {
                let pieces = job
                    .ranges
                    .iter()
                    .flat_map(|r| pieces(&job.text, r.clone(), self.chunk_bytes))
                    .collect();
                (job, pieces, (0, 0))
            });
        }
    }

    /// The document position of byte `byte` of the current block's text,
    /// counting on from the last one asked (bytes are asked in order).
    fn pos(&mut self, byte: usize) -> usize {
        let (job, _, (b, c)) = self.current.as_mut().expect("a block is being checked");
        *c += job.text[*b..byte].chars().count();
        *b = byte;
        job.start + *c
    }
}

/// `range` of `text` cut at white space into pieces of about `chunk`
/// bytes.
fn pieces(text: &str, range: Range<usize>, chunk: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = range.start;
    while start < range.end {
        let mut end = (start + chunk.max(1)).min(range.end);
        while !text.is_char_boundary(end) {
            end += 1;
        }
        if end < range.end {
            end = text[end..range.end]
                .find(char::is_whitespace)
                .map_or(range.end, |i| end + i);
        }
        out.push(start..end);
        start = end;
    }
    out
}

/// The handle's half: when to check, and the trip to the checking thread (or,
/// on the web, the slices).
#[derive(Default)]
pub(crate) struct Driver {
    timer: Option<rinch_core::timer::TimeoutHandle>,
    listener: Option<u64>,
    /// The web's check in progress, and the timer that runs its next slice.
    slicer: Option<Slicer>,
    slice_timer: Option<rinch_core::timer::TimeoutHandle>,
    /// The user pressed in or typed into the editor before it had the
    /// plugin (the web, before the app's dictionaries arrived).
    interacted: bool,
    /// The [`spell::on_words_accepted`] listener.
    words: Option<u64>,
}

impl Drop for Driver {
    fn drop(&mut self) {
        for t in [self.timer.take(), self.slice_timer.take()]
            .into_iter()
            .flatten()
        {
            rinch_core::timer::clear_timeout(t);
        }
        if let Some(id) = self.listener.take() {
            spell::remove_config_listener(id);
        }
        if let Some(id) = self.words.take() {
            spell::remove_words_listener(id);
        }
    }
}

impl EditorHandle {
    /// The spellchecker's state, when the plugin is installed.
    pub(crate) fn spell_state(&self) -> Option<SpellState> {
        self.spell_peek(SpellState::clone)
    }

    /// `f` of the spellchecker's state, when the plugin is installed, without
    /// copying its marks.
    fn spell_peek<R>(&self, f: impl FnOnce(&SpellState) -> R) -> Option<R> {
        self.with_state(|s| s.plugin_state::<SpellState>(SPELLCHECK_KEY).map(f))
    }

    /// Whether this editor is to be checked now: the plugin is installed, the
    /// app has spellcheck on with a dictionary, the editor is editable, and
    /// its container's `spellcheck` state (its own or the nearest ancestor's)
    /// is on.
    fn spell_session(&self) -> Option<spell::Session> {
        if !ON {
            return None;
        }
        self.spell_peek(|_| ())?;
        let session = spell::session()?;
        if self.is_read_only() {
            return None;
        }
        if let Some(container) = self.container_node() {
            let on = spell::resolve_inherited(
                container,
                |n| n.parent_node(),
                |n| {
                    n.get_attribute("spellcheck")
                        .and_then(|v| spell::attribute_state(&v))
                },
            );
            if !on {
                return None;
            }
        }
        Some(session)
    }

    /// The misspelled word at `pos` (a mark that contains it, its ends
    /// included), if the spellchecker marked one there. What the text
    /// context menu asks at a right press (#2862, stage 3).
    pub fn misspelling_at(&self, pos: Pos) -> Option<Misspelling> {
        let st = self.spell_state()?;
        let &(a, b) = st.marks.iter().find(|&&(a, b)| a <= pos.0 && pos.0 <= b)?;
        let word = self.with_state(|s| text_of(&s.doc, a, b));
        Some(Misspelling {
            from: Pos(a),
            to: Pos(b),
            word,
        })
    }

    /// Every word the spellchecker has marked, in document order. Empty when
    /// the plugin is not installed or nothing is marked.
    pub fn misspellings(&self) -> Vec<Misspelling> {
        let Some(st) = self.spell_state() else {
            return Vec::new();
        };
        self.with_state(|s| {
            st.marks
                .iter()
                .map(|&(a, b)| Misspelling {
                    from: Pos(a),
                    to: Pos(b),
                    word: text_of(&s.doc, a, b),
                })
                .collect()
        })
    }

    /// The checker accepted more words: drop the marks whose word it now
    /// takes. Asks only the loaded checker, and only about marked words.
    fn spell_words_accepted(&self) {
        let Some(st) = self.spell_state() else {
            return;
        };
        let Some(checker) = spell::session().and_then(|s| s.loaded_checker()) else {
            return;
        };
        let gone: Vec<(usize, usize)> = self.with_state(|s| {
            st.marks
                .iter()
                .copied()
                .filter(|&(a, b)| checker.check(&text_of(&s.doc, a, b)))
                .collect()
        });
        if !gone.is_empty() {
            self.spell_meta(SpellMeta::Drop(gone));
        }
    }

    /// The user focused the editor (a press in it, a Tab onto it): its first
    /// interaction checks the whole document. The runtime calls it; nothing
    /// happens unless the app has spellcheck on (#2862).
    pub fn spell_user_focus(&self) {
        let Some(session) = self.spell_session() else {
            if self.spell_peek(|_| ()).is_none() {
                // Remembered for dictionaries that arrive later (the web).
                self.spell_driver(|d| d.interacted = true);
            }
            return;
        };
        self.spell_listen();
        if self.spell_peek(|s| !s.activated) == Some(true) {
            self.spell_meta(SpellMeta::Activate);
            self.spell_check(&session);
        }
    }

    /// After a dispatch: an edit the user made (`input`) activates the
    /// checker, which then checks all of it after the pause. Scheduling the
    /// check of later edits is not this path's: any change that leaves words
    /// dirty arms it on its way out of the core ([`Self::spell_owed`]), so a
    /// command, an undo, a picture insert or a collaborator's edit is checked
    /// too.
    pub(crate) fn spell_after_dispatch(&self, doc_changed: bool, input: bool) {
        if !doc_changed || !input {
            return;
        }
        match self.spell_peek(|s| s.activated) {
            None => {
                // Remembered for dictionaries that arrive later (the web).
                self.spell_driver(|d| d.interacted = true);
                return;
            }
            Some(true) => return,
            Some(false) => {}
        }
        if self.spell_session().is_none() {
            return;
        }
        self.spell_listen();
        self.spell_meta(SpellMeta::Activate);
    }

    /// The core left an activated editor with newly dirty words (any path):
    /// check them once typing pauses.
    pub(crate) fn spell_owed(&self) {
        if let Some(session) = self.spell_session() {
            self.spell_arm(session.debounce_ms());
        }
    }

    /// Arm the pause again when words are still waiting.
    fn spell_rearm_if_dirty(&self) {
        if self.spell_peek(|s| s.activated && !s.dirty.is_empty()) == Some(true) {
            self.spell_owed();
        }
    }

    /// Whether the textblock at `pos` is turned off by a `spellcheck`
    /// attribute on its host element or an element up to the container.
    fn spell_block_off(&self, pos: usize) -> bool {
        let (Some(container), Some(host)) = (self.container_node(), self.block_host(pos)) else {
            return false;
        };
        let mut cur = Some(host);
        while let Some(n) = cur {
            if n.node_id() == container.node_id() {
                return false;
            }
            if let Some(on) = n
                .get_attribute("spellcheck")
                .and_then(|v| spell::attribute_state(&v))
            {
                return !on;
            }
            cur = n.parent_node();
        }
        false
    }

    /// Make the marks follow whether the editor is to be checked now: when it
    /// is not (read-only, its `spellcheck` turned off, the app's switch off)
    /// every mark goes; when it is again, an editor the user had interacted
    /// with is checked again. [`set_read_only`](Self::set_read_only) and the
    /// `Editor` component call it; an app that changes a `spellcheck`
    /// attribute on the editor or an ancestor calls it after.
    pub fn refresh_spellcheck(&self) {
        let Some(st) = self.spell_state() else {
            return;
        };
        match self.spell_session() {
            None => {
                if !st.marks.is_empty() || !st.dirty.is_empty() {
                    self.spell_meta(SpellMeta::Clear);
                }
            }
            Some(session) if st.activated => {
                self.spell_meta(SpellMeta::Reset);
                self.spell_check(&session);
            }
            Some(_) => {}
        }
    }

    fn spell_meta(&self, meta: SpellMeta) {
        self.update_internal(move |s| {
            let mut tr = s.tr();
            tr.set_meta(META, meta);
            tr.set_add_to_history(false);
            Some(tr)
        });
    }

    fn spell_arm(&self, delay_ms: u32) {
        let weak = self.downgrade();
        let timer = rinch_core::reactive::unowned(|| {
            rinch_core::timer::set_timeout(delay_ms, move || {
                if let Some(handle) = weak.upgrade() {
                    handle.spell_driver(|d| d.timer = None);
                    if let Some(session) = handle.spell_session() {
                        handle.spell_check(&session);
                    }
                }
            })
        });
        if let Some(old) = self.spell_driver(|d| d.timer.replace(timer)).flatten() {
            rinch_core::timer::clear_timeout(old);
        }
    }

    /// Send the dirty words to the checking thread (on the web: start
    /// checking them in slices).
    pub(crate) fn spell_check(&self, session: &spell::Session) {
        let Some(st) = self.spell_state() else {
            return;
        };
        if st.dirty.is_empty() {
            return;
        }
        if web_path() {
            self.spell_start_slices(&st);
            return;
        }
        let block_off = |pos: usize| self.spell_block_off(pos);
        let jobs = self.with_state(|s| jobs(&s.doc, &st.dirty, &st.skip, &block_off));
        let generation = st.generation;
        let weak = self.downgrade();
        let reply = rinch_core::reactive::unowned(|| {
            rinch_core::park_main_callback(move |(checked, found): CheckResult| {
                if let Some(handle) = weak.upgrade() {
                    handle.spell_meta(SpellMeta::Result {
                        generation,
                        checked,
                        found,
                        upto: None,
                    });
                    // A stale result was dropped and words are still dirty:
                    // check them again after the pause.
                    handle.spell_rearm_if_dirty();
                }
            })
        });
        let session = session.clone();
        let ui_thread = std::thread::current().id();
        spell::spawn(move || {
            let result = run_jobs(&session, &jobs);
            if std::thread::current().id() == ui_thread {
                // Run inline (`set_inline_jobs`): already on the UI thread.
                rinch_core::resume_main_callback(reply, result);
            } else {
                rinch_core::dispatch_main_callback(Box::new(move || {
                    rinch_core::resume_main_callback(reply, result);
                }));
            }
        });
    }

    /// Listen for configuration changes (once).
    fn spell_listen(&self) {
        if self.spell_driver(|d| d.listener.is_some()) == Some(true) {
            return;
        }
        let weak = self.downgrade();
        let id = spell::on_config_change(move || {
            if let Some(handle) = weak.upgrade() {
                handle.spell_config_changed();
            }
        });
        self.spell_driver(|d| d.listener = Some(id));
        let weak = self.downgrade();
        let words = spell::on_words_accepted(move || {
            if let Some(handle) = weak.upgrade() {
                handle.spell_words_accepted();
            }
        });
        self.spell_driver(|d| d.words = Some(words));
        // The checker's reports from other threads (a word the app adds to
        // its personal dictionary) are delivered on this one.
        static WAKE: std::sync::Once = std::sync::Once::new();
        WAKE.call_once(|| {
            spell::set_change_wake(|| {
                rinch_core::dispatch_main_callback(Box::new(spell::deliver_changes));
            });
        });
    }

    fn spell_config_changed(&self) {
        if self.spell_peek(|_| ()).is_none() {
            // The web: the app's dictionaries have arrived (or checking was
            // turned on). The plugin joins the state as it is, history kept;
            // an editor the user already pressed in or typed into is checked
            // now.
            if wanted()
                && self.install_plugin(Rc::new(SpellcheckPlugin::new()), true, |plugins| {
                    plugins.len()
                })
                && self.spell_driver(|d| d.interacted) == Some(true)
            {
                self.spell_user_focus();
            }
            return;
        }
        self.spell_meta(SpellMeta::Reset);
        if let Some(session) = self.spell_session() {
            self.spell_check(&session);
        }
    }

    /// Start (or restart) checking the dirty words in slices, the first in
    /// a task of its own so a press or the debounce's timer returns at once.
    fn spell_start_slices(&self, st: &SpellState) {
        let running = self.spell_driver(|d| {
            d.slicer
                .as_ref()
                .is_some_and(|s| s.generation == st.generation)
        });
        if running == Some(true) {
            // Already on it, for this very document.
            return;
        }
        let blocks = self.with_state(|s| dirty_blocks(&s.doc, &st.dirty, &st.skip));
        let slicer = Slicer {
            generation: st.generation,
            blocks,
            next: 0,
            dirty: st.dirty.clone(),
            skip: st.skip.clone(),
            current: None,
            chunk_bytes: spell::session()
                .map_or(spell::DEFAULT_SLICE_CHUNK_BYTES, |s| s.slices().chunk_bytes),
            last_apply: std::time::Duration::ZERO,
        };
        self.spell_driver(|d| d.slicer = Some(slicer));
        self.spell_schedule_slice();
    }

    fn spell_schedule_slice(&self) {
        let weak = self.downgrade();
        let timer = rinch_core::reactive::unowned(|| {
            rinch_core::timer::yield_now(move || {
                if let Some(handle) = weak.upgrade() {
                    handle.spell_driver(|d| d.slice_timer = None);
                    handle.spell_slice();
                }
            })
        });
        if let Some(old) = self
            .spell_driver(|d| d.slice_timer.replace(timer))
            .flatten()
        {
            rinch_core::timer::clear_timeout(old);
        }
    }

    /// One slice of the web's check: words until the configured budget,
    /// less what applying the last slice's result took, has passed (or the
    /// configured most misspellings are found), the result applied, the next
    /// slice scheduled (`yield_now`: one browser task each). A slice for a document that changed since the check was
    /// taken is dropped with the rest of it: the change put its words on the
    /// dirty list and armed a check of its own, and the words already
    /// checked were taken off it.
    pub(crate) fn spell_slice(&self) {
        let Some(mut slicer) = self.spell_driver(|d| d.slicer.take()).flatten() else {
            return;
        };
        if self.spell_peek(|s| s.activated && s.generation == slicer.generation) != Some(true) {
            return;
        }
        let Some(session) = self.spell_session() else {
            return;
        };
        let started = rinch_core::timer::now();
        let limits = session.slices();
        let budget = std::time::Duration::from_millis(u64::from(limits.budget_ms))
            .saturating_sub(slicer.last_apply);
        let one_chunk = {
            #[cfg(test)]
            {
                ONE_CHUNK_SLICES.with(Cell::get)
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        let block_off = |pos: usize| self.spell_block_off(pos);
        let mut checked = Vec::new();
        let mut found = Vec::new();
        let mut words = 0u64;
        let mut upto = None;
        while let Some(piece) = slicer.next_piece(&block_off) {
            let (from, to) = (piece.start, piece.end);
            let text = &slicer.current.as_ref().expect("a piece has a block").0.text;
            let bad = session.check_ranges(text, std::slice::from_ref(&piece));
            words += 1 + text[piece].matches(char::is_whitespace).count() as u64;
            let a = slicer.pos(from);
            for w in bad {
                let wa = slicer.pos(w.start);
                found.push((wa, slicer.pos(w.end)));
            }
            let b = slicer.pos(to);
            checked.push((a, b));
            if one_chunk
                || found.len() >= limits.max_found
                || rinch_core::timer::now().saturating_duration_since(started) >= budget
            {
                upto = Some(b);
                break;
            }
        }
        // A budget that ran out on the last piece leaves one more slice,
        // which finds nothing and clears the dirty list.
        let generation = slicer.generation;
        let applying = rinch_core::timer::now();
        self.spell_meta(SpellMeta::Result {
            generation,
            checked,
            found,
            upto,
        });
        let end = rinch_core::timer::now();
        if upto.is_some() {
            slicer.last_apply = end.saturating_duration_since(applying);
            self.spell_driver(|d| d.slicer = Some(slicer));
            self.spell_schedule_slice();
        } else {
            self.spell_rearm_if_dirty();
        }
        let us = end.saturating_duration_since(started).as_micros() as u64;
        let apply_us = end.saturating_duration_since(applying).as_micros() as u64;
        STATS.with(|s| {
            let mut v = s.get();
            v.slices += 1;
            v.max_us = v.max_us.max(us);
            v.max_apply_us = v.max_apply_us.max(apply_us);
            v.total_us += us;
            v.words += words;
            s.set(v);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_spell::HunspellChecker;
    use rinch_spell::runtime::SpellcheckConfig;

    const AFF: &[u8] = include_bytes!("../../rinch-spell/tests/fixtures/en_TEST.aff");
    const DIC: &[u8] = include_bytes!("../../rinch-spell/tests/fixtures/en_TEST.dic");

    fn configure() -> spell::Session {
        spell::set_inline_jobs(true);
        spell::set_config(
            SpellcheckConfig::new()
                .checker(HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap())
                .debounce_ms(0),
        );
        spell::session().unwrap()
    }

    /// The marked words, read off the document.
    fn marked(h: &EditorHandle) -> Vec<String> {
        let st = h.spell_state().unwrap();
        h.with_state(|s| {
            st.marks
                .iter()
                .map(|&(a, b)| {
                    let mut out = String::new();
                    s.doc.nodes_between(a, b, &mut |n, at, _| {
                        if let Some(t) = n.text() {
                            let (from, to) = (a.max(at) - at, b.min(at + n.text_len()) - at);
                            out.extend(t.chars().skip(from).take(to - from));
                        }
                        true
                    });
                    out
                })
                .collect()
        })
    }

    /// Insert `text` at `pos`, as an app's edit.
    fn insert(h: &EditorHandle, pos: usize, text: &str) {
        assert!(h.update(|s| {
            let node = s.schema().text(text).unwrap();
            let mut tr = s.tr();
            tr.replace_with(pos, pos, rinch_editor_core::Fragment::from_node(node))
                .unwrap();
            Some(tr)
        }));
    }

    /// A focused editor marks its misspelled words, in every paragraph.
    #[test]
    fn a_focus_marks_the_whole_document() {
        let session = configure();
        let h = crate::create_editor();
        h.load_html("<p>helo the wrld</p><p>a cta</p>");
        assert!(marked(&h).is_empty());
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        assert_eq!(marked(&h), ["helo", "wrld", "cta"]);
    }

    /// The menu's questions and answers (#2862, stage 3): the mark under a
    /// position, ends included, and "Add to dictionary" dropping that word's
    /// marks in every editor at once while the other words stay marked.
    #[test]
    fn add_to_dictionary_drops_the_word_in_every_editor() {
        configure();
        let a = crate::create_editor();
        a.load_html("<p>the helo wrld helo</p>");
        let b = crate::create_editor();
        b.load_html("<p>a helo</p>");
        a.spell_user_focus();
        b.spell_user_focus();
        assert_eq!(marked(&a), ["helo", "wrld", "helo"]);
        assert_eq!(marked(&b), ["helo"]);

        // "the helo": the word is positions 5..9 (paragraph opens at 0).
        let m = a.misspelling_at(Pos(7)).unwrap();
        assert_eq!((m.from, m.to, m.word.as_str()), (Pos(5), Pos(9), "helo"));
        assert_eq!(a.misspelling_at(Pos(9)).unwrap().from, Pos(5), "its end");
        assert_eq!(a.misspelling_at(Pos(3)), None, "a correct word");

        assert!(spell::add_to_dictionary("helo"));
        assert_eq!(marked(&a), ["wrld"]);
        assert!(marked(&b).is_empty());
        assert_eq!(a.misspelling_at(Pos(7)), None);
    }

    /// The marks map through edits that do not touch them: text inserted
    /// before them, in another paragraph, a whole paragraph added above. An
    /// edit of the word itself drops its mark at once and puts it on the
    /// dirty list; the next check re-marks only what is still wrong.
    #[test]
    fn marks_survive_unrelated_transactions() {
        let session = configure();
        let h = crate::create_editor();
        h.load_html("<p>helo the wrld</p><p>a cta</p>");
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        assert_eq!(marked(&h), ["helo", "wrld", "cta"]);

        // Typed at the start of the second paragraph: "the " (all correct).
        let second = h.with_state(|s| s.doc.child(0).node_size()) + 1;
        insert(&h, second, "the ");
        assert_eq!(
            marked(&h),
            ["helo", "wrld", "cta"],
            "a mark moved off its word"
        );
        // Typed space after "wrld" keeps its mark.
        let after_wrld = 1 + "helo the wrld".chars().count();
        insert(&h, after_wrld, " ");
        assert_eq!(marked(&h), ["helo", "wrld", "cta"]);
        // Fix "helo": its mark goes at once.
        insert(&h, 4, "l");
        assert_eq!(marked(&h), ["wrld", "cta"]);
        let dirty = h.spell_state().unwrap().dirty;
        assert!(!dirty.is_empty());
        h.spell_check(&session);
        assert_eq!(marked(&h), ["wrld", "cta"]);
    }

    /// A result for a document that changed since it was taken is dropped.
    #[test]
    fn a_stale_result_is_dropped() {
        configure();
        let h = crate::create_editor();
        h.load_html("<p>helo</p>");
        h.spell_meta(SpellMeta::Activate);
        let generation = h.spell_state().unwrap().generation;
        insert(&h, 1, "x");
        h.spell_meta(SpellMeta::Result {
            generation,
            checked: vec![(1, 5)],
            found: vec![(1, 5)],
            upto: None,
        });
        assert!(marked(&h).is_empty(), "a stale result was applied");
    }

    /// Without a configuration the plugin is not installed at all.
    #[test]
    fn no_configuration_no_plugin() {
        spell::clear_config();
        let h = crate::create_editor();
        assert!(h.spell_state().is_none());
        h.spell_user_focus();
    }

    /// `spellcheck="false"` on the container or an ancestor turns it off.
    #[test]
    fn the_container_attribute_is_inherited() {
        let session = configure();
        let _ = session;
        let doc = std::rc::Rc::new(std::cell::RefCell::new(
            rinch_core::dom::mock::MockDomDocument::new(),
        ));
        let body = rinch_core::dom::DomDocument::body(&*doc.borrow());
        let mut scope = rinch_core::dom::RenderScope::new(doc.clone(), body);
        let wrap = scope.create_element("div");
        wrap.set_attribute("spellcheck", "false");
        let h = crate::create_editor();
        h.load_html("<p>helo</p>");
        let container = h.mount(&mut scope);
        wrap.append_child(&container);
        h.spell_user_focus();
        assert!(
            !h.spell_state().unwrap().activated,
            "checked under spellcheck=false"
        );
        container.set_attribute("spellcheck", "true");
        h.spell_user_focus();
        assert_eq!(marked(&h), ["helo"]);
    }

    /// Take the pending debounce timer, answering whether there was one.
    fn take_timer(h: &EditorHandle) -> bool {
        match h.spell_driver(|d| d.timer.take()).flatten() {
            Some(t) => {
                rinch_core::timer::clear_timeout(t);
                true
            }
            None => false,
        }
    }

    /// Review of #2877: a check sent for one document must not land on the
    /// next one `load_html` puts in, however many edits that one has had.
    /// Red before: `init_state` restarted the generation at 0, so after as
    /// many edits as the old generation the stale marks were applied.
    #[test]
    fn a_result_for_a_replaced_document_is_dropped() {
        configure();
        let h = crate::create_editor();
        h.load_html("<p>helo wrld</p>");
        h.spell_meta(SpellMeta::Activate);
        let stale = h.spell_state().unwrap().generation;
        h.load_html("<p>the cat and the dog</p>");
        h.spell_meta(SpellMeta::Activate);
        for _ in 0..stale.min(64) {
            insert(&h, 1, "a ");
            if h.spell_state().unwrap().generation == stale {
                break;
            }
        }
        h.spell_meta(SpellMeta::Result {
            generation: stale,
            checked: vec![(1, 10)],
            found: vec![(1, 5)],
            upto: None,
        });
        assert!(
            marked(&h).is_empty(),
            "the old document's result was applied"
        );
    }

    /// Review of #2877: every edit path arms the re-check, not only
    /// `update`/typing. Red before: a command (Backspace, undo) left the word
    /// dirty with no timer, so it was never checked again.
    #[test]
    fn a_command_and_an_undo_arm_the_recheck() {
        let session = configure();
        let h = crate::create_editor();
        h.load_html("<p>the wrld</p>");
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        assert_eq!(marked(&h), ["wrld"]);
        take_timer(&h);
        h.set_selection(rinch_editor_core::Selection::cursor(Pos(9)));
        assert!(h.command("deleteCharBackward"));
        assert!(marked(&h).is_empty(), "the edited word kept its mark");
        assert!(take_timer(&h), "Backspace armed no re-check");
        h.spell_check(&session);
        assert_eq!(marked(&h), ["wrl"]);
        assert!(h.command("undo"));
        assert!(take_timer(&h), "undo armed no re-check");
        h.spell_check(&session);
        assert_eq!(marked(&h), ["wrld"]);
    }

    /// Review of #2877: `spellcheck="false"` on a block's host element (as
    /// inside a contenteditable in Chrome), and the plugin's node and mark
    /// exclusions.
    #[test]
    fn blocks_and_marks_can_be_left_out() {
        let session = configure();
        let doc = std::rc::Rc::new(std::cell::RefCell::new(
            rinch_core::dom::mock::MockDomDocument::new(),
        ));
        let body = rinch_core::dom::DomDocument::body(&*doc.borrow());
        let mut scope = rinch_core::dom::RenderScope::new(doc.clone(), body);
        let h = crate::create_editor();
        h.load_html("<p>helo</p><p>wrld</p>");
        let _container = h.mount(&mut scope);
        let second = h.with_state(|s| s.doc.child(0).node_size());
        h.block_host(second)
            .unwrap()
            .set_attribute("spellcheck", "false");
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        assert_eq!(marked(&h), ["helo"]);

        let schema = std::rc::Rc::new(rinch_editor_core::Schema::starter_kit());
        let mut plugins = rinch_editor_core::default_plugins();
        plugins.push(Rc::new(
            SpellcheckPlugin::new()
                .skip_nodes(["code_block"])
                .skip_marks(["code"]),
        ));
        let empty = crate::empty_doc(&schema);
        let h = EditorHandle::unmounted(schema, empty, plugins);
        h.load_html("<p>helo <code>wrld</code></p><pre><code>cta</code></pre><p>dgo</p>");
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        assert_eq!(marked(&h), ["helo", "dgo"]);
    }

    /// Review of #2877: a read-only editor shows no marks, and is checked
    /// again when it is editable again.
    #[test]
    fn read_only_clears_the_marks_and_editable_brings_them_back() {
        let session = configure();
        let h = crate::create_editor();
        h.load_html("<p>helo</p>");
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        assert_eq!(marked(&h), ["helo"]);
        h.set_read_only(true);
        assert!(marked(&h).is_empty(), "a read-only editor kept its marks");
        h.set_read_only(false);
        assert_eq!(marked(&h), ["helo"]);
    }

    /// The web's path on this thread: slices of one piece each.
    fn web_path(on: bool) {
        WEB_PATH.with(|w| w.set(on));
        ONE_CHUNK_SLICES.with(|w| w.set(on));
    }

    /// Run the pending slices as the browser would, one task each; answers
    /// how many ran.
    fn run_slices(h: &EditorHandle) -> u32 {
        let mut n = 0;
        while let Some(t) = h.spell_driver(|d| d.slice_timer.take()).flatten() {
            rinch_core::timer::clear_timeout(t);
            h.spell_slice();
            n += 1;
            assert!(n < 10_000, "the slices never end");
        }
        n
    }

    /// A document of `n` paragraphs, every third word misspelled, longer
    /// than one piece each.
    fn long_doc(n: usize) -> String {
        let line = "the cat and the dog helo ".repeat(30);
        (0..n)
            .map(|i| format!("<p>{line}wrld{i}x cta</p>"))
            .collect()
    }

    /// The web (#2862): the whole-document check of a long document runs in
    /// many slices, each applied as it comes, and ends with what one
    /// desktop check marks.
    #[test]
    fn the_web_checks_a_long_document_in_slices() {
        let session = configure();
        let html = long_doc(6);
        let desktop = crate::create_editor();
        desktop.load_html(&html);
        desktop.spell_meta(SpellMeta::Activate);
        desktop.spell_check(&session);
        let want = marked(&desktop);
        assert_eq!(want.len(), 6 * 31, "{want:?}");

        web_path(true);
        let h = crate::create_editor();
        h.load_html(&html);
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        assert!(
            marked(&h).is_empty(),
            "the first slice ran inside the press"
        );
        // One slice in: marks for the first piece only, the rest still dirty.
        assert!(
            h.spell_driver(|d| d.slice_timer.take())
                .flatten()
                .map(rinch_core::timer::clear_timeout)
                .is_some()
        );
        h.spell_slice();
        let first = marked(&h).len();
        assert!(first > 0 && first < want.len(), "{first}");
        assert!(!h.spell_state().unwrap().dirty.is_empty());
        let slices = run_slices(&h);
        assert!(slices > 6, "{slices} slices");
        assert_eq!(marked(&h), want);
        assert!(h.spell_state().unwrap().dirty.is_empty());
        web_path(false);
    }

    /// An edit between two slices drops the rest of that check; the words
    /// already checked keep their marks and are not checked again, and the
    /// edit's own check (after the pause) finishes the document.
    #[test]
    fn an_edit_between_slices_keeps_what_was_checked() {
        let session = configure();
        web_path(true);
        let h = crate::create_editor();
        h.load_html(&long_doc(4));
        h.spell_meta(SpellMeta::Activate);
        h.spell_check(&session);
        take_timer(&h);
        for _ in 0..3 {
            let t = h.spell_driver(|d| d.slice_timer.take()).flatten().unwrap();
            rinch_core::timer::clear_timeout(t);
            h.spell_slice();
        }
        let before = marked(&h);
        assert!(!before.is_empty());
        let dirty_start = h.spell_state().unwrap().dirty[0].0;
        assert!(dirty_start > 1, "nothing was taken off the dirty list");
        insert(&h, 1, "zzq ");
        // The edit armed its check; the rest of the old one goes.
        assert!(take_timer(&h), "the edit armed no check");
        assert_eq!(run_slices(&h), 1, "a stale slice was not dropped");
        assert_eq!(marked(&h), before, "marks moved off their words");
        h.spell_check(&session);
        run_slices(&h);
        web_path(false);
        let fresh = crate::create_editor();
        fresh.load_html(&long_doc(4).replacen("<p>", "<p>zzq ", 1));
        fresh.spell_meta(SpellMeta::Activate);
        fresh.spell_check(&session);
        let want = marked(&fresh);
        assert_eq!(want[0], "zzq");
        assert_eq!(marked(&h), want);
    }

    /// The web (#2862): with no dictionaries of the app's, an editor has no
    /// plugin and does nothing; when they arrive, the plugin joins with the
    /// undo history kept, and an editor the user already pressed in is
    /// checked.
    #[test]
    fn dictionaries_that_arrive_later_install_the_checker() {
        spell::set_inline_jobs(true);
        spell::set_config(SpellcheckConfig {
            dictionaries: spell::Dictionaries::None,
            ..SpellcheckConfig::default()
        });
        web_path(true);
        let h = crate::create_editor();
        h.load_html("<p>the cat</p>");
        assert!(
            h.spell_state().is_none(),
            "a plugin with nothing to check against"
        );
        insert(&h, 8, " helo");
        h.spell_user_focus();
        assert!(h.spell_state().is_none());

        configure();
        assert!(
            h.spell_state().is_some(),
            "the arriving dictionaries installed nothing"
        );
        run_slices(&h);
        assert_eq!(marked(&h), ["helo"]);
        assert!(h.command("undo"), "the undo history was lost");
        web_path(false);
    }

    /// Review of #2930: an editor made before the app configured anything at
    /// all (a handle made before `App::run`, an island whose app configures
    /// after its fetch) is checked once dictionaries arrive. Red before: no
    /// listener was registered without a configuration, and the first
    /// `set_config` told no listener.
    #[test]
    fn an_editor_made_before_any_configuration_is_checked_later() {
        spell::set_inline_jobs(true);
        spell::clear_config();
        web_path(true);
        let h = crate::create_editor();
        h.load_html("<p>helo the wrld</p>");
        h.spell_user_focus();
        assert!(h.spell_state().is_none());
        configure();
        assert!(
            h.spell_state().is_some(),
            "the dictionaries installed nothing"
        );
        run_slices(&h);
        assert_eq!(marked(&h), ["helo", "wrld"]);
        web_path(false);
    }

    /// The slice settings are the app's: a smaller piece makes more slices.
    #[test]
    fn the_slice_settings_are_the_apps() {
        spell::set_inline_jobs(true);
        let count = |chunk: usize| {
            spell::set_config(
                SpellcheckConfig::new()
                    .checker(HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap())
                    .debounce_ms(0)
                    .slice_chunk_bytes(chunk),
            );
            let session = spell::session().unwrap();
            let h = crate::create_editor();
            h.load_html(&long_doc(2));
            h.spell_meta(SpellMeta::Activate);
            h.spell_check(&session);
            run_slices(&h)
        };
        web_path(true);
        let (big, small) = (count(4096), count(64));
        assert!(small > big * 4, "{small} slices of 64 bytes, {big} of 4096");
        web_path(false);
    }
}
