#!/usr/bin/env python3
"""
Generate Chrome-baseline HTML files for all render-test cases.

Each HTML file uses the actual rinch CSS (theme + components) and recreates
the DOM structure that each test component produces. This enables pixel-level
comparison between rinch software/GPU renderers and Chrome.

Usage:
    # 1. Dump rinch CSS:
    cargo run -p render-test -- --dump-css > /tmp/rinch-theme.css
    # 2. Generate HTML files:
    python3 scripts/generate-chrome-baselines.py
    # 3. Capture Chrome screenshots:
    python3 scripts/renderer-compare.py capture-chrome
"""

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HTML_DIR = ROOT / "renderer-compare" / "html"
CSS_FILE = Path("/tmp/rinch-theme.css")


def load_css():
    if not CSS_FILE.exists():
        print("ERROR: CSS file not found. Run:")
        print("  cargo run -p render-test -- --dump-css > /tmp/rinch-theme.css")
        sys.exit(1)
    return CSS_FILE.read_text()


def html_page(title, body, width, height, css, extra_style=""):
    """Generate a complete HTML page with rinch CSS."""
    return f"""<!DOCTYPE html>
<html><head><meta charset="utf-8">
<title>{title}</title>
<style>
{css}
/* Override body background for screenshots */
body {{ background: white !important; margin: 0; padding: 0; }}
{extra_style}
</style>
</head><body>
{body}
</body></html>"""


# ── Button helpers ────────────────────────────────────────────────────────────

def button(text, variant="filled", size="md", color="", radius="",
           disabled=False, full_width=False, loading=False):
    classes = ["rinch-button", f"rinch-button--{size}", f"rinch-button--{variant}"]
    style_parts = []
    if color:
        classes.append("rinch-button--colored")
        style_parts.append(f"--rinch-button-color: var(--rinch-color-{color}-6)")
        style_parts.append(f"--rinch-button-color-hover: var(--rinch-color-{color}-7)")
        style_parts.append(f"--rinch-button-color-active: var(--rinch-color-{color}-8)")
        style_parts.append(f"--rinch-button-color-light: var(--rinch-color-{color}-0)")
        style_parts.append(f"--rinch-button-color-light-hover: var(--rinch-color-{color}-1)")
        style_parts.append(f"--rinch-button-color-outline: var(--rinch-color-{color}-6)")
    if radius:
        classes.append(f"rinch-button--radius-{radius}")
    if disabled:
        classes.append("rinch-button--disabled")
    if full_width:
        classes.append("rinch-button--full-width")
    if loading:
        classes.append("rinch-button--loading")
    cls = " ".join(classes)
    style = f' style="{"; ".join(style_parts)}"' if style_parts else ""
    dis = ' disabled=""' if disabled else ""
    inner = f'<span class="rinch-button__loader"></span>{text}' if loading else text
    return f'<button class="{cls}"{style}{dis}>{inner}</button>'


def action_icon(variant="filled", size="md", color="", disabled=False):
    classes = ["rinch-action-icon", f"rinch-action-icon--{variant}", f"rinch-action-icon--{size}"]
    style_parts = []
    if color:
        style_parts.append(f"--rinch-action-icon-color: var(--rinch-color-{color}-6)")
    if disabled:
        classes.append("rinch-action-icon--disabled")
    cls = " ".join(classes)
    style = f' style="{"; ".join(style_parts)}"' if style_parts else ""
    dis = ' disabled=""' if disabled else ""
    # Simple icon placeholder
    return f'<button type="button" class="{cls}"{style}{dis}><span style="display:inline-block;width:1rem;height:1rem;"></span></button>'


def close_button(size="md", disabled=False):
    classes = ["rinch-close-button", f"rinch-close-button--{size}"]
    if disabled:
        classes.append("rinch-close-button--disabled")
    cls = " ".join(classes)
    dis = ' disabled=""' if disabled else ""
    return f'<button type="button" class="{cls}"{dis} aria-label="Close"><span class="rinch-icon rinch-icon--close"></span></button>'


# ── Layout helpers ────────────────────────────────────────────────────────────

def stack(children, gap="md", align="stretch", justify=""):
    classes = ["rinch-stack", f"rinch-stack--gap-{gap}", f"rinch-stack--align-{align}"]
    if justify:
        classes.append(f"rinch-stack--justify-{justify}")
    return f'<div class="{" ".join(classes)}">{"".join(children)}</div>'


def group(children, gap="md", align="center", justify="", wrap=False, grow=False):
    classes = ["rinch-group", f"rinch-group--gap-{gap}", f"rinch-group--align-{align}"]
    if justify:
        classes.append(f"rinch-group--justify-{justify}")
    if wrap:
        classes.append("rinch-group--wrap")
    if grow:
        classes.append("rinch-group--grow")
    return f'<div class="{" ".join(classes)}">{"".join(children)}</div>'


def center(child):
    return f'<div class="rinch-center">{child}</div>'


def space(h="md"):
    return f'<div class="rinch-space rinch-space--{h}"></div>'


def container(child, size="sm"):
    return f'<div class="rinch-container rinch-container--{size}">{child}</div>'


def simple_grid(children, cols=3, spacing="sm"):
    return f'<div class="rinch-simple-grid" style="display:grid;grid-template-columns:repeat({cols},1fr);gap:var(--rinch-spacing-{spacing});">{"".join(children)}</div>'


# ── Typography helpers ────────────────────────────────────────────────────────

def title(text, order=1, align=""):
    tag = f"h{order}"
    classes = ["rinch-title", f"rinch-title--{order}"]
    if align:
        classes.append(f"rinch-title--{align}")
    return f'<{tag} class="{" ".join(classes)}">{text}</{tag}>'


def text(content, size="md", weight="", color="", align="", inline=False):
    tag = "span" if inline else "p"
    classes = ["rinch-text", f"rinch-text--{size}"]
    if weight:
        classes.append(f"rinch-text--{weight}")
    if color:
        classes.append(f"rinch-text--{color}")
    if align:
        classes.append(f"rinch-text--{align}")
    return f'<{tag} class="{" ".join(classes)}">{content}</{tag}>'


def code(content, block=False, color=""):
    classes = ["rinch-code"]
    if block:
        classes.append("rinch-code--block")
    if color:
        classes.append(f"rinch-code--{color}")
    cls = " ".join(classes)
    if block:
        return f'<pre><code class="{cls}">{content}</code></pre>'
    return f'<code class="{cls}">{content}</code>'


def kbd(content, size="md"):
    return f'<kbd class="rinch-kbd rinch-kbd--{size}">{content}</kbd>'


