//! Spelling marks in `<input>` and `<textarea>` (#2862, stage 2).
//!
//! A text field is checked once the user has **interacted** with it, which is
//! Chrome's rule (measured in Chrome 158, the probe in
//! `docs/internals/spellcheck.md`): a focus by pointer or Tab, or an edit. A
//! focus from script (`NodeHandle::focus`, `request_focus`) alone does not
//! count. On that first interaction the whole value is checked; after an edit
//! only the words the edit touched are, once typing has paused for the
//! configured debounce (Chrome's ~1 s idle). The marks of the other words move
//! with the text in the meantime, and the edited word's is dropped at once.
//!
//! The words are checked on the `rinch-spellcheck` thread
//! ([`rinch_spell::runtime::spawn`]), which also loads the dictionaries the
//! first time; the result comes back through the app's deferred-work inbox and
//! is written to the control as [`rinch_dom::SpellingMarks`], a paint-only
//! change. Nothing here runs for an app that never configured spellcheck, has
//! it off, or found no dictionary: [`rinch_spell::runtime::session`] answers
//! `None` and every entry point returns at its first line.

use std::collections::HashMap;
use std::ops::Range;

use rinch_spell::runtime as spell;

use super::RinchApp;

/// The fields the user has interacted with, and their marks.
#[derive(Default)]
pub(crate) struct FieldSpelling {
    fields: HashMap<u64, Field>,
    /// The pending debounce wake and when it is due.
    timer: Option<(rinch_core::timer::TimeoutHandle, rinch_core::timer::Instant)>,
    /// The configuration listener, registered at the first interaction.
    pub(super) listener: Option<u64>,
    /// The last [`Field::generation`] handed out, across all fields, so a
    /// field replaced by a new one never takes a result meant for the old.
    generations: u64,
}

struct Field {
    /// The value `marks` and `dirty` are byte ranges of.
    text: String,
    /// The misspelled words, sorted.
    marks: Vec<Range<usize>>,
    /// What of `text` is waiting to be checked, sorted and disjoint.
    dirty: Vec<Range<usize>>,
    /// Bumped at every change of `text`: a result for an older one is stale.
    generation: u64,
    /// When the debounce after the last edit runs out.
    due: Option<rinch_core::timer::Instant>,
}

impl FieldSpelling {
    /// A field to check whole, its value `text`.
    fn fresh(&mut self, text: String) -> Field {
        self.generations += 1;
        Field {
            dirty: std::iter::once(0..text.len()).collect(),
            text,
            marks: Vec::new(),
            generation: self.generations,
            due: None,
        }
    }

    /// Forget everything and stop listening (the app is going away).
    pub(crate) fn shut_down(&mut self) {
        if let Some((timer, _)) = self.timer.take() {
            rinch_core::timer::clear_timeout(timer);
        }
        if let Some(id) = self.listener.take() {
            spell::remove_config_listener(id);
        }
        self.fields.clear();
    }
}

impl RinchApp {
    /// Whether text control `node_id` is one spellcheck marks: a `<textarea>`
    /// or a prose `<input>` (no type, `text`, `search`), not read-only, whose
    /// `spellcheck` state (its own, else its nearest ancestor's) is on.
    pub(crate) fn field_is_spellchecked(&self, node_id: u64) -> bool {
        let Some(doc) = &self.doc else { return false };
        let d = doc.borrow();
        let tree = &d.tree;
        let Some(node) = tree.get(node_id) else {
            return false;
        };
        let prose = match node.tag() {
            Some("textarea") => true,
            Some("input") => {
                spell::input_type_is_checked(node.attributes.get("type").map(String::as_str))
            }
            _ => false,
        };
        prose
            && !Self::node_is_readonly(node)
            && spell::resolve_inherited(
                node_id,
                |&n| tree.get(n).and_then(|n| n.parent),
                |&n| {
                    tree.get(n)
                        .and_then(|n| n.attributes.get("spellcheck"))
                        .and_then(|v| spell::attribute_state(v))
                },
            )
    }

