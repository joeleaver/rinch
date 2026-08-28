# rinch-visual-test

Visual regression testing framework for Rinch applications. Compares Rinch's rendering output against browser rendering to ensure pixel-perfect cross-platform compatibility.

## Overview

The visual test system works by:

1. **Capturing** screenshots from a running Rinch application via the debug protocol
2. **Exporting** the DOM tree to standards-compliant HTML/CSS
3. **Rendering** the exported HTML in a browser (Chromium via Playwright)
4. **Comparing** the two screenshots using SSIM (Structural Similarity Index)
5. **Reporting** differences with visual diffs and detailed metrics

## Architecture

```
┌─────────────────┐
│  Rinch App      │ (with debug feature enabled)
│  (ui-zoo-desktop)    │
└────────┬────────┘
         │ TCP IPC (debug protocol)
         ▼
┌─────────────────┐
│ rinch-visual-   │
│ test            │
│ ┌─────────────┐ │
│ │ RinchCapture│ │ Screenshots + DOM tree
│ └─────────────┘ │
│ ┌─────────────┐ │
│ │ HTML Export │ │ Convert DOM → HTML/CSS
│ └─────────────┘ │
│ ┌─────────────┐ │
│ │ Browser     │ │ Render HTML via Playwright
│ │ Capture     │ │
│ └─────────────┘ │
│ ┌─────────────┐ │
│ │ SSIM Compare│ │ Pixel-level comparison
│ └─────────────┘ │
│ ┌─────────────┐ │
│ │ HTML Report │ │ Generate visual report
│ └─────────────┘ │
└─────────────────┘
```

## Components

| Module | Purpose |
|--------|---------|
| `capture.rs` | Connects to Rinch app via debug protocol |
| `css_export.rs` | Exports computed styles to CSS |
| `html_serializer.rs` | Converts DOM tree to HTML |
| `browser.rs` | Captures browser screenshots via Playwright |
| `compare.rs` | SSIM image comparison and diff generation |
| `runner.rs` | Test orchestration |
| `report.rs` | HTML report generation |
| `bin/visual_test.rs` | CLI binary |
| `examples/gen_browser_screenshot.rs` | Standalone tool to generate a browser screenshot from a DOM dump |

## Prerequisites

1. **Rust** - For building the test runner and Rinch apps
2. **Node.js** - For running Playwright
3. **Xvfb** - For headless testing (Linux)

```bash
# Ubuntu/Debian
sudo apt-get install xvfb libxkbcommon-dev libwayland-dev

# macOS
# Xvfb not needed - use native windowing
```

## Installation

```bash
# Install Playwright dependencies
cd crates/rinch-visual-test/scripts
npm install
npx playwright install chromium

# Build the visual test runner
cargo build -p rinch-visual-test --release
```

## Usage

### 1. Create Test Configuration

The default config lives at `tests/visual/tests.json` within this crate:

```json
{
  "viewport": [800, 600],
  "background": "#1a1a1a",
  "tests": [
    {
      "name": "initial_render",
      "threshold": 0.99
    },
    {
      "name": "buttons_section",
      "section": 1,
      "threshold": 0.99
    },
    {
      "name": "after_click",
      "setup_clicks": [[400, 300]],
      "threshold": 0.95
    }
  ]
}
```

**Test options:**
- `name` - Unique test identifier
- `threshold` - SSIM threshold (0.0-1.0, default: 0.99)
- `section` - Navigate to this section index before capturing
- `setup_clicks` - Click coordinates to set up state before capture

### 2. Launch Your App

Build and run your Rinch app with the `debug` feature enabled:

```bash
cargo run -p ui-zoo-desktop --release
```

The app will automatically start the debug server and write discovery info to `~/.rinch/debug/{pid}.json`.

### 3. Run Tests

```bash
# Run tests with default config (tests/visual/tests.json)
./target/release/visual-test

# Use custom config
./target/release/visual-test --config path/to/tests.json

# Update baselines (saves browser screenshots as new baselines)
./target/release/visual-test --update
```

### 4. View Results

Test artifacts are written to `tests/visual/output/`:

```
tests/visual/output/
├── report.html              # Interactive HTML report
├── full_window_actual.png   # Rinch screenshot
├── full_window_expected.png # Browser screenshot
├── full_window_diff.png     # Visual diff (if failed)
└── full_window.html         # Exported HTML
```

Open `report.html` in a browser to see:
- Summary: Pass/fail counts
- Per-test results with SSIM scores
- Side-by-side image comparison
- Diff images highlighting pixel differences

