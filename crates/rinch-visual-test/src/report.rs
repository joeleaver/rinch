//! Report generation - creates HTML reports of test results.

use std::path::Path;
use crate::runner::TestResult;

/// Generate an HTML report of test results.
pub fn generate_html_report(results: &[TestResult], output_path: &Path) -> std::io::Result<()> {
    let mut html = String::new();

    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.iter().filter(|r| !r.passed).count();
    let errored = results.iter().filter(|r| r.error.is_some()).count();

    html.push_str("<!DOCTYPE html>\n");
    html.push_str("<html>\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str("  <title>Visual Regression Test Report</title>\n");
    html.push_str("  <style>\n");
    html.push_str("    body { font-family: -apple-system, sans-serif; margin: 20px; background: #f5f5f5; }\n");
    html.push_str("    .summary { background: white; padding: 20px; border-radius: 8px; margin-bottom: 20px; }\n");
    html.push_str("    .pass { color: #22c55e; }\n");
    html.push_str("    .fail { color: #ef4444; }\n");
    html.push_str("    .test { background: white; padding: 20px; border-radius: 8px; margin-bottom: 20px; }\n");
    html.push_str("    .test.failed { border-left: 4px solid #ef4444; }\n");
    html.push_str("    .test.passed { border-left: 4px solid #22c55e; }\n");
    html.push_str("    .images { display: flex; gap: 20px; margin-top: 15px; flex-wrap: wrap; }\n");
    html.push_str("    .image-box { text-align: center; }\n");
    html.push_str("    .image-box img { max-width: 400px; border: 1px solid #ddd; }\n");
    html.push_str("    .image-box p { margin: 5px 0; font-size: 14px; color: #666; }\n");
    html.push_str("    h1 { margin-bottom: 10px; }\n");
    html.push_str("    h2 { margin: 0 0 10px 0; }\n");
    html.push_str("    .score { font-size: 14px; color: #666; }\n");
    html.push_str("    .error { background: #fef2f2; padding: 10px; border-radius: 4px; color: #991b1b; }\n");
    html.push_str("  </style>\n");
    html.push_str("</head>\n<body>\n");

    // Summary
    html.push_str("<div class=\"summary\">\n");
    html.push_str("  <h1>Visual Regression Test Report</h1>\n");
    html.push_str(&format!(
        "  <p><span class=\"pass\">✓ {} passed</span> | <span class=\"fail\">✗ {} failed</span></p>\n",
        passed, failed
    ));
    if errored > 0 {
        html.push_str(&format!("  <p class=\"fail\">{} tests errored</p>\n", errored));
    }
    html.push_str("</div>\n");

    // Individual test results
    for result in results {
        let status_class = if result.passed { "passed" } else { "failed" };
        let status_icon = if result.passed { "✓" } else { "✗" };

        html.push_str(&format!("<div class=\"test {}\">\n", status_class));
        html.push_str(&format!(
            "  <h2>{} {}</h2>\n",
            status_icon, result.name
        ));
        html.push_str(&format!(
            "  <p class=\"score\">SSIM Score: {:.4} (threshold: 0.99)</p>\n",
            result.ssim_score
        ));

        if let Some(error) = &result.error {
            html.push_str(&format!("  <div class=\"error\">{}</div>\n", html_escape(error)));
        }

        html.push_str("  <div class=\"images\">\n");

        // Actual image
        if result.actual_path.exists() {
            html.push_str("    <div class=\"image-box\">\n");
            html.push_str(&format!(
                "      <img src=\"{}\" alt=\"Actual (Rinch)\">\n",
                result.actual_path.file_name().unwrap().to_string_lossy()
            ));
            html.push_str("      <p>Actual (Rinch)</p>\n");
            html.push_str("    </div>\n");
        }

        // Expected image
        if result.expected_path.exists() {
            html.push_str("    <div class=\"image-box\">\n");
            html.push_str(&format!(
                "      <img src=\"{}\" alt=\"Expected (Browser)\">\n",
                result.expected_path.file_name().unwrap().to_string_lossy()
            ));
            html.push_str("      <p>Expected (Browser)</p>\n");
            html.push_str("    </div>\n");
        }

        // Diff image
        if let Some(diff_path) = &result.diff_path {
            if diff_path.exists() {
                html.push_str("    <div class=\"image-box\">\n");
                html.push_str(&format!(
                    "      <img src=\"{}\" alt=\"Diff\">\n",
                    diff_path.file_name().unwrap().to_string_lossy()
                ));
                html.push_str("      <p>Difference</p>\n");
                html.push_str("    </div>\n");
            }
        }

        html.push_str("  </div>\n");

        // Link to HTML
        if result.html_path.exists() {
            html.push_str(&format!(
                "  <p><a href=\"{}\">View exported HTML</a></p>\n",
                result.html_path.file_name().unwrap().to_string_lossy()
            ));
        }

        html.push_str("</div>\n");
    }

    html.push_str("</body>\n</html>\n");

    std::fs::write(output_path, html)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Print a summary of test results to stdout.
pub fn print_summary(results: &[TestResult]) {
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.iter().filter(|r| !r.passed).count();

    println!("\n{}", "=".repeat(60));
    println!("Visual Regression Test Results");
    println!("{}", "=".repeat(60));

    for result in results {
        let status = if result.passed { "✓ PASS" } else { "✗ FAIL" };
        println!("{} {} (SSIM: {:.4})", status, result.name, result.ssim_score);
        if let Some(error) = &result.error {
            println!("  Error: {}", error);
        }
    }

    println!("{}", "-".repeat(60));
    println!("Total: {} passed, {} failed", passed, failed);
    println!("{}", "=".repeat(60));
}
