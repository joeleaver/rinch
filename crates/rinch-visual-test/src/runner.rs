//! Test runner - orchestrates visual regression testing.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::browser::{BrowserCapture, BrowserError};
use crate::capture::{CaptureError, RinchCapture};
use crate::compare::{CompareError, compare_images};
use crate::html_serializer::{HtmlConfig, serialize_to_html};

#[derive(Error, Debug)]
pub enum RunnerError {
    #[error("Failed to load test config: {0}")]
    ConfigError(String),

    #[error("Rinch capture error: {0}")]
    CaptureError(#[from] CaptureError),

    #[error("Browser error: {0}")]
    BrowserError(#[from] BrowserError),

    #[error("Comparison error: {0}")]
    CompareError(#[from] CompareError),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Where a step points: the one node that matches `selector` and, when given,
/// whose trimmed text content is exactly `text`.
///
/// Targets are resolved through the debug protocol's `query_selector` at the
/// moment the step runs and clicked at the node's reported `absolute` centre.
/// They used to be hard-coded pixels, which is what went stale (#368): the
/// coordinates silently stopped reaching the nav items they named.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// A selector `query_selector` understands: `tag`, `.class`, `[attr]` or
    /// `[attr=value]`.
    pub selector: String,
    /// Exact (trimmed) text content the node must have.
    #[serde(default)]
    pub text: Option<String>,
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.text {
            Some(text) => write!(f, "{} with text {:?}", self.selector, text),
            None => write!(f, "{}", self.selector),
        }
    }
}

/// One action that drives the app towards the state a scenario captures.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// Click the centre of the single visible node matching the target.
    /// Zero or several matches is an error, never a guess.
    Click(Target),
    /// Press one key (a `key_press` name such as `"Escape"`).
    Key(String),
}

/// Test definition loaded from JSON.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TestDefinition {
    /// Unique test name.
    pub name: String,
    /// Steps that navigate from any screen to the one this scenario captures.
    #[serde(default)]
    pub steps: Vec<Step>,
    /// What must be on screen before the capture is taken. Required, and
    /// checked every run: a scenario that asserts nothing about its state
    /// captures whatever the previous one left behind (#368).
    #[serde(default)]
    pub expect: Vec<Target>,
    /// SSIM threshold (default: 0.99).
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

fn default_threshold() -> f64 {
    0.99
}

/// Test configuration loaded from tests.json.
///
/// Unknown keys are refused rather than ignored, so a config still written
/// against the old `setup_clicks` / `section` fields fails to load instead of
/// running every scenario with no navigation at all.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TestConfig {
    /// Viewport dimensions.
    #[serde(default = "default_viewport")]
    pub viewport: (u32, u32),
    /// Background color.
    #[serde(default = "default_background")]
    pub background: String,
    /// Steps run before every scenario's own, to put the app back in a
    /// neutral state (e.g. `Escape` to close an overlay a scenario opened).
    #[serde(default)]
    pub before_each: Vec<Step>,
    /// Milliseconds to let transitions finish after each step.
    #[serde(default = "default_settle_ms")]
    pub settle_ms: u64,
    /// Test definitions.
    pub tests: Vec<TestDefinition>,
}

fn default_viewport() -> (u32, u32) {
    (800, 600)
}

fn default_background() -> String {
    "#1a1a1a".to_string()
}

fn default_settle_ms() -> u64 {
    500
}

impl TestConfig {
    /// Refuse a config whose scenarios cannot be told apart or do not say
    /// what they capture.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for test in &self.tests {
            if !seen.insert(test.name.as_str()) {
                return Err(format!("duplicate test name {:?}", test.name));
            }
            if test.expect.is_empty() {
                return Err(format!(
                    "test {:?} has no `expect`: it would capture whatever screen \
                     the app happens to be showing",
                    test.name
                ));
            }
        }
        Ok(())
    }
}