def highlight(content, highlighted, color=""):
    cls = "rinch-highlight"
    style = ""
    if color:
        style = f' style="--rinch-highlight-color: var(--rinch-color-{color}-2)"'
    # Simple approximation: wrap the highlighted text in <mark>
    parts = content.split(highlighted, 1)
    if len(parts) == 2:
        return f'<span class="{cls}"{style}>{parts[0]}<mark class="rinch-highlight__match">{highlighted}</mark>{parts[1]}</span>'
    return f'<span class="{cls}"{style}>{content}</span>'


def blockquote(content, cite="", color=""):
    classes = ["rinch-blockquote"]
    style = ""
    if color:
        style = f' style="--rinch-blockquote-color: var(--rinch-color-{color}-6)"'
    inner = f'<p class="rinch-blockquote__body">{content}</p>'
    if cite:
        inner += f'<cite class="rinch-blockquote__cite">{cite}</cite>'
    return f'<blockquote class="{" ".join(classes)}"{style}>{inner}</blockquote>'


def anchor(content, size="md", underline=False):
    classes = ["rinch-anchor", f"rinch-anchor--{size}"]
    if underline:
        classes.append("rinch-anchor--underline")
    return f'<a href="#" class="{" ".join(classes)}">{content}</a>'


# ── Badge helpers ─────────────────────────────────────────────────────────────

def badge(content, variant="filled", size="md", color=""):
    classes = ["rinch-badge", f"rinch-badge--{size}", f"rinch-badge--{variant}"]
    dc = f' data-color="{color}"' if color else ""
    return f'<span class="{" ".join(classes)}"{dc}>{content}</span>'


# ── Data display helpers ──────────────────────────────────────────────────────

def avatar(name="", size="md", color="", variant="filled"):
    classes = ["rinch-avatar", f"rinch-avatar--{size}", f"rinch-avatar--{variant}"]
    style_parts = []
    if color:
        style_parts.append(f"--rinch-avatar-color: var(--rinch-color-{color}-6)")
    style = f' style="{"; ".join(style_parts)}"' if style_parts else ""
    initials = "".join(w[0].upper() for w in name.split()[:2]) if name else "?"
    return f'<div class="{" ".join(classes)}"{style}><span class="rinch-avatar__placeholder">{initials}</span></div>'


def list_component(items, ordered=False, icon=None):
    cls = "rinch-list"
    tag = "ol" if ordered else "ul"
    items_html = "".join(f'<li class="rinch-list__item"><span class="rinch-list__item-wrapper">{item}</span></li>' for item in items)
    return f'<{tag} class="{cls} rinch-list--{"ordered" if ordered else "unordered"}">{items_html}</{tag}>'


def breadcrumbs(items, separator="/"):
    inner = f'<span class="rinch-breadcrumbs__separator">{separator}</span>'.join(
        f'<span class="rinch-breadcrumbs__item">{item}</span>' for item in items
    )
    return f'<div class="rinch-breadcrumbs">{inner}</div>'


# ── Input helpers ─────────────────────────────────────────────────────────────

def text_input(label="", placeholder="", description="", error="", size="md",
               disabled=False, value="", input_type="text"):
    classes = ["rinch-text-input", f"rinch-text-input--{size}"]
    if error:
        classes.append("rinch-text-input--error")
    inner = ""
    if label:
        inner += f'<label class="rinch-text-input__label">{label}</label>'
    dis = ' disabled=""' if disabled else ""
    ph = f' placeholder="{placeholder}"' if placeholder else ""
    val = f' value="{value}"' if value else ""
    inner += f'<input class="rinch-text-input__input" type="{input_type}"{ph}{val}{dis}/>'
    if description:
        inner += f'<div class="rinch-text-input__description">{description}</div>'
    if error:
        inner += f'<div class="rinch-text-input__error">{error}</div>'
    return f'<div class="{" ".join(classes)}">{inner}</div>'


def textarea_comp(label="", placeholder="", description="", error="", size="md",
                  disabled=False, rows=3):
    classes = ["rinch-text-input", f"rinch-text-input--{size}"]
    if error:
        classes.append("rinch-text-input--error")
    inner = ""
    if label:
        inner += f'<label class="rinch-text-input__label">{label}</label>'
    dis = ' disabled=""' if disabled else ""
    ph = f' placeholder="{placeholder}"' if placeholder else ""
    inner += f'<textarea class="rinch-text-input__input" rows="{rows}"{ph}{dis}></textarea>'
    if description:
        inner += f'<div class="rinch-text-input__description">{description}</div>'
    if error:
        inner += f'<div class="rinch-text-input__error">{error}</div>'
    return f'<div class="{" ".join(classes)}">{inner}</div>'


def checkbox(label="", checked=False, disabled=False, size="md", indeterminate=False):
    classes = ["rinch-checkbox", f"rinch-checkbox--{size}"]
    if checked:
        classes.append("rinch-checkbox--checked")
    if disabled:
        classes.append("rinch-checkbox--disabled")
    chk = ' checked=""' if checked else ""
    dis = ' disabled=""' if disabled else ""
    icon_cls = "rinch-checkbox__icon"
    icon = '<svg viewBox="0 0 10 7" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M4 4.586L1.707 2.293A1 1 0 1 0 .293 3.707l3 3a1 1 0 0 0 1.414 0l5-5A1 1 0 1 0 8.293.293L4 4.586z" fill="currentColor"/></svg>' if checked else ""
    if indeterminate:
        icon = '<svg viewBox="0 0 12 12"><rect x="1" y="5" width="10" height="2" fill="currentColor"/></svg>'
        classes.append("rinch-checkbox--checked")
    label_html = f'<span class="rinch-checkbox__label">{label}</span>' if label else ""
    return f'<label class="{" ".join(classes)}"><input type="checkbox" class="rinch-checkbox__input"{chk}{dis}/><span class="rinch-checkbox__box"><span class="{icon_cls}">{icon}</span></span>{label_html}</label>'


def switch(label="", checked=False, disabled=False, size="md"):
    classes = ["rinch-switch", f"rinch-switch--{size}"]
    if checked:
        classes.append("rinch-switch--checked")
    if disabled:
        classes.append("rinch-switch--disabled")
    dis = ' disabled=""' if disabled else ""
    chk = ' checked=""' if checked else ""
    label_html = f'<span class="rinch-switch__label">{label}</span>' if label else ""
    return f'<label class="{" ".join(classes)}"><input type="checkbox" class="rinch-switch__input"{chk}{dis}/><span class="rinch-switch__track"><span class="rinch-switch__thumb"></span></span>{label_html}</label>'


