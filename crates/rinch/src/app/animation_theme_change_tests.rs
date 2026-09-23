//! A theme change must not stop the app's spinners (issue #762).
//!
//! `resolve_and_repaint`'s theme branch calls `RinchDocument::update_theme_variables`
//! and then `recompute_all_styles_full`. That pair used to clear every running
//! `@keyframes` animation and re-register none, because the re-cascade it runs
//! deliberately turns `transitions_enabled` off and the animation half of
//! `apply_stylo_styles_to_taffy` was reading the same flag. Toggling dark mode
//! therefore killed every `Loader`, `Skeleton` and `Progress` stripe in the app,
//! permanently.
//!
//! These fixtures mount the **real** `Loader` under the **real** component
//! stylesheet, so what is pinned is the shipped component rather than a copy of
//! its CSS. The mechanism itself is pinned in
//! `rinch-dom/tests/animation_start_gating_tests.rs`, which also records the
//! Chrome 150 measurement the "keeps its clock" rule comes from.

use super::*;

use rinch_components::Loader;
use rinch_core::Component;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The element that carries `animation: rinch-loader-spin 0.8s linear infinite`.
const OVAL: &str = "rinch-loader__oval";

/// The one node carrying `class` exactly, as a raw node id.
fn node_with_class(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one node carrying `{class}`, found {}",
        matches.len()
    );
    matches[0]
}

fn running(app: &RinchApp, node: usize) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.active_animations.get(&node).map_or(0, |a| a.len())
}

fn start_time(app: &RinchApp, node: usize) -> f64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .active_animations
        .get(&node)
        .expect("the loader's spin should be running")[0]
        .start_time_ms
}

/// Move the spin's clock back by `ms` and answer the timestamp it now carries.
/// A restart writes the wall clock of the restyle, which is ~`ms` later — far
/// enough apart that the assertion cannot be satisfied by two registrations
/// microseconds apart.
fn backdate(app: &mut RinchApp, node: usize, ms: f64) -> f64 {
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    let anim = &mut d
        .tree
        .active_animations
        .get_mut(&node)
        .expect("the loader's spin should be running")[0];
    anim.start_time_ms -= ms;
    anim.start_time_ms
}

/// A mounted app whose whole content is one `Loader`, under the real component
/// stylesheet, plus one frame of `resolve_and_repaint`.
///
/// The stylesheet is rendered by the component itself, as a `<style>` element
/// appended **before** the loader. That is not a convenience: `mount_component`
/// runs the first `resolve_layout` itself, so CSS loaded after it returns
/// arrives a frame late and the loader's first cascade would find no
/// `animation` at all — which is a different fixed point from the one #762 is
/// about. A real app is in the first shape, not the second: the component CSS
/// is appended to the theme sheet (`rinch::generate_theme_css_string`) and
/// `set_theme_css`'d into the document before that same first layout.
fn mount_loader() -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");

        let style_el = scope.create_element("style");
        let css = scope.create_text(&rinch_components::generate_component_css());
        style_el.append_child(&css);
        root.append_child(&style_el);

        let loader = Loader {
            r#type: "oval".to_string(),
            size: "md".to_string(),
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&loader);
        root
    });

    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

/// The first frame a `Loader` appears in is the frame it starts spinning in,
/// with no resize, no scale-factor change and no second restyle to rescue it.
///
/// This is the shape an embedded `RinchContext` at a fixed size hits, and the
/// reason there is no separate embed fixture: `RinchContext::new` *is*
/// `RinchApp::new` plus `mount_component` (`embed.rs`), so the first frame it
/// produces is the one below. What it does not have is the desktop shell's
/// first resize or scale-factor event, and that incidental re-cascade was the
/// only thing starting a spinner before #762 — with no such event, the
/// animation stayed dead for the life of the context.
///
/// Kills the mutant "restore `if self.tree.transitions_enabled` around the
/// animation block".
#[test]
fn a_loader_spins_in_the_frame_it_is_mounted_in() {
    let app = mount_loader();
    let oval = node_with_class(&app, OVAL);

    {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        assert_eq!(
            d.tree.get(oval).unwrap().animation_specs.len(),
            1,
            "precondition: the shipped stylesheet really does put an `animation` \
             on the loader's oval"
        );
    }
    assert_eq!(
        running(&app, oval),
        1,
        "the loader spins from its first frame, as it does in a browser"
    );
}

