//! [`Command`] — the unit of editing intent that toolbar buttons, keymap entries,
//! and input rules resolve to.
//!
//! A command **queries the state and, if it applies, builds a transaction and
//! dispatches it**, returning `true`. Called with `dispatch = None` it only reports
//! applicability — which is exactly what a toolbar needs to enable/disable a
//! button without performing the edit (design §5; amendment A20 fixes the command
//! type to the closure form so a command can close over data like a heading level
//! or a link href).
//!
//! ```ignore
//! let toggle_bold: Command = Rc::new(|state, dispatch| {
//!     // ... query state.selection / state.stored_marks ...
//!     if let Some(dispatch) = dispatch { dispatch(tr); }
//!     true
//! });
//! ```

use crate::state::{EditorState, Transaction};
use std::rc::Rc;

/// The sink a command hands its built [`Transaction`] to. The runtime's dispatch
/// is `|tr| new_state = state.apply(tr)`.
pub type Dispatch<'a> = &'a mut dyn FnMut(Transaction);

/// A command: `(state, Option<dispatch>) -> applied?`.
///
/// - With `Some(dispatch)`: if the command applies, it builds a transaction, calls
///   `dispatch(tr)`, and returns `true`; otherwise returns `false` without
///   dispatching.
/// - With `None`: returns whether the command *would* apply (toolbar enablement),
///   performing no edit.
///
/// `Transaction` owns its schema (no lifetime), so this is an ordinary
/// `Fn` trait object — no lifetime parameter leaks into the alias.
pub type Command = Rc<dyn Fn(&EditorState, Option<Dispatch>) -> bool>;

/// Run two commands in sequence, stopping at the first that applies — the `chain`
/// combinator from prosemirror-commands. Used to bind e.g. Backspace to
/// "join-backward else delete-selection".
pub fn chain(commands: Vec<Command>) -> Command {
    Rc::new(move |state, dispatch| match dispatch {
        Some(d) => {
            for cmd in &commands {
                // Reborrow `*d` for just this attempt (`Option<&mut _>` isn't Copy).
                if cmd(state, Some(&mut *d)) {
                    return true;
                }
            }
            false
        }
        // Applicability query: any sub-command that would apply makes the chain apply.
        None => commands.iter().any(|cmd| cmd(state, None)),
    })
}