def radio(label, name="radio", value="", checked=False):
    classes = ["rinch-radio"]
    if checked:
        classes.append("rinch-radio--checked")
    chk = ' checked=""' if checked else ""
    return f'<label class="{" ".join(classes)}"><input type="radio" class="rinch-radio__input" name="{name}" value="{value}"{chk}/><span class="rinch-radio__box"></span><span class="rinch-radio__label">{label}</span></label>'


def radio_group(radios, label=""):
    inner = ""
    if label:
        inner += f'<div class="rinch-radio-group__label">{label}</div>'
    inner += "".join(radios)
    return f'<div class="rinch-radio-group">{inner}</div>'


def slider(value=50, min_val=0, max_val=100, color="", size="md"):
    classes = ["rinch-slider", f"rinch-slider--{size}"]
    pct = (value - min_val) / (max_val - min_val) if max_val > min_val else 0
    style_parts = []
    if color:
        style_parts.append(f"--rinch-slider-color: var(--rinch-color-{color}-6)")
    style = f' style="{"; ".join(style_parts)}"' if style_parts else ""
    return f'''<div class="{" ".join(classes)}"{style}>
  <div class="rinch-slider__track"><div class="rinch-slider__bar" style="transform: scaleX({pct});"></div></div>
  <div class="rinch-slider__thumb-wrapper" style="left: {pct*100}%;"><div class="rinch-slider__thumb-anchor"><div class="rinch-slider__thumb"></div></div></div>
</div>'''


def select_comp(label="", placeholder="", disabled=False, size="md"):
    classes = ["rinch-text-input", f"rinch-text-input--{size}"]
    inner = ""
    if label:
        inner += f'<label class="rinch-text-input__label">{label}</label>'
    dis = ' disabled=""' if disabled else ""
    ph = f' placeholder="{placeholder}"' if placeholder else ""
    inner += f'<input class="rinch-text-input__input" type="text"{ph}{dis} readonly/>'
    return f'<div class="{" ".join(classes)}">{inner}</div>'


def color_swatch(color, size=28, radius=None):
    r = f"border-radius: {radius}px;" if radius else "border-radius: var(--rinch-radius-default);"
    return f'<div class="rinch-color-swatch" style="background: {color}; width: {size}px; height: {size}px; {r}"></div>'


# ── Paper / Card helpers ──────────────────────────────────────────────────────

def paper(child, shadow="", radius="", p="", with_border=False):
    classes = ["rinch-paper"]
    if shadow:
        classes.append(f"rinch-paper--shadow-{shadow}")
    if radius:
        classes.append(f"rinch-paper--radius-{radius}")
    if p:
        classes.append(f"rinch-paper--p-{p}")
    if with_border:
        classes.append("rinch-paper--with-border")
    return f'<div class="{" ".join(classes)}">{child}</div>'


def card(child, shadow="sm", radius="md", p="lg", with_border=False):
    classes = ["rinch-card"]
    if shadow:
        classes.append(f"rinch-card--shadow-{shadow}")
    if radius:
        classes.append(f"rinch-card--radius-{radius}")
    if p:
        classes.append(f"rinch-card--p-{p}")
    if with_border:
        classes.append("rinch-card--with-border")
    return f'<div class="{" ".join(classes)}">{child}</div>'


def card_section(child, with_border=False, inherit_padding=False):
    classes = ["rinch-card__section"]
    if with_border:
        classes.append("rinch-card__section--with-border")
    if inherit_padding:
        classes.append("rinch-card__section--inherit-padding")
    return f'<div class="{" ".join(classes)}">{child}</div>'


def divider(label="", orientation="horizontal", size="xs", label_position=""):
    classes = ["rinch-divider", f"rinch-divider--{orientation}", f"rinch-divider--{size}"]
    if label:
        classes.append("rinch-divider--with-label")
        if label_position:
            classes.append(f"rinch-divider--label-{label_position}")
        return f'<div class="{" ".join(classes)}"><span class="rinch-divider__label">{label}</span></div>'
    return f'<hr class="{" ".join(classes)}"/>'


def fieldset(legend, child):
    return f'<fieldset class="rinch-fieldset"><legend class="rinch-fieldset__legend">{legend}</legend>{child}</fieldset>'


# ── Navigation helpers ────────────────────────────────────────────────────────

def tabs(tabs_list, panels, variant="default", active_value=""):
    classes = ["rinch-tabs", f"rinch-tabs--{variant}", "rinch-tabs--horizontal", "rinch-tabs--position-top"]
    tabs_html = "".join(
        f'<button class="rinch-tabs__tab{" rinch-tabs__tab--active" if val == active_value else ""}" role="tab" data-tab-value="{val}">{label}</button>'
        for val, label in tabs_list
    )
    panels_html = "".join(
        f'<div class="rinch-tabs__panel" role="tabpanel" data-panel-value="{val}"{" style=\"display:none\"" if val != active_value else ""}>{content}</div>'
        for val, content in panels
    )
    return f'<div class="{" ".join(classes)}" data-active-tab="{active_value}"><div class="rinch-tabs__list" role="tablist">{tabs_html}</div>{panels_html}</div>'


def navlink(label, description="", active=False, color="", disabled=False, variant="default"):
    classes = ["rinch-navlink", f"rinch-navlink--{variant}"]
    if active:
        classes.append("rinch-navlink--active")
    if disabled:
        classes.append("rinch-navlink--disabled")
    style = ""
    if color:
        style = f' style="--rinch-navlink-color: var(--rinch-color-{color}-6)"'
    desc_html = f'<span class="rinch-navlink__description">{description}</span>' if description else ""
    inner = f'<span class="rinch-navlink__inner"><div class="rinch-navlink__body"><span class="rinch-navlink__label">{label}</span>{desc_html}</div></span>'
    return f'<div class="rinch-navlink__wrapper"><button class="{" ".join(classes)}"{style}>{inner}</button></div>'


def pagination(total, value, siblings=1):
    # Generate page buttons
    pages = []
    pages.append(f'<button class="rinch-pagination__control" disabled>&lt;</button>')
    for i in range(1, min(total + 1, 8)):
        active = ' rinch-pagination__control--active' if i == value else ''
        pages.append(f'<button class="rinch-pagination__control{active}">{i}</button>')
    if total > 7:
        pages.append(f'<button class="rinch-pagination__control">...</button>')
        pages.append(f'<button class="rinch-pagination__control">{total}</button>')
    pages.append(f'<button class="rinch-pagination__control">&gt;</button>')
    return f'<div class="rinch-pagination">{"".join(pages)}</div>'


