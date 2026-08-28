//! Test runner - orchestrates visual regression testing.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::browser::{BrowserCapture, BrowserError};
use crate::capture::{CaptureError, RinchCapture};
use crate::compare::{compare_images, CompareError};
use crate::html_serializer::{serialize_to_html, HtmlConfig};

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

/// Test definition loaded from JSON.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestDefinition {
    /// Unique test name.
    pub name: String,
    /// Section index to navigate to (optional).
    #[serde(default)]
    pub section: Option<usize>,
    /// Click coordinates to set up state (optional).
    #[serde(default)]
    pub setup_clicks: Vec<(f64, f64)>,
    /// SSIM threshold (default: 0.99).
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

fn default_threshold() -> f64 {
    0.99
}

/// Test configuration loaded from tests.json.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestConfig {
    /// Viewport dimensions.
    #[serde(default = "default_viewport")]
    pub viewport: (u32, u32),
    /// Background color.
    #[serde(default = "default_background")]
    pub background: String,
    /// Test definitions.
    pub tests: Vec<TestDefinition>,
}

fn default_viewport() -> (u32, u32) {
    (800, 600)
}

fn default_background() -> String {
    "#1a1a1a".to_string()
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
        serde_json::from_str(&content).map_err(|e| {
            RunnerError::ConfigError(format!("Failed to parse {}: {}", path.display(), e))
        })
    }

    /// Run all tests in the configuration.
    pub fn run_all(&self, config: &TestConfig) -> Vec<TestResult> {
        // Ensure output directory exists
        if let Err(e) = std::fs::create_dir_all(&self.output_dir) {
            return vec![TestResult {
                name: "setup".to_string(),
                passed: false,
                ssim_score: 0.0,
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

        let mut results = Vec::new();

        for test in &config.tests {
            let result = self.run_test(test, &mut rinch, &browser, &html_config, config.viewport);
            results.push(result);
        }

        results
    }

    /// Run a single test.
    fn run_test(
        &self,
        test: &TestDefinition,
        rinch: &mut RinchCapture,
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
            actual_path: actual_path.clone(),
            expected_path: expected_path.clone(),
            diff_path: None,
            html_path: html_path.clone(),
            error: Some(error),
        };

        // Run setup clicks if any
        for (x, y) in &test.setup_clicks {
            if let Err(e) = rinch.click(*x, *y) {
                return error_result(format!("Setup click failed: {}", e));
            }
            if let Err(e) = rinch.wait_frame() {
                return error_result(format!("Wait frame failed: {}", e));
            }
        }

        // Wait for render to stabilize
        if let Err(e) = rinch.wait_frame() {
            return error_result(format!("Wait frame failed: {}", e));
        }

        // Capture rinch screenshot
        let actual_png = match rinch.screenshot() {
            Ok(png) => png,
            Err(e) => return error_result(format!("Screenshot failed: {}", e)),
        };

        // Save actual screenshot
        if let Err(e) = std::fs::write(&actual_path, &actual_png) {
            return error_result(format!("Failed to save actual: {}", e));
        }

        // Get DOM tree
        let dom = match rinch.dom_tree() {
            Ok(d) => d,
            Err(e) => return error_result(format!("DOM tree failed: {}", e)),
        };

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
            "tests": [
                {"name": "buttons", "threshold": 0.99},
                {"name": "typography", "section": 4}
            ]
        }"##;

        let config: TestConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.viewport, (800, 600));
        assert_eq!(config.tests.len(), 2);
        assert_eq!(config.tests[0].name, "buttons");
        assert_eq!(config.tests[1].section, Some(4));
    }

    #[test]
    fn test_default_threshold() {
        let json = r#"{"name": "test"}"#;
        let test: TestDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(test.threshold, 0.99);
    }
}
