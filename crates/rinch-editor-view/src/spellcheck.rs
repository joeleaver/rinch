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
//! The plugin is installed by [`create_editor`](crate::create_editor) only
//! when the thread has a spellcheck configuration (an app run through
//! `App::run`, an embedded `RinchContext`), never on the web (the browser
//! checks there) and never in a test that configured none.

use std::any::Any;
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
    /// A check came back: `found` are the misspelled words in `checked`, for
    /// the document at `generation`.
    Result {
        generation: u64,
        checked: Vec<(usize, usize)>,
        found: Vec<(usize, usize)>,
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
            Some(SpellMeta::Result {
                generation,
                checked,
                found,
            }) if st.activated && *generation == st.generation => {
                st.marks
                    .retain(|&(a, b)| !checked.iter().any(|&(c, d)| a < d && b > c));
                st.marks.extend(found.iter().copied());
                st.marks.sort_unstable();
                st.dirty.clear();
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
                    if !blocks.iter().any(|(p, _)| *p == pos) && !block_off(pos) {
                        blocks.push((pos, node.clone()));
                    }
                    false
                } else {
                    true
                }
            },
        );
    }
    let mut out = Vec::new();
    for (pos, block) in blocks {
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
        if !ranges.is_empty() {
            out.push(BlockJob {
                start,
                text,
                ranges,
            });
        }
    }
    out
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

/// The handle's half: when to check, and the trip to the checking thread.
#[derive(Default)]
pub(crate) struct Driver {
    timer: Option<rinch_core::timer::TimeoutHandle>,
    listener: Option<u64>,
}

impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(t) = self.timer.take() {
            rinch_core::timer::clear_timeout(t);
        }
        if let Some(id) = self.listener.take() {
            spell::remove_config_listener(id);
        }
    }
}

impl EditorHandle {
    /// The spellchecker's state, when the plugin is installed.
    pub(crate) fn spell_state(&self) -> Option<SpellState> {
        self.with_state(|s| s.plugin_state::<SpellState>(SPELLCHECK_KEY).cloned())
    }

    /// Whether this editor is to be checked now: the plugin is installed, the
    /// app has spellcheck on with a dictionary, the editor is editable, and
    /// its container's `spellcheck` state (its own or the nearest ancestor's)
    /// is on.
    fn spell_session(&self) -> Option<spell::Session> {
        self.spell_state()?;
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

    /// The user focused the editor (a press in it, a Tab onto it): its first
    /// interaction checks the whole document. The runtime calls it; nothing
    /// happens unless the app has spellcheck on (#2862).
    pub fn spell_user_focus(&self) {
        let Some(session) = self.spell_session() else {
            return;
        };
        self.spell_listen();
        if self.spell_state().is_some_and(|s| !s.activated) {
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
        if self.spell_state().is_none_or(|s| s.activated) || self.spell_session().is_none() {
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
        if self
            .spell_state()
            .is_some_and(|s| s.activated && !s.dirty.is_empty())
        {
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

    /// Send the dirty words to the checking thread.
    pub(crate) fn spell_check(&self, session: &spell::Session) {
        let Some(st) = self.spell_state() else {
            return;
        };
        if st.dirty.is_empty() {
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
                handle.spell_meta(SpellMeta::Reset);
                if let Some(session) = handle.spell_session() {
                    handle.spell_check(&session);
                }
            }
        });
        self.spell_driver(|d| d.listener = Some(id));
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
}