/// A node as `query_selector` reports it: its text and on-screen box, in the
/// logical pixels the input commands take.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeMatch {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The app side of the harness: what the runner needs to drive and capture a
/// screen. [`RinchCapture`] is the real one; tests substitute a model.
pub trait AppDriver {
    fn query(&mut self, selector: &str) -> Result<Vec<NodeMatch>, CaptureError>;
    fn click(&mut self, x: f64, y: f64) -> Result<(), CaptureError>;
    fn key(&mut self, key: &str) -> Result<(), CaptureError>;
    /// Let `ms` of wall-clock time pass (transitions run on it), then render.
    fn settle(&mut self, ms: u64) -> Result<(), CaptureError>;
    fn screenshot(&mut self) -> Result<Vec<u8>, CaptureError>;
    fn dom_tree(&mut self) -> Result<serde_json::Value, CaptureError>;
}

impl AppDriver for RinchCapture {
    fn query(&mut self, selector: &str) -> Result<Vec<NodeMatch>, CaptureError> {
        RinchCapture::query_selector(self, selector)
    }
    fn click(&mut self, x: f64, y: f64) -> Result<(), CaptureError> {
        RinchCapture::click(self, x, y)
    }
    fn key(&mut self, key: &str) -> Result<(), CaptureError> {
        RinchCapture::key_press(self, key)
    }
    fn settle(&mut self, ms: u64) -> Result<(), CaptureError> {
        self.wait_frame()?;
        std::thread::sleep(std::time::Duration::from_millis(ms));
        self.wait_frame()
    }
    fn screenshot(&mut self) -> Result<Vec<u8>, CaptureError> {
        RinchCapture::screenshot(self)
    }
    fn dom_tree(&mut self) -> Result<serde_json::Value, CaptureError> {
        RinchCapture::dom_tree(self)
    }
}

fn matching(driver: &mut dyn AppDriver, target: &Target) -> Result<Vec<NodeMatch>, String> {
    let nodes = driver
        .query(&target.selector)
        .map_err(|e| format!("query {} failed: {}", target, e))?;
    Ok(nodes
        .into_iter()
        .filter(|n| match &target.text {
            Some(text) => n.text.trim() == text,
            None => true,
        })
        .collect())
}

fn run_step(driver: &mut dyn AppDriver, step: &Step, settle_ms: u64) -> Result<(), String> {
    match step {
        Step::Click(target) => {
            // A laid-out but hidden or collapsed node still answers the query;
            // only a node with an area can be what the step means to click.
            let visible: Vec<_> = matching(driver, target)?
                .into_iter()
                .filter(|n| n.width > 0.0 && n.height > 0.0)
                .collect();
            let node = match visible.as_slice() {
                [one] => one,
                [] => return Err(format!("click: no visible node matches {}", target)),
                many => {
                    return Err(format!(
                        "click: {} visible nodes match {}; the target must be unique",
                        many.len(),
                        target
                    ));
                }
            };
            driver
                .click(node.x + node.width / 2.0, node.y + node.height / 2.0)
                .map_err(|e| format!("click on {} failed: {}", target, e))?;
        }
        Step::Key(key) => driver
            .key(key)
            .map_err(|e| format!("key {:?} failed: {}", key, e))?,
    }
    driver
        .settle(settle_ms)
        .map_err(|e| format!("settle failed: {}", e))
}

/// What a scenario captured from the app.
#[derive(Debug, Clone)]
pub struct Capture {
    /// The app's own screenshot, PNG.
    pub png: Vec<u8>,
    /// The verbose DOM tree the browser reference is exported from.
    pub dom: serde_json::Value,
}

/// Drive the app to one scenario's screen, check it is that screen, and
/// capture it. Depends on nothing the previous scenario left behind beyond
/// what `before_each` and the steps undo.
pub fn capture_scenario(
    driver: &mut dyn AppDriver,
    config: &TestConfig,
    test: &TestDefinition,
) -> Result<Capture, String> {
    for step in config.before_each.iter().chain(&test.steps) {
        run_step(driver, step, config.settle_ms)?;
    }
    for target in &test.expect {
        if matching(driver, target)?.is_empty() {
            return Err(format!(
                "expected {} on screen before capturing; none found",
                target
            ));
        }
    }
    let png = driver
        .screenshot()
        .map_err(|e| format!("Screenshot failed: {}", e))?;
    let dom = driver
        .dom_tree()
        .map_err(|e| format!("DOM tree failed: {}", e))?;
    Ok(Capture { png, dom })
}

