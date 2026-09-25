//! Scenario navigation, idempotency and distinctness (#368).
//!
//! The suite used to navigate by hard-coded pixels, and `00_overview`
//! navigated not at all: it photographed whatever screen the long-lived app
//! was showing, so a second run against the same instance captured the
//! editor the previous run ended on. The coordinates had also gone stale, so
//! the 12 scenarios captured only 8 distinct screens.
//!
//! These fixtures drive the runner's capture phase against a model of the UI
//! Zoo shell — a titlebar toggle opening a drawer of nav links, each of which
//! selects a section and closes the drawer, plus a modal the Overlays section
//! can open — and check the shipped `tests.json` against it. The model
//! answers the same selectors the real app does, so a config that stops
//! naming real nodes fails here before it fails on a live app.

use std::path::PathBuf;

use rinch_visual_test::capture::CaptureError;
use rinch_visual_test::runner::{
    AppDriver, NodeMatch, Step, Target, TestConfig, TestDefinition, TestRunner, capture_all,
};

/// The UI Zoo drawer's nav labels, in order.
const SECTIONS: &[&str] = &[
    "Overview",
    "Buttons",
    "Inputs",
    "Typography",
    "Layout",
    "Navigation",
    "Data Display",
    "Feedback",
    "Overlays",
    "Icons",
    "Tree",
    "Rich Text Editor",
    "CSS Features",
];

const OVERLAYS: usize = 8;

fn b(x: f64, y: f64, w: f64, h: f64, text: &str) -> NodeMatch {
    NodeMatch {
        text: text.to_string(),
        x,
        y,
        width: w,
        height: h,
    }
}

fn inside(n: &NodeMatch, x: f64, y: f64) -> bool {
    x >= n.x && x < n.x + n.width && y >= n.y && y < n.y + n.height
}

/// A model of the UI Zoo desktop shell. It starts wherever it is told to,
/// which is the point: a suite must not depend on where it starts.
struct FakeZoo {
    section: usize,
    drawer_open: bool,
    modal_open: bool,
}

impl FakeZoo {
    fn at(section: usize) -> Self {
        Self {
            section,
            drawer_open: false,
            modal_open: false,
        }
    }

    fn toggle(&self) -> NodeMatch {
        b(10.0, 4.0, 28.0, 28.0, "")
    }

    /// A closed drawer's links keep their full size and are translated off
    /// the left edge, as the real app reports them (x = -260).
    fn link(&self, i: usize) -> NodeMatch {
        let x = if self.drawer_open { 0.0 } else { -260.0 };
        b(x, 60.0 + i as f64 * 40.0, 252.0, 36.0, SECTIONS[i])
    }

    fn open_modal_button(&self) -> NodeMatch {
        let size = if self.section == OVERLAYS && !self.drawer_open {
            40.0
        } else {
            0.0
        };
        b(400.0, 300.0, size * 3.0, size, "Open Modal")
    }
}

impl AppDriver for FakeZoo {
    fn query(&mut self, selector: &str) -> Result<Vec<NodeMatch>, CaptureError> {
        Ok(match selector {
            ".ui-zoo-nav-toggle" => vec![self.toggle()],
            ".rinch-navlink" => (0..SECTIONS.len()).map(|i| self.link(i)).collect(),
            ".rinch-navlink--active" => vec![self.link(self.section)],
            "button" => vec![self.toggle(), self.open_modal_button()],
            _ => Vec::new(),
        })
    }

    fn click(&mut self, x: f64, y: f64) -> Result<(), CaptureError> {
        if self.modal_open {
            // The modal's backdrop covers everything, the toggle included.
            self.modal_open = false;
        } else if inside(&self.toggle(), x, y) {
            self.drawer_open = !self.drawer_open;
        } else if self.drawer_open {
            if let Some(i) = (0..SECTIONS.len()).find(|&i| inside(&self.link(i), x, y)) {
                self.section = i;
            }
            self.drawer_open = false;
        } else if inside(&self.open_modal_button(), x, y) {
            self.modal_open = true;
        }
        Ok(())
    }

    fn key(&mut self, key: &str) -> Result<(), CaptureError> {
        if key == "Escape" {
            if self.modal_open {
                self.modal_open = false;
            } else {
                self.drawer_open = false;
            }
        }
        Ok(())
    }

    fn settle(&mut self, _ms: u64) -> Result<(), CaptureError> {
        Ok(())
    }

