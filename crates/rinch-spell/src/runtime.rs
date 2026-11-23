//! What a UI runtime needs to mark misspelled words (#2862, stage 2).
//!
//! The rinch runtime's text fields and its rich-text editor share this:
//!
//! - **Which fields are checked**: [`attribute_state`] reads a `spellcheck`
//!   attribute as HTML's enumerated states, [`resolve_inherited`] takes the
//!   nearest ancestor's state (HTML's inheritance rule), and
//!   [`input_type_is_checked`] keeps the non-prose `<input>` types out.
//! - **The app's configuration** ([`SpellcheckConfig`]): the switch, the
//!   dictionaries (the system's for the user's locales by default, or the
//!   app's own checker), the personal dictionary, the skip rules and the
//!   debounce. It is held per UI thread ([`set_config`]); the runtime installs
//!   it at startup from the `App` builder.
//! - **Checking off the UI thread**: a [`Session`] is a cheap, `Send` snapshot
//!   of the configuration that checks byte ranges of a text from any thread,
//!   loading the system dictionaries there the first time; [`spawn`] runs a
//!   job on one shared background thread.
//! - **Keeping marks across an edit**: [`map_through_edit`] moves marks
//!   through a text change and [`edit_region`] says which part of the new text
//!   needs checking again, so only the edited words are re-checked.
//!
//! Nothing here runs on its own: a thread that never calls [`set_config`] has
//! no session ([`session`] answers `None`), and no thread is started until the
//! first [`spawn`].

use std::ops::Range;

/// The state a `spellcheck` attribute's value names, HTML's enumerated
/// attribute rule: `"true"` and `""` are on, `"false"` is off (ASCII case
/// insensitive), and anything else is the *default* state, which inherits
/// (`None`).
pub fn attribute_state(value: &str) -> Option<bool> {
    if value.is_empty() || value.eq_ignore_ascii_case("true") {
        Some(true)
    } else if value.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

/// Whether `spellcheck` is on for `node`: its own state, else its nearest
/// ancestor's that names one (HTML: an element in the default state takes its
/// parent's), else `true` (on by default, as in Chrome).
///
/// `state(n)` is `n`'s [`attribute_state`] (`None` when it has no
/// `spellcheck` attribute), `parent(n)` its parent.
pub fn resolve_inherited<N>(
    node: N,
    parent: impl Fn(&N) -> Option<N>,
    state: impl Fn(&N) -> Option<bool>,
) -> bool {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if let Some(on) = state(&n) {
            return on;
        }
        cur = parent(&n);
    }
    true
}

/// Whether an `<input>` of `type` holds prose to check: no type, `text` and
/// `search` do, and so does an unknown type (HTML falls back to the Text
/// state for one). `password`, `email`, `url`, `number`, `tel` and every
/// other known type do not.
pub fn input_type_is_checked(ty: Option<&str>) -> bool {
    let Some(ty) = ty else { return true };
    let ty = ty.trim();
    if ty.is_empty() || ty.eq_ignore_ascii_case("text") || ty.eq_ignore_ascii_case("search") {
        return true;
    }
    const NOT_PROSE: &[&str] = &[
        "password",
        "email",
        "url",
        "number",
        "tel",
        "hidden",
        "checkbox",
        "radio",
        "file",
        "submit",
        "image",
        "reset",
        "button",
        "color",
        "range",
        "date",
        "datetime-local",
        "month",
        "week",
        "time",
    ];
    !NOT_PROSE.iter().any(|t| ty.eq_ignore_ascii_case(t))
}

/// The part of `old` and `new` that differs, as `(start, old_end, new_end)`
/// byte offsets on char boundaries: `old[start..old_end]` became
/// `new[start..new_end]`. `None` when the texts are equal.
pub fn text_change(old: &str, new: &str) -> Option<(usize, usize, usize)> {
    if old == new {
        return None;
    }
    let (ob, nb) = (old.as_bytes(), new.as_bytes());
    let mut prefix = ob.iter().zip(nb).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let max_suffix = ob.len().min(nb.len()) - prefix;
    let mut suffix = ob
        .iter()
        .rev()
        .zip(nb.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    Some((prefix, old.len() - suffix, new.len() - suffix))
}

/// Moves `marks` (byte ranges of `old`) through the change
/// [`text_change`]`(old, new)` reports: a mark before it stays, a mark after
/// it shifts by the change in length, and a mark the change touches is
/// dropped (its word changed and is checked again) — except one the change
/// only abuts with inserted white space, a space typed after a word, which
/// leaves the word as it was.
pub fn map_through_edit(marks: &mut Vec<Range<usize>>, change: (usize, usize, usize), new: &str) {
    let (start, old_end, new_end) = change;
    let inserted = &new[start..new_end];
    let space_after = inserted.chars().next().is_some_and(char::is_whitespace);
    let space_before = inserted
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace);
    marks.retain_mut(|m| {
        if m.end < start || (m.end == start && space_after) {
            true
        } else if m.start > old_end || (m.start == old_end && space_before) {
            m.start = m.start - old_end + new_end;
            m.end = m.end - old_end + new_end;
            true
        } else {
            false
        }
    });
}