/// Capture every scenario in order. A scenario whose screenshot is
/// byte-identical to an earlier one's is an error: two names measuring one
/// screen is a coverage hole, not two results (#368).
pub fn capture_all(
    driver: &mut dyn AppDriver,
    config: &TestConfig,
) -> Vec<(String, Result<Capture, String>)> {
    let mut out: Vec<(String, Result<Capture, String>)> = Vec::new();
    for test in &config.tests {
        let mut result = capture_scenario(driver, config, test);
        if let Ok(capture) = &result
            && let Some((earlier, _)) = out
                .iter()
                .find(|(_, r)| matches!(r, Ok(c) if c.png == capture.png))
        {
            result = Err(format!(
                "captured exactly the same screen as {:?}; the scenario does not \
                 reach the state it names",
                earlier
            ));
        }
        out.push((test.name.clone(), result));
    }
    out
}

/// Result of running a single test.
#[derive(Debug)]
pub struct TestResult {
    /// Test name.
    pub name: String,
    /// Whether the test passed.
    pub passed: bool,
    /// SSIM score.
    pub ssim_score: f64,
    /// The threshold this result was judged against (from the test definition).
    pub threshold: f64,
    /// Path to actual (rinch) screenshot.
    pub actual_path: PathBuf,
    /// Path to expected (browser) screenshot.
    pub expected_path: PathBuf,
    /// Path to diff image (if failed).
    pub diff_path: Option<PathBuf>,
    /// Path to exported HTML.
    pub html_path: PathBuf,
    /// Error message if test errored.
    pub error: Option<String>,
}

/// Visual test runner.
pub struct TestRunner {
    /// Path to scripts directory (contains playwright_capture.js).
    scripts_dir: PathBuf,
    /// Output directory for test artifacts.
    output_dir: PathBuf,
    /// Baselines directory.
    baselines_dir: PathBuf,
    /// Whether to update baselines.
    update_baselines: bool,
}

impl TestRunner {
    /// Create a new test runner.
    pub fn new(
        scripts_dir: PathBuf,
        output_dir: PathBuf,
        baselines_dir: PathBuf,
        update_baselines: bool,
    ) -> Self {
        Self {
            scripts_dir,
            output_dir,
            baselines_dir,
            update_baselines,
        }
    }

