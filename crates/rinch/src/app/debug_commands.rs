use super::*;

fn parse_button(s: &Option<String>) -> MouseButton {
    match s.as_deref() {
        Some("right") => MouseButton::Right,
        Some("middle") => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

/// The error a `modifiers` array with a name `fold_modifier_names` does not
/// know answers with — the same sentence for `key_press` and the pointer
/// commands.
fn unknown_modifier(name: &str) -> DebugResult {
    DebugResult::Error {
        message: format!(
            "Unknown modifier name: {name:?} (expected ctrl/control, shift, alt/option, meta/cmd/super)"
        ),
    }
}

/// A pointer command's `modifiers` array as the exact modifier state to hold:
/// `Ok(None)` when the field was absent, the named modifiers (and no others)
/// when present, an error naming the first unknown name.
fn pointer_modifiers(names: &Option<Vec<String>>) -> Result<Option<Modifiers>, DebugResult> {
    let Some(names) = names else {
        return Ok(None);
    };
    let mut m = Modifiers::default();
    rinch_debug::fold_modifier_names(names, &mut m.shift, &mut m.ctrl, &mut m.alt, &mut m.meta)
        .map_err(|name| unknown_modifier(&name))?;
    Ok(Some(m))
}

/// Map a printable character to a physical `KeyCode` for synthesizing keystrokes
/// (the `text` field carries the actual character). Characters without a
/// dedicated keycode map to `KeyCode::Other` — exactly how real hardware
/// delivers punctuation (`shell/rinch_runtime.rs` translates unlisted winit
/// keys to `Other`) — so the keyboard hook's key string falls through to the
/// `text` field instead of masquerading as a named key. Space keeps its
/// explicit arm: an injected `' '` must still read as the spacebar —
/// `code = "Space"`, `key = " "` — matching a physical spacebar press (issue
/// #151; the `key` spelling is #1161's).
fn char_to_keycode(c: char) -> KeyCode {
    match c.to_ascii_lowercase() {
        ' ' => KeyCode::Space,
        'a' => KeyCode::KeyA,
        'b' => KeyCode::KeyB,
        'c' => KeyCode::KeyC,
        'd' => KeyCode::KeyD,
        'e' => KeyCode::KeyE,
        'f' => KeyCode::KeyF,
        'g' => KeyCode::KeyG,
        'h' => KeyCode::KeyH,
        'i' => KeyCode::KeyI,
        'j' => KeyCode::KeyJ,
        'k' => KeyCode::KeyK,
        'l' => KeyCode::KeyL,
        'm' => KeyCode::KeyM,
        'n' => KeyCode::KeyN,
        'o' => KeyCode::KeyO,
        'p' => KeyCode::KeyP,
        'q' => KeyCode::KeyQ,
        'r' => KeyCode::KeyR,
        's' => KeyCode::KeyS,
        't' => KeyCode::KeyT,
        'u' => KeyCode::KeyU,
        'v' => KeyCode::KeyV,
        'w' => KeyCode::KeyW,
        'x' => KeyCode::KeyX,
        'y' => KeyCode::KeyY,
        'z' => KeyCode::KeyZ,
        '0' => KeyCode::Digit0,
        '1' => KeyCode::Digit1,
        '2' => KeyCode::Digit2,
        '3' => KeyCode::Digit3,
        '4' => KeyCode::Digit4,
        '5' => KeyCode::Digit5,
        '6' => KeyCode::Digit6,
        '7' => KeyCode::Digit7,
        '8' => KeyCode::Digit8,
        '9' => KeyCode::Digit9,
        _ => KeyCode::Other,
    }
}

/// Map a debug `key_press` key-name string to a `KeyCode`. Single-character
/// names go through [`char_to_keycode`] (punctuation → `KeyCode::Other`, with
/// the character delivered via the event's `text` field); unknown multi-char
/// names return `None` so the caller can fail loud instead of synthesizing a
/// silent no-text `Other` press.
fn keyname_to_keycode(key: &str) -> Option<KeyCode> {
    Some(match key {
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Tab" => KeyCode::Tab,
        "Escape" => KeyCode::Escape,
        "Space" => KeyCode::Space,
        "F12" => KeyCode::F12,
        k if k.chars().count() == 1 => char_to_keycode(k.chars().next().unwrap()),
        _ => return None,
    })
}

/// The W3C `KeyboardEvent.code` name for a debug `key_press` key name — the
/// same string [`crate::menu::match_shortcut_code`] (and the winit shell's own
/// `KeyboardInput` translation) key menu-chord matching off of.
///
/// Issue #533: `key_press` used to build a `PlatformEvent::KeyDown` directly
/// and hand it to `handle_event`, entering the pipeline *downstream* of the
/// menu-shortcut check the real winit `WindowEvent::KeyboardInput` arm makes
/// first — so an injected `Ctrl+D` never reached a registered "Ctrl+D" menu
/// item, whether or not the app's real shortcut worked. Mirrors
/// [`keyname_to_keycode`]'s table exactly (named keys verbatim, a single
/// character through [`char_to_w3c_code`]) so the two stay in lockstep; a
/// punctuation character has no entry here either, since `char_to_keycode`
/// never gave it a dedicated `KeyCode` and a debug `key_press` could not
/// reach a punctuation-bound chord before this change and still cannot.
fn keyname_to_w3c_code(key: &str) -> Option<&'static str> {
    Some(match key {
        "ArrowLeft" => "ArrowLeft",
        "ArrowRight" => "ArrowRight",
        "ArrowUp" => "ArrowUp",
        "ArrowDown" => "ArrowDown",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "PageUp",
        "PageDown" => "PageDown",
        "Enter" => "Enter",
        "Backspace" => "Backspace",
        "Delete" => "Delete",
        "Tab" => "Tab",
        "Escape" => "Escape",
        "Space" => "Space",
        "F12" => "F12",
        k if k.chars().count() == 1 => char_to_w3c_code(k.chars().next().unwrap())?,
        _ => return None,
    })
}

/// The W3C `code` name for a single character, mirroring [`char_to_keycode`]'s
/// table one-for-one (and returning `None` exactly where that one falls back
/// to `KeyCode::Other`).
fn char_to_w3c_code(c: char) -> Option<&'static str> {
    Some(match c.to_ascii_lowercase() {
        ' ' => "Space",
        'a' => "KeyA",
        'b' => "KeyB",
        'c' => "KeyC",
        'd' => "KeyD",
        'e' => "KeyE",
        'f' => "KeyF",
        'g' => "KeyG",
        'h' => "KeyH",
        'i' => "KeyI",
        'j' => "KeyJ",
        'k' => "KeyK",
        'l' => "KeyL",
        'm' => "KeyM",
        'n' => "KeyN",
        'o' => "KeyO",
        'p' => "KeyP",
        'q' => "KeyQ",
        'r' => "KeyR",
        's' => "KeyS",
        't' => "KeyT",
        'u' => "KeyU",
        'v' => "KeyV",
        'w' => "KeyW",
        'x' => "KeyX",
        'y' => "KeyY",
        'z' => "KeyZ",
        '0' => "Digit0",
        '1' => "Digit1",
        '2' => "Digit2",
        '3' => "Digit3",
        '4' => "Digit4",
        '5' => "Digit5",
        '6' => "Digit6",
        '7' => "Digit7",
        '8' => "Digit8",
        '9' => "Digit9",
        _ => return None,
    })
}

/// The single lowercase ASCII letter a debug `key_press`'s `key` name spells,
/// or `None` for anything else — the debug equivalent of winit's
/// `key_without_modifiers` / a browser's lowercased `event.key()` that
/// [`crate::menu::chord_key_matches`] reads for issue #1170's layout-character
/// matching. MCP has no layout to consult, so (as `logical_key` below already
/// assumes) the key name IS the character: `key_press(key: "D", shift: true)`
/// types the same letter a QWERTY `Shift+D` does.
fn keyname_typed_letter(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    c.is_ascii_alphabetic().then(|| c.to_ascii_lowercase())
}

