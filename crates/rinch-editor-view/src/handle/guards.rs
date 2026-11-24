//! The core borrow guards, which suspend effect flushing and tracking while
//! the core is borrowed.

use super::*;

/// A borrow of an editor's core, held together with a
/// [`suppress_effect_flush`](rinch_core::reactive::suppress_effect_flush)
/// guard.
///
/// The view patches the DOM through `NodeHandle`s while the core is borrowed,
/// and a `NodeHandle` operation made inside a batch (every event handler is
/// one) runs the effects the batch has queued so far. Those are user effects,
/// and a toolbar effect that reads `is_mark_active` would re-borrow the core
/// mid-patch: a `BorrowMutError` (PR #882 review, E1). Every borrow of the core
/// therefore goes through [`EditorHandle::core`] / [`EditorHandle::core_mut`],
/// which suppress the flush for as long as the borrow lives. Fields drop in
/// order: the borrow first, then the guard.
///
/// Only the **mutable** borrows also run the pending effects first (so the
/// caller's earlier writes are in the DOM the patch works on — program order).
/// A query (`is_mark_active`, `doc()`, `selection()`, …) reads model state no
/// effect makes stale and does no DOM work, so it must not run user effects:
/// a caller holding a `RefCell` of its own that one of those effects borrows
/// would panic inside a plain read (PR #882 review, E3). The one immutable
/// borrow that does DOM work, [`EditorHandle::set_dark_mode`], flushes itself.
///
/// **Dependency tracking is suspended for as long as the borrow lives**
/// (issue #943). What runs under a borrow of the core is the editor's code, a
/// plugin's — `Plugin::apply`, `decorations` (the view's decoration diff),
/// `handle_paste`, `init_state`, a plugin's command, an input rule — and the
/// app's collaboration `outbound` sink (#948, which also wraps itself in
/// `untracked_handler`), all synchronously inside whatever app effect called
/// the handle. A plugin that reads an app signal would otherwise make that
/// signal a dependency of every effect that moves the selection. Holding the
/// suspension here rather than at each call site covers every site the handle
/// reaches internally, including ones added later. The exception is the
/// caller's own [`EditorHandle::update`] `build` closure, which
/// [`EditorHandle::dispatch_inner`] runs with the suspension lifted — so plugin
/// code *that closure* calls (`state.apply(..)` inside it) is tracked, as the
/// caller's own code.
pub(super) struct CoreGuard<B> {
    borrow: B,
    _no_flush: rinch_core::reactive::EffectFlushSuppressed,
    _untracked: rinch_core::reactive::TrackingSuspended,
}

impl<B: std::ops::Deref<Target = EditorCore>> std::ops::Deref for CoreGuard<B> {
    type Target = EditorCore;
    fn deref(&self) -> &EditorCore {
        &self.borrow
    }
}

/// A **mutable** borrow of an editor's core: [`CoreGuard`]'s flush rules, plus
/// the calls a mutable borrow can owe on its way out — the
/// [`on_selection_change`](EditorHandle::on_selection_change) call, and the
/// [`on_caret_moved`](EditorHandle::on_caret_moved) one (owed by
/// [`EditorHandle::update_caret`]).
///
/// Every change to the stored selection happens under one of these (local
/// changes in [`EditorCore::commit`], remote ones in the collaboration
/// integrate), and [`EditorCore::note_selection`] records it there. The drop
/// releases the borrow and the flush suppression **first**, then makes the
/// call, so the callback may re-enter the handle freely — read the document,
/// move the selection again (which owes, and makes, a call of its own), or run
/// a command.
///
/// A drop that unwinds a panic raised while the borrow was held makes no call.
/// That is judged against `panicking` at the borrow, not by
/// `std::thread::panicking()` alone: on `wasm32` a panic aborts without
/// unwinding, the panic count is never taken back, and every later borrow
/// would read as unwinding — one caught panic would silence every callback
/// for the page's lifetime.
pub(super) struct CoreMutGuard<'a> {
    /// The editor, to run its node-view work once the borrow is released.
    editor: &'a Rc<RefCell<EditorCore>>,
    borrow: Option<std::cell::RefMut<'a, EditorCore>>,
    no_flush: Option<rinch_core::reactive::EffectFlushSuppressed>,
    /// See [`CoreGuard`]: tracking is suspended while the borrow is held.
    /// `None` only while [`EditorHandle::dispatch`] runs a caller's `build`.
    pub(super) untracked: Option<rinch_core::reactive::TrackingSuspended>,
    /// `std::thread::panicking()` when the borrow was taken.
    panicking: bool,
}

