//! Visual regression test CLI.

use std::path::PathBuf;
use std::process::ExitCode;

use rinch_visual_test::report::{generate_html_report, print_summary};
use rinch_visual_test::runner::TestRunner;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let update_baselines = args.iter().any(|a| a == "--update" || a == "-u");
    let config_path = args
        .iter()
        .position(|a| a == "--config" || a == "-c")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests/visual/tests.json"));

    // Determine paths relative to crate root
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scripts_dir = crate_root.join("scripts");
    let output_dir = crate_root.join("tests/visual/output");
    let baselines_dir = crate_root.join("tests/visual/baselines");

    // Resolve config path
    let config_path = if config_path.is_absolute() {
        config_path
    } else {
        crate_root.join(&config_path)
    };

    println!("Visual Regression Test Runner");
    println!("Config: {}", config_path.display());
    println!("Output: {}", output_dir.display());
    if update_baselines {
        println!("Mode: UPDATE BASELINES");
    }
    println!();

    // Load config
    let config = match TestRunner::load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            return ExitCode::FAILURE;
        }
    };

    println!("Loaded {} tests", config.tests.len());

    // Create runner
    let runner = TestRunner::new(
        scripts_dir,
        output_dir.clone(),
        baselines_dir,
        update_baselines,
    );

    // Run tests
    let results = runner.run_all(&config);

    // Print summary
    print_summary(&results);

    // Generate HTML report
    let report_path = output_dir.join("report.html");
    if let Err(e) = generate_html_report(&results, &report_path) {
        eprintln!("Warning: Failed to generate HTML report: {}", e);
    } else {
        println!("\nHTML report: {}", report_path.display());
    }

    // Return exit code based on results
    let all_passed = results.iter().all(|r| r.passed);
    if all_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
