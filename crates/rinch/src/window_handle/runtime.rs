//! The window registry's runtime side: what the desktop runtime reads when
//! it is made, and what it reports as windows are made, mounted and closed.

use super::*;

thread_local! {
    /// What `App::exit_when` set, read when the runtime is made.
    static EXIT_WHEN: std::cell::Cell<ExitWhen> = const { std::cell::Cell::new(ExitWhen::MainWindowCloses) };
}

/// `App::exit_when`'s choice, for the runtime about to be made.
pub(crate) fn set_exit_when(when: ExitWhen) {
    EXIT_WHEN.with(|w| w.set(when));
}

/// When the runtime being made ends its event loop.
pub(crate) fn exit_when() -> ExitWhen {
    EXIT_WHEN.with(|w| w.get())
}

thread_local! {
    /// `App::windowless` was asked for, read when the runtime is made.
    static WINDOWLESS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the runtime about to be made has no main window (`App::windowless`).
pub(crate) fn set_windowless(on: bool) {
    WINDOWLESS.with(|w| w.set(on));
}

/// Whether the runtime being made has no main window.
pub(crate) fn windowless() -> bool {
    WINDOWLESS.with(|w| w.get())
}

/// The main window's handle, made with the runtime.
pub(crate) fn register_main() -> u64 {
    let (id, _) = with(|r| {
        let added = r.add();
        r.main = Some(added.0);
        added
    });
    publish_list();
    id
}

/// The window `open_window` opened as `id`, for the runtime to make, if it is
/// still waiting (it was not closed first).
pub(crate) fn take_pending(id: u64) -> Option<Pending> {
    with(|r| {
        let at = r.pending.iter().position(|p| p.id == id)?;
        Some(r.pending.remove(at))
    })
}

/// Whether the window `open_window` (or `open_popup`) opened as `id` is
/// still waiting to be made.
pub(crate) fn is_pending(id: u64) -> bool {
    with(|r| r.pending.iter().any(|p| p.id == id))
}

/// Run `mount`, the mount of window `id`'s document: [`WindowHandle::current`]
/// in its content is that window from the content's first line.
pub(crate) fn mounting<R>(id: u64, mount: impl FnOnce() -> R) -> R {
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MOUNTING.with(|m| m.set(self.0));
        }
    }
    let _restore = Restore(MOUNTING.with(|m| m.replace(Some(id))));
    mount()
}

/// The window `id` is mounted, as document `doc`: its code finds it with
/// [`WindowHandle::current`].
pub(crate) fn note_mounted(id: u64, doc: u64) {
    with(|r| {
        if let Some(e) = r.entries.iter_mut().find(|e| e.id == id) {
            e.doc = Some(doc);
        }
    });
}

/// The window `id` closed: `is_open` turns false and it leaves [`windows`].
/// One `open_window` closed before it was made is dropped from the pending
/// list too.
pub(crate) fn note_closed(id: u64) {
    let closed = with(|r| {
        r.pending.retain(|p| p.id != id);
        r.pressed.retain(|&(p, _)| p != id);
        let at = r.entries.iter().position(|e| e.id == id)?;
        Some(r.entries.remove(at))
    });
    if let Some(mut entry) = closed {
        // A popup the system or the user dismissed is told why, once,
        // whoever closes it after (its owner included).
        let dismissed = entry.popup.as_mut().and_then(|p| p.take_dismissed());
        // Its owner keeps nothing of it (a no-op when the owner's going is
        // what closed it).
        if let Some(cleanup) = entry.owner_cleanup {
            cleanup.remove();
        }
        // Its readers hear `false`; one woken by it finds no entry and reads
        // nothing more, so the signal is freed with the window's scope.
        entry.open.set(false);
        publish_list();
        if let Some((on_dismissed, why)) = dismissed {
            rinch_core::reactive::untracked_handler(|| on_dismissed(why));
        }
        entry.scope.dispose();
        if !entry.owner_gone
            && let Some(on_closed) = entry.on_closed
        {
            rinch_core::reactive::untracked_handler(on_closed);
        }
    }
}

/// How the main window and another are named in a warning: the main window
/// as such, another by the title it was opened with.
pub(crate) fn describe(doc: Option<u64>) -> String {
    let Some(doc) = doc else {
        return "the main window".to_string();
    };
    with(|r| {
        r.entries
            .iter()
            .find(|e| e.doc == Some(doc))
            .map(|e| format!("the window \"{}\"", e.title))
            .unwrap_or_else(|| "a window that has closed".to_string())
    })
}

/// Forget every window (a runtime is dropped): tests make several runtimes on
/// one thread.
pub(crate) fn reset() {
    let entries = with(|r| {
        r.pending.clear();
        r.pressed.clear();
        r.main = None;
        std::mem::take(&mut r.entries)
    });
    publish_list();
    for entry in entries {
        entry.scope.dispose();
    }
}