impl std::ops::Deref for CoreMutGuard<'_> {
    type Target = EditorCore;
    fn deref(&self) -> &EditorCore {
        self.borrow.as_deref().expect("held until drop")
    }
}

impl std::ops::DerefMut for CoreMutGuard<'_> {
    fn deref_mut(&mut self) -> &mut EditorCore {
        self.borrow.as_deref_mut().expect("held until drop")
    }
}

impl Drop for CoreMutGuard<'_> {
    fn drop(&mut self) {
        let Some(owed) = self.borrow.as_deref_mut().map(EditorCore::take_owed) else {
            return;
        };
        // Release in field order — the borrow, then the suppressions — before
        // any user code runs. (The hooks are untracked on their own, #931.)
        self.borrow = None;
        self.no_flush = None;
        self.untracked = None;
        if std::thread::panicking() && !self.panicking {
            return;
        }
        if let Some((cb, selection)) = owed.selection {
            cb.invoke(|cb| cb(&selection));
        }
        if let Some(cb) = owed.caret_moved {
            cb.invoke(|cb| cb());
        }
        if !owed.node_views.is_empty() {
            node_views::run(
                &EditorHandle {
                    inner: self.editor.clone(),
                },
                owed.node_views,
            );
        }
        if owed.spell {
            EditorHandle {
                inner: self.editor.clone(),
            }
            .spell_owed();
        }
    }
}

impl EditorHandle {
    /// Borrow the core immutably; see [`CoreGuard`].
    pub(super) fn core(&self) -> CoreGuard<std::cell::Ref<'_, EditorCore>> {
        let no_flush = rinch_core::reactive::suppress_effect_flush();
        CoreGuard {
            borrow: self.inner.borrow(),
            _no_flush: no_flush,
            _untracked: rinch_core::reactive::suspend_tracking(),
        }
    }

    /// Borrow the core mutably; see [`CoreGuard`] and [`CoreMutGuard`].
    pub(super) fn core_mut(&self) -> CoreMutGuard<'_> {
        rinch_core::reactive::flush_pending_effects();
        let no_flush = rinch_core::reactive::suppress_effect_flush();
        CoreMutGuard {
            editor: &self.inner,
            borrow: Some(self.inner.borrow_mut()),
            no_flush: Some(no_flush),
            untracked: Some(rinch_core::reactive::suspend_tracking()),
            panicking: std::thread::panicking(),
        }
    }

    /// [`Self::core_mut`], answering `Err` instead of panicking when the core
    /// is already borrowed. Only the collaboration methods need the soft form.
    #[cfg_attr(not(feature = "collaboration"), allow(dead_code))]
    pub(super) fn try_core_mut(&self) -> Result<CoreMutGuard<'_>, std::cell::BorrowMutError> {
        rinch_core::reactive::flush_pending_effects();
        let no_flush = rinch_core::reactive::suppress_effect_flush();
        Ok(CoreMutGuard {
            editor: &self.inner,
            borrow: Some(self.inner.try_borrow_mut()?),
            no_flush: Some(no_flush),
            untracked: Some(rinch_core::reactive::suspend_tracking()),
            panicking: std::thread::panicking(),
        })
    }
}