def stepper(steps, active=0):
    steps_html = ""
    for i, (label, desc) in enumerate(steps):
        state = "completed" if i < active else ("active" if i == active else "inactive")
        step_cls = f"rinch-stepper__step rinch-stepper__step--{state}"
        icon = f'<div class="rinch-stepper__step-icon">{i + 1}</div>'
        body = f'<div class="rinch-stepper__step-body"><div class="rinch-stepper__step-label">{label}</div><div class="rinch-stepper__step-description">{desc}</div></div>'
        sep = '<div class="rinch-stepper__separator"></div>' if i < len(steps) - 1 else ""
        steps_html += f'<div class="{step_cls}">{icon}{body}</div>{sep}'
    return f'<div class="rinch-stepper">{steps_html}</div>'


def accordion(items, variant="default"):
    classes = ["rinch-accordion", f"rinch-accordion--{variant}"]
    items_html = ""
    for val, label, content in items:
        items_html += f'''<div class="rinch-accordion__item" data-item-value="{val}">
  <button class="rinch-accordion__control"><span class="rinch-accordion__label">{label}</span><svg class="rinch-accordion__chevron" viewBox="0 0 15 15" width="15" height="15"><path d="M3.13523 6.15803C3.3241 5.95657 3.64052 5.94637 3.84197 6.13523L7.5 9.56464L11.158 6.13523C11.3595 5.94637 11.6759 5.95657 11.8648 6.15803C12.0536 6.35949 12.0434 6.67591 11.842 6.86477L7.84197 10.6148C7.64964 10.7951 7.35036 10.7951 7.15803 10.6148L3.15803 6.86477C2.95657 6.67591 2.94637 6.35949 3.13523 6.15803Z" fill="currentColor"></path></svg></button>
  <div class="rinch-accordion__panel"><div class="rinch-accordion__content">{content}</div></div>
</div>'''
    return f'<div class="{" ".join(classes)}">{items_html}</div>'


# ── Feedback helpers ──────────────────────────────────────────────────────────

def alert(content, title_text="", variant="light", color="blue", with_close=False):
    classes = ["rinch-alert", f"rinch-alert--{variant}", f"rinch-alert--{color}"]
    if title_text:
        classes.append("rinch-alert--with-title")
    inner = '<div class="rinch-alert__wrapper">'
    if title_text:
        inner += f'<div class="rinch-alert__title">{title_text}</div>'
    inner += f'<div class="rinch-alert__message">{content}</div></div>'
    if with_close:
        inner += '<button class="rinch-alert__close" type="button" aria-label="Close">&times;</button>'
    return f'<div class="{" ".join(classes)}">{inner}</div>'


def loader(type_="oval", size="md", color=""):
    classes = ["rinch-loader", f"rinch-loader--{type_}", f"rinch-loader--{size}"]
    style = ""
    if color:
        style = f' style="--rinch-loader-color: var(--rinch-color-{color}-6)"'
    if type_ == "bars":
        inner = '<span class="rinch-loader__bar"></span>' * 4
    elif type_ == "dots":
        inner = '<span class="rinch-loader__dot"></span>' * 3
    else:
        inner = '<span class="rinch-loader__oval"></span>'
    return f'<span class="{" ".join(classes)}" role="status" aria-label="Loading"{style}>{inner}</span>'


def progress(value=50, color="", size="md", striped=False, animated=False):
    classes = ["rinch-progress", f"rinch-progress--{size}"]
    style_parts = []
    if color:
        style_parts.append(f"--rinch-progress-color: var(--rinch-color-{color}-6)")
    style = f' style="{"; ".join(style_parts)}"' if style_parts else ""
    bar_cls = "rinch-progress__bar"
    if striped:
        bar_cls += " rinch-progress__bar--striped"
    if animated:
        bar_cls += " rinch-progress__bar--animated"
    return f'<div class="{" ".join(classes)}" role="progressbar" aria-valuenow="{value}"{style}><div class="{bar_cls}" style="width: {value}%;"></div></div>'


def skeleton(width="100%", height="auto", circle=False, radius="", animate=True):
    classes = ["rinch-skeleton"]
    if circle:
        classes.append("rinch-skeleton--circle")
    if radius:
        classes.append(f"rinch-skeleton--radius-{radius}")
    if animate:
        classes.append("rinch-skeleton--animate")
    style = f"width: {width}; height: {height};"
    return f'<div class="{" ".join(classes)}" style="{style}"></div>'


def notification(content, title_text="", color="", with_border=False):
    classes = ["rinch-notification"]
    if with_border:
        classes.append("rinch-notification--with-border")
    style = ""
    if color:
        style = f' style="--rinch-notification-color: var(--rinch-color-{color}-6)"'
    inner = '<div class="rinch-notification__body">'
    if title_text:
        inner += f'<div class="rinch-notification__title">{title_text}</div>'
    inner += f'<div class="rinch-notification__description">{content}</div></div>'
    return f'<div class="{" ".join(classes)}"{style}>{inner}</div>'


def loading_overlay(visible=True):
    if not visible:
        return ""
    return '<div class="rinch-loading-overlay rinch-loading-overlay--visible"><div class="rinch-loading-overlay__overlay"></div><span class="rinch-loader rinch-loader--oval rinch-loader--md" role="status"></span></div>'


# ── Wrapper ───────────────────────────────────────────────────────────────────

def wrap(body):
    """Standard test wrapper: white bg, 12px padding."""
    return f'<div style="padding: 12px; background: white;">{body}</div>'


# ═══════════════════════════════════════════════════════════════════════════════
# Test case HTML generators
# ═══════════════════════════════════════════════════════════════════════════════

def gen_button_variants():
    return wrap(group([
        button("Filled", "filled"),
        button("Light", "light"),
        button("Outline", "outline"),
        button("Subtle", "subtle"),
        button("Default", "default"),
    ], gap="sm"))


def gen_button_sizes():
    return wrap(group([
        button("XS", size="xs"),
        button("SM", size="sm"),
        button("MD", size="md"),
        button("LG", size="lg"),
        button("XL", size="xl"),
    ], gap="sm"))


def gen_button_colors():
    row1 = group([
        button("Blue", color="blue"), button("Cyan", color="cyan"),
        button("Teal", color="teal"), button("Green", color="green"),
        button("Lime", color="lime"), button("Yellow", color="yellow"),
    ], gap="xs")
    row2 = group([
        button("Orange", color="orange"), button("Red", color="red"),
        button("Pink", color="pink"), button("Grape", color="grape"),
        button("Violet", color="violet"), button("Indigo", color="indigo"),
    ], gap="xs")
    return wrap(stack([row1, row2], gap="xs"))


def gen_button_radius():
    return wrap(group([
        button("Sharp", radius="xs"), button("SM", radius="sm"),
        button("MD", radius="md"), button("LG", radius="lg"),
        button("Pill", radius="xl"),
    ], gap="sm"))