    fn screenshot(&mut self) -> Result<Vec<u8>, CaptureError> {
        Ok(format!(
            "section={} drawer={} modal={}",
            SECTIONS[self.section], self.drawer_open, self.modal_open
        )
        .into_bytes())
    }

    fn dom_tree(&mut self) -> Result<serde_json::Value, CaptureError> {
        Ok(serde_json::json!({}))
    }
}

fn shipped_config() -> TestConfig {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/visual/tests.json");
    TestRunner::load_config(&path).expect("the shipped tests.json loads")
}

fn screen(png: &[u8]) -> String {
    String::from_utf8(png.to_vec()).unwrap()
}

fn nav_to(name: &str, label: &str) -> TestDefinition {
    TestDefinition {
        name: name.to_string(),
        steps: vec![
            Step::Click(Target {
                selector: ".ui-zoo-nav-toggle".into(),
                text: None,
            }),
            Step::Click(Target {
                selector: ".rinch-navlink".into(),
                text: Some(label.into()),
            }),
        ],
        expect: vec![Target {
            selector: ".rinch-navlink--active".into(),
            text: Some(label.into()),
        }],
        threshold: 0.9,
    }
}

fn config(before_each: Vec<Step>, tests: Vec<TestDefinition>) -> TestConfig {
    TestConfig {
        viewport: (1200, 800),
        background: "#ffffff".into(),
        before_each,
        settle_ms: 0,
        tests,
    }
}

/// Every scenario of the shipped suite reaches the section it is named for,
/// and no two capture the same screen. At the old coordinates the real app
/// collapsed 12 scenarios onto 8 screens.
#[test]
fn the_shipped_suite_captures_the_screen_each_scenario_names() {
    let config = shipped_config();
    assert_eq!(config.tests.len(), 12);
    let mut app = FakeZoo::at(0);
    let captures = capture_all(&mut app, &config);
    let mut seen = std::collections::HashSet::new();
    for (test, (name, capture)) in config.tests.iter().zip(&captures) {
        let capture = capture
            .as_ref()
            .unwrap_or_else(|e| panic!("{name} failed: {e}"));
        let label = test.expect[0].text.as_deref().unwrap();
        assert_eq!(
            screen(&capture.png),
            format!("section={label} drawer=false modal=false"),
            "{name}"
        );
        assert!(seen.insert(capture.png.clone()), "{name} repeats a screen");
    }
    // Section `n` of the nav is scenario `n`: the names are not decoration.
    for (i, test) in config.tests.iter().enumerate() {
        assert_eq!(test.expect[0].text.as_deref(), Some(SECTIONS[i]));
    }
}

/// Re-running the suite against the same app instance captures the same
/// screens, whatever the first run left behind — and so does starting on a
/// section no scenario visits. The first scenario used to capture whatever was
/// showing (the editor, after a previous run).
#[test]
fn rerunning_the_suite_on_one_app_captures_the_same_screens() {
    let config = shipped_config();
    let mut app = FakeZoo::at(0);
    let first: Vec<_> = capture_all(&mut app, &config)
        .into_iter()
        .map(|(_, c)| screen(&c.unwrap().png))
        .collect();
    assert_ne!(app.section, 0, "the first run must end away from its start");
    let second: Vec<_> = capture_all(&mut app, &config)
        .into_iter()
        .map(|(_, c)| screen(&c.unwrap().png))
        .collect();
    assert_eq!(first, second);

    let mut elsewhere = FakeZoo::at(12);
    elsewhere.drawer_open = true;
    let third: Vec<_> = capture_all(&mut elsewhere, &config)
        .into_iter()
        .map(|(_, c)| screen(&c.unwrap().png))
        .collect();
    assert_eq!(first, third);
}

/// Two scenarios that land on one screen are a coverage hole, and the runner
/// says so rather than reporting two scores.
#[test]
fn a_scenario_that_repeats_an_earlier_screen_fails() {
    let mut second = nav_to("b", "Buttons");
    second.name = "c".into();
    let config = config(
        vec![],
        vec![nav_to("a", "Overview"), nav_to("b", "Buttons"), second],
    );
    let captures = capture_all(&mut FakeZoo::at(3), &config);
    assert!(captures[0].1.is_ok());
    assert!(captures[1].1.is_ok());
    let err = captures[2].1.as_ref().unwrap_err();
    assert!(
        err.contains("same screen") && err.contains("\"b\""),
        "{err}"
    );
}