#[cfg(feature = "debug")]
impl RinchApp {
    // ── Debug commands ───────────────────────────────────────────────────

    pub(crate) fn handle_debug_commands(
        &mut self,
        actions: &mut Vec<AppAction>,
        scale_factor: f64,
        window_size: (u32, u32),
    ) {
        let Some(rx) = self.debug_cmd_rx.take() else {
            return;
        };

        while let Ok(cmd) = rx.0.try_recv() {
            let response = self.execute_debug_command(cmd.kind, actions, scale_factor, window_size);
            let _ = cmd.response_tx.send(response);
        }

        self.debug_cmd_rx = Some(rx);
    }

    /// A `ModifiersChanged` a debug command synthesizes. A *real* one clears
    /// `debug_modifiers_to_restore` (it supersedes what a modified
    /// `mouse_down` remembered); a synthesized one is part of the debug
    /// sequence itself, so the remembered state is kept across it.
    fn debug_modifiers_changed(
        &mut self,
        m: Modifiers,
        window_size: (u32, u32),
        scale_factor: f64,
    ) -> Vec<AppAction> {
        let remembered = self.debug_modifiers_to_restore;
        let actions = self.handle_event(
            PlatformEvent::ModifiersChanged(m),
            window_size,
            scale_factor,
        );
        self.debug_modifiers_to_restore = remembered;
        actions
    }

