#!/usr/bin/env python3
"""
Renderer comparison tool for rinch.

Connects to a running rinch app via the debug protocol, navigates to key
sections, scrolls through each section capturing screenshots, and saves
them to renderer-compare/{gpu,software}/.

Then diffs GPU vs software screenshots at the pixel level and reports
differences.

Usage:
    # 1. Launch app in GPU mode:
    cargo run -p ui-zoo-desktop --features gpu
    # 2. Capture GPU screenshots:
    python3 scripts/renderer-compare.py capture gpu
    # 3. Close app, launch in software mode:
    cargo run -p ui-zoo-desktop
    # 4. Capture software screenshots:
    python3 scripts/renderer-compare.py capture software
    # 5. Compare:
    python3 scripts/renderer-compare.py compare
"""

import argparse
import base64
import json
import math
import os
import socket
import struct
import sys
import time
from pathlib import Path

try:
    from PIL import Image, ImageChops
    import numpy as np
    HAS_PIL = True
except ImportError:
    HAS_PIL = False


ROOT = Path(__file__).resolve().parent.parent
OUTPUT_DIR = ROOT / "renderer-compare"

# Hamburger menu button position (top-left corner)
HAMBURGER_X, HAMBURGER_Y = 15, 18

# Nav link positions in the drawer (x center, approximate y center)
# These are stable because the drawer always opens at the same position.
# Order matches lib.rs: Overview(0), Buttons(1), Inputs(2), Typography(3),
# Layout(4), Navigation(5), DataDisplay(6), Feedback(7), Overlays(8),
# Icons(9), Tree(10), RichTextEditor(11), CSSFeatures(12), Video(13),
# RenderSurface(14), ContextMenu(15), FileDrop(16)
NAV_LINK_X = 130
NAV_LINK_FIRST_Y = 106  # Center of first nav link (Overview)
NAV_LINK_SPACING = 58   # Approximate spacing between nav link centers

# Content area center for scrolling (after drawer closes)
CONTENT_CENTER_X = 600
CONTENT_CENTER_Y = 400
SCROLL_STEP = 550  # Pixels per scroll step (~80% of visible area)


# --- Debug protocol client ---