def gen_button_states():
    return wrap(group([
        button("Disabled", disabled=True),
        button("Disabled Outline", variant="outline", disabled=True),
        button("Loading", loading=True),
        button("Loading Light", variant="light", loading=True),
    ], gap="sm"))


def gen_button_full_width():
    return wrap(stack([
        button("Full Width Filled", full_width=True, variant="filled"),
        button("Full Width Outline", full_width=True, variant="outline"),
        button("Full Width Light", full_width=True, variant="light"),
        button("Full Width Subtle", full_width=True, variant="subtle"),
    ], gap="xs"))


def gen_action_icon_variants():
    return wrap(group([
        action_icon("filled"), action_icon("light"),
        action_icon("outline"), action_icon("subtle"),
        action_icon("default"),
        action_icon("filled", color="red"),
        action_icon("filled", color="green"),
        action_icon("filled", disabled=True),
    ], gap="sm"))


def gen_close_button():
    return wrap(group([
        close_button("md"), close_button("sm"),
        close_button("lg"), close_button("md", disabled=True),
    ], gap="sm"))


# ── Typography ────────────────────────────────────────────────────────────────

def gen_title_levels():
    return wrap(stack([title(f"Heading {i}", order=i) for i in range(1, 7)], gap="xs"))


def gen_text_sizes():
    labels = {"xs": "Extra Small Text (xs)", "sm": "Small Text (sm)", "md": "Medium Text (md)",
              "lg": "Large Text (lg)", "xl": "Extra Large Text (xl)"}
    return wrap(stack([text(labels[s], size=s) for s in ["xs", "sm", "md", "lg", "xl"]], gap="xs"))


def gen_text_weights():
    # Rust uses numeric weights: "400", "500", "600", "700", "900"
    weights = [("400", "Normal (400)"), ("500", "Medium (500)"), ("600", "Semibold (600)"),
               ("700", "Bold (700)"), ("900", "Black (900)")]
    return wrap(stack([text(label, weight=w) for w, label in weights], gap="xs"))


def gen_text_colors():
    row1 = group([text(c.title(), color=c) for c in ["blue", "cyan", "teal", "green", "red"]], gap="md")
    row2 = group([text(c.title(), color=c) for c in ["orange", "yellow", "violet", "dimmed", "grape"]], gap="md")
    return wrap(stack([row1, row2], gap="xs"))


def gen_text_align():
    return wrap(stack([
        text("Left aligned text", align="left"),
        text("Center aligned text", align="center"),
        text("Right aligned text", align="right"),
    ], gap="xs"))


def gen_code_variants():
    return wrap(stack([
        group([text("Inline: ", inline=True), code("let x = 42;")], gap="sm"),
        code("fn main() {\n    println!(\"Hello\");\n}", block=True),
        code("blue code", color="blue"),
        code("red code", color="red"),
    ], gap="sm"))


def gen_kbd_variants():
    return wrap(group([
        kbd("Ctrl"), kbd("+"), kbd("C"), kbd("Shift"), kbd("Cmd"), kbd("Enter"),
    ], gap="sm"))


def gen_highlight_colors():
    return wrap(group([
        highlight("Default highlight", "default"),
        highlight("Yellow highlight", "yellow", color="yellow"),
        highlight("Cyan highlight", "cyan", color="cyan"),
        highlight("Pink highlight", "pink", color="pink"),
    ], gap="md"))


def gen_blockquote():
    return wrap(stack([
        blockquote("Life is what happens when you're busy making other plans.", cite="— Author"),
        blockquote("The important thing is not to stop questioning.", cite="— Scientist", color="green"),
    ], gap="md"))


def gen_anchor():
    return wrap(group([
        anchor("Default link"),
        anchor("Small link", size="sm"),
        anchor("Large link", size="lg"),
    ], gap="md"))


# ── Inputs ────────────────────────────────────────────────────────────────────

def gen_text_input_states():
    return wrap(stack([
        text_input(label="Username", placeholder="Enter username"),
        text_input(label="Email", placeholder="name@example.com", description="We'll never share your email"),
        text_input(label="Error field", error="This field is required"),
        text_input(label="Disabled", placeholder="Cannot edit", disabled=True),
    ], gap="sm"))


def gen_text_input_sizes():
    return wrap(stack([
        text_input(label="XS", size="xs", placeholder="Extra small"),
        text_input(label="SM", size="sm", placeholder="Small"),
        text_input(label="MD", size="md", placeholder="Medium"),
        text_input(label="LG", size="lg", placeholder="Large"),
    ], gap="sm"))


def gen_textarea():
    return wrap(stack([
        textarea_comp(label="Comment", placeholder="Write a comment...", description="Markdown supported"),
        textarea_comp(label="Error", error="Too short"),
    ], gap="sm"))


def gen_password_input():
    return wrap(stack([
        text_input(label="Password", placeholder="Enter password", input_type="password"),
    ], gap="sm"))


def gen_number_input():
    return wrap(stack([
        text_input(label="Quantity", placeholder="0", input_type="number"),
        text_input(label="Price", placeholder="0.00", input_type="number"),
    ], gap="sm"))


def gen_checkbox_states():
    return wrap(stack([
        checkbox("Unchecked"),
        checkbox("Checked", checked=True),
        checkbox("Disabled unchecked", disabled=True),
        checkbox("Disabled checked", checked=True, disabled=True),
        checkbox("Indeterminate", indeterminate=True),
    ], gap="sm"))


def gen_checkbox_sizes():
    return wrap(group([
        checkbox("XS", checked=True, size="xs"),
        checkbox("SM", checked=True, size="sm"),
        checkbox("MD", checked=True, size="md"),
        checkbox("LG", checked=True, size="lg"),
    ], gap="lg"))


def gen_switch_states():
    return wrap(stack([
        switch("Off"),
        switch("On", checked=True),
        switch("Disabled off", disabled=True),
        switch("Disabled on", checked=True, disabled=True),
    ], gap="sm"))


def gen_switch_sizes():
    return wrap(group([
        switch("XS", checked=True, size="xs"),
        switch("SM", checked=True, size="sm"),
        switch("MD", checked=True, size="md"),
        switch("LG", checked=True, size="lg"),
    ], gap="lg"))


def gen_radio_group():
    return wrap(radio_group([
        radio("Free", name="plan", value="free"),
        radio("Pro", name="plan", value="pro", checked=True),
        radio("Enterprise", name="plan", value="enterprise"),
    ], label="Select a plan"))


def gen_slider_basic():
    return wrap(stack([
        slider(40),
        slider(70, color="orange"),
    ], gap="md"))