/// A scenario that leaves an overlay open does not change what the next one
/// captures: `before_each` closes it. Without `before_each` the overlay
/// swallows the next scenario's first click, and the scenario fails loudly
/// instead of capturing the wrong screen.
#[test]
fn an_overlay_left_open_is_closed_before_the_next_scenario() {
    let mut opens_modal = nav_to("overlays", "Overlays");
    opens_modal.steps.push(Step::Click(Target {
        selector: "button".into(),
        text: Some("Open Modal".into()),
    }));
    let tests = vec![opens_modal, nav_to("buttons", "Buttons")];

    let with_reset = config(vec![Step::Key("Escape".into())], tests.clone());
    let captures = capture_all(&mut FakeZoo::at(0), &with_reset);
    assert_eq!(
        screen(&captures[0].1.as_ref().unwrap().png),
        "section=Overlays drawer=false modal=true"
    );
    assert_eq!(
        screen(&captures[1].1.as_ref().unwrap().png),
        "section=Buttons drawer=false modal=false"
    );

    let without = config(vec![], tests);
    let captures = capture_all(&mut FakeZoo::at(0), &without);
    let err = captures[1].1.as_ref().unwrap_err();
    assert!(err.contains("\"Buttons\""), "{err}");
}

/// A click target must name one visible node: several is an error, not a
/// click on the first.
#[test]
fn an_ambiguous_click_target_fails() {
    let mut test = nav_to("a", "Buttons");
    test.steps[1] = Step::Click(Target {
        selector: ".rinch-navlink".into(),
        text: None,
    });
    let captures = capture_all(&mut FakeZoo::at(0), &config(vec![], vec![test]));
    let err = captures[0].1.as_ref().unwrap_err();
    assert!(err.contains("13 visible nodes"), "{err}");
}

/// A scenario must say what it captures, and a config written against the
/// removed `setup_clicks` field must not load as a suite with no navigation.
#[test]
fn a_config_that_cannot_pin_its_screens_is_refused() {
    let mut no_expect = nav_to("a", "Buttons");
    no_expect.expect.clear();
    let err = config(vec![], vec![no_expect]).validate().unwrap_err();
    assert!(err.contains("no `expect`"), "{err}");

    let err = config(vec![], vec![nav_to("a", "Buttons"), nav_to("a", "Inputs")])
        .validate()
        .unwrap_err();
    assert!(err.contains("duplicate test name"), "{err}");

    let old = r#"{"tests": [{"name": "x", "setup_clicks": [[27, 17]]}]}"#;
    assert!(serde_json::from_str::<TestConfig>(old).is_err());
}

/// A scenario whose steps all run but leave the app on another screen fails
/// on its `expect` rather than capturing that other screen.
#[test]
fn an_expect_that_does_not_hold_fails_the_capture() {
    let mut test = nav_to("a", "Buttons");
    test.steps.clear();
    let captures = capture_all(&mut FakeZoo::at(0), &config(vec![], vec![test]));
    let err = captures[0].1.as_ref().unwrap_err();
    assert!(
        err.contains("expected .rinch-navlink--active with text \"Buttons\""),
        "{err}"
    );
}

/// Distinctness is checked against every earlier capture, not only the one
/// before: scenario 3 repeating scenario 1's screen fails too.
#[test]
fn a_scenario_that_repeats_a_non_adjacent_earlier_screen_fails() {
    let config = config(
        vec![],
        vec![
            nav_to("a", "Overview"),
            nav_to("b", "Buttons"),
            nav_to("c", "Overview"),
        ],
    );
    let captures = capture_all(&mut FakeZoo::at(3), &config);
    assert!(captures[0].1.is_ok());
    assert!(captures[1].1.is_ok());
    let err = captures[2].1.as_ref().unwrap_err();
    assert!(
        err.contains("same screen") && err.contains("\"a\""),
        "{err}"
    );
}

/// A closed drawer's links are full-size but off-screen; a click step does
/// not take one of them for its target.
#[test]
fn an_off_screen_node_is_not_a_click_target() {
    let mut test = nav_to("a", "Buttons");
    test.steps.remove(0); // never opens the drawer
    let captures = capture_all(&mut FakeZoo::at(0), &config(vec![], vec![test]));
    let err = captures[0].1.as_ref().unwrap_err();
    assert!(err.contains("no visible node matches"), "{err}");
}