    pub(crate) fn execute_debug_command(
        &mut self,
        kind: DebugCommandKind,
        actions: &mut Vec<AppAction>,
        scale_factor: f64,
        window_size: (u32, u32),
    ) -> DebugResult {
        match kind {
            DebugCommandKind::Screenshot => {
                // Screenshot is handled by the shell -- we signal that we need
                // a screenshot capture. The shell will paint + capture.
                // For now, return an error indicating the shell must handle this.
                DebugResult::Error {
                    message: "__SCREENSHOT_DELEGATE__".into(),
                }
            }
            DebugCommandKind::DomTree {
                max_depth,
                root_id,
                verbose,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: rinch_dom::testing::serialize_tree_full(
                        &d.tree,
                        max_depth.or(Some(3)),
                        root_id,
                        verbose,
                    ),
                }
            }
            DebugCommandKind::QuerySelector { selector } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                let ids = rinch_dom::testing::query_selector(&d.tree, &selector);
                let nodes: Vec<_> = ids
                    .iter()
                    .filter_map(|&id| rinch_dom::testing::get_node_summary(&d.tree, id))
                    .collect();
                DebugResult::Json { data: json!(nodes) }
            }
            DebugCommandKind::GetNode { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                match rinch_dom::testing::get_node_detail(&d.tree, id) {
                    Some(detail) => DebugResult::Json { data: detail },
                    None => DebugResult::Error {
                        message: format!("Node {} not found", id),
                    },
                }
            }
            DebugCommandKind::GetTextContent { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: json!(rinch_dom::testing::get_text_content(&d.tree, id)),
                }
            }
            DebugCommandKind::Click {
                x,
                y,
                ref button,
                ref modifiers,
            } => {
                // Route through the REAL input path (press + release) so MCP
                // exercises exactly what a real mouse does — `handle_event` owns all
                // the click logic (contextmenu, focus, editor, drag). Injecting via a
                // parallel path here is what hid the dual-arm MouseDown bug.
                //
                // Requested modifiers arrive the way a keyboard delivers them:
                // a `ModifiersChanged` before the press, and another after the
                // release putting back what was held before.
                let requested = match pointer_modifiers(modifiers) {
                    Ok(m) => m,
                    Err(e) => return e,
                };
                let mouse_button = parse_button(button);
                self.cursor_pos = Some((x, y));
                let before = self.modifiers;
                if let Some(m) = requested {
                    actions.extend(self.debug_modifiers_changed(m, window_size, scale_factor));
                }
                actions.extend(self.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseUp {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                if requested.is_some() {
                    actions.extend(self.debug_modifiers_changed(before, window_size, scale_factor));
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseDown {
                x,
                y,
                ref button,
                ref modifiers,
            } => {
                // Route through the real input path so MCP matches a real press.
                //
                // Requested modifiers stay held after the press (a Shift- or
                // Alt-drag goes on through `mouse_move`s); the next `mouse_up`
                // restores the state remembered here. A second modified press
                // before that release keeps the first remembered state, so the
                // release still returns to what was held before either.
                let requested = match pointer_modifiers(modifiers) {
                    Ok(m) => m,
                    Err(e) => return e,
                };
                let mouse_button = parse_button(button);
                self.cursor_pos = Some((x, y));
                if let Some(m) = requested {
                    // Read before the change, remembered after it: the change
                    // itself clears the remembered state (a real modifier
                    // change supersedes it — see `handle_event`), and the
                    // helper keeps that clear from reaching a save made by an
                    // earlier press.
                    let prev = self.modifiers;
                    actions.extend(self.debug_modifiers_changed(m, window_size, scale_factor));
                    self.debug_modifiers_to_restore.get_or_insert(prev);
                }
                actions.extend(self.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseUp {
                x,
                y,
                ref button,
                ref modifiers,
            } => {
                // Route through the real input path so MCP matches a real release.
                //
                // Its own `modifiers` are held for the release; afterwards the
                // state from before this release (or, if a modified
                // `mouse_down` came first, from before that press) comes back.
                let requested = match pointer_modifiers(modifiers) {
                    Ok(m) => m,
                    Err(e) => return e,
                };
                let mouse_button = parse_button(button);
                self.cursor_pos = Some((x, y));
                let restore = self
                    .debug_modifiers_to_restore
                    .take()
                    .or(requested.map(|_| self.modifiers));
                if let Some(m) = requested {
                    actions.extend(self.debug_modifiers_changed(m, window_size, scale_factor));
                }
                actions.extend(self.handle_event(
                    PlatformEvent::MouseUp {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                if let Some(m) = restore {
                    actions.extend(self.debug_modifiers_changed(m, window_size, scale_factor));
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseMove { x, y } => {
                // Route through the real input path so MCP matches a real move
                // (drag-select, hover, surface dispatch all live in `handle_event`).
                self.cursor_pos = Some((x, y));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseMove { x, y },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                // Route through the real input path so MCP matches a real
                // wheel, exactly as `Click`/`MouseDown`/`MouseUp`/`MouseMove`/
                // `TypeText` above already do.
                //
                // What used to be here was a hand-written parallel of the
                // `MouseWheel` arm, and it had drifted in three ways (#401):
                //
                //  * it never called `tree.push_dirty`, which is what fills
                //    `paint_dirty_nodes` — `compute_dirty_region`'s only input —
                //    so the software renderer narrowed the repaint to whatever
                //    else happened to be dirty and left most of the scrolled
                //    container showing its pre-scroll pixels, *persistently*;
                //  * it ignored `delta_x` for document scrolling, so a
                //    horizontal scroller could not be driven at all;
                //  * its sign was the opposite of `PlatformEvent::MouseWheel`'s
                //    for the document, but *not* for a render surface, so MCP
                //    handed a surface the reverse of what a real wheel does.
                //
                // The sign converts once, here at the boundary: the MCP tool
                // documents positive as "scroll down"/"scroll right", while a
                // wheel delta is negative in that direction (winit's
                // `LineDelta`, and `MouseWheel`'s arm subtracts it from the
                // scroll offset). Negating makes both the document and any
                // render surface see what the physical gesture would produce.
                self.cursor_pos = Some((x, y));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseWheel {
                        x,
                        y,
                        delta_x: -delta_x,
                        delta_y: -delta_y,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::TypeText { text } => {
                // Synthesize a real KeyDown per character and route through
                // `handle_event`, so MCP `type_text` drives the exact same path as
                // physical keystrokes.
                for ch in text.chars() {
                    let (key, txt) = match ch {
                        '\n' => (KeyCode::Enter, None),
                        '\t' => (KeyCode::Tab, None),
                        '\x08' => (KeyCode::Backspace, None),
                        c => (char_to_keycode(c), Some(c.to_string())),
                    };
                    // The typed character is its own key value (MCP has no
                    // layout to consult) — case-accurate, like the field asks.
                    // The '\n'/'\t'/'\x08' branches fall out as `None` via the
                    // control-character test, and their named `KeyCode`s spell
                    // them instead.
                    let logical_key = (!ch.is_control()).then(|| ch.to_string());
                    actions.extend(self.handle_event(
                        PlatformEvent::KeyDown {
                            key,
                            logical_key,
                            text: txt,
                            modifiers: rinch_platform::Modifiers::default(),
                            // One synthesized press per character, and the
                            // channel sends no `KeyUp` at all — so `Unknown`
                            // would leave the activation latch armed after the
                            // first Enter and swallow every later one (#463).
                            repeat: rinch_platform::KeyRepeat::Fresh,
                        },
                        window_size,
                        scale_factor,
                    ));
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::PerfStats { reset } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let data = {
                    let d = doc.borrow();
                    let perf = &d.tree.perf;
                    json!({
                        "frames": perf.frames(),
                        "last_frame": perf.last_frame().to_json(),
                        "current_frame": perf.frame().to_json(),
                        "total": perf.total().to_json(),
                    })
                };
                if reset {
                    self.reset_perf();
                }
                DebugResult::Json { data }
            }
            DebugCommandKind::WaitFrame => {
                // Layout is resolved at the logical viewport, not the physical
                // surface size (see `RinchApp::layout_viewport`).
                let (w, h) = Self::layout_viewport(window_size, scale_factor);
                self.resolve_and_repaint(w, h);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetComputedStyles { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                match d.tree.get(id) {
                    Some(node) => DebugResult::Json {
                        data: json!(&node.computed_style),
                    },
                    None => DebugResult::Error {
                        message: format!("Node {} not found", id),
                    },
                }
            }
            DebugCommandKind::CloseApp => {
                actions.push(AppAction::Exit);
                DebugResult::Json {
                    data: json!({"status": "closing"}),
                }
            }
            DebugCommandKind::KeyPress {
                key,
                mut shift,
                mut ctrl,
                mut alt,
                modifiers,
            } => {
                // Synthesize a real KeyDown and route through `handle_event` (which
                // owns Escape/F12/inspect/editor/CE handling) so MCP `key_press`
                // matches a physical keystroke.
                //
                // Fold the optional `modifiers` name array into the flat booleans
                // (issue #152) — the only path that can request `meta`. An unknown
                // name fails loud instead of silently altering the simulated input.
                let mut meta = false;
                if let Err(name) = rinch_debug::fold_modifier_names(
                    &modifiers, &mut shift, &mut ctrl, &mut alt, &mut meta,
                ) {
                    return unknown_modifier(&name);
                }
                // Unknown multi-char key names fail loud too — a silent no-text
                // `Other` press would be indistinguishable from a dead key (#151).
                let Some(key_code) = keyname_to_keycode(&key) else {
                    return DebugResult::Error {
                        message: format!("Unknown key name: {key:?}"),
                    };
                };
                // Check menu shortcuts first — exactly as the real winit
                // `WindowEvent::KeyboardInput` arm does in `rinch_runtime.rs`
                // — so a debug-injected chord that a menu owns is consumed
                // here rather than falling through to `PlatformEvent::KeyDown`
                // (issue #533). `text_target_holds_keyboard` lets a
                // modifier-less chord yield to a focused text field (#1169),
                // same as a real keystroke; it is desktop-only machinery, and
                // every real `debug` build pairs with `desktop` (`debug` alone
                // compiles this module only under `desktop`/`android`/`embed`
                // via `lib.rs`'s `pub mod app` gate), so the fallback below is
                // dead code in practice and exists only so a hypothetical
                // `android`/`embed`-without-`desktop` debug build still
                // compiles — with shortcuts simply never yielding to text
                // focus there.
                #[cfg(feature = "desktop")]
                let text_focus = self.text_target_holds_keyboard();
                #[cfg(not(feature = "desktop"))]
                let text_focus = false;
                if let Some(code) = keyname_to_w3c_code(&key) {
                    let yields = text_focus
                        && crate::menu::chord_yields_to_text_input(ctrl, meta, alt, code);
                    if !yields {
                        let typed_letter = keyname_typed_letter(&key);
                        if crate::menu::match_shortcut_code(
                            ctrl,
                            meta,
                            alt,
                            shift,
                            code,
                            typed_letter,
                        ) {
                            actions.push(AppAction::RequestRedraw);
                            return DebugResult::Json { data: json!(null) };
                        }
                    }
                }
                let text = match key.as_str() {
                    "Enter" => Some("\n".to_string()),
                    k if k.chars().count() == 1 => Some(k.to_string()),
                    _ => None,
                };
                // A single-character key name is its own key value, case and
                // all (MCP has no layout to consult). Named keys stay `None`:
                // their `KeyCode` spells them, and fabricating the name here
                // would just shadow that table.
                let logical_key = {
                    let mut it = key.chars();
                    match (it.next(), it.next()) {
                        (Some(c), None) if !c.is_control() => Some(c.to_string()),
                        _ => None,
                    }
                };
                actions.extend(self.handle_event(
                    PlatformEvent::KeyDown {
                        key: key_code,
                        logical_key,
                        text,
                        modifiers: rinch_platform::Modifiers {
                            shift,
                            ctrl,
                            alt,
                            meta,
                        },
                        repeat: rinch_platform::KeyRepeat::Fresh,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::Ime {
                action,
                text,
                cursor,
            } => {
                // Synthesize a real `PlatformEvent::Ime` and route it through
                // `handle_event` — exactly the path winit's `WindowEvent::Ime` takes
                // — so a debug-injected composition drives the focus arbiter and the
                // focused target's preedit/commit identically to a physical IME.
                let ime_event = match action.as_str() {
                    "enable" => Some(rinch_platform::ImeEvent::Enabled),
                    "preedit" => Some(rinch_platform::ImeEvent::Preedit { text, cursor }),
                    "commit" => Some(rinch_platform::ImeEvent::Commit(text)),
                    "disable" => Some(rinch_platform::ImeEvent::Disabled),
                    other => {
                        return DebugResult::Error {
                            message: format!("Unknown ime action: {other}"),
                        };
                    }
                };
                if let Some(ev) = ime_event {
                    actions.extend(self.handle_event(
                        PlatformEvent::Ime(ev),
                        window_size,
                        scale_factor,
                    ));
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetCaretPosition {
                node_id,
                byte_offset,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };

                // Everything below is in logical (CSS) px, like `absolute` and
                // the input tools: the box comes from
                // `compute_absolute_position_and_transform(.., 1.0)`, so the
                // offset inside it — paddings, and the probe layout's font size
                // against its logical `max_width` — must be unscaled too (#421).
                // The window's scale factor plays no part.
                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error {
                        message: format!("Node {} not found", node_id),
                    };
                };

                // The node's painted frame. Every coordinate reported below is a
                // position *inside* the box, so it is pushed forward through the
                // composed transform rather than added to an origin — under a
                // `scale()` container the two differ (#203). Copied out before
                // the document borrow is released for the Parley rebuild.
                let (box_x, box_y, node_transform) =
                    rinch_dom::paint::compute_absolute_position_and_transform(
                        &d.tree, node_id, 1.0,
                    );
                let fwd = move |lx: f64, ly: f64| -> (f64, f64) {
                    let p = node_transform * peniko::kurbo::Point::new(box_x + lx, box_y + ly);
                    (p.x, p.y)
                };

                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = rinch_dom::form_control::control_value(&d.tree.nodes, node_id)
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    if value.is_empty() {
                        let padding_left = node.computed_style.padding_left.to_px() as f64;
                        let padding_top = node.computed_style.padding_top.to_px() as f64;
                        let (cx, cy) = fwd(padding_left, padding_top);
                        return DebugResult::Json {
                            data: json!({ "x": cx, "y": cy }),
                        };
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        1.0,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    let (x, y) = caret_position_for_offset_layout(&layout, byte_offset);
                    let padding_left = computed_style.padding_left.to_px() as f64;
                    let padding_top = computed_style.padding_top.to_px() as f64;

                    let (cx, cy) = fwd(padding_left + x as f64, padding_top + y as f64);
                    return DebugResult::Json {
                        data: json!({ "x": cx, "y": cy }),
                    };
                }

                if let Some(ref inline_layout) = node.text_layout {
                    let (x, y) =
                        caret_position_for_offset_layout(&inline_layout.layout, byte_offset);
                    let (cx, cy) = fwd(x as f64, y as f64);
                    return DebugResult::Json {
                        data: json!({ "x": cx, "y": cy }),
                    };
                }

                DebugResult::Error {
                    message: "Node does not have text layout".into(),
                }
            }
            DebugCommandKind::GetGlyphBounds {
                node_id,
                byte_offset,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };

                // Everything below is in logical (CSS) px, like `absolute` and
                // the input tools: the box comes from
                // `compute_absolute_position_and_transform(.., 1.0)`, so the
                // offset inside it — paddings, and the probe layout's font size
                // against its logical `max_width` — must be unscaled too (#421).
                // The window's scale factor plays no part.
                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error {
                        message: format!("Node {} not found", node_id),
                    };
                };

                // The node's painted frame. Every coordinate reported below is a
                // position *inside* the box, so it is pushed forward through the
                // composed transform rather than added to an origin — under a
                // `scale()` container the two differ (#203). Copied out before
                // the document borrow is released for the Parley rebuild.
                let (box_x, box_y, node_transform) =
                    rinch_dom::paint::compute_absolute_position_and_transform(
                        &d.tree, node_id, 1.0,
                    );
                let fwd = move |lx: f64, ly: f64| -> (f64, f64) {
                    let p = node_transform * peniko::kurbo::Point::new(box_x + lx, box_y + ly);
                    (p.x, p.y)
                };

                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = rinch_dom::form_control::control_value(&d.tree.nodes, node_id)
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    if value.is_empty() {
                        return DebugResult::Error {
                            message: "No text content".into(),
                        };
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        1.0,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    match glyph_bounds_for_offset_layout(&layout, byte_offset) {
                        Some(bounds) => {
                            let padding_left = computed_style.padding_left.to_px() as f64;
                            let padding_top = computed_style.padding_top.to_px() as f64;
                            let (gx, gy) = fwd(
                                padding_left + bounds.x as f64,
                                padding_top + bounds.y as f64,
                            );
                            return DebugResult::Json {
                                data: json!({
                                    "x": gx,
                                    "y": gy,
                                    "width": bounds.width,
                                    "height": bounds.height,
                                }),
                            };
                        }
                        None => {
                            return DebugResult::Error {
                                message: "Byte offset out of bounds".into(),
                            };
                        }
                    }
                }

                if let Some(ref inline_layout) = node.text_layout {
                    match glyph_bounds_for_offset_layout(&inline_layout.layout, byte_offset) {
                        Some(bounds) => {
                            let (gx, gy) = fwd(bounds.x as f64, bounds.y as f64);
                            return DebugResult::Json {
                                data: json!({
                                    "x": gx,
                                    "y": gy,
                                    "width": bounds.width,
                                    "height": bounds.height,
                                }),
                            };
                        }
                        None => {
                            return DebugResult::Error {
                                message: "Byte offset out of bounds".into(),
                            };
                        }
                    }
                }

                DebugResult::Error {
                    message: "Node does not have text layout".into(),
                }
            }
        }
    }
}

#[cfg(test)]
mod keycode_mapping_tests {
    use super::{char_to_keycode, keyname_to_keycode};
    use rinch_platform::KeyCode;

    #[test]
    fn punctuation_maps_to_other_not_space() {
        // Punctuation must not masquerade as the spacebar (issue #151): with
        // `Other`, the hook's key string falls through to the `text` field.
        assert_eq!(char_to_keycode('.'), KeyCode::Other);
    }

    #[test]
    fn space_char_keeps_its_named_keycode() {
        // Space-parity gotcha: an injected ' ' must still read as the
        // spacebar (`code = "Space"`), exactly like a physical press.
        assert_eq!(char_to_keycode(' '), KeyCode::Space);
    }

    #[test]
    fn keyname_space_maps_to_space() {
        assert_eq!(keyname_to_keycode("Space"), Some(KeyCode::Space));
    }

    #[test]
    fn keyname_single_char_punctuation_maps_to_other() {
        assert_eq!(keyname_to_keycode("."), Some(KeyCode::Other));
    }

    #[test]
    fn unknown_multi_char_keyname_is_rejected() {
        // The KeyPress handler turns this into a DebugResult::Error rather
        // than a silent no-text `Other` press.
        assert_eq!(keyname_to_keycode("NoSuchKey"), None);
    }
}

#[cfg(test)]
mod tests {
    //! MCP `scroll()` drives the real wheel path (#401).
    //!
    //! These live in this module rather than `app/mod.rs` so they exist under
    //! exactly the `cfg` the code does: the module is `#[cfg(feature =
    //! "debug")]`, and a test placed outside it would silently compile to
    //! nothing whenever `debug` is off.

    use super::*;
    use std::cell::{Cell, RefCell};

    /// Physical == logical; every other test in this crate mounts at 800x600.
    const VIEWPORT: (u32, u32) = (800, 600);

    /// A 200x100 `overflow: auto` scroller at the document origin holding a
    /// 900x1100 child, so it overflows on **both** axes and either delta moves
    /// it — plus a 20x20 box parked at (600, 500), far enough away that a dirty
    /// region computed from it alone cannot reach the scroller.
    ///
    /// Returns `(scroller, bystander)`.
    fn app_with_scroller(ids: Rc<Cell<Option<(usize, usize)>>>) -> RinchApp {
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "position: relative; width: 800px; height: 600px");
            let sc = scope.create_element("div");
            sc.set_attribute(
                "style",
                "position: absolute; left: 0; top: 0; width: 200px; height: 100px; \
                 overflow: auto",
            );
            let content = scope.create_element("div");
            content.set_attribute("style", "width: 900px; height: 1100px");
            sc.append_child(&content);
            root.append_child(&sc);

            let bystander = scope.create_element("div");
            bystander.set_attribute(
                "style",
                "position: absolute; left: 600px; top: 500px; width: 20px; height: 20px",
            );
            root.append_child(&bystander);

            ids.set(Some((sc.node_id().0, bystander.node_id().0)));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        app
    }

    fn mcp_scroll(app: &mut RinchApp, x: f32, y: f32, delta_x: f64, delta_y: f64) {
        let mut actions = Vec::new();
        app.execute_debug_command(
            DebugCommandKind::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            },
            &mut actions,
            1.0,
            VIEWPORT,
        );
    }

    fn offsets(app: &RinchApp, id: usize) -> (f64, f64) {
        app.doc.as_ref().unwrap().borrow().tree.nodes[id].scroll_offset
    }

    /// The bug itself: `push_dirty` is what fills `paint_dirty_nodes`, which is
    /// `compute_dirty_region`'s only input. Without it the software renderer
    /// narrows the repaint to whatever else was dirty and leaves most of the
    /// scrolled container showing pre-scroll pixels — persistently, because
    /// nothing later re-dirties it.
    ///
    /// Asserting on `paint_dirty_nodes` rather than on a rasterised frame is
    /// deliberate: the missing entry is the defect, and a screenshot is a lossy
    /// view of it. (Since #886 an empty damage repaints nothing, so without the
    /// entry the scrolled container keeps its pre-scroll pixels even when
    /// nothing else is dirty.)
    #[test]
    fn an_mcp_scroll_pushes_the_container_paint_dirty() {
        let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let mut app = app_with_scroller(ids.clone());
        let (sc, _bystander) = ids.get().expect("the node ids");

        // Start from a clean slate so the assertion is about this scroll only.
        app.doc
            .as_ref()
            .unwrap()
            .borrow_mut()
            .tree
            .paint_dirty_nodes
            .clear();

        mcp_scroll(&mut app, 100.0, 50.0, 0.0, 300.0);

        let dirty = app
            .doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .paint_dirty_nodes
            .clone();
        assert!(
            dirty.contains(&sc),
            "the scrolled container must be in paint_dirty_nodes, got {dirty:?}"
        );
    }

    /// The live failure mode, reproduced: when this was reported, an *empty*
    /// `paint_dirty_nodes` meant a full repaint and a correct frame, so the
    /// corruption only appeared when something *else* was already dirty (in
    /// the report, a drawer that had just closed). Seed one unrelated dirty
    /// node and the damage has to cover the container anyway. (Since #886 an
    /// empty damage repaints nothing, so the entry is needed either way.)
    #[test]
    fn an_mcp_scroll_widens_a_dirty_region_that_already_has_other_nodes_in_it() {
        let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let mut app = app_with_scroller(ids.clone());
        let (_sc, bystander) = ids.get().expect("the node ids");

        // Something unrelated repainted this frame: the 20x20 box at
        // (600, 500), whose own dirty region stops well short of the scroller.
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.tree.paint_dirty_nodes.clear();
            d.tree.paint_dirty_nodes.push(bystander);
            let alone = rinch_dom::paint::compute_dirty_region(&d.tree, 1.0, 800.0, 600.0)
                .expect("the bystander is dirty");
            assert!(
                alone.x0 > 200.0 && alone.y0 > 100.0,
                "the bystander must not already cover the scroller, got {alone:?}"
            );
        }

        mcp_scroll(&mut app, 100.0, 50.0, 0.0, 300.0);

        let region = {
            let doc = app.doc.as_ref().unwrap();
            let d = doc.borrow();
            rinch_dom::paint::compute_dirty_region(&d.tree, 1.0, 800.0, 600.0)
        }
        .expect("something is dirty, so there is a region");

        // The container is the 200x100 box at the origin, and the bystander
        // sits at (600, 500) — so it is the *near* corner that proves the
        // region grew: `x1`/`y1` alone are already past 200/100 from the
        // bystander's own rect and would pass without the fix.
        assert!(
            region.x0 <= 0.0 && region.y0 <= 0.0,
            "the region must reach the scrolled container at the origin, got {region:?}"
        );
        assert!(
            region.x1 >= 200.0 && region.y1 >= 100.0,
            "and cover all of it, got {region:?}"
        );
    }

    /// The MCP tool documents positive as "scroll down" / "scroll right", while
    /// a wheel delta is negative in those directions — so the sign has to
    /// convert exactly once, at the boundary. Both axes, because the old
    /// hand-written arm ignored `delta_x` for document scrolling entirely.
    #[test]
    fn an_mcp_scroll_moves_both_axes_in_the_documented_direction() {
        let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let mut app = app_with_scroller(ids.clone());
        let (sc, _bystander) = ids.get().expect("the node ids");
        assert_eq!(offsets(&app, sc), (0.0, 0.0));

        mcp_scroll(&mut app, 100.0, 50.0, 120.0, 300.0);
        let (left, top) = offsets(&app, sc);
        assert!(top > 0.0, "positive delta_y must scroll down, got {top}");
        assert!(left > 0.0, "positive delta_x must scroll right, got {left}");

        // And back, so a wrong sign cannot pass by clamping at 0.
        mcp_scroll(&mut app, 100.0, 50.0, -120.0, -300.0);
        assert_eq!(
            offsets(&app, sc),
            (0.0, 0.0),
            "a negative delta must undo it"
        );
    }

    /// `perf_stats` answers the document's counters as JSON keyed by counter
    /// name, and `reset: true` zeroes them after the read.
    #[test]
    fn perf_stats_reports_and_resets_the_counters() {
        let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let mut app = app_with_scroller(ids);
        app.end_perf_frame();
        let mut actions = Vec::new();
        let DebugResult::Json { data } = app.execute_debug_command(
            DebugCommandKind::PerfStats { reset: true },
            &mut actions,
            1.0,
            VIEWPORT,
        ) else {
            panic!("perf_stats answers JSON");
        };
        assert_eq!(data["frames"], 1);
        assert!(
            data["last_frame"]["layout_resolves"].as_u64().unwrap() > 0,
            "the mount's layout is in the frame it ended: {data}"
        );
        assert!(data["total"]["elements_cascaded"].as_u64().unwrap() > 0);
        assert!(data["current_frame"].is_object());
        assert_eq!(
            data["last_frame"].as_object().unwrap().len(),
            rinch_dom::perf::Counter::COUNT
        );

        let DebugResult::Json { data } = app.execute_debug_command(
            DebugCommandKind::PerfStats { reset: false },
            &mut actions,
            1.0,
            VIEWPORT,
        ) else {
            panic!("perf_stats answers JSON");
        };
        assert_eq!(data["frames"], 0, "reset zeroed the frame count: {data}");
        assert_eq!(data["total"]["elements_cascaded"], 0);
    }

    // ── Modified clicks: `click` / `mouse_down` / `mouse_up` `modifiers` ──

    /// `(shift, ctrl, alt, meta)`, as a handler read it from its
    /// `ClickContext`, or as `RinchApp::modifiers` holds it.
    type Mods = (bool, bool, bool, bool);

    /// What each handler saw: `("down" | "up" | "click", modifiers)`.
    type Seen = Rc<RefCell<Vec<(&'static str, Mods)>>>;

    fn mods(m: Modifiers) -> Mods {
        (m.shift, m.ctrl, m.alt, m.meta)
    }

    const NONE: Mods = (false, false, false, false);
    const SHIFT: Mods = (true, false, false, false);
    const CTRL: Mods = (false, true, false, false);
    const ALT: Mods = (false, false, true, false);

    /// A 200x100 box at the origin with a click handler (`data-rid`) and
    /// `data-onmousedown`/`data-onmouseup` handlers, each recording the
    /// modifiers its `ClickContext` carries.
    fn app_with_button(seen: Seen) -> RinchApp {
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let button = scope.create_element("div");
            button.set_attribute("style", "width: 200px; height: 100px");
            for (attr, name) in [
                ("data-rid", "click"),
                ("data-onmousedown", "down"),
                ("data-onmouseup", "up"),
            ] {
                let seen = seen.clone();
                let id = scope.register_handler(move || {
                    let m = events::get_click_context().modifiers;
                    seen.borrow_mut()
                        .push((name, (m.shift, m.ctrl, m.alt, m.meta)));
                });
                button.set_attribute(attr, &id.0.to_string());
            }
            root.append_child(&button);
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        app
    }

    fn run(app: &mut RinchApp, kind: DebugCommandKind) -> DebugResult {
        let mut actions = Vec::new();
        app.execute_debug_command(kind, &mut actions, 1.0, VIEWPORT)
    }

    fn names(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(|n| n.to_string()).collect())
    }

    fn click(modifiers: Option<Vec<String>>) -> DebugCommandKind {
        DebugCommandKind::Click {
            x: 50.0,
            y: 50.0,
            button: None,
            modifiers,
        }
    }

    fn hold(app: &mut RinchApp, m: Modifiers) {
        app.handle_event(PlatformEvent::ModifiersChanged(m), VIEWPORT, 1.0);
    }

    /// The feature: a click with `["ctrl"]` reaches every handler with Ctrl
    /// held, and afterwards the app holds no modifier again.
    #[test]
    fn a_ctrl_click_reaches_the_app_with_ctrl_held_and_then_releases_it() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        assert!(matches!(
            run(&mut app, click(names(&["ctrl"]))),
            DebugResult::Json { .. }
        ));
        assert_eq!(
            *seen.borrow(),
            vec![("down", CTRL), ("click", CTRL), ("up", CTRL)]
        );
        assert_eq!(mods(app.modifiers), NONE, "released after the click");
    }

    /// The requested set is exact for the click (a held Shift is not added
    /// to `["ctrl"]`), and what was held before comes back afterwards.
    #[test]
    fn a_modified_click_restores_the_state_held_before_it() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        hold(
            &mut app,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        run(&mut app, click(names(&["Control"])));
        assert_eq!(
            *seen.borrow(),
            vec![("down", CTRL), ("click", CTRL), ("up", CTRL)]
        );
        assert_eq!(mods(app.modifiers), SHIFT, "the held Shift is back");
    }

    /// Every alias `key_press` takes, folded the same way.
    #[test]
    fn the_modifier_names_are_key_press_names() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        run(&mut app, click(names(&["shift", "option", "cmd"])));
        assert_eq!(seen.borrow()[1], ("click", (true, false, true, true)));
        seen.borrow_mut().clear();
        run(&mut app, click(names(&["SUPER", "alt", "meta"])));
        assert_eq!(seen.borrow()[1], ("click", (false, false, true, true)));
    }

    /// An unknown name fails loud, with `key_press`'s sentence, before any
    /// input reaches the app — on all three commands.
    #[test]
    fn an_unknown_modifier_name_is_rejected_and_nothing_is_pressed() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        let commands = [
            click(names(&["ctrl", "hyper"])),
            DebugCommandKind::MouseDown {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: names(&["hyper"]),
            },
            DebugCommandKind::MouseUp {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: names(&["hyper"]),
            },
        ];
        for command in commands {
            let DebugResult::Error { message } = run(&mut app, command) else {
                panic!("an unknown modifier name is an error");
            };
            assert_eq!(
                message,
                "Unknown modifier name: \"hyper\" (expected ctrl/control, shift, alt/option, meta/cmd/super)"
            );
        }
        assert!(seen.borrow().is_empty(), "nothing was pressed or released");
        assert_eq!(mods(app.modifiers), NONE);
        assert_eq!(app.debug_modifiers_to_restore, None);
    }

    /// Without the field, a click is what it always was: it sees the
    /// modifiers the app holds and leaves them held.
    #[test]
    fn without_modifiers_a_click_sees_and_keeps_the_held_state() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        run(&mut app, click(None));
        assert_eq!(
            *seen.borrow(),
            vec![("down", NONE), ("click", NONE), ("up", NONE)]
        );
        seen.borrow_mut().clear();
        hold(
            &mut app,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        run(&mut app, click(None));
        assert_eq!(
            *seen.borrow(),
            vec![("down", SHIFT), ("click", SHIFT), ("up", SHIFT)]
        );
        assert_eq!(mods(app.modifiers), SHIFT, "still held: nothing restored");
    }

    /// An empty array is not an absent field: it asks for no modifier, and
    /// what was held comes back afterwards.
    #[test]
    fn an_empty_array_clicks_with_no_modifier() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        hold(
            &mut app,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        run(&mut app, click(names(&[])));
        assert_eq!(seen.borrow()[1], ("click", NONE));
        assert_eq!(mods(app.modifiers), SHIFT);
    }

    /// Separate press and release: the press sets the modifiers and they stay
    /// held (through a move) for a release that names none; that release
    /// restores the state from before the press.
    #[test]
    fn mouse_down_holds_its_modifiers_until_mouse_up_restores_them() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        run(
            &mut app,
            DebugCommandKind::MouseDown {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: names(&["shift"]),
            },
        );
        assert_eq!(mods(app.modifiers), SHIFT, "held after the press");
        run(&mut app, DebugCommandKind::MouseMove { x: 60.0, y: 50.0 });
        assert_eq!(mods(app.modifiers), SHIFT, "held through a move");
        run(
            &mut app,
            DebugCommandKind::MouseUp {
                x: 60.0,
                y: 50.0,
                button: None,
                modifiers: None,
            },
        );
        assert_eq!(
            *seen.borrow(),
            vec![("down", SHIFT), ("click", SHIFT), ("up", SHIFT)]
        );
        assert_eq!(mods(app.modifiers), NONE, "the release restored them");
        assert_eq!(app.debug_modifiers_to_restore, None);
    }

    /// A release that names its own modifiers holds them for the release, and
    /// still returns to the state from before the modified press.
    #[test]
    fn mouse_up_with_its_own_modifiers_returns_to_the_state_before_the_press() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        hold(
            &mut app,
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        run(
            &mut app,
            DebugCommandKind::MouseDown {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: names(&["shift"]),
            },
        );
        run(
            &mut app,
            DebugCommandKind::MouseUp {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: names(&["alt"]),
            },
        );
        assert_eq!(
            *seen.borrow(),
            vec![("down", SHIFT), ("click", SHIFT), ("up", ALT)]
        );
        assert_eq!(mods(app.modifiers), CTRL, "the state before the press");

        // A lone modified release restores its own prior state.
        seen.borrow_mut().clear();
        run(
            &mut app,
            DebugCommandKind::MouseUp {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: names(&["alt"]),
            },
        );
        assert_eq!(seen.borrow()[0], ("up", ALT));
        assert_eq!(mods(app.modifiers), CTRL);
    }

    /// Two modified presses before one release: the release goes back to
    /// what was held before the first, not to the first press's modifiers.
    #[test]
    fn a_second_modified_press_keeps_the_first_remembered_state() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        for m in [&["shift"], &["alt"]] {
            run(
                &mut app,
                DebugCommandKind::MouseDown {
                    x: 50.0,
                    y: 50.0,
                    button: None,
                    modifiers: names(m),
                },
            );
        }
        assert_eq!(mods(app.modifiers), ALT);
        run(
            &mut app,
            DebugCommandKind::MouseUp {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: None,
            },
        );
        assert_eq!(mods(app.modifiers), NONE);
    }

    /// Unmodified press and release are what they always were: nothing is
    /// remembered, nothing restored.
    #[test]
    fn unmodified_mouse_down_and_up_leave_the_held_state_alone() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        hold(
            &mut app,
            Modifiers {
                alt: true,
                ..Default::default()
            },
        );
        for kind in [
            DebugCommandKind::MouseDown {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: None,
            },
            DebugCommandKind::MouseUp {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: None,
            },
        ] {
            run(&mut app, kind);
            assert_eq!(app.debug_modifiers_to_restore, None);
            assert_eq!(mods(app.modifiers), ALT);
        }
        assert_eq!(
            *seen.borrow(),
            vec![("down", ALT), ("click", ALT), ("up", ALT)]
        );
    }

    fn press_ctrl(app: &mut RinchApp) {
        run(
            app,
            DebugCommandKind::MouseDown {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: names(&["ctrl"]),
            },
        );
        assert_eq!(mods(app.modifiers), CTRL);
    }

    /// A modified `mouse_down` whose `mouse_up` never comes (the client went
    /// away): a real modifier change supersedes it, so a real click is not
    /// modified, and a later debug `mouse_up` does not resurrect the ctrl
    /// the press remembered.
    #[test]
    fn a_real_modifier_change_supersedes_a_stranded_modified_press() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        press_ctrl(&mut app);
        // The release is lost; the user then presses and releases a real key.
        app.handle_event(
            PlatformEvent::MouseUp {
                x: 50.0,
                y: 50.0,
                button: MouseButton::Left,
            },
            VIEWPORT,
            1.0,
        );
        hold(&mut app, Modifiers::default());
        assert_eq!(app.debug_modifiers_to_restore, None);
        seen.borrow_mut().clear();
        for event in [
            PlatformEvent::MouseDown {
                x: 50.0,
                y: 50.0,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseUp {
                x: 50.0,
                y: 50.0,
                button: MouseButton::Left,
            },
        ] {
            app.handle_event(event, VIEWPORT, 1.0);
        }
        assert_eq!(
            *seen.borrow(),
            vec![("down", NONE), ("click", NONE), ("up", NONE)]
        );

        // Hold alt for real; a later unmodified debug release keeps it.
        hold(
            &mut app,
            Modifiers {
                alt: true,
                ..Default::default()
            },
        );
        run(
            &mut app,
            DebugCommandKind::MouseUp {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: None,
            },
        );
        assert_eq!(mods(app.modifiers), ALT, "ctrl is not resurrected");
    }

    /// A modified click between a modified press and its release is part of
    /// the debug sequence, not a real change: the release still restores.
    #[test]
    fn a_modified_click_inside_a_held_press_keeps_the_remembered_state() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        press_ctrl(&mut app);
        run(&mut app, click(names(&["shift"])));
        assert_eq!(
            mods(app.modifiers),
            CTRL,
            "the click put back the press's ctrl"
        );
        run(
            &mut app,
            DebugCommandKind::MouseUp {
                x: 50.0,
                y: 50.0,
                button: None,
                modifiers: None,
            },
        );
        assert_eq!(mods(app.modifiers), NONE);
    }

    /// Losing the window's keyboard releases what a debug press is holding.
    #[test]
    fn window_blur_releases_a_debug_press_s_modifiers() {
        let seen: Seen = Rc::default();
        let mut app = app_with_button(seen.clone());
        hold(
            &mut app,
            Modifiers {
                alt: true,
                ..Default::default()
            },
        );
        press_ctrl(&mut app);
        app.handle_event(PlatformEvent::WindowFocus(false), VIEWPORT, 1.0);
        assert_eq!(
            mods(app.modifiers),
            ALT,
            "back to the state before the press"
        );
        assert_eq!(app.debug_modifiers_to_restore, None);
    }
}

#[cfg(test)]
mod text_geometry_units_tests {
    //! `get_caret_position` / `get_glyph_bounds` answer in **logical** px at
    //! every scale factor (#421), like `absolute` and the input tools.
    //!
    //! Before #421 the two handlers took the node's box at scale 1.0 and the
    //! offset inside it (paddings, the probe Parley layout's font size) at the
    //! window's scale factor, so at scale 2 the answer was the box's logical
    //! origin plus a doubled intra-box offset — and the probe layout paired a
    //! doubled font with the logical `max_width`, wrapping at the wrong width.
    //! Every fixture compares scale 2 against scale 1: at scale 1 the two units
    //! coincide, which is the fixed point that hid the bug.

    use super::*;
    use std::cell::Cell;

    /// `(input, textarea, paragraph)` node ids.
    type Ids = Rc<Cell<Option<(usize, usize, usize)>>>;

    /// An `<input>` and a `<textarea>` with off-zero paddings, placed off the
    /// origin, and a `<p>` holding IFC text. Fonts and line heights are
    /// declared; the textarea's text fits its 300px width at 14px but not at
    /// 28px, so a probe layout built with a doubled font wraps it.
    fn app_with_text_fields(ids: Ids) -> RinchApp {
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "position: relative; width: 800px; height: 600px; \
                 font-family: sans-serif; font-size: 14px; line-height: 20px",
            );
            let input = scope.create_element("input");
            input.set_attribute(
                "style",
                "position: absolute; left: 40px; top: 30px; width: 300px; height: 28px; \
                 padding: 5px 0 0 13px; font-size: 14px; line-height: 20px",
            );
            input.set_attribute("value", "hello world");
            root.append_child(&input);

            let textarea = scope.create_element("textarea");
            textarea.set_attribute(
                "style",
                "position: absolute; left: 40px; top: 100px; width: 300px; height: 120px; \
                 padding: 7px 0 0 11px; font-size: 14px; line-height: 20px",
            );
            textarea.set_attribute("value", "one two three four five six");
            root.append_child(&textarea);

            let p = scope.create_element("p");
            p.set_attribute(
                "style",
                "position: absolute; left: 40px; top: 300px; width: 300px; margin: 0; \
                 font-size: 14px; line-height: 20px",
            );
            let text = scope.create_text("hello paragraph");
            p.append_child(&text);
            root.append_child(&p);

            ids.set(Some((
                input.node_id().0,
                textarea.node_id().0,
                p.node_id().0,
            )));
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        app
    }

    fn query(app: &mut RinchApp, kind: DebugCommandKind, scale: f64) -> serde_json::Value {
        let mut actions = Vec::new();
        let size = ((800.0 * scale) as u32, (600.0 * scale) as u32);
        match app.execute_debug_command(kind, &mut actions, scale, size) {
            DebugResult::Json { data } => data,
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    fn caret(app: &mut RinchApp, node_id: usize, byte_offset: usize, scale: f64) -> (f64, f64) {
        let v = query(
            app,
            DebugCommandKind::GetCaretPosition {
                node_id,
                byte_offset,
            },
            scale,
        );
        (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap())
    }

    fn glyph(app: &mut RinchApp, node_id: usize, byte_offset: usize, scale: f64) -> [f64; 4] {
        let v = query(
            app,
            DebugCommandKind::GetGlyphBounds {
                node_id,
                byte_offset,
            },
            scale,
        );
        ["x", "y", "width", "height"].map(|k| v[k].as_f64().unwrap())
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    /// The input's caret at scale 2 is where it is at scale 1, and at byte 0
    /// it is the box's logical origin plus its logical paddings.
    #[test]
    fn an_input_caret_is_logical_at_scale_two() {
        let ids: Ids = Rc::default();
        let mut app = app_with_text_fields(ids.clone());
        let (input, _, _) = ids.get().unwrap();

        let (x0, y0) = caret(&mut app, input, 0, 1.0);
        assert!(
            close(x0, 40.0 + 13.0) && close(y0, 30.0 + 5.0),
            "byte 0 sits at the logical content origin (53, 35), got ({x0}, {y0})"
        );
        for byte in [0, 5, 11] {
            let one = caret(&mut app, input, byte, 1.0);
            let two = caret(&mut app, input, byte, 2.0);
            assert!(
                close(one.0, two.0) && close(one.1, two.1),
                "byte {byte}: scale 2 must answer the scale-1 (logical) caret {one:?}, got {two:?}"
            );
        }
    }

    /// The same for an input's glyph box, width and height included.
    #[test]
    fn an_input_glyph_box_is_logical_at_scale_two() {
        let ids: Ids = Rc::default();
        let mut app = app_with_text_fields(ids.clone());
        let (input, _, _) = ids.get().unwrap();
        for byte in [0, 6] {
            let one = glyph(&mut app, input, byte, 1.0);
            let two = glyph(&mut app, input, byte, 2.0);
            assert!(
                one.iter().zip(two).all(|(a, b)| close(*a, b)),
                "byte {byte}: scale 2 must answer the scale-1 (logical) box {one:?}, got {two:?}"
            );
        }
    }

    /// A textarea's probe layout wraps at the logical width: its last caret
    /// stays on the first line at scale 2, where a doubled font wrapped it.
    #[test]
    fn a_textarea_caret_wraps_at_the_logical_width_at_scale_two() {
        let ids: Ids = Rc::default();
        let mut app = app_with_text_fields(ids.clone());
        let (_, textarea, _) = ids.get().unwrap();
        let end = "one two three four five six".len();
        let one = caret(&mut app, textarea, end, 1.0);
        let two = caret(&mut app, textarea, end, 2.0);
        assert!(
            close(one.1, 100.0 + 7.0),
            "at scale 1 the text fits one line, got {one:?}"
        );
        assert!(
            close(one.0, two.0) && close(one.1, two.1),
            "scale 2 must answer the scale-1 (logical) caret {one:?}, got {two:?}"
        );
    }

    /// An `<input>` with an empty value takes its own early return (its
    /// caret is the content origin, no layout built), with its own paddings:
    /// logical at scale 2 too. From the PR #1133 review, which found this
    /// branch unpinned by the fixtures above.
    #[test]
    fn an_empty_input_caret_is_logical_at_scale_two() {
        let id: Rc<Cell<usize>> = Rc::default();
        let id2 = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "width: 800px; height: 600px; font-family: sans-serif; \
                 font-size: 14px; line-height: 20px",
            );
            let input = scope.create_element("input");
            input.set_attribute(
                "style",
                "position: absolute; left: 40px; top: 30px; width: 300px; height: 20px; \
                 padding: 3px 0 0 13px; font-size: 14px; line-height: 20px",
            );
            input.set_attribute("value", "");
            root.append_child(&input);
            id2.set(input.node_id().0);
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let input = id.get();

        let one = caret(&mut app, input, 0, 1.0);
        assert!(
            close(one.0, 40.0 + 13.0) && close(one.1, 30.0 + 3.0),
            "the empty input's caret is its logical content origin (53, 33), got {one:?}"
        );
        let two = caret(&mut app, input, 0, 2.0);
        assert!(
            close(one.0, two.0) && close(one.1, two.1),
            "scale 2 must answer the scale-1 (logical) caret {one:?}, got {two:?}"
        );
    }

    /// IFC text was already logical; pinned so it stays that way.
    #[test]
    fn a_paragraph_caret_and_glyph_are_the_same_at_both_scales() {
        let ids: Ids = Rc::default();
        let mut app = app_with_text_fields(ids.clone());
        let (_, _, p) = ids.get().unwrap();
        assert_eq!(caret(&mut app, p, 6, 1.0), caret(&mut app, p, 6, 2.0));
        assert_eq!(glyph(&mut app, p, 6, 1.0), glyph(&mut app, p, 6, 2.0));
        let (x, y) = caret(&mut app, p, 0, 1.0);
        assert!(close(x, 40.0) && close(y, 300.0), "got ({x}, {y})");
    }
}

#[cfg(test)]
mod key_press_menu_shortcut_533_tests {
    //! Issue #533: the real winit `WindowEvent::KeyboardInput` arm in
    //! `shell/rinch_runtime.rs` checks menu shortcuts *before* ever building a
    //! `PlatformEvent::KeyDown`, but the debug `KeyPress` command used to build
    //! one directly and hand it to `handle_event`, entering the pipeline
    //! downstream of that check. A debug-injected `key_press` for a registered
    //! menu chord therefore never ran the menu's callback, whether or not the
    //! app's real shortcut worked (the issue's own repro:
    //! `key_press(key: "d", ctrl: true)` for a "Ctrl+D" menu item did nothing
    //! through MCP while the same item fired correctly when clicked).
    //!
    //! `crate::menu::register_menu_shortcuts` is the platform-independent entry
    //! point rinch-web's own keydown listener drives through
    //! `match_shortcut_code` — no muda/winit/window needed — so these fixtures
    //! exercise `execute_debug_command` directly with no live app or mount.

    use super::*;
    use crate::menu::{Menu, MenuItem, register_menu_shortcuts};
    use std::cell::Cell;

    fn counting_menu(shortcut: &str) -> (Rc<Cell<u32>>, crate::menu::MenuBarChords) {
        let fired = Rc::new(Cell::new(0));
        let fired_cb = fired.clone();
        let menu = Menu::new().item(
            MenuItem::new("Probe")
                .shortcut(shortcut)
                .on_click(move || fired_cb.set(fired_cb.get() + 1)),
        );
        let chords = register_menu_shortcuts(&[("Test", &menu)]);
        (fired, chords)
    }

    fn key_press(app: &mut RinchApp, key: &str, shift: bool, ctrl: bool, alt: bool) -> DebugResult {
        let mut actions = Vec::new();
        app.execute_debug_command(
            DebugCommandKind::KeyPress {
                key: key.to_string(),
                shift,
                ctrl,
                alt,
                modifiers: Vec::new(),
            },
            &mut actions,
            1.0,
            (800, 600),
        )
    }

    /// The bug itself, at the `execute_debug_command` level: a debug
    /// `key_press` for a letter chord must fire the menu callback, not just
    /// synthesize a `PlatformEvent::KeyDown` nothing is listening for.
    ///
    /// Kills reverting the `KeyPress` arm to its pre-#533 shape (straight to
    /// `handle_event`, no `match_shortcut_code` call): `fired` stays `0`.
    #[test]
    fn a_debug_key_press_fires_a_registered_letter_chord() {
        let (fired, _chords) = counting_menu("Ctrl+D");
        let mut app = RinchApp::new(|scope| scope.create_element("div"));
        key_press(&mut app, "d", false, true, false);
        assert_eq!(fired.get(), 1, "Ctrl+D must fire the menu callback once");
    }

    /// The issue's second repro: a non-letter chord (`Alt+ArrowRight`) must
    /// also route through the matcher — pinned separately from the letter case
    /// because #1170's layout-character branch only applies to `"Key*"` codes,
    /// so this exercises `keyname_to_w3c_code`'s named-key arm instead of its
    /// single-character one.
    #[test]
    fn a_debug_key_press_fires_a_registered_named_key_chord() {
        let (fired, _chords) = counting_menu("Alt+ArrowRight");
        let mut app = RinchApp::new(|scope| scope.create_element("div"));
        key_press(&mut app, "ArrowRight", false, false, true);
        assert_eq!(
            fired.get(),
            1,
            "Alt+ArrowRight must fire the menu callback once"
        );
    }

    /// A debug `key_press` with no matching chord still falls through to a
    /// plain keystroke dispatch (unchanged behaviour) — not a universal
    /// "shortcuts always win" regression. Fixed-point guard: without this, a
    /// mutant that made `match_shortcut_code` (or the new `KeyPress` wiring)
    /// unconditionally consume every press would pass the two tests above but
    /// break ordinary typing.
    #[test]
    fn a_debug_key_press_with_no_matching_chord_still_dispatches_a_keydown() {
        let clicks: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let clicks_in = clicks.clone();
        let id: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let div = scope.create_element("div");
            div.set_attribute("style", "width: 200px; height: 40px");
            div.set_attribute("tabindex", "0");
            let rid = scope.register_handler({
                let clicks = clicks_in.clone();
                move || clicks.set(clicks.get() + 1)
            });
            div.set_attribute("data-rid", &rid.0.to_string());
            id_in.set(div.node_id().0);
            root.append_child(&div);
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        app.set_focus_target(FocusTarget::Node(id.get()));

        key_press(&mut app, "Enter", false, false, false);
        assert_eq!(
            clicks.get(),
            1,
            "an unclaimed Enter must still activate the focused node"
        );
    }

    /// Issue #1169: a modifier-less chord yields to a focused text field, on
    /// the debug path exactly as it does on a real keystroke — a `key_press`
    /// of a bare letter chord with a text input focused must type the letter,
    /// not fire the menu.
    #[test]
    fn a_debug_key_press_yields_a_bare_chord_to_a_focused_text_input() {
        let (fired, _chords) = counting_menu("N");
        let id: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let id_in = id.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let input = scope.create_element("input");
            input.set_attribute("value", "");
            id_in.set(input.node_id().0);
            root.append_child(&input);
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        app.set_focus_target(FocusTarget::Input(id.get()));

        key_press(&mut app, "n", false, false, false);
        assert_eq!(
            fired.get(),
            0,
            "a bare N must be the focused input's to type, not the menu's"
        );
    }
}