def gen_select_basic():
    return wrap(stack([
        select_comp(label="Framework", placeholder="Pick one"),
        select_comp(label="Disabled", placeholder="Cannot change", disabled=True),
    ], gap="sm"))


def gen_color_swatch():
    return wrap(group([
        color_swatch("#228be6", 30),
        color_swatch("#40c057", 30),
        color_swatch("#fab005", 30),
        color_swatch("#fa5252", 30),
        color_swatch("#7950f2", 30),
        color_swatch("rgba(0,0,0,0.5)", 30),
        color_swatch("#228be6", 30, radius=32),  # radius: "xl" = 32px
    ], gap="sm"))


# ── Layout ────────────────────────────────────────────────────────────────────

def gen_stack_layout():
    # Rust: padding: 16px
    return f'<div style="padding: 16px; background: white;">' + stack([
        '<div style="background: #228be6; color: white; padding: 8px; height: 35px;">Item 1</div>',
        '<div style="background: #40c057; color: white; padding: 8px; height: 35px;">Item 2</div>',
        '<div style="background: #fab005; color: white; padding: 8px; height: 35px;">Item 3</div>',
        '<div style="background: #fa5252; color: white; padding: 8px; height: 35px;">Item 4</div>',
    ], gap="sm") + '</div>'


def gen_stack_align():
    # Each sub-stack has a single item with the alignment name
    return wrap(stack([
        stack(['<div style="background: #228be6; color: white; padding: 4px 12px;">Start</div>'], gap="xs", align="start"),
        stack(['<div style="background: #40c057; color: white; padding: 4px 12px;">Center</div>'], gap="xs", align="center"),
        stack(['<div style="background: #fab005; color: white; padding: 4px 12px;">End</div>'], gap="xs", align="end"),
        stack(['<div style="background: #fa5252; color: white; padding: 4px 12px;">Stretch</div>'], gap="xs", align="stretch"),
    ], gap="sm"))


def gen_group_layout():
    # Rust: padding: 16px
    return f'<div style="padding: 16px; background: white;">' + group([
        '<div style="background: #228be6; color: white; padding: 8px 16px; height: 35px;">Left</div>',
        '<div style="background: #40c057; color: white; padding: 8px 16px; height: 35px;">Center</div>',
        '<div style="background: #fab005; color: white; padding: 8px 16px; height: 35px;">Right</div>',
    ]) + '</div>'


def gen_group_justify():
    # Rust uses Badge components, not plain divs
    def items(label1, label2):
        return [badge(label1), badge(label2, color="green")]
    return wrap(stack([
        group(items("Start", "Items"), justify="start"),
        group(items("Center", "Items"), justify="center"),
        group(items("End", "Items"), justify="end"),
        group(items("Between", "Items"), justify="between"),
        group(items("Around", "Items"), justify="around"),
    ], gap="sm"))


def gen_center_component():
    return wrap(center(badge("Centered Badge")))


def gen_space_component():
    return wrap(stack([
        '<div style="background: #228be6; color: white; padding: 8px;">Above xs</div>',
        space("xs"),
        '<div style="background: #40c057; color: white; padding: 8px;">Above md</div>',
        space("md"),
        '<div style="background: #fab005; color: white; padding: 8px;">Above xl</div>',
        space("xl"),
        '<div style="background: #fa5252; color: white; padding: 8px;">Bottom</div>',
    ], gap="0"))


def gen_container():
    return f'<div style="background: #f0f0f0;">' + container('<div style="background: #228be6; color: white; padding: 12px;">Container sm</div>', size="sm") + '</div>'


def gen_simple_grid():
    colors = ["#228be6", "#40c057", "#fab005", "#fa5252", "#7950f2", "#e64980"]
    items = [f'<div style="background: {c}; color: white; padding: 12px; text-align: center;">{i+1}</div>' for i, c in enumerate(colors)]
    return wrap(simple_grid(items, cols=3, spacing="sm"))


def gen_paper_shadows():
    return f'<div style="padding: 16px; background: #f0f0f0;">' + group([
        paper(text("Shadow xs"), shadow="xs", p="md"),
        paper(text("Shadow sm"), shadow="sm", p="md"),
        paper(text("Shadow md"), shadow="md", p="md"),
        paper(text("Shadow lg"), shadow="lg", p="md"),
        paper(text("Shadow xl"), shadow="xl", p="md"),
    ], gap="md") + '</div>'


def gen_paper_radius():
    return f'<div style="padding: 16px; background: #f0f0f0;">' + group([
        paper(text("xs"), shadow="sm", radius="xs", p="md"),
        paper(text("sm"), shadow="sm", radius="sm", p="md"),
        paper(text("md"), shadow="sm", radius="md", p="md"),
        paper(text("lg"), shadow="sm", radius="lg", p="md"),
        paper(text("xl"), shadow="sm", radius="xl", p="md"),
    ], gap="md") + '</div>'


def gen_paper_border():
    return f'<div style="padding: 16px; background: white;">' + stack([
        paper(text("With border"), with_border=True, p="md"),
        paper(text("Border + shadow"), with_border=True, shadow="sm", p="md"),
    ], gap="md") + '</div>'


def gen_card_basic():
    return f'<div style="padding: 16px; background: #f0f0f0;">' + card(
        text("Card Title", weight="600") + text("Card description with some content.", size="sm", color="dimmed"),
        shadow="sm", p="lg", radius="md"
    ) + '</div>'


def gen_card_sections():
    header = card_section(text("Section Header", weight="600"), inherit_padding=True)
    body = text("Card body content between sections.", size="sm", color="dimmed")
    footer = card_section(group([
        button("Cancel", variant="subtle"), button("Submit"),
    ], justify="end"), inherit_padding=True)
    return f'<div style="padding: 16px; background: #f0f0f0;">' + card(header + body + footer, shadow="sm", p="lg", radius="md", with_border=True) + '</div>'


def gen_divider_variants():
    return wrap(stack([
        text("Above divider"),
        divider(),
        text("Below divider"),
        divider(label="OR"),
        text("Below labeled divider"),
        divider(size="sm"),
        text("Thick divider above"),
    ], gap="md"))


def gen_fieldset():
    return wrap(fieldset("Personal Info", stack([
        text_input(label="Name", placeholder="Your name"),
        text_input(label="Email", placeholder="Your email"),
    ], gap="sm")))


# ── Data Display ──────────────────────────────────────────────────────────────

def gen_badge_variants():
    return wrap(group([
        badge("Filled", "filled"), badge("Light", "light"),
        badge("Outline", "outline"), badge("Dot", "dot"),
    ], gap="sm"))


def gen_badge_sizes():
    return wrap(group([
        badge("XS", size="xs"), badge("SM", size="sm"),
        badge("MD", size="md"), badge("LG", size="lg"),
        badge("XL", size="xl"),
    ], gap="sm"))