    fn field_value(&self, node_id: u64) -> Option<String> {
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        d.tree.get(node_id)?;
        Some(
            rinch_dom::form_control::control_value(&d.tree.nodes, node_id)
                .map(|v| v.into_owned())
                .unwrap_or_default(),
        )
    }

    /// The user focused text control `node_id` (a press or Tab). Its first
    /// interaction checks the whole value now; a value the app changed while
    /// it was away is checked again.
    pub(crate) fn spell_user_focus(&mut self, node_id: u64) {
        let Some(session) = spell::session() else {
            return;
        };
        if !self.field_is_spellchecked(node_id) {
            return;
        }
        let Some(value) = self.field_value(node_id) else {
            return;
        };
        self.spell_listen();
        match self.spelling.fields.get(&node_id) {
            Some(field) if field.text == value => {}
            _ => {
                let field = self.spelling.fresh(value);
                self.spelling.fields.insert(node_id, field);
                self.spell_write(node_id);
                self.spell_send(node_id, &session);
            }
        }
    }

    /// The user edited text control `node_id`, which now holds `value`. The
    /// marks move with the text, the edited word's go, and the words the edit
    /// touched are checked once typing pauses. A field the user had not
    /// interacted with yet (one focused by script) is checked whole then.
    pub(crate) fn spell_user_edit(&mut self, node_id: u64, value: &str) {
        let Some(session) = spell::session() else {
            return;
        };
        if !self.field_is_spellchecked(node_id) {
            return;
        }
        self.spell_listen();
        if !self.spelling.fields.contains_key(&node_id) {
            let field = self.spelling.fresh(value.to_owned());
            self.spelling.fields.insert(node_id, field);
        }
        self.spelling.generations += 1;
        let next_generation = self.spelling.generations;
        let Some(field) = self.spelling.fields.get_mut(&node_id) else {
            return;
        };
        if let Some(change) = spell::text_change(&field.text, value) {
            spell::map_through_edit(&mut field.marks, change, value);
            let region = spell::edit_region(value, change);
            let (start, old_end, new_end) = change;
            // Dirty ranges move like marks, except that one the edit touches
            // is stretched over the edit instead of going.
            let mut dirty = std::mem::take(&mut field.dirty);
            for r in &mut dirty {
                if r.end < start {
                    continue;
                }
                if r.start > old_end {
                    r.start = r.start - old_end + new_end;
                    r.end = r.end - old_end + new_end;
                } else {
                    r.start = r.start.min(start);
                    r.end = if r.end > old_end {
                        r.end - old_end + new_end
                    } else {
                        new_end
                    };
                }
            }
            dirty.sort_by_key(|r| r.start);
            let mut merged = Vec::with_capacity(dirty.len() + 1);
            for r in dirty {
                spell::add_range(&mut merged, r);
            }
            spell::add_range(&mut merged, region);
            let dirty = merged;
            field.dirty = dirty;
            field.text = value.to_owned();
            field.generation = next_generation;
        }
        if field.dirty.is_empty() {
            return;
        }
        let due = rinch_core::timer::now()
            + std::time::Duration::from_millis(u64::from(session.debounce_ms()));
        field.due = Some(due);
        self.spell_write(node_id);
        self.spell_arm(due);
    }

    /// Arm the debounce wake for `due`, unless one is armed for no later.
    fn spell_arm(&mut self, due: rinch_core::timer::Instant) {
        if let Some((timer, at)) = self.spelling.timer {
            if at <= due {
                return;
            }
            rinch_core::timer::clear_timeout(timer);
        }
        let delay = due.saturating_duration_since(rinch_core::timer::now());
        let sender = self.app_work_sender();
        let timer = rinch_core::reactive::unowned(|| {
            rinch_core::timer::set_timeout(
                delay.as_millis().min(u32::MAX as u128) as u32,
                move || {
                    sender.send(|app| {
                        app.spelling.timer = None;
                        app.spell_tick();
                    });
                },
            )
        });
        self.spelling.timer = Some((timer, due));
    }

