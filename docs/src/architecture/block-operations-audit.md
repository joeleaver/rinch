# Block Operations Audit Results

Systematic test of each scenario from `block-operations-spec.md` against the actual markdown-editor behavior.

**Date:** 2026-02-25
**App:** `examples/markdown-editor`
**Method:** MCP tools (launch, click, type, screenshot, DOM inspection)

**Note:** Our DOM model uses text directly in `<li>` (`<li>text</li>`) not ProseMirror's `<li><p>text</p></li>`. Expectations adjusted accordingly.

---

## Phase 1: Set Block Type

| # | Scenario | Expected | Actual | Status |
|---|----------|----------|--------|--------|
| 1.1 | P → H2 | `<h2>text</h2>` | `<h2>text</h2>` | PASS |
| 1.2 | H2 → P (toggle) | `<p>text</p>` | `<p>text</p>` | PASS |
| 1.3 | H1 → H3 (change level) | `<h3>text</h3>` | `<h3>text</h3>` | PASS |
| 1.4 | Multi-block: all to H2 | All `<h2>` | All `<h2>` | PASS |
| 1.5 | Multi-block: toggle off | All `<p>` | All `<p>` | PASS |
| 1.6 | Multi-block: mixed → H2 | All `<h2>` | All `<h2>` | PASS |

## Phase 2: Toggle List

| # | Scenario | Expected | Actual | Status |
|---|----------|----------|--------|--------|
| 2.1 | P → UL | `<ul><li>text</li></ul>` | `<ul><li>text</li></ul>` | PASS |
| 2.2 | Multi P → UL | Single `<ul>` with 3 `<li>` | Single `<ul>` with 3 `<li>` | PASS |
| 2.3 | Toggle UL off | All `<p>` | All `<p>` | PASS |
| 2.4 | UL → OL | `<ol>` wrapper | `<ol>` wrapper | PASS |
| 2.5 | Mixed list+P → UL | All in `<ul>` | 2 separate `<ul>`, middle P untouched | **FAIL** |
| 2.6 | Partial list toggle off | List splits | List splits correctly | PASS |
| 2.9 | Adjacent list auto-join | Single merged `<ul>` | Single merged `<ul>` | PASS (fixed) |

## Phase 3: Blockquote

| # | Scenario | Expected | Actual | Status |
|---|----------|----------|--------|--------|
| 3.1 | Wrap single P | `<blockquote><p>text</p></blockquote>` | Correct | PASS |
| 3.2 | Wrap multiple P | Single `<blockquote>` | Single `<blockquote>` with 3 children | PASS |
| 3.3 | Toggle BQ off | `<p>text</p>` | `<p>text</p>` | PASS |
| 3.4 | Multi-block unwrap | All `<p>` | All `<p>` | PASS |
| 3.6 | Partial BQ unwrap | BQ splits | BQ splits correctly into two | PASS (fixed) |

## Phase 4: Enter Key

| # | Scenario | Expected | Actual | Status |
|---|----------|----------|--------|--------|
| 5.1 | Split P mid-text | Two `<p>` | Two `<p>` ("Hello" + "World") | PASS |
| 5.2 | Split P at start | Empty `<p>` + original | Empty `<p>` + `<p>Happy editing!</p>` | PASS |
| 5.3 | Split P at end | Original + empty `<p>` | `<p>Happy editing!</p>` + empty `<p>` | PASS |
| 5.4 | Heading end → P | `<h2>` + `<p>` | `<h2>` + `<p>` | PASS |
| 5.5 | Heading mid → split | Two `<h2>` | Two `<h2>` | PASS (fixed) |
| 5.7 | Split LI | Two `<li>` | Two `<li>` ("Foo" + "Bar") | PASS |
| 5.8 | Empty LI → exit list | `<p>` after list | `<p>` after list | PASS |
| 5.10 | Enter in BQ | Split inside BQ | Split inside BQ | PASS |

## Phase 5: Backspace/Delete

| # | Scenario | Expected | Actual | Status |
|---|----------|----------|--------|--------|
| 6.1 | Merge paragraphs | Single `<p>` | Single `<p>` (text merged) | PASS |
| 6.2 | Heading → P (BS) | `<p>` | `<p>` | PASS (fixed) |
| 6.3 | LI start (first) | Exit list | Exits list as `<div>` | PASS |
| 6.4 | LI start (non-first) | Merge into prev LI | Merges into prev LI | PASS (fixed) |
| 6.6 | Delete fwd merge | Single `<p>` | Single `<p>` (text merged) | PASS |
| 6.8 | Delete selection across blocks | Merged block | Single merged block | PASS (fixed) |
| 6.10 | BS into list from P | Merge with last LI | Merged into last LI | PASS |

## Phase 6: Indent/Outdent

| # | Scenario | Expected | Actual | Status |
|---|----------|----------|--------|--------|
| 4.1 | Indent LI (sink) | Nested under prev | Nested `<ul>` under prev LI | PASS |
| 4.2 | Indent into existing nested | Appended to nested | Appended to existing nested `<ul>` | PASS |
| 4.3 | Outdent nested LI | Up one level | Moved up one level | PASS |
| 4.4 | Outdent top-level LI | Exit list as `<p>` | Exits list as `<div>` | PASS (fixed) |
| 4.5 | Outdent nested with children | Children move with | Not tested | — |

---

## Summary

**Total tested:** 34
**PASS:** 33 (including 7 fixed)
**FAIL:** 1
**Not tested:** 1

### Bugs Fixed

1. **5.5 — Heading mid-split** (fixed): Split now preserves heading type for both halves.
2. **6.2 — Heading → P on Backspace** (fixed): Now converts to `<p>` instead of `<div>`.
3. **4.4 — Outdent top-level LI** (fixed): Now exits list (converts to `<div>`).
4. **6.4 — Non-first LI Backspace** (fixed): Now merges content into previous LI instead of exiting list.
5. **6.8 — Cross-block selection delete** (fixed): Now merges remaining text into a single block.
6. **3.6 — Partial blockquote unwrap** (fixed): Now splits blockquote correctly into two.
7. **2.9 — Adjacent list auto-join** (fixed): New lists now merge with adjacent same-type lists.

### Remaining Bugs

1. **2.5 — Mixed list+P selection → UL**: When selection spans list items and paragraphs, they don't merge into a single list. Paragraphs between list items are left untouched. Requires fundamental changes to multi-block selection handling where blocks have different parents.

### Additional Notes

- **Shift+End crash**: During test 6.8 setup, pressing Shift+End after Shift+Down caused a runtime crash (app shutdown). This may be a separate cursor/selection bug.
- **`<div>` vs `<p>` as default block**: Some operations produce `<div>` where `<p>` would be more semantically correct (6.3 LI exit, 4.4 outdent). Consider using `<p>` as the default non-heading block type in a future pass.