def gen_badge_colors():
    row1 = group([badge(c.title(), color=c) for c in ["blue", "cyan", "teal", "green", "lime", "yellow"]], gap="xs")
    row2 = group([badge(c.title(), color=c) for c in ["orange", "red", "pink", "grape", "violet", "indigo"]], gap="xs")
    return wrap(stack([row1, row2], gap="xs"))


def gen_avatar_initials():
    return wrap(group([
        avatar("John Doe", color="blue"),
        avatar("Jane Smith", color="green"),
        avatar("Bob Wilson", color="red"),
        avatar("Alice Brown", color="violet"),
    ], gap="sm"))


def gen_avatar_sizes():
    return wrap(group([
        avatar("JD", size="xs"), avatar("JD", size="sm"),
        avatar("JD", size="md"), avatar("JD", size="lg"),
        avatar("JD", size="xl"),
    ], gap="sm"))


def gen_list_unordered():
    return wrap(list_component(["First item in the list", "Second item with content", "Third item here"]))


def gen_list_ordered():
    return wrap(list_component(["Step one", "Step two", "Step three"], ordered=True))


def gen_breadcrumbs():
    return wrap(stack([
        breadcrumbs(["Home", "Products", "Details"]),
        breadcrumbs(["Root", "Folder", "File"], separator=">"),
    ], gap="md"))


# ── Navigation ────────────────────────────────────────────────────────────────

def gen_tabs_default():
    return wrap(tabs(
        [("gallery", "Gallery"), ("messages", "Messages"), ("settings", "Settings")],
        [("gallery", text("Gallery tab content", size="sm")), ("messages", ""), ("settings", "")],
        active_value="gallery"
    ))


def gen_tabs_pills():
    return wrap(tabs(
        [("first", "First"), ("second", "Second"), ("third", "Third")],
        [("first", text("First tab content", size="sm")), ("second", ""), ("third", "")],
        variant="pills", active_value="first"
    ))


def gen_navlink_basic():
    return wrap(stack([
        navlink("Home", active=True), navlink("Dashboard"),
        navlink("Settings"), navlink("Profile"),
    ], gap="0"))


def gen_navlink_colors():
    return wrap(stack([
        navlink("Blue", active=True, color="blue"),
        navlink("Green", active=True, color="green"),
        navlink("Red", active=True, color="red"),
        navlink("Violet", active=True, color="violet"),
    ], gap="0"))


def gen_pagination_basic():
    return wrap(pagination(10, 3))


def gen_stepper_basic():
    return wrap(stepper([
        ("Account", "Create account"),
        ("Verify", "Verify email"),
        ("Complete", "All done"),
    ], active=1))


def gen_accordion_basic():
    return wrap(accordion([
        ("item-1", "First Section", text("Content of the first section.", size="sm")),
        ("item-2", "Second Section", text("Content of the second section.", size="sm")),
        ("item-3", "Third Section", text("Content of the third section.", size="sm")),
    ]))


# ── Feedback ──────────────────────────────────────────────────────────────────

def gen_alert_colors():
    return wrap(stack([
        alert("This is an info alert.", title_text="Information", color="blue"),
        alert("Operation completed.", title_text="Success", color="green"),
        alert("Please be careful.", title_text="Warning", color="yellow"),
        alert("Something went wrong.", title_text="Error", color="red"),
    ], gap="sm"))


def gen_alert_variants():
    return wrap(stack([
        alert("Light variant alert.", title_text="Light", variant="light", color="blue"),
        alert("Filled variant alert.", title_text="Filled", variant="filled", color="blue"),
        alert("Outline variant alert.", title_text="Outline", variant="outline", color="blue"),
    ], gap="sm"))


def gen_loader_types():
    return wrap(group([loader("oval"), loader("dots"), loader("bars")], gap="lg"))


def gen_loader_sizes():
    return wrap(group([loader(size=s) for s in ["xs", "sm", "md", "lg", "xl"]], gap="lg"))


def gen_loader_colors():
    return wrap(group([loader(color=c) for c in ["blue", "cyan", "green", "violet", "red"]], gap="lg"))


def gen_progress_basic():
    return wrap(stack([
        progress(25), progress(50, color="green"),
        progress(75, color="orange", size="lg"),
    ], gap="md"))


def gen_progress_colors():
    return wrap(stack([
        progress(60, color="cyan", size="xs"),
        progress(60, color="teal", size="sm"),
        progress(60, color="green", size="md"),
        progress(60, color="violet", size="lg"),
    ], gap="sm"))


def gen_skeleton_patterns():
    return wrap(stack([
        # Text skeleton
        skeleton(height="8px", radius="xl"),
        skeleton(width="70%", height="8px", radius="xl"),
        # Profile skeleton
        group([
            skeleton(width="40px", height="40px", circle=True),
            stack([skeleton(width="120px", height="8px"), skeleton(width="80px", height="8px")], gap="xs"),
        ], gap="sm"),
        # Card skeleton
        skeleton(height="120px", radius="md"),
        skeleton(width="60%", height="12px"),
        skeleton(height="8px"),
        skeleton(width="80%", height="8px"),
    ], gap="md"))


def gen_notification_basic():
    return wrap(stack([
        notification("Your changes have been saved.", title_text="Success", color="green"),
        notification("Failed to save changes.", title_text="Error", color="red"),
        notification("A new update is available.", title_text="Info", color="blue"),
    ], gap="md"))


# ── Overlays ──────────────────────────────────────────────────────────────────

def gen_tooltip_positions():
    # Rust: padding: 40px 12px; Group justify: center, gap: xl
    # Tooltips are positioned by Tooltip component — in HTML we approximate
    return f'<div style="padding: 40px 12px; background: white;">' + group([
        '<div style="position: relative; display: inline-block;">' +
        button("Top", variant="outline") +
        '<div class="rinch-tooltip rinch-tooltip--top" style="position: absolute; bottom: 100%; left: 50%; transform: translateX(-50%); margin-bottom: 8px;">Top tooltip</div></div>',
        '<div style="position: relative; display: inline-block;">' +
        button("Right", variant="outline") +
        '<div class="rinch-tooltip rinch-tooltip--right" style="position: absolute; left: 100%; top: 50%; transform: translateY(-50%); margin-left: 8px;">Right tooltip</div></div>',
        '<div style="position: relative; display: inline-block;">' +
        button("Bottom", variant="outline") +
        '<div class="rinch-tooltip rinch-tooltip--bottom" style="position: absolute; top: 100%; left: 50%; transform: translateX(-50%); margin-top: 8px;">Bottom tooltip</div></div>',
    ], gap="xl", justify="center") + '</div>'