/// The part of `new` to check again after the change [`text_change`]
/// reports: the changed bytes widened to the white space around them, so
/// every word the change touches (and a URL or address around it, which the
/// skip rules judge whole) is checked whole. An inserted text that starts
/// (ends) with white space is not widened backward (forward): a space typed
/// after a word leaves that word as it was.
pub fn edit_region(new: &str, change: (usize, usize, usize)) -> Range<usize> {
    let (start, _, end) = change;
    let inserted = &new[start..end];
    let widened = widen_to_tokens(new, start..end);
    let from = if inserted.chars().next().is_some_and(char::is_whitespace) {
        start
    } else {
        widened.start
    };
    let to = if inserted
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
    {
        end
    } else {
        widened.end
    };
    from..to
}

/// `range` of `text` widened to the white space on either side.
pub fn widen_to_tokens(text: &str, range: Range<usize>) -> Range<usize> {
    let start = text[..range.start]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[range.end..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map_or(text.len(), |(i, _)| range.end + i);
    start..end
}

/// Adds `range` to a sorted list of disjoint ranges, merging what it touches.
pub fn add_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    let mut range = range;
    let mut out = Vec::with_capacity(ranges.len() + 1);
    let mut placed = false;
    for r in ranges.drain(..) {
        if r.end < range.start {
            out.push(r);
        } else if r.start > range.end {
            if !placed {
                out.push(range.clone());
                placed = true;
            }
            out.push(r);
        } else {
            range = range.start.min(r.start)..range.end.max(r.end);
        }
    }
    if !placed {
        out.push(range);
    }
    *ranges = out;
}

pub use native::*;