/// Toggling the theme leaves the spin running **and** leaves its clock alone.
///
/// The two calls here are exactly what `resolve_and_repaint`'s
/// `#[cfg(feature = "theme")]` branch makes when `effective_theme_css` changes
/// (`app/mod.rs`, "Check if theme CSS has changed"); they are made directly so
/// the fixture runs under a plain `cargo test -p rinch`, where the `theme`
/// feature is off. `a_real_dark_mode_toggle_keeps_the_loader_spinning` below
/// drives the branch itself when the feature is on.
///
/// Kills the mutant "clear `active_animations` in `recompute_all_styles_full`"
/// (M2 in `rinch-dom/tests/animation_start_gating_tests.rs`). With the gate
/// split, that mutant no longer *stops* the spinner — the re-cascade
/// re-registers what was cleared — it restarts it, with a clock 400ms younger,
/// which is why this fixture asserts the timestamp rather than merely that
/// something is running. The stop-for-good symptom needed both halves of #762
/// at once, and is what `main` did.
#[test]
fn a_theme_restyle_keeps_the_loader_spinning_without_restarting_it() {
    let mut app = mount_loader();
    let oval = node_with_class(&app, OVAL);
    assert_eq!(
        running(&app, oval),
        1,
        "precondition: the loader is spinning"
    );

    let expected_start = backdate(&mut app, oval, 400.0);

    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.update_theme_variables(":root { --rinch-primary-color: #123456; }");
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        running(&app, oval),
        1,
        "a theme change must not stop the app's spinners"
    );
    assert_eq!(
        start_time(&app, oval),
        expected_start,
        "…and must not restart them: Chrome 150 leaves a running animation's \
         currentTime alone across a stylesheet swap"
    );
}

/// The same thing through the branch itself: change the app's theme CSS and let
/// `resolve_and_repaint` notice.
///
/// Only compiled with the `theme` feature, which `cargo test -p rinch` does not
/// turn on — run it with `--features theme` (the workspace build unifies it on).
/// The un-gated fixture above is what guards the mechanism in the default build.
#[cfg(feature = "theme")]
#[test]
fn a_real_dark_mode_toggle_keeps_the_loader_spinning() {
    let mut app = mount_loader();
    // Take the document's theme off the thread-global slot, so this test cannot
    // be perturbed by (or perturb) anything else on the thread.
    app.set_owned_theme_css(Some(
        ":root { --rinch-primary-color: #4dabf7; }".to_string(),
    ));
    app.last_theme_key = Some(app.theme_key());

    let oval = node_with_class(&app, OVAL);
    assert_eq!(
        running(&app, oval),
        1,
        "precondition: the loader is spinning"
    );
    let expected_start = backdate(&mut app, oval, 400.0);

    // "Dark mode on": a different theme sheet. `resolve_and_repaint` compares it
    // against `last_theme_key` and takes the full-restyle branch by itself.
    let light = app.theme_key();
    app.set_owned_theme_css(Some(
        ":root { --rinch-primary-color: #1864ab; }".to_string(),
    ));
    assert_ne!(app.theme_key(), light, "precondition: the theme key moved");
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        app.last_theme_key,
        Some(app.theme_key()),
        "precondition: `resolve_and_repaint` really did take the theme-change branch"
    );
    assert_eq!(
        running(&app, oval),
        1,
        "toggling dark mode must not kill every animation in the app"
    );
    assert_eq!(start_time(&app, oval), expected_start, "…nor restart them");
}

/// The **thread-global** theme path: a document that follows the global slot
/// (desktop, web, Android — anything that is not an embed context) restyles
/// when `set_current_theme_css` changes it, which is what a `ThemeProvider`
/// dark-mode toggle does. `resolve_and_repaint` notices through the slot's
/// generation (`ThemeKey::Global`), not a string compare; the fixture above
/// covers only the owned (embed) arm.
#[cfg(feature = "theme")]
#[test]
fn a_thread_global_theme_change_restyles_a_document_that_follows_it() {
    // Thread-local slot: this test's thread owns it for the duration.
    rinch_core::set_current_theme_css(Some(":root { --probe-c: rgb(255, 0, 0); }".into()));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let probe = scope.create_element("div");
        probe.set_attribute("class", "theme-probe");
        probe.set_attribute("style", "color: var(--probe-c)");
        root.append_child(&probe);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let probe = node_with_class(&app, "theme-probe");
    let color = |app: &RinchApp| {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .get(probe)
            .unwrap()
            .computed_style
            .color
            .map(|c| c.to_rgba8())
            .map(|c| (c.r, c.g, c.b))
    };
    assert_eq!(color(&app), Some((255, 0, 0)), "precondition");

    rinch_core::set_current_theme_css(Some(":root { --probe-c: rgb(0, 0, 255); }".into()));
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(color(&app), Some((0, 0, 255)));

    rinch_core::set_current_theme_css(None);
}