## Headless Testing

For CI or headless environments:

```bash
# Start Xvfb
Xvfb :99 -screen 0 1280x720x24 &
export DISPLAY=:99

# Run your app
cargo run -p ui-zoo-desktop --release &
APP_PID=$!

# Wait for startup
sleep 5

# Run tests
./target/release/visual-test

# Cleanup
kill $APP_PID
```

## Test Workflow Example

```bash
# 1. Build everything
cargo build -p rinch-visual-test --release
cargo build -p ui-zoo-desktop --release

# 2. Start app in background (headless)
DISPLAY=:99 ./target/release/ui-zoo-desktop &
APP_PID=$!
sleep 5

# 3. Run tests
./target/release/visual-test

# 4. Check results
echo "Exit code: $?"
cat tests/visual/output/report.html

# 5. Cleanup
kill $APP_PID
```

## CI Integration

See `.github/workflows/visual-tests.yml` for GitHub Actions integration.

Key steps:
1. Install Rust, Node.js, Xvfb
2. Install Playwright and Chromium
3. Build test runner and app
4. Start Xvfb and app
5. Run tests
6. Upload artifacts (screenshots, reports)

## Why this is not a merge gate yet

The workflow is `workflow_dispatch` only, on purpose. It is not a gate because
4 of the 12 scenarios are still red for reasons that have nothing to do with a
regression, and gating on a red suite teaches everyone to ignore it.

Measured at viewport 1200x800, before and after the CSS-export repair described
under "What the exporter must emit" below:

| Scenario | before | after | |
|---|---|---|---|
| `00_overview` | 0.9133 | 0.9726 | pass |
| `01_buttons` | 0.9133 | 0.9726 | pass — *identical to `00_overview`; its `setup_clicks` never left the overview* |
| `02_inputs` | 0.8808 | 0.9651 | pass |
| `03_typography` | 0.6982 | 0.9847 | pass |
| `04_layout` | 0.8506 | 0.8883 | |
| `05_navigation` | 0.8891 | 0.9369 | pass |
| `06_data_display` | 0.4566 | 0.8949 | |
| `07_feedback` | 0.8891 | 0.9369 | pass — *same score as 05/09; stale coordinates* |
| `08_overlays` | 0.4566 | 0.8949 | *same score as 06* |
| `09_icons` | 0.8891 | 0.9369 | pass |
| `10_tree` | 0.7597 | 0.8116 | |
| `11_editor` | 0.9490 | 0.9900 | pass |

Total: **3 passed / 9 failed** before, **8 passed / 4 failed** after.

Most of what looked like "real rinch-vs-Chromium divergence" was the exporter
dropping properties. What remains:

1. **Stale `setup_clicks`.** Several scenarios share a score exactly, because
   their click coordinates no longer land on the nav item they name and they
   capture the same screen. `01_buttons` never leaves the overview at all.
   These need re-deriving against the current UI Zoo nav, ideally by querying
   the nav node's `absolute` box rather than hard-coding pixels.
   Note also that `run_test` never resets the app between scenarios: the clicks
   replay onto whatever state the previous scenario left behind, so a scenario
   that opens an overlay changes what the next one sees.
2. **Window chrome.** The borderless titlebar and menu bar rinch paints itself
   have no counterpart in the exported HTML, so the top ~36px never matches.
   Either crop it out of both sides or export it.
3. **Properties still not exported** — `transform`, `box-shadow`, `text-shadow`,
   `outline`, `text-transform`, `object-fit`, and grid placement. `10_tree` and
   `04_layout` are the scenarios most exposed to these.

**The plan to green it:** fix (1), then re-measure; crop or export the chrome for
(2); export the remaining properties in (3); set each scenario's threshold from
its own settled score with a margin, rather than one global 0.90; then add
`pull_request` to the trigger list.

### What the exporter must emit

`css_export.rs` reads the JSON shape `rinch_dom::computed_style::ComputedStyle`
actually serializes. Two whole classes of property used to be read from keys
that do not exist on it, which is invisible at runtime — the lookup just misses
and nothing is emitted:

- **Background** is a `background: BackgroundValue` enum
  (`"None"` / `{"Color": "#rrggbb"}` / gradient / image), **not** a flat
  `background_color`. Reading the flat key meant a 202 KB export carried exactly
  one `background-color` declaration — the body's, from the config.
- **Border colors** are per-side (`border_top_color`, …), **not** an aggregate
  `border_color`; and border *styles* are per-side `border_*_style` fields
  rather than something to infer from a non-zero width.