class RinchDebugClient:
    def __init__(self, host="127.0.0.1", port=None):
        self.host = host
        self.port = port
        self.sock = None
        self._req_id = 0

    def discover(self, app_filter=None):
        """Find a running rinch app via discovery files."""
        discovery_dir = Path.home() / ".rinch" / "debug"
        if not discovery_dir.exists():
            raise RuntimeError("No discovery directory found at ~/.rinch/debug/")

        entries = []
        for f in discovery_dir.glob("*.json"):
            try:
                data = json.loads(f.read_text())
                pid = data.get("pid")
                if pid and os.path.exists(f"/proc/{pid}"):
                    entries.append(data)
            except (json.JSONDecodeError, KeyError):
                continue

        if not entries:
            raise RuntimeError("No running rinch apps found")

        # Filter by app name if specified
        if app_filter:
            filtered = [e for e in entries if app_filter.lower() in e.get("app_name", "").lower()]
            if filtered:
                entries = filtered

        # Prefer UI Zoo over other apps
        ui_zoo = [e for e in entries if "ui zoo" in e.get("app_name", "").lower()]
        if ui_zoo:
            entries = ui_zoo

        if len(entries) > 1:
            names = [e.get("app_name", "?") for e in entries]
            print(f"Multiple apps found: {names}, using first")

        entry = entries[0]
        self.port = entry["port"]
        print(f"Discovered app '{entry['app_name']}' on port {self.port} (PID {entry['pid']})")

    def connect(self):
        if self.port is None:
            self.discover()

        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.connect((self.host, self.port))
        self._handshake()

    def _handshake(self):
        req = {"protocol": "rinch-debug", "version": 1}
        self._write_frame(json.dumps(req).encode())
        resp_data = self._read_frame()
        resp = json.loads(resp_data)
        print(f"Connected: {resp.get('app_name', '?')} (protocol v{resp.get('version', '?')})")

    def _write_frame(self, data: bytes):
        self.sock.sendall(struct.pack(">I", len(data)) + data)

    def _read_frame(self) -> bytes:
        len_buf = self._recv_exact(4)
        length = struct.unpack(">I", len_buf)[0]
        return self._recv_exact(length)

    def _recv_exact(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("Connection closed")
            buf += chunk
        return buf

    def send_command(self, method: str, **params) -> dict:
        self._req_id += 1
        req = {"id": self._req_id, "method": method}
        if params:
            req["params"] = params
        self._write_frame(json.dumps(req).encode())
        resp_data = self._read_frame()
        return json.loads(resp_data)

    def screenshot(self) -> bytes:
        """Take a screenshot, return PNG bytes."""
        resp = self.send_command("screenshot")
        if resp.get("type") == "error":
            raise RuntimeError(f"Screenshot failed: {resp.get('message', resp)}")
        b64 = resp["data"]
        return base64.b64decode(b64)

    def click(self, x: float, y: float):
        return self.send_command("click", x=x, y=y)

    def scroll(self, x: float, y: float, delta_x: float = 0, delta_y: float = 0):
        return self.send_command("scroll", x=x, y=y, delta_x=delta_x, delta_y=delta_y)

    def wait_frame(self):
        return self.send_command("wait_frame")

    def close(self):
        if self.sock:
            self.sock.close()


# --- Navigation helpers ---

def nav_link_y(index: int) -> float:
    """Get the Y position for a nav link by index."""
    return NAV_LINK_FIRST_Y + index * NAV_LINK_SPACING


def navigate_to_section(client, section_index: int):
    """Open hamburger menu, click the nav link, wait for section to render."""
    # For sections beyond ~11, we need to scroll the nav drawer first
    nav_y = nav_link_y(section_index)

    # Open hamburger menu
    client.click(HAMBURGER_X, HAMBURGER_Y)
    client.wait_frame()

    # If the nav link is below the visible area (~760px), scroll the nav
    if nav_y > 720:
        scroll_amount = nav_y - 400
        client.scroll(NAV_LINK_X, 400, delta_y=-scroll_amount)
        client.wait_frame()
        nav_y = nav_y - scroll_amount

    client.click(NAV_LINK_X, nav_y)
    client.wait_frame()
    client.wait_frame()


# --- Sections to capture ---

SECTIONS = [
    (1, "buttons"),
    (7, "feedback"),
    (12, "css_features"),
    (3, "typography"),
    (4, "layout"),
]


def capture_section(client, section_index: int, section_name: str, output_dir: Path):
    """Navigate to a section and capture scrolled screenshots."""
    print(f"\n--- Capturing section: {section_name} (index {section_index}) ---")

    navigate_to_section(client, section_index)

    # Reset scroll to top by scrolling up a lot
    client.scroll(CONTENT_CENTER_X, CONTENT_CENTER_Y, delta_y=10000)
    client.wait_frame()

    # Capture screenshots while scrolling down
    max_screenshots = 12
    prev_png = None

    for i in range(max_screenshots):
        client.wait_frame()

        png_data = client.screenshot()
        out_path = output_dir / f"{section_name}_{i:02d}.png"
        out_path.write_bytes(png_data)
        print(f"  Saved {out_path.name} ({len(png_data)} bytes)", flush=True)

        # Check if we've reached the bottom (screenshot identical to previous)
        if prev_png is not None and png_data == prev_png:
            print(f"  Reached bottom (screenshot {i} identical to {i-1})", flush=True)
            out_path.unlink()
            break

        prev_png = png_data

        # Scroll down
        client.scroll(CONTENT_CENTER_X, CONTENT_CENTER_Y, delta_y=-SCROLL_STEP)
        client.wait_frame()


def cmd_capture(args):
    """Capture screenshots from a running app."""
    mode = args.mode  # "gpu" or "software"
    output_dir = OUTPUT_DIR / mode
    output_dir.mkdir(parents=True, exist_ok=True)

    # Clean old screenshots
    for f in output_dir.glob("*.png"):
        f.unlink()

    client = RinchDebugClient()
    client.connect()

    try:
        for section_index, section_name in SECTIONS:
            capture_section(client, section_index, section_name, output_dir)
    finally:
        client.close()

    print(f"\nDone! Screenshots saved to {output_dir}")


def cmd_compare(args):
    """Compare GPU and software screenshots pixel-by-pixel."""
    if not HAS_PIL:
        print("ERROR: Pillow and numpy required. Install with:")
        print("  pip install Pillow numpy")
        sys.exit(1)

    left_name = args.left
    right_name = args.right
    left_dir = OUTPUT_DIR / left_name
    right_dir = OUTPUT_DIR / right_name

    if not left_dir.exists() or not right_dir.exists():
        print(f"ERROR: Need both renderer-compare/{left_name}/ and renderer-compare/{right_name}/")
        sys.exit(1)

    diff_dir = OUTPUT_DIR / f"diff-{left_name}-vs-{right_name}"
    diff_dir.mkdir(parents=True, exist_ok=True)

    # Clean old diffs
    for f in diff_dir.glob("*.png"):
        f.unlink()

    left_files = sorted(left_dir.glob("*.png"))
    right_files = sorted(right_dir.glob("*.png"))

    # Match by filename
    left_map = {f.name: f for f in left_files}
    right_map = {f.name: f for f in right_files}
    common = sorted(set(left_map.keys()) & set(right_map.keys()))

    if not common:
        print(f"No matching screenshot filenames found between {left_name}/ and {right_name}/")
        print(f"Left files: {[f.name for f in left_files]}")
        print(f"Right files: {[f.name for f in right_files]}")
        sys.exit(1)

    print(f"Comparing {len(common)} pairs: {left_name} vs {right_name}\n")
    print(f"{'File':<30} {'Size':>12} {'Diff pixels':>12} {'%':>7} {'RMSE':>8}")
    print("-" * 72)

    total_diff_pixels = 0
    total_pixels = 0

    for name in common:
        left_img = Image.open(left_map[name]).convert("RGBA")
        right_img = Image.open(right_map[name]).convert("RGBA")

        # Resize to match if needed
        if left_img.size != right_img.size:
            print(f"{name:<30} SIZE MISMATCH: {left_img.size} vs {right_img.size}")
            w = min(left_img.width, right_img.width)
            h = min(left_img.height, right_img.height)
            left_img = left_img.crop((0, 0, w, h))
            right_img = right_img.crop((0, 0, w, h))

        # Composite both onto white background (alpha-blend) so we compare
        # what the user actually sees, regardless of alpha representation
        def composite_on_white(arr):
            """Alpha-composite RGBA onto white background, return RGB float."""
            alpha = arr[:, :, 3:4] / 255.0
            rgb = arr[:, :, :3]
            return rgb * alpha + 255.0 * (1.0 - alpha)

        left_arr = np.array(left_img, dtype=np.float32)
        right_arr = np.array(right_img, dtype=np.float32)

        left_rgb = composite_on_white(left_arr)
        right_rgb = composite_on_white(right_arr)

        diff = np.abs(left_rgb - right_rgb)
        pixel_count = left_img.width * left_img.height

        # Count pixels that differ (any channel differs by > threshold)
        threshold = 2  # Allow tiny rounding differences
        max_diff_per_pixel = diff.max(axis=2)  # Max across RGB channels
        diff_mask = max_diff_per_pixel > threshold
        diff_pixel_count = int(diff_mask.sum())

        # RMSE across RGB channels
        rmse = math.sqrt(np.mean(diff ** 2))

        pct = 100.0 * diff_pixel_count / pixel_count if pixel_count else 0
        size_str = f"{left_img.width}x{left_img.height}"

        total_diff_pixels += diff_pixel_count
        total_pixels += pixel_count

        print(f"{name:<30} {size_str:>12} {diff_pixel_count:>12} {pct:>6.1f}% {rmse:>7.2f}")

        # Save diff image (amplified for visibility)
        if diff_pixel_count > 0:
            diff_vis = np.zeros((left_img.height, left_img.width, 4), dtype=np.uint8)
            diff_vis[:, :, 3] = 255

            # Background: right screenshot dimmed
            right_gray = np.array(right_img.convert("L"))
            diff_vis[:, :, 0] = right_gray // 2
            diff_vis[:, :, 1] = right_gray // 2
            diff_vis[:, :, 2] = right_gray // 2

            # Highlight differences in red
            diff_vis[diff_mask, 0] = 255
            diff_vis[diff_mask, 1] = 0
            diff_vis[diff_mask, 2] = 0

            diff_img = Image.fromarray(diff_vis, "RGBA")
            diff_path = diff_dir / name
            diff_img.save(diff_path)

    print("-" * 72)
    total_pct = 100.0 * total_diff_pixels / total_pixels if total_pixels else 0
    print(f"{'TOTAL':<30} {'':>12} {total_diff_pixels:>12} {total_pct:>6.1f}%")
    print(f"\nDiff images saved to {diff_dir}/")


# --- Viewport sizes matching render-test fixture ---

# Maps HTML basename (without .html) to (width, height) matching the Rust test cases.
HTML_VIEWPORT_SIZES = {
    # Buttons
    "button_variants": (800, 60), "button_sizes": (800, 60),
    "button_colors": (800, 120), "button_radius": (800, 60),
    "button_states": (600, 60), "button_full_width": (400, 200),
    "action_icon_variants": (600, 60), "close_button": (400, 60),
    # Typography
    "title_levels": (600, 300), "text_sizes": (800, 200),
    "text_weights": (600, 200), "text_colors": (800, 200),
    "text_align": (600, 150), "code_variants": (600, 150),
    "kbd_variants": (600, 60), "highlight_colors": (800, 60),
    "blockquote": (600, 200), "anchor": (600, 60),
    # Inputs
    "text_input_states": (400, 350), "text_input_sizes": (400, 300),
    "textarea": (400, 200), "password_input": (400, 120),
    "number_input": (400, 200), "checkbox_states": (400, 200),
    "checkbox_sizes": (600, 60), "switch_states": (400, 200),
    "switch_sizes": (600, 60), "radio_group": (400, 200),
    "slider_basic": (400, 120), "select_basic": (400, 120),
    "color_swatch": (600, 60),
    # Layout
    "stack_layout": (400, 250), "stack_align": (400, 250),
    "group_layout": (800, 60), "group_justify": (800, 250),
    "center_component": (400, 100), "space_component": (400, 200),
    "container": (800, 100), "simple_grid": (600, 200),
    "paper_shadows": (800, 120), "paper_radius": (800, 120),
    "paper_border": (400, 120), "card_basic": (400, 200),
    "card_sections": (400, 300), "divider_variants": (600, 200),
    "fieldset": (400, 200),
    # Data Display
    "badge_variants": (800, 60), "badge_sizes": (600, 60),
    "badge_colors": (800, 120), "avatar_initials": (600, 80),
    "avatar_sizes": (600, 80), "list_unordered": (400, 150),
    "list_ordered": (400, 150), "breadcrumbs": (600, 100),
    # Navigation
    "tabs_default": (600, 150), "tabs_pills": (600, 150),
    "navlink_basic": (300, 250), "navlink_colors": (300, 200),
    "pagination_basic": (600, 60), "stepper_basic": (600, 100),
    "accordion_basic": (400, 250),
    # Feedback
    "alert_colors": (600, 350), "alert_variants": (600, 300),
    "loader_types": (600, 80), "loader_sizes": (600, 80),
    "loader_colors": (600, 60), "progress_basic": (600, 100),
    "progress_colors": (600, 150), "skeleton_patterns": (400, 300),
    "notification_basic": (400, 250),
    # Overlays
    "tooltip_positions": (600, 150), "modal_static": (600, 400),
    "loading_overlay": (400, 200),
    # CSS Primitives
    "css_borders": (800, 200), "css_border_radius": (800, 200),
    "css_shadows": (800, 200), "css_gradients": (800, 200),
    "css_opacity": (800, 100), "css_transforms": (800, 200),
    "css_overflow": (400, 150), "css_flexbox": (600, 300),
}


def cmd_capture_chrome(args):
    """Capture Chrome baseline screenshots from HTML files using Playwright."""
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("ERROR: Playwright required. Install with:")
        print("  pip install playwright && playwright install chromium")
        sys.exit(1)

    html_dir = ROOT / "renderer-compare" / "html"
    output_dir = OUTPUT_DIR / "chrome"
    output_dir.mkdir(parents=True, exist_ok=True)

    # Clean old screenshots
    for f in output_dir.glob("*.png"):
        f.unlink()

    html_files = sorted(html_dir.glob("*.html"))
    if not html_files:
        print(f"No HTML files found in {html_dir}")
        sys.exit(1)

    print(f"Capturing {len(html_files)} Chrome baselines...\n")

    with sync_playwright() as p:
        browser = p.chromium.launch()

        for html_file in html_files:
            name = html_file.stem
            w, h = HTML_VIEWPORT_SIZES.get(name, (800, 200))

            page = browser.new_page(viewport={"width": w, "height": h})
            page.goto(f"file://{html_file.resolve()}")
            # Wait for fonts/layout to settle
            page.wait_for_load_state("networkidle")
            page.wait_for_timeout(100)

            out_path = output_dir / f"{name}.png"
            page.screenshot(path=str(out_path), full_page=False)
            size = out_path.stat().st_size
            print(f"  {name}.png ({w}x{h}) -> {size} bytes")
            page.close()

        browser.close()

    print(f"\nDone! Chrome baselines saved to {output_dir}")


def cmd_compare3(args):
    """Three-way comparison: chrome (master) vs gpu vs software."""
    if not HAS_PIL:
        print("ERROR: Pillow and numpy required.")
        sys.exit(1)

    # Run chrome vs software
    print("=" * 72)
    print("CHROME vs SOFTWARE")
    print("=" * 72)
    args.left = "chrome"
    args.right = "software"
    cmd_compare(args)

    # Run chrome vs gpu
    print("\n" + "=" * 72)
    print("CHROME vs GPU")
    print("=" * 72)
    args.left = "chrome"
    args.right = "gpu"
    cmd_compare(args)

    # Run gpu vs software
    print("\n" + "=" * 72)
    print("GPU vs SOFTWARE")
    print("=" * 72)
    args.left = "gpu"
    args.right = "software"
    cmd_compare(args)


def main():
    parser = argparse.ArgumentParser(description="Rinch renderer comparison tool")
    sub = parser.add_subparsers(dest="cmd")

    cap = sub.add_parser("capture", help="Capture screenshots from running app")
    cap.add_argument("mode", choices=["gpu", "software"], help="Renderer mode")
    cap.set_defaults(func=cmd_capture)

    cmp = sub.add_parser("compare", help="Compare two screenshot directories")
    cmp.add_argument("--left", default="gpu", help="Left directory name (default: gpu)")
    cmp.add_argument("--right", default="software", help="Right directory name (default: software)")
    cmp.set_defaults(func=cmd_compare)

    chrome = sub.add_parser("capture-chrome", help="Capture Chrome baselines from HTML files")
    chrome.set_defaults(func=cmd_capture_chrome)

    tri = sub.add_parser("compare3", help="Three-way compare: chrome vs gpu vs software")
    tri.set_defaults(func=cmd_compare3)

    args = parser.parse_args()
    if not args.cmd:
        parser.print_help()
        sys.exit(1)

    args.func(args)


if __name__ == "__main__":
    main()
