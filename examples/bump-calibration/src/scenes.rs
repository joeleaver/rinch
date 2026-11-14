//! The scenes the calibration renders: a deliberately curve-heavy one, and
//! the sections of ui-zoo, rinch's component showcase.

use rinch::prelude::*;
use std::cell::Cell;
use ui_zoo::Section;

/// A scene the calibration renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// [`curves`]: one step.
    Curves,
    /// ui-zoo's sections, one step each, but for those that use media or the
    /// network ([`Section::uses_media_or_network`]): the calibration runs
    /// headless and touches no device but the GPU.
    UiZoo,
}

thread_local! {
    /// The section ui-zoo shows, set at each step.
    static SECTION: Cell<Option<Signal<usize>>> = const { Cell::new(None) };
}

impl Scene {
    /// Every scene, in the order a run with no `--scene` renders them.
    pub const ALL: [Scene; 2] = [Scene::Curves, Scene::UiZoo];

    /// Its name on the command line and in the report.
    pub fn name(self) -> &'static str {
        match self {
            Scene::Curves => "curves",
            Scene::UiZoo => "ui-zoo",
        }
    }

    /// The scene named `name`.
    pub fn named(name: &str) -> Option<Scene> {
        Self::ALL.into_iter().find(|scene| scene.name() == name)
    }

    /// How many steps it has.
    pub fn steps(self) -> usize {
        match self {
            Scene::Curves => 1,
            Scene::UiZoo => ui_zoo_sections().count(),
        }
    }

    /// Its content at `width` x `height`, showing its first step.
    pub fn build(self, scope: &mut RenderScope, width: u32, height: u32) -> NodeHandle {
        match self {
            Scene::Curves => curves(scope, width, height),
            Scene::UiZoo => {
                ui_zoo::init_all_sections();
                let first = ui_zoo_sections().next().expect("a ui-zoo section");
                let section = Signal::new(first.index());
                SECTION.with(|s| s.set(Some(section)));
                ui_zoo::section_content(scope, section)
            }
        }
    }

    /// Shows step `step` (of [`Self::steps`]) in the content [`Self::build`]
    /// made.
    pub fn show_step(self, step: usize) {
        match self {
            Scene::Curves => {}
            Scene::UiZoo => {
                let section = SECTION
                    .with(Cell::get)
                    .expect("ui-zoo is built before its steps");
                let shown = ui_zoo_sections().nth(step).expect("a step of ui-zoo");
                section.set(shown.index());
            }
        }
    }

    /// Done with the content [`Self::build`] made.
    pub fn finish(self) {
        SECTION.with(|s| s.set(None));
    }
}

/// ui-zoo's sections the calibration renders, in ui-zoo's order.
fn ui_zoo_sections() -> impl Iterator<Item = Section> {
    Section::ALL
        .iter()
        .copied()
        .filter(|section| !section.uses_media_or_network())
}

/// Large filled and stroked circles, wide round strokes along cubics, and
/// target-high diagonals, drawn in a 1280x720 view box scaled to
/// `width` x `height`: the scene whose lines and segments per path tag set
/// those rules (#2466).
fn curves(scope: &mut RenderScope, width: u32, height: u32) -> NodeHandle {
    let el = |scope: &mut RenderScope, tag: &str, attrs: &[(&str, String)]| {
        let node = scope.create_element(tag);
        for (k, v) in attrs {
            node.set_attribute(k, v);
        }
        node
    };
    let root = el(
        scope,
        "div",
        &[("style", "margin: 0; padding: 0; overflow: hidden".into())],
    );
    let svg = el(
        scope,
        "svg",
        &[
            ("width", width.to_string()),
            ("height", height.to_string()),
            ("viewBox", "0 0 1280 720".into()),
            (
                "style",
                format!("width: {width}px; height: {height}px; display: block"),
            ),
        ],
    );
    for (i, r) in [340, 260, 200, 150, 110, 80].into_iter().enumerate() {
        let attrs = [
            ("cx", (200 + i * 170).to_string()),
            ("cy", "360".into()),
            ("r", r.to_string()),
            ("fill", format!("rgba({}, 90, 200, 0.25)", 40 * i)),
        ];
        svg.append_child(&el(scope, "circle", &attrs));
    }
    for (i, r) in [300, 220, 140].into_iter().enumerate() {
        let attrs = [
            ("cx", (640 + i * 40).to_string()),
            ("cy", "360".into()),
            ("r", r.to_string()),
            ("fill", "none".into()),
            ("stroke", "#c04020".into()),
            ("stroke-width", "30".into()),
        ];
        svg.append_child(&el(scope, "circle", &attrs));
    }
    for i in 0..20_i64 {
        let y = 20 + i * 34;
        let attrs = [
            (
                "d",
                format!("M 10 {y} C 300 {} 900 {} 1270 {y}", y - 300, y + 300),
            ),
            ("fill", "none".into()),
            ("stroke", "#2080c0".into()),
            ("stroke-width", "24".into()),
            ("stroke-linecap", "round".into()),
            ("stroke-linejoin", "round".into()),
        ];
        svg.append_child(&el(scope, "path", &attrs));
    }
    for i in 0..40 {
        let x = i * 32;
        let attrs = [
            ("x1", x.to_string()),
            ("y1", "0".into()),
            ("x2", (1280 - x).to_string()),
            ("y2", "720".into()),
            ("stroke", "#208040".into()),
            ("stroke-width", if i % 2 == 0 { "1" } else { "12" }.into()),
        ];
        svg.append_child(&el(scope, "line", &attrs));
    }
    root.append_child(&svg);
    root
}