    /// Load test configuration from a JSON file.
    pub fn load_config(path: &Path) -> Result<TestConfig, RunnerError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            RunnerError::ConfigError(format!("Failed to read {}: {}", path.display(), e))
        })?;
        let config: TestConfig = serde_json::from_str(&content).map_err(|e| {
            RunnerError::ConfigError(format!("Failed to parse {}: {}", path.display(), e))
        })?;
        config
            .validate()
            .map_err(|e| RunnerError::ConfigError(format!("{}: {}", path.display(), e)))?;
        Ok(config)
    }

    /// Run all tests in the configuration.
    pub fn run_all(&self, config: &TestConfig) -> Vec<TestResult> {
        // Ensure output directory exists
        if let Err(e) = std::fs::create_dir_all(&self.output_dir) {
            return vec![TestResult {
                name: "setup".to_string(),
                passed: false,
                ssim_score: 0.0,
                threshold: 0.0,
                actual_path: PathBuf::new(),
                expected_path: PathBuf::new(),
                diff_path: None,
                html_path: PathBuf::new(),
                error: Some(format!("Failed to create output dir: {}", e)),
            }];
        }

        // Connect to rinch app
        let mut rinch = match RinchCapture::connect() {
            Ok(r) => r,
            Err(e) => {
                return vec![TestResult {
                    name: "connect".to_string(),
                    passed: false,
                    ssim_score: 0.0,
                    threshold: 0.0,
                    actual_path: PathBuf::new(),
                    expected_path: PathBuf::new(),
                    diff_path: None,
                    html_path: PathBuf::new(),
                    error: Some(format!("Failed to connect to rinch: {}", e)),
                }];
            }
        };

        // Create browser capture
        let browser = match BrowserCapture::new(&self.scripts_dir) {
            Ok(b) => b,
            Err(e) => {
                return vec![TestResult {
                    name: "browser_setup".to_string(),
                    passed: false,
                    ssim_score: 0.0,
                    threshold: 0.0,
                    actual_path: PathBuf::new(),
                    expected_path: PathBuf::new(),
                    diff_path: None,
                    html_path: PathBuf::new(),
                    error: Some(format!("Failed to setup browser: {}", e)),
                }];
            }
        };

        let html_config = HtmlConfig {
            viewport_width: config.viewport.0,
            viewport_height: config.viewport.1,
            background_color: config.background.clone(),
        };

        let captures = capture_all(&mut rinch, config);

        config
            .tests
            .iter()
            .zip(captures)
            .map(|(test, (_, capture))| {
                self.run_test(test, capture, &browser, &html_config, config.viewport)
            })
            .collect()
    }

    /// Compare one scenario's capture against the browser's rendering of it.
    fn run_test(
        &self,
        test: &TestDefinition,
        capture: Result<Capture, String>,
        browser: &BrowserCapture,
        html_config: &HtmlConfig,
        viewport: (u32, u32),
    ) -> TestResult {
        let actual_path = self.output_dir.join(format!("{}_actual.png", test.name));
        let expected_path = self.output_dir.join(format!("{}_expected.png", test.name));
        let diff_path = self.output_dir.join(format!("{}_diff.png", test.name));
        let html_path = self.output_dir.join(format!("{}.html", test.name));

        // Helper to create error result
        let error_result = |error: String| TestResult {
            name: test.name.clone(),
            passed: false,
            ssim_score: 0.0,
            threshold: test.threshold,
            actual_path: actual_path.clone(),
            expected_path: expected_path.clone(),
            diff_path: None,
            html_path: html_path.clone(),
            error: Some(error),
        };

        let Capture {
            png: actual_png,
            dom,
        } = match capture {
            Ok(c) => c,
            Err(e) => return error_result(e),
        };

        // Save actual screenshot
        if let Err(e) = std::fs::write(&actual_path, &actual_png) {
            return error_result(format!("Failed to save actual: {}", e));
        }

        // Serialize to HTML
        let html = serialize_to_html(&dom, html_config);
        if let Err(e) = std::fs::write(&html_path, &html) {
            return error_result(format!("Failed to save HTML: {}", e));
        }

        // Capture browser screenshot
        let expected_png = match browser.capture_html(&html, viewport.0, viewport.1) {
            Ok(png) => png,
            Err(e) => return error_result(format!("Browser capture failed: {}", e)),
        };

        // Save expected screenshot
        if let Err(e) = std::fs::write(&expected_path, &expected_png) {
            return error_result(format!("Failed to save expected: {}", e));
        }

        // Compare images
        let comparison = match compare_images(&actual_png, &expected_png, test.threshold) {
            Ok(c) => c,
            Err(e) => return error_result(format!("Comparison failed: {}", e)),
        };

        // Save diff image if failed
        let diff_path_result = if !comparison.passed {
            if let Some(ref diff_img) = comparison.diff_image {
                if diff_img.save(&diff_path).is_ok() {
                    Some(diff_path.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Update baseline if requested and test passed
        if self.update_baselines && comparison.passed {
            let baseline_path = self
                .baselines_dir
                .join(format!("{}_baseline.png", test.name));
            let _ = std::fs::create_dir_all(&self.baselines_dir);
            let _ = std::fs::copy(&expected_path, &baseline_path);
        }

        TestResult {
            name: test.name.clone(),
            passed: comparison.passed,
            ssim_score: comparison.ssim_score,
            threshold: test.threshold,
            actual_path,
            expected_path,
            diff_path: diff_path_result,
            html_path,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parsing() {
        let json = r##"{
            "viewport": [800, 600],
            "background": "#1a1a1a",
            "before_each": [{"key": "Escape"}],
            "tests": [
                {"name": "buttons", "threshold": 0.99,
                 "steps": [{"click": {"selector": ".nav", "text": "Buttons"}}],
                 "expect": [{"selector": ".active"}]}
            ]
        }"##;

        let config: TestConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.viewport, (800, 600));
        assert_eq!(config.settle_ms, 500);
        assert_eq!(config.before_each, vec![Step::Key("Escape".into())]);
        assert_eq!(config.tests.len(), 1);
        assert_eq!(config.tests[0].name, "buttons");
        assert_eq!(
            config.tests[0].steps,
            vec![Step::Click(Target {
                selector: ".nav".into(),
                text: Some("Buttons".into()),
            })]
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_default_threshold() {
        let json = r#"{"name": "test"}"#;
        let test: TestDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(test.threshold, 0.99);
    }
}
