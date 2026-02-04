//! Browser capture - renders HTML via Playwright/Chromium and captures screenshots.

use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("Playwright script not found at {0}")]
    ScriptNotFound(PathBuf),

    #[error("Failed to write temp HTML file: {0}")]
    TempFileError(#[from] std::io::Error),

    #[error("Playwright process failed: {0}")]
    ProcessFailed(String),

    #[error("Failed to read screenshot: {0}")]
    ReadError(String),

    #[error("Node.js not found - please install Node.js")]
    NodeNotFound,
}

/// Browser screenshot capture using Playwright.
pub struct BrowserCapture {
    script_path: PathBuf,
    node_path: String,
}

impl BrowserCapture {
    /// Create a new browser capture instance.
    ///
    /// `script_dir` should point to the directory containing playwright_capture.js
    pub fn new(script_dir: &Path) -> Result<Self, BrowserError> {
        let script_path = script_dir.join("playwright_capture.js");
        if !script_path.exists() {
            return Err(BrowserError::ScriptNotFound(script_path));
        }

        // Check if node is available
        let node_path = which_node().ok_or(BrowserError::NodeNotFound)?;

        Ok(Self {
            script_path,
            node_path,
        })
    }

    /// Capture a screenshot of an HTML string.
    ///
    /// Returns PNG bytes.
    pub fn capture_html(&self, html: &str, width: u32, height: u32) -> Result<Vec<u8>, BrowserError> {
        // Create temp files
        let temp_dir = std::env::temp_dir();
        let html_path = temp_dir.join(format!("rinch_vtest_{}.html", std::process::id()));
        let png_path = temp_dir.join(format!("rinch_vtest_{}.png", std::process::id()));

        // Write HTML to temp file
        std::fs::write(&html_path, html)?;

        // Cleanup on drop
        let _cleanup = TempCleanup {
            paths: vec![html_path.clone(), png_path.clone()],
        };

        // Run Playwright
        let output = Command::new(&self.node_path)
            .arg(&self.script_path)
            .arg(&html_path)
            .arg(&png_path)
            .arg(width.to_string())
            .arg(height.to_string())
            .output()
            .map_err(|e| BrowserError::ProcessFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(BrowserError::ProcessFailed(format!(
                "Exit code: {:?}\nstderr: {}\nstdout: {}",
                output.status.code(),
                stderr,
                stdout
            )));
        }

        // Read PNG
        std::fs::read(&png_path)
            .map_err(|e| BrowserError::ReadError(e.to_string()))
    }

    /// Capture a screenshot of an HTML file.
    pub fn capture_html_file(&self, html_path: &Path, width: u32, height: u32) -> Result<Vec<u8>, BrowserError> {
        let temp_dir = std::env::temp_dir();
        let png_path = temp_dir.join(format!("rinch_vtest_{}.png", std::process::id()));

        let _cleanup = TempCleanup {
            paths: vec![png_path.clone()],
        };

        let output = Command::new(&self.node_path)
            .arg(&self.script_path)
            .arg(html_path)
            .arg(&png_path)
            .arg(width.to_string())
            .arg(height.to_string())
            .output()
            .map_err(|e| BrowserError::ProcessFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BrowserError::ProcessFailed(format!(
                "Playwright failed: {}",
                stderr
            )));
        }

        std::fs::read(&png_path)
            .map_err(|e| BrowserError::ReadError(e.to_string()))
    }
}

/// Find node executable path
fn which_node() -> Option<String> {
    // Try common paths
    for cmd in &["node", "nodejs"] {
        if Command::new(cmd).arg("--version").output().is_ok() {
            return Some(cmd.to_string());
        }
    }
    None
}

/// Cleanup helper that deletes temp files on drop
struct TempCleanup {
    paths: Vec<PathBuf>,
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_which_node() {
        // This might fail in CI without Node.js
        if which_node().is_some() {
            println!("Node.js found");
        }
    }
}