    /// Check every field whose debounce has run out, and re-arm for the rest.
    pub(crate) fn spell_tick(&mut self) {
        let now = rinch_core::timer::now();
        let due: Vec<u64> = self
            .spelling
            .fields
            .iter()
            .filter(|(_, f)| f.due.is_some_and(|d| d <= now))
            .map(|(&id, _)| id)
            .collect();
        if let Some(session) = spell::session() {
            for id in due {
                self.spell_send(id, &session);
            }
        }
        if let Some(next) = self.spelling.fields.values().filter_map(|f| f.due).min() {
            self.spell_arm(next);
        }
    }

    /// Check what of `node_id`'s value is dirty, on the checking thread.
    fn spell_send(&mut self, node_id: u64, session: &spell::Session) {
        let Some(field) = self.spelling.fields.get_mut(&node_id) else {
            return;
        };
        field.due = None;
        if field.dirty.is_empty() {
            return;
        }
        let text = field.text.clone();
        let ranges = field.dirty.clone();
        let generation = field.generation;
        let config = session.generation();
        let session = session.clone();
        let sender = self.app_work_sender();
        spell::spawn(move || {
            let found = session.check_ranges(&text, &ranges);
            sender.send(move |app| app.spell_apply(node_id, generation, config, ranges, found));
        });
    }

    /// A result from the checking thread: the misspelled words `found` in
    /// `checked`, for the value at `generation`.
    fn spell_apply(
        &mut self,
        node_id: u64,
        generation: u64,
        config: u64,
        checked: Vec<Range<usize>>,
        found: Vec<Range<usize>>,
    ) {
        if config != spell::generation() {
            return;
        }
        if self.field_value(node_id).is_none() {
            self.spelling.fields.remove(&node_id);
            return;
        }
        let Some(field) = self.spelling.fields.get_mut(&node_id) else {
            return;
        };
        if field.generation != generation {
            return;
        }
        field
            .marks
            .retain(|m| !checked.iter().any(|c| m.start < c.end && m.end > c.start));
        field.marks.extend(found);
        field.marks.sort_by_key(|m| m.start);
        field.dirty.clear();
        self.spell_write(node_id);
    }

    /// Show `node_id`'s marks on the control.
    fn spell_write(&mut self, node_id: u64) {
        let Some(field) = self.spelling.fields.get(&node_id) else {
            return;
        };
        let marks = rinch_dom::SpellingMarks {
            text: field.text.clone(),
            ranges: field.marks.clone(),
        };
        if let Some(doc) = &self.doc {
            doc.borrow_mut().set_spelling_marks(node_id, Some(marks));
        }
    }

    /// Listen for configuration changes (once).
    fn spell_listen(&mut self) {
        if self.spelling.listener.is_some() {
            return;
        }
        let sender = self.app_work_sender();
        self.spelling.listener = Some(spell::on_config_change(move || {
            sender.send(|app| app.spell_config_changed());
        }));
    }

    /// The app changed its spellcheck configuration: every mark goes, and
    /// the fields the user interacted with are checked again under the new
    /// one. Turned off, they are remembered unchecked, so turning it back on
    /// marks them again.
    fn spell_config_changed(&mut self) {
        let ids: Vec<u64> = self.spelling.fields.keys().copied().collect();
        if let Some((timer, _)) = self.spelling.timer.take() {
            rinch_core::timer::clear_timeout(timer);
        }
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            for &id in &ids {
                d.set_spelling_marks(id, None);
            }
        }
        self.spelling.fields.clear();
        let session = spell::session();
        for id in ids {
            let Some(value) = self.field_value(id) else {
                continue;
            };
            let field = self.spelling.fresh(value);
            self.spelling.fields.insert(id, field);
            if let Some(session) = &session
                && self.field_is_spellchecked(id)
            {
                self.spell_send(id, session);
            }
        }
    }
}