mod native {
    use std::cell::RefCell;
    use std::ops::Range;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex, OnceLock};

    use crate::{
        ListenerId, PersonalDictionary, PersonalDictionaryChange, SkipRules, SpellChange,
        SpellChecker, words,
    };

    /// Chrome's idle delay before a typed word is checked: measured in Chrome
    /// 158 (#2862), a word typed with no separator after it is unmarked 0.5 s
    /// after the last key and marked at 1 s.
    pub const DEFAULT_DEBOUNCE_MS: u32 = 1000;

    /// How many suggestions the text context menu offers: Chrome 158's
    /// context menu shows three (measured, #2862), although its engine asks
    /// for [`crate::DEFAULT_MAX_SUGGESTIONS`].
    pub const DEFAULT_MENU_SUGGESTIONS: usize = 3;

    /// How long a right press on a misspelled word waits for its suggestions
    /// before the menu opens without them (they fill in when they come).
    pub const DEFAULT_MENU_SUGGEST_WAIT_MS: u32 = 100;

    /// Where the words come from.
    #[derive(Clone)]
    pub enum Dictionaries {
        /// The system's Hunspell dictionaries (`/usr/share/hunspell`, …) for
        /// `locales`, or for the user's locales when `None`
        /// ([`crate::system::user_locales`]). Loaded on the checking thread
        /// the first time a word is checked. None found marks nothing.
        System {
            /// The locales to load (`en_US`, `de`); `None` is the user's.
            locales: Option<Vec<String>>,
            /// The personal dictionary to share with the checker; `None`
            /// gives it an empty one of its own.
            personal: Option<Arc<PersonalDictionary>>,
        },
        /// The app's own checker (bundled dictionaries, a platform engine).
        Checker(Arc<dyn SpellChecker>),
        /// Nothing to check against: nothing is ever marked.
        None,
    }

    impl std::fmt::Debug for Dictionaries {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Dictionaries::System { locales, personal } => f
                    .debug_struct("System")
                    .field("locales", locales)
                    .field("personal", &personal.as_ref().map(|p| p.len()))
                    .finish(),
                Dictionaries::Checker(c) => f.debug_tuple("Checker").field(&c.languages()).finish(),
                Dictionaries::None => f.write_str("None"),
            }
        }
    }

    /// The app's spellcheck settings. [`Default`] is Chrome's: on, the system
    /// dictionaries for the user's locales, [`SkipRules::default`] (which
    /// skips URLs, emails, words with digits and all-caps words, a deliberate
    /// exception to Chrome), and a [`DEFAULT_DEBOUNCE_MS`] pause after typing.
    /// `SpellcheckConfig::from(false)` turns it off.
    #[derive(Clone, Debug)]
    pub struct SpellcheckConfig {
        /// The app-wide switch. Off, no field and no editor is checked, and
        /// no marks are kept.
        pub enabled: bool,
        /// Where the words come from.
        pub dictionaries: Dictionaries,
        /// What is never checked.
        pub skip_rules: SkipRules,
        /// How long typing must pause before the edited words are checked.
        pub debounce_ms: u32,
        /// Check a field only once the user has interacted with it (pressed
        /// in it, tabbed onto it, typed in it), as Chrome does. Off, every
        /// checked field on screen is checked as soon as it is laid out, and
        /// a value the app writes is checked after the debounce, at the cost
        /// of a walk over the document's controls on each frame that changed
        /// something.
        pub require_interaction: bool,
        /// Hide a single-line `<input>`'s marks while it does not have the
        /// keyboard, as Chrome does (a `<textarea>` and the editor keep
        /// theirs). Off, an `<input>` shows its marks blurred too.
        pub hide_input_marks_on_blur: bool,
        /// How many suggestions the text context menu shows above its edit
        /// rows for a misspelled word ([`DEFAULT_MENU_SUGGESTIONS`], Chrome's
        /// three). `0` shows none and never asks the engine for any.
        pub menu_suggestions: usize,
        /// Whether the text context menu offers "Add to dictionary" for a
        /// misspelled word (default `true`, as Chrome does).
        pub menu_add_to_dictionary: bool,
        /// Whether the text context menu shows a disabled "No spelling
        /// suggestions" row for a misspelled word the engine has no
        /// suggestion for. Default `false`: Chrome 158 shows no row then
        /// (measured, #2862), only "Add to dictionary".
        pub menu_no_suggestions_row: bool,
        /// How long the press that opens the text context menu waits for a
        /// misspelled word's suggestions ([`DEFAULT_MENU_SUGGEST_WAIT_MS`]).
        /// Past it the menu opens without them and fills them in when they
        /// arrive. `0` never waits.
        pub menu_suggest_wait_ms: u32,
    }

    impl Default for SpellcheckConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                dictionaries: Dictionaries::System {
                    locales: None,
                    personal: None,
                },
                skip_rules: SkipRules::default(),
                debounce_ms: DEFAULT_DEBOUNCE_MS,
                require_interaction: true,
                hide_input_marks_on_blur: true,
                menu_suggestions: DEFAULT_MENU_SUGGESTIONS,
                menu_add_to_dictionary: true,
                menu_no_suggestions_row: false,
                menu_suggest_wait_ms: DEFAULT_MENU_SUGGEST_WAIT_MS,
            }
        }
    }

    impl From<bool> for SpellcheckConfig {
        fn from(enabled: bool) -> Self {
            Self {
                enabled,
                ..Self::default()
            }
        }
    }

    impl SpellcheckConfig {
        /// The default (on, system dictionaries).
        pub fn new() -> Self {
            Self::default()
        }

        /// Turns checking on or off.
        pub fn enabled(mut self, enabled: bool) -> Self {
            self.enabled = enabled;
            self
        }

        /// The system dictionaries for these locales instead of the user's.
        pub fn languages<I, S>(mut self, locales: I) -> Self
        where
            I: IntoIterator<Item = S>,
            S: Into<String>,
        {
            let locales = Some(locales.into_iter().map(Into::into).collect());
            self.dictionaries = match self.dictionaries {
                Dictionaries::System { personal, .. } => Dictionaries::System { locales, personal },
                _ => Dictionaries::System {
                    locales,
                    personal: None,
                },
            };
            self
        }

        /// Shares `personal` with the system-dictionary checker (the app loads
        /// and saves it).
        pub fn personal_dictionary(mut self, personal: Arc<PersonalDictionary>) -> Self {
            self.dictionaries = match self.dictionaries {
                Dictionaries::System { locales, .. } => Dictionaries::System {
                    locales,
                    personal: Some(personal),
                },
                _ => Dictionaries::System {
                    locales: None,
                    personal: Some(personal),
                },
            };
            self
        }

        /// Checks with the app's own checker.
        pub fn checker(mut self, checker: impl SpellChecker + 'static) -> Self {
            self.dictionaries = Dictionaries::Checker(Arc::new(checker));
            self
        }

        /// Checks with a checker the app keeps a handle to.
        pub fn shared_checker(mut self, checker: Arc<dyn SpellChecker>) -> Self {
            self.dictionaries = Dictionaries::Checker(checker);
            self
        }

        /// The skip rules.
        pub fn skip_rules(mut self, rules: SkipRules) -> Self {
            self.skip_rules = rules;
            self
        }

        /// The pause after typing before edited words are checked.
        pub fn debounce_ms(mut self, ms: u32) -> Self {
            self.debounce_ms = ms;
            self
        }

        /// Whether a field waits for the user's interaction before it is
        /// checked (default `true`, Chrome's rule).
        pub fn require_interaction(mut self, on: bool) -> Self {
            self.require_interaction = on;
            self
        }

        /// Whether a blurred `<input>` hides its marks (default `true`, as
        /// Chrome does).
        pub fn hide_input_marks_on_blur(mut self, on: bool) -> Self {
            self.hide_input_marks_on_blur = on;
            self
        }

        /// How many suggestions the text context menu shows (default
        /// [`DEFAULT_MENU_SUGGESTIONS`]; `0` for none).
        pub fn menu_suggestions(mut self, count: usize) -> Self {
            self.menu_suggestions = count;
            self
        }

        /// Whether the text context menu offers "Add to dictionary" (default
        /// `true`).
        pub fn menu_add_to_dictionary(mut self, on: bool) -> Self {
            self.menu_add_to_dictionary = on;
            self
        }

        /// Whether the text context menu shows a disabled "No spelling
        /// suggestions" row when there are none (default `false`, Chrome's).
        pub fn menu_no_suggestions_row(mut self, on: bool) -> Self {
            self.menu_no_suggestions_row = on;
            self
        }

        /// How long the menu's press waits for suggestions (default
        /// [`DEFAULT_MENU_SUGGEST_WAIT_MS`]).
        pub fn menu_suggest_wait_ms(mut self, ms: u32) -> Self {
            self.menu_suggest_wait_ms = ms;
            self
        }
    }

    type Resolved = Option<Arc<dyn SpellChecker>>;

    struct Active {
        config: SpellcheckConfig,
        generation: u64,
        checker: Arc<OnceLock<Resolved>>,
        changes: Arc<Changes>,
    }

    type ChangeListener = Rc<dyn Fn()>;

    /// What the checker reported changing in the words it accepts beyond its
    /// dictionaries, since [`deliver_changes`] last looked. Its listener runs
    /// on whatever thread made the change; the flags carry it to the UI
    /// thread.
    #[derive(Default)]
    struct Changes {
        /// A word was added to the personal dictionary or ignored: some marks
        /// may now be correct.
        accepted: AtomicBool,
        /// A word was removed, or the ignores forgotten: a correct word may
        /// now be misspelled, so everything is checked again.
        rejected: AtomicBool,
        /// The checker listened to, and the listener's id.
        subscription: Mutex<Option<(Arc<dyn SpellChecker>, ListenerId)>>,
    }

    impl Changes {
        fn subscribe(self: &Arc<Self>, checker: &Arc<dyn SpellChecker>) {
            let weak = Arc::downgrade(self);
            let id = checker.add_change_listener(Box::new(move |change| {
                let Some(changes) = weak.upgrade() else {
                    return;
                };
                let flag = match change {
                    SpellChange::Personal(PersonalDictionaryChange::Added(_))
                    | SpellChange::Ignored(_) => &changes.accepted,
                    SpellChange::Personal(PersonalDictionaryChange::Removed(_))
                    | SpellChange::IgnoresCleared => &changes.rejected,
                };
                flag.store(true, Ordering::Release);
                wake();
            }));
            if let Some(id) = id {
                *self.subscription.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((Arc::clone(checker), id));
            }
        }

        fn unsubscribe(&self) {
            let taken = self
                .subscription
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some((checker, id)) = taken {
                checker.remove_change_listener(id);
            }
        }
    }

    type Wake = Arc<dyn Fn() + Send + Sync>;

    static WAKE: Mutex<Option<Wake>> = Mutex::new(None);

    fn wake() {
        let wake = WAKE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    /// Sets what is called, on any thread, when the checker reports a change
    /// to the words it accepts (a word added to the personal dictionary,
    /// ignored, removed). The host makes it run [`deliver_changes`] on its UI
    /// thread; rinch's runtime installs one at the first interaction with a
    /// checked field.
    pub fn set_change_wake(wake: impl Fn() + Send + Sync + 'static) {
        *WAKE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(wake));
    }

    thread_local! {
        static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
        static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        static LISTENERS: RefCell<Vec<(u64, ChangeListener)>> = const { RefCell::new(Vec::new()) };
        static NEXT_LISTENER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        static INLINE_JOBS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static WORD_LISTENERS: RefCell<Vec<(u64, ChangeListener)>> = const { RefCell::new(Vec::new()) };
    }

    /// Installs `config` for this (UI) thread and tells every
    /// [`on_config_change`] listener, which drop their marks and check again.
    pub fn set_config(config: impl Into<SpellcheckConfig>) {
        let config = config.into();
        let checker = Arc::new(OnceLock::new());
        let changes = Arc::new(Changes::default());
        match &config.dictionaries {
            Dictionaries::Checker(c) => {
                changes.subscribe(c);
                let _ = checker.set(Some(Arc::clone(c)));
            }
            Dictionaries::None => {
                let _ = checker.set(None);
            }
            Dictionaries::System { .. } => {}
        }
        let generation = GENERATION.with(|g| {
            g.set(g.get() + 1);
            g.get()
        });
        let old = ACTIVE.with(|a| {
            a.borrow_mut().replace(Active {
                config,
                generation,
                checker,
                changes,
            })
        });
        if let Some(old) = old {
            old.changes.unsubscribe();
            notify();
        }
    }

    /// Turns checking on or off, keeping the rest of the configuration.
    pub fn set_enabled(enabled: bool) {
        let mut config = config().unwrap_or_default();
        config.enabled = enabled;
        set_config(config);
    }

    /// Forgets this thread's configuration: no field is checked after it.
    pub fn clear_config() {
        let old = ACTIVE.with(|a| a.borrow_mut().take());
        if let Some(old) = old {
            old.changes.unsubscribe();
            GENERATION.with(|g| g.set(g.get() + 1));
            notify();
        }
    }

    /// Calls `listener` (on this thread, from [`deliver_changes`]) after the
    /// checker accepted more words: a word added to the personal dictionary
    /// or ignored. A UI drops the marks that are now correct. Answers an id
    /// for [`remove_words_listener`]. (A word removed calls the
    /// [`on_config_change`] listeners instead: everything is checked again.)
    pub fn on_words_accepted(listener: impl Fn() + 'static) -> u64 {
        let id = NEXT_LISTENER.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        WORD_LISTENERS.with(|l| l.borrow_mut().push((id, Rc::new(listener))));
        id
    }

    /// Removes an [`on_words_accepted`] listener.
    pub fn remove_words_listener(id: u64) {
        WORD_LISTENERS.with(|l| l.borrow_mut().retain(|(i, _)| *i != id));
    }

    /// Hands what the checker reported since the last call to this thread's
    /// listeners: a word accepted calls the [`on_words_accepted`] listeners,
    /// a word removed (or the ignores cleared) starts a new [`generation`]
    /// and calls the [`on_config_change`] listeners, which check everything
    /// again. Call on the UI thread; [`set_change_wake`] says when.
    pub fn deliver_changes() {
        let Some(changes) = ACTIVE.with(|a| a.borrow().as_ref().map(|a| Arc::clone(&a.changes)))
        else {
            return;
        };
        let rejected = changes.rejected.swap(false, Ordering::AcqRel);
        let accepted = changes.accepted.swap(false, Ordering::AcqRel);
        if rejected {
            let generation = GENERATION.with(|g| {
                g.set(g.get() + 1);
                g.get()
            });
            ACTIVE.with(|a| {
                if let Some(a) = a.borrow_mut().as_mut() {
                    a.generation = generation;
                }
            });
            notify();
        } else if accepted {
            let listeners: Vec<ChangeListener> =
                WORD_LISTENERS.with(|l| l.borrow().iter().map(|(_, f)| Rc::clone(f)).collect());
            for f in listeners {
                f();
            }
        }
    }

    /// Adds `word` to the checker's personal dictionary (which tells the
    /// app's [`PersonalDictionary`] listeners, so it can save it) and drops
    /// every mark of it at once, in every field and editor of this thread.
    /// What the text context menu's "Add to dictionary" runs. Call on the UI
    /// thread. `false` (nothing done) when no checker is loaded: spellcheck
    /// off, or nothing checked yet.
    pub fn add_to_dictionary(word: &str) -> bool {
        let Some((checker, changes)) = ACTIVE.with(|a| {
            let a = a.borrow();
            let a = a.as_ref()?;
            let checker = a.checker.get()?.clone()?;
            Some((checker, Arc::clone(&a.changes)))
        }) else {
            return false;
        };
        checker.add_to_dictionary(word);
        // Also for a checker that reports no changes (no listener).
        changes.accepted.store(true, Ordering::Release);
        deliver_changes();
        true
    }

    fn notify() {
        let listeners: Vec<ChangeListener> =
            LISTENERS.with(|l| l.borrow().iter().map(|(_, f)| Rc::clone(f)).collect());
        for f in listeners {
            f();
        }
    }

    /// This thread's configuration, `None` before [`set_config`].
    pub fn config() -> Option<SpellcheckConfig> {
        ACTIVE.with(|a| a.borrow().as_ref().map(|a| a.config.clone()))
    }

    /// Counts [`set_config`] and [`clear_config`] calls: marks made under an
    /// older generation are stale.
    pub fn generation() -> u64 {
        GENERATION.with(|g| g.get())
    }

    /// Calls `listener` after each [`set_config`] that replaced a
    /// configuration, and each [`clear_config`]. Answers an id for
    /// [`remove_config_listener`].
    pub fn on_config_change(listener: impl Fn() + 'static) -> u64 {
        let id = NEXT_LISTENER.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        LISTENERS.with(|l| l.borrow_mut().push((id, Rc::new(listener))));
        id
    }

    /// Removes a [`on_config_change`] listener.
    pub fn remove_config_listener(id: u64) {
        LISTENERS.with(|l| l.borrow_mut().retain(|(i, _)| *i != id));
    }

    /// Whether [`session`] would answer `Some`, without building one: the
    /// cheap test a hot path asks first.
    pub fn is_active() -> bool {
        ACTIVE.with(|a| {
            a.borrow()
                .as_ref()
                .is_some_and(|a| a.config.enabled && !matches!(a.checker.get(), Some(None)))
        })
    }

    /// What checks words for this thread's configuration, `None` when there
    /// is nothing to do: no configuration, checking off, or dictionaries that
    /// turned out to hold nothing (none installed for the locale).
    pub fn session() -> Option<Session> {
        ACTIVE.with(|a| {
            let a = a.borrow();
            let a = a.as_ref()?;
            if !a.config.enabled {
                return None;
            }
            if let Some(None) = a.checker.get() {
                return None;
            }
            Some(Session {
                generation: a.generation,
                checker: Arc::clone(&a.checker),
                changes: Arc::clone(&a.changes),
                dictionaries: a.config.dictionaries.clone(),
                rules: a.config.skip_rules.clone(),
                debounce_ms: a.config.debounce_ms,
                require_interaction: a.config.require_interaction,
                hide_input_marks_on_blur: a.config.hide_input_marks_on_blur,
                menu_suggestions: a.config.menu_suggestions,
                menu_add_to_dictionary: a.config.menu_add_to_dictionary,
                menu_no_suggestions_row: a.config.menu_no_suggestions_row,
                menu_suggest_wait_ms: a.config.menu_suggest_wait_ms,
            })
        })
    }

    /// A snapshot of the configuration that checks text on any thread.
    #[derive(Clone)]
    pub struct Session {
        generation: u64,
        checker: Arc<OnceLock<Resolved>>,
        changes: Arc<Changes>,
        dictionaries: Dictionaries,
        rules: SkipRules,
        debounce_ms: u32,
        require_interaction: bool,
        hide_input_marks_on_blur: bool,
        menu_suggestions: usize,
        menu_add_to_dictionary: bool,
        menu_no_suggestions_row: bool,
        menu_suggest_wait_ms: u32,
    }

    impl std::fmt::Debug for Session {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Session")
                .field("generation", &self.generation)
                .field("dictionaries", &self.dictionaries)
                .finish()
        }
    }

    impl Session {
        /// The [`generation`] it was taken at.
        pub fn generation(&self) -> u64 {
            self.generation
        }

        /// The pause after typing before edited words are checked.
        pub fn debounce_ms(&self) -> u32 {
            self.debounce_ms
        }

        /// See [`SpellcheckConfig::require_interaction`].
        pub fn require_interaction(&self) -> bool {
            self.require_interaction
        }

        /// See [`SpellcheckConfig::hide_input_marks_on_blur`].
        pub fn hide_input_marks_on_blur(&self) -> bool {
            self.hide_input_marks_on_blur
        }

        /// See [`SpellcheckConfig::menu_suggestions`].
        pub fn menu_suggestions(&self) -> usize {
            self.menu_suggestions
        }

        /// See [`SpellcheckConfig::menu_add_to_dictionary`].
        pub fn menu_add_to_dictionary(&self) -> bool {
            self.menu_add_to_dictionary
        }

        /// See [`SpellcheckConfig::menu_no_suggestions_row`].
        pub fn menu_no_suggestions_row(&self) -> bool {
            self.menu_no_suggestions_row
        }

        /// See [`SpellcheckConfig::menu_suggest_wait_ms`].
        pub fn menu_suggest_wait_ms(&self) -> u32 {
            self.menu_suggest_wait_ms
        }

        /// The skip rules.
        pub fn skip_rules(&self) -> &SkipRules {
            &self.rules
        }

        /// The checker, loading the system dictionaries on this thread the
        /// first time (slow: call it off the UI thread). `None` when there is
        /// none.
        pub fn checker(&self) -> Option<Arc<dyn SpellChecker>> {
            self.checker
                .get_or_init(|| {
                    let loaded = load(&self.dictionaries);
                    if let (Some(checker), Dictionaries::System { .. }) =
                        (&loaded, &self.dictionaries)
                    {
                        self.changes.subscribe(checker);
                    }
                    loaded
                })
                .clone()
        }

        /// The checker if it is loaded already, without loading it (cheap,
        /// for the UI thread). A word is only ever marked by a loaded one.
        pub fn loaded_checker(&self) -> Option<Arc<dyn SpellChecker>> {
            self.checker.get().cloned().flatten()
        }

        /// The misspelled words of `text` inside `ranges` (byte ranges on
        /// char boundaries; each is segmented on its own, so it should start
        /// and end at white space, as [`super::edit_region`] gives it), as
        /// byte ranges of `text`, sorted.
        pub fn check_ranges(&self, text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
            let Some(checker) = self.checker() else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for range in ranges {
                let Some(part) = text.get(range.clone()) else {
                    continue;
                };
                out.extend(
                    words(part, &self.rules)
                        .filter(|w| !checker.check(&part[w.clone()]))
                        .map(|w| w.start + range.start..w.end + range.start),
                );
            }
            out.sort_by_key(|r| r.start);
            out
        }
    }

    fn load(dictionaries: &Dictionaries) -> Resolved {
        match dictionaries {
            Dictionaries::Checker(c) => Some(Arc::clone(c)),
            Dictionaries::None => None,
            Dictionaries::System { locales, personal } => load_system(locales.as_deref(), personal),
        }
    }

    #[cfg(all(feature = "system-dictionaries", not(target_family = "wasm")))]
    fn load_system(
        locales: Option<&[String]>,
        personal: &Option<Arc<PersonalDictionary>>,
    ) -> Resolved {
        let locales = match locales {
            Some(l) => l.to_vec(),
            None => crate::system::user_locales(),
        };
        let mut checker = crate::HunspellChecker::new();
        let mut loaded: Vec<String> = Vec::new();
        for locale in &locales {
            if let Some(files) = crate::system::find(locale)
                && !loaded.contains(&files.name)
                && let Ok(dictionary) = files.load()
            {
                loaded.push(files.name.clone());
                checker.add_dictionary(dictionary);
            }
        }
        if loaded.is_empty() {
            return None;
        }
        if let Some(personal) = personal {
            checker = checker.with_personal_dictionary(Arc::clone(personal));
        }
        Some(Arc::new(checker))
    }

    #[cfg(not(all(feature = "system-dictionaries", not(target_family = "wasm"))))]
    fn load_system(_: Option<&[String]>, _: &Option<Arc<PersonalDictionary>>) -> Resolved {
        None
    }

    type Job = Box<dyn FnOnce() + Send>;

    static WORKER: Mutex<Option<mpsc::Sender<Job>>> = Mutex::new(None);

    /// Runs `job` on the shared checking thread (`rinch-spellcheck`, started
    /// by the first call), in the order sent. Under [`set_inline_jobs`] it
    /// runs here, at once.
    pub fn spawn(job: impl FnOnce() + Send + 'static) {
        // No threads on wasm32 (and rinch-web leaves checking to the browser).
        if cfg!(target_family = "wasm") || INLINE_JOBS.with(|i| i.get()) {
            job();
            return;
        }
        let mut worker = WORKER.lock().unwrap_or_else(|e| e.into_inner());
        let job: Job = Box::new(job);
        let job = match worker.as_ref() {
            Some(tx) => match tx.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(job)) => job,
            },
            None => job,
        };
        let (tx, rx) = mpsc::channel::<Job>();
        let started = std::thread::Builder::new()
            .name("rinch-spellcheck".into())
            .spawn(move || {
                for job in rx {
                    // A panicking checker must not take the thread (and every
                    // later check) with it.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            });
        if started.is_ok() && tx.send(job).is_ok() {
            *worker = Some(tx);
        }
    }

    /// Runs [`spawn`]ed jobs on this thread, synchronously (tests, and hosts
    /// that want checking to be deterministic).
    pub fn set_inline_jobs(inline: bool) {
        INLINE_JOBS.with(|i| i.set(inline));
    }

    /// Whether [`set_inline_jobs`] is on for this thread (always on wasm32,
    /// which has no threads).
    pub fn inline_jobs() -> bool {
        cfg!(target_family = "wasm") || INLINE_JOBS.with(|i| i.get())
    }

    type SuggestJob = (u64, Box<dyn FnOnce() + Send>);

    static SUGGEST_WORKER: Mutex<Option<mpsc::Sender<SuggestJob>>> = Mutex::new(None);
    static SUGGEST_LATEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Asks `checker` for up to `max` suggestions for `word` on the one
    /// suggestion thread (`rinch-spell-suggest`, started by the first call;
    /// apart from the checking thread, which may be busy with a whole
    /// document), and calls `done` there with them. Each call **supersedes**
    /// the ones before it: a request still queued when a newer one is made is
    /// dropped unrun (its `done` is never called), so repeated presses never
    /// pile up. A request already running finishes. Under
    /// [`set_inline_jobs`] (and on wasm32) it runs here, at once.
    pub fn suggest(
        checker: Arc<dyn SpellChecker>,
        word: String,
        max: usize,
        done: impl FnOnce(Vec<String>) + Send + 'static,
    ) {
        let job = move || done(checker.suggest(&word, max));
        if inline_jobs() {
            job();
            return;
        }
        let id = SUGGEST_LATEST.fetch_add(1, Ordering::AcqRel) + 1;
        let mut worker = SUGGEST_WORKER.lock().unwrap_or_else(|e| e.into_inner());
        let job: SuggestJob = (id, Box::new(job));
        let job = match worker.as_ref() {
            Some(tx) => match tx.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(job)) => job,
            },
            None => job,
        };
        let (tx, rx) = mpsc::channel::<SuggestJob>();
        let started = std::thread::Builder::new()
            .name("rinch-spell-suggest".into())
            .spawn(move || {
                for (id, job) in rx {
                    if id != SUGGEST_LATEST.load(Ordering::Acquire) {
                        continue; // superseded while it waited
                    }
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            });
        if started.is_ok() && tx.send(job).is_ok() {
            *worker = Some(tx);
        }
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // one mark is a list of one
mod tests {
    use super::*;

    #[test]
    fn attribute_states_follow_html() {
        assert_eq!(attribute_state(""), Some(true));
        assert_eq!(attribute_state("TRUE"), Some(true));
        assert_eq!(attribute_state("False"), Some(false));
        assert_eq!(attribute_state("no"), None);
        assert_eq!(attribute_state(" false"), None);
    }

    #[test]
    fn nearest_state_wins_and_default_is_on() {
        // 0 ← 1 ← 2: the chain of parents.
        let states = [Some(false), None, None];
        let parent = |n: &usize| n.checked_sub(1);
        assert!(!resolve_inherited(2, parent, |n| states[*n]));
        let states = [Some(false), Some(true), None];
        assert!(resolve_inherited(2, parent, |n| states[*n]));
        let states = [None, None, None];
        assert!(resolve_inherited(2, parent, |n| states[*n]));
    }

    #[test]
    fn prose_input_types() {
        for ty in [None, Some(""), Some("text"), Some("SEARCH"), Some("bogus")] {
            assert!(input_type_is_checked(ty), "{ty:?}");
        }
        for ty in ["password", "email", "url", "number", "tel", "Date"] {
            assert!(!input_type_is_checked(Some(ty)), "{ty}");
        }
    }

    #[test]
    fn text_change_finds_the_differing_part() {
        assert_eq!(text_change("abc", "abc"), None);
        assert_eq!(text_change("helo wrld", "hello wrld"), Some((3, 3, 4)));
        assert_eq!(text_change("aaa", "aa"), Some((2, 3, 2)));
        assert_eq!(text_change("café x", "cafe x"), Some((3, 5, 4)));
        // Suffix and prefix never overlap.
        assert_eq!(text_change("abab", "ab"), Some((2, 4, 2)));
    }

    #[test]
    fn marks_move_through_an_edit_and_the_edited_word_is_dropped() {
        // "helo wrld zzz" → "helo world zzz"
        let old = "helo wrld zzz";
        let new = "helo world zzz";
        let change = text_change(old, new).unwrap();
        let mut marks = vec![0..4, 5..9, 10..13];
        map_through_edit(&mut marks, change, new);
        assert_eq!(marks, vec![0..4, 11..14]);
        assert_eq!(&new[11..14], "zzz");
        assert_eq!(edit_region(new, change), 5..10);
        // A typed space checks no word; a letter after it, only its own.
        assert_eq!(edit_region("helo wrld ", (9, 9, 10)), 9..10);
        assert_eq!(edit_region("helo wrld z", (10, 10, 11)), 10..11);
        // A deletion joining two words checks the joined word.
        assert_eq!(edit_region("helowrld x", (4, 5, 4)), 0..8);

        // A space typed after a word, or before one, leaves it marked; a
        // letter typed onto it, or a deleted space joining two, does not.
        let mut marks = vec![0..4];
        map_through_edit(&mut marks, text_change("helo", "helo ").unwrap(), "helo ");
        assert_eq!(marks, vec![0..4]);
        let mut marks = vec![0..4];
        map_through_edit(&mut marks, text_change("helo", " helo").unwrap(), " helo");
        assert_eq!(marks, vec![1..5]);
        let mut marks = vec![0..4];
        map_through_edit(&mut marks, text_change("helo", "helos").unwrap(), "helos");
        assert!(marks.is_empty());
        let mut marks = vec![0..4, 5..9];
        map_through_edit(
            &mut marks,
            text_change("helo wrld", "helowrld").unwrap(),
            "helowrld",
        );
        assert!(marks.is_empty());
    }

    /// "Add to dictionary" (#2862, stage 3): the word goes into the shared
    /// personal dictionary, whose listener the app saves from, and this
    /// thread's mark holders hear of it at once. A word the app removes on
    /// another thread wakes the host, and delivering it starts a new
    /// generation and checks everything again.
    #[test]
    fn add_to_dictionary_reaches_the_app_and_the_marks() {
        use crate::{HunspellChecker, PersonalDictionary};
        use std::cell::Cell;
        use std::rc::Rc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        const AFF: &[u8] = include_bytes!("../tests/fixtures/en_TEST.aff");
        const DIC: &[u8] = include_bytes!("../tests/fixtures/en_TEST.dic");
        let personal = Arc::new(PersonalDictionary::new());
        let saved = Arc::new(Mutex::new(Vec::new()));
        let saved_in = Arc::clone(&saved);
        personal.add_listener(move |change| saved_in.lock().unwrap().push(format!("{change:?}")));
        let checker = HunspellChecker::from_bytes("en_TEST", AFF, DIC)
            .unwrap()
            .with_personal_dictionary(Arc::clone(&personal));
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_in = Arc::clone(&wakes);
        set_change_wake(move || {
            wakes_in.fetch_add(1, Ordering::SeqCst);
        });

        // Nothing configured: nothing to add to.
        clear_config();
        assert!(!add_to_dictionary("helo"));

        set_config(SpellcheckConfig::new().checker(checker));
        let accepted = Rc::new(Cell::new(0));
        let rechecks = Rc::new(Cell::new(0));
        let (a, r) = (accepted.clone(), rechecks.clone());
        let words = on_words_accepted(move || a.set(a.get() + 1));
        let config = on_config_change(move || r.set(r.get() + 1));

        let s = super::session().unwrap();
        assert!(!s.loaded_checker().unwrap().check("helo"));
        assert!(add_to_dictionary("helo"));
        assert!(s.loaded_checker().unwrap().check("helo"));
        assert_eq!(accepted.get(), 1, "the marks are told at once");
        assert_eq!(rechecks.get(), 0);
        assert_eq!(*saved.lock().unwrap(), [r#"Added("helo")"#]);

        // The app removes it on another thread: a wake, then a full re-check
        // under a new generation once delivered on this one.
        let before = generation();
        let p = Arc::clone(&personal);
        std::thread::spawn(move || p.remove("helo")).join().unwrap();
        assert!(wakes.load(Ordering::SeqCst) >= 1);
        assert_eq!(rechecks.get(), 0, "not before delivery");
        deliver_changes();
        assert_eq!(rechecks.get(), 1);
        assert!(generation() > before);
        assert_eq!(super::session().unwrap().generation(), generation());
        deliver_changes();
        assert_eq!((accepted.get(), rechecks.get()), (1, 1), "delivered once");

        // A replaced configuration stops listening to the old checker.
        set_config(SpellcheckConfig::from(false));
        let r2 = rechecks.get();
        personal.add("again");
        deliver_changes();
        assert_eq!((accepted.get(), rechecks.get()), (1, r2));

        remove_words_listener(words);
        remove_config_listener(config);
        clear_config();
    }

    /// One suggestion thread, and a request superseded while it waited is
    /// dropped unrun: three quick requests behind a slow one run the slow one
    /// (already started) and the newest, never the middle one.
    #[test]
    fn suggestions_supersede_older_requests() {
        use crate::{HunspellChecker, SpellChecker};
        use std::sync::Arc;
        use std::sync::mpsc;
        use std::time::Duration;

        struct Slow(HunspellChecker);
        impl SpellChecker for Slow {
            fn check(&self, w: &str) -> bool {
                self.0.check(w)
            }
            fn suggest(&self, w: &str, max: usize) -> Vec<String> {
                if w == "first" {
                    std::thread::sleep(Duration::from_millis(300));
                }
                vec![format!("{w}/{max}")]
            }
            fn add_to_dictionary(&self, w: &str) {
                self.0.add_to_dictionary(w)
            }
            fn ignore(&self, w: &str) {
                self.0.ignore(w)
            }
            fn languages(&self) -> Vec<String> {
                self.0.languages()
            }
        }
        const AFF: &[u8] = include_bytes!("../tests/fixtures/en_TEST.aff");
        const DIC: &[u8] = include_bytes!("../tests/fixtures/en_TEST.dic");
        let checker: Arc<dyn SpellChecker> = Arc::new(Slow(
            HunspellChecker::from_bytes("en_TEST", AFF, DIC).unwrap(),
        ));
        set_inline_jobs(false);
        let (tx, rx) = mpsc::channel();
        for word in ["first", "middle", "last"] {
            let tx = tx.clone();
            suggest(Arc::clone(&checker), word.into(), 2, move |found| {
                let _ = tx.send(found);
            });
            if word == "first" {
                std::thread::sleep(Duration::from_millis(50)); // let it start
            }
        }
        drop(tx);
        let got: Vec<Vec<String>> = rx.iter().collect();
        assert_eq!(
            got,
            [vec!["first/2".to_string()], vec!["last/2".to_string()]]
        );
    }

    #[test]
    fn ranges_merge() {
        let mut r = vec![0..2, 5..7];
        add_range(&mut r, 2..5);
        assert_eq!(r, vec![0..7]);
        add_range(&mut r, 9..10);
        assert_eq!(r, vec![0..7, 9..10]);
    }
}