- **`position`'s default is `Static`**, so `static` is the value to omit and
  `relative` the value to emit. Inverting that dropped every
  `position: relative`, which silently re-parented every absolutely-positioned
  descendant in the browser reference.

When adding a property here, check the field name and variant spelling against
`crates/rinch-dom/src/computed_style/`, and add a unit test using the *serialized*
shape — a fixture with an invented key name passes while exporting nothing.

### What this suite could and could not have caught

It is worth being precise, because the harness was assumed to be a safety net
that merely happened to be switched off.

Against the double-paint regression fixed by #342 — where every text node in an
inline formatting context was painted twice, plainly visible as the UI Zoo hero
heading struck through itself — the suite's verdict is **identical on the broken
and the fixed tree**:

| | fixed (`b906885`) | broken (`b906885^`) |
|---|---|---|
| `00_overview` | 0.9133 pass | 0.9090 **pass** |
| suite total | 3 passed, 9 failed | 3 passed, 9 failed |

(Measured with the pre-repair exporter, so the absolute numbers are the "before"
column above. The argument does not depend on them: it is about the *size* of
the defect relative to the noise floor, and repairing the exporter lowers both.)

The defect moves the score by 0.0043 and changes no verdict. It is not a
threshold-calibration problem: the double paint alters ~0.5% of the screen's
pixels, while rinch and Chromium already differ on 6-11% of them for benign
reasons. A whole-screen similarity score cannot resolve a defect that local,
whatever the threshold.

What did catch it was a *targeted* oracle — the two tests #342 added in
`crates/rinch-dom/tests/stacking_tests.rs`, one asserting the stacking sequence
and one a pixel test over the padding strip, the one region where a correct
render puts no ink at all. Treat this suite as a broad smoke test for layout
drift, and keep writing local oracles for specific paint invariants.

## SSIM Comparison

The test system uses SSIM (Structural Similarity Index) for image comparison:

- **SSIM score**: 0.0 (completely different) to 1.0 (identical)
- **Windowed**: computed over 8x8 non-overlapping windows and averaged, which
  is what makes the number track what the eye sees. It was briefly computed as
  a single global statistic over the whole image instead; that is a correlation
  dominated by overall brightness and total contrast, and it scored two renders
  a human reads as ~91% alike at 0.45, which made every threshold here
  meaningless. Do not "simplify" it back.
- **Default threshold**: 0.99 in code (`tests.json` overrides to 0.90). The
  summary and the HTML report print each result's *own* threshold — they used to
  print a hardcoded 0.99 next to a verdict reached at 0.90.
- **Per-pixel diff**: Highlights pixels that differ by >10 in any RGB channel
- **Diff image**: Red overlay on differences, grayscale on matches

SSIM is more robust than pixel-perfect comparison because it accounts for:
- Luminance changes
- Contrast variations
- Structural patterns

This makes tests resilient to minor rendering differences while catching real
visual bugs — but only ones that are *large*; see "Why this is not a merge gate
yet" above for the limits.

## Troubleshooting

### "No rinch app found"

- Ensure your app enables the `debug` feature on its `rinch` dependency
  (ui-zoo-desktop already does — do **not** pass `--features debug` to it,
  it declares no such feature of its own and cargo will error)
- Check `~/.rinch/debug/` for discovery files
- Verify the app is still running (not crashed)

### Playwright errors

```bash
# Reinstall Playwright
cd crates/rinch-visual-test/scripts
rm -rf node_modules package-lock.json
npm install
npx playwright install chromium
```

### SSIM threshold too strict

Lower the threshold in your test config:

```json
{
  "threshold": 0.95  // More lenient (default: 0.99)
}
```

### Viewport size mismatch

Ensure the test viewport matches your app window size:

```json
{
  "viewport": [800, 600]  // Must match app window
}
```

## Development

### Running Tests

```bash
# Unit tests
cargo test -p rinch-visual-test

# Integration test (requires running app)
cargo run -p ui-zoo-desktop --release &
sleep 3
cargo test -p rinch-visual-test -- --ignored
```

### Adding New Tests

1. Add test definition to `tests/visual/tests.json`
2. Run with `--update` to generate initial baselines
3. Commit baselines to version control
4. CI will verify future changes against baselines

### Debugging

Enable debug output:

```bash
RUST_LOG=debug ./target/release/visual-test
```

Inspect exported HTML:

```bash
# Open exported HTML in browser to see what Playwright renders
firefox tests/visual/output/test_name.html
```

## License

Same as the Rinch project.