def gen_modal_static():
    return wrap(f'''<div style="background: rgba(0,0,0,0.5); padding: 40px; border-radius: 8px;">
  <div style="background: white; padding: 20px; border-radius: 8px; max-width: 500px;">
    {stack([
        title("Modal Title", order=3),
        text("This is the modal body content. It demonstrates how a modal looks when rendered.", size="sm"),
        group([button("Cancel", variant="subtle"), button("Confirm", variant="filled")], justify="end"),
    ], gap="md")}
  </div>
</div>''')


def gen_loading_overlay():
    return wrap(f'''<div style="position: relative; height: 160px;">
  {stack([text("Content behind overlay"), text("This text should be dimmed by the overlay.", size="sm", color="dimmed")], gap="sm")}
  {loading_overlay()}
</div>''')


# ── CSS Primitives (already exist as standalone HTML, skip) ───────────────────
# css_borders, css_border_radius, css_shadows, css_gradients, css_opacity,
# css_transforms, css_overflow, css_flexbox are hand-written HTML files.


# ═══════════════════════════════════════════════════════════════════════════════
# Test case registry mapping test names to generators and viewport sizes
# ═══════════════════════════════════════════════════════════════════════════════

TEST_CASES = {
    # Buttons
    "button_variants": (gen_button_variants, 800, 60),
    "button_sizes": (gen_button_sizes, 800, 60),
    "button_colors": (gen_button_colors, 800, 120),
    "button_radius": (gen_button_radius, 800, 60),
    "button_states": (gen_button_states, 600, 60),
    "button_full_width": (gen_button_full_width, 400, 200),
    "action_icon_variants": (gen_action_icon_variants, 600, 60),
    "close_button": (gen_close_button, 400, 60),
    # Typography
    "title_levels": (gen_title_levels, 600, 300),
    "text_sizes": (gen_text_sizes, 800, 200),
    "text_weights": (gen_text_weights, 600, 200),
    "text_colors": (gen_text_colors, 800, 200),
    "text_align": (gen_text_align, 600, 150),
    "code_variants": (gen_code_variants, 600, 150),
    "kbd_variants": (gen_kbd_variants, 600, 60),
    "highlight_colors": (gen_highlight_colors, 800, 60),
    "blockquote": (gen_blockquote, 600, 200),
    "anchor": (gen_anchor, 600, 60),
    # Inputs
    "text_input_states": (gen_text_input_states, 400, 350),
    "text_input_sizes": (gen_text_input_sizes, 400, 300),
    "textarea": (gen_textarea, 400, 200),
    "password_input": (gen_password_input, 400, 120),
    "number_input": (gen_number_input, 400, 200),
    "checkbox_states": (gen_checkbox_states, 400, 200),
    "checkbox_sizes": (gen_checkbox_sizes, 600, 60),
    "switch_states": (gen_switch_states, 400, 200),
    "switch_sizes": (gen_switch_sizes, 600, 60),
    "radio_group": (gen_radio_group, 400, 200),
    "slider_basic": (gen_slider_basic, 400, 120),
    "select_basic": (gen_select_basic, 400, 120),
    "color_swatch": (gen_color_swatch, 600, 60),
    # Layout
    "stack_layout": (gen_stack_layout, 400, 250),
    "stack_align": (gen_stack_align, 400, 250),
    "group_layout": (gen_group_layout, 800, 60),
    "group_justify": (gen_group_justify, 800, 250),
    "center_component": (gen_center_component, 400, 100),
    "space_component": (gen_space_component, 400, 200),
    "container": (gen_container, 800, 100),
    "simple_grid": (gen_simple_grid, 600, 200),
    "paper_shadows": (gen_paper_shadows, 800, 120),
    "paper_radius": (gen_paper_radius, 800, 120),
    "paper_border": (gen_paper_border, 400, 120),
    "card_basic": (gen_card_basic, 400, 200),
    "card_sections": (gen_card_sections, 400, 300),
    "divider_variants": (gen_divider_variants, 600, 200),
    "fieldset": (gen_fieldset, 400, 200),
    # Data Display
    "badge_variants": (gen_badge_variants, 800, 60),
    "badge_sizes": (gen_badge_sizes, 600, 60),
    "badge_colors": (gen_badge_colors, 800, 120),
    "avatar_initials": (gen_avatar_initials, 600, 80),
    "avatar_sizes": (gen_avatar_sizes, 600, 80),
    "list_unordered": (gen_list_unordered, 400, 150),
    "list_ordered": (gen_list_ordered, 400, 150),
    "breadcrumbs": (gen_breadcrumbs, 600, 100),
    # Navigation
    "tabs_default": (gen_tabs_default, 600, 150),
    "tabs_pills": (gen_tabs_pills, 600, 150),
    "navlink_basic": (gen_navlink_basic, 300, 250),
    "navlink_colors": (gen_navlink_colors, 300, 200),
    "pagination_basic": (gen_pagination_basic, 600, 60),
    "stepper_basic": (gen_stepper_basic, 600, 100),
    "accordion_basic": (gen_accordion_basic, 400, 250),
    # Feedback
    "alert_colors": (gen_alert_colors, 600, 350),
    "alert_variants": (gen_alert_variants, 600, 300),
    "loader_types": (gen_loader_types, 600, 80),
    "loader_sizes": (gen_loader_sizes, 600, 80),
    "loader_colors": (gen_loader_colors, 600, 60),
    "progress_basic": (gen_progress_basic, 600, 100),
    "progress_colors": (gen_progress_colors, 600, 150),
    "skeleton_patterns": (gen_skeleton_patterns, 400, 300),
    "notification_basic": (gen_notification_basic, 400, 250),
    # Overlays
    "tooltip_positions": (gen_tooltip_positions, 600, 150),
    "modal_static": (gen_modal_static, 600, 400),
    "loading_overlay": (gen_loading_overlay, 400, 200),
}


def main():
    css = load_css()
    HTML_DIR.mkdir(parents=True, exist_ok=True)

    print(f"Generating {len(TEST_CASES)} HTML baselines to {HTML_DIR}")

    for name, (gen_fn, width, height) in TEST_CASES.items():
        body = gen_fn()
        html = html_page(name, body, width, height, css)
        path = HTML_DIR / f"{name}.html"
        path.write_text(html)
        print(f"  {name}.html ({width}x{height})")

    print(f"\nDone! {len(TEST_CASES)} HTML files generated.")
    print(f"\nNext steps:")
    print(f"  python3 scripts/renderer-compare.py capture-chrome")
    print(f"  python3 scripts/renderer-compare.py compare --left chrome --right software")


if __name__ == "__main__":
    main()
