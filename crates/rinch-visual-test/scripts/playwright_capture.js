#!/usr/bin/env node
/**
 * Playwright screenshot capture script for visual regression testing.
 *
 * Usage: node playwright_capture.js <html_path> <output_png_path> <width> <height>
 */

const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

async function main() {
    const args = process.argv.slice(2);

    if (args.length < 4) {
        console.error('Usage: playwright_capture.js <html_path> <output_png_path> <width> <height>');
        process.exit(1);
    }

    const [htmlPath, outputPath, widthStr, heightStr] = args;
    const width = parseInt(widthStr, 10);
    const height = parseInt(heightStr, 10);

    if (!fs.existsSync(htmlPath)) {
        console.error(`HTML file not found: ${htmlPath}`);
        process.exit(1);
    }

    const absoluteHtmlPath = path.resolve(htmlPath);

    const browser = await chromium.launch({
        headless: true,
        args: ['--no-sandbox', '--disable-setuid-sandbox']
    });

    try {
        const page = await browser.newPage();
        await page.setViewportSize({ width, height });

        // Load the HTML file
        await page.goto(`file://${absoluteHtmlPath}`, {
            waitUntil: 'networkidle'
        });

        // Extra settle time for fonts
        await page.waitForTimeout(100);

        // Capture screenshot
        await page.screenshot({
            path: outputPath,
            type: 'png',
            fullPage: false,
            omitBackground: false
        });

        console.log(`Screenshot saved to: ${outputPath}`);
    } finally {
        await browser.close();
    }
}

main().catch(err => {
    console.error('Error:', err.message);
    process.exit(1);
});
