# Rich Text Editor Block-Level Operations Specification

This document specifies the expected behavior of block-level operations in a rich text editor,
compiled from ProseMirror, Tiptap, the W3C execCommand draft, Slate.js, and observed behavior
in Google Docs and Notion. Where implementations diverge, divergences are noted.

Throughout this document, DOM structure is shown in HTML-like notation. The cursor position is
indicated by `|`. Selection ranges are indicated by `[` and `]`.

---

## Table of Contents

1. [Set Block Type (Paragraph, Headings)](#1-set-block-type)
2. [Toggle List (UL/OL)](#2-toggle-list)
3. [Blockquote (Wrap/Unwrap)](#3-blockquote)
4. [Indent/Outdent](#4-indent-outdent)
5. [Enter Key](#5-enter-key)
6. [Backspace/Delete](#6-backspace-delete)
7. [Tab in Lists](#7-tab-in-lists)

---

## 1. Set Block Type

**Commands:** ProseMirror `setBlockType`, Tiptap `toggleHeading`/`toggleNode`, W3C `formatBlock`

### 1.1 Single Block: Set Paragraph to Heading

**Action:** Apply heading level 2 to a paragraph.

```
BEFORE:
<p>Hello world|</p>

AFTER:
<h2>Hello world|</h2>
```

The block type changes; inline content and marks are preserved. The cursor position within
the text is maintained.

### 1.2 Single Block: Toggle Same Heading Level (Heading to Paragraph)

**Action:** Apply heading level 2 when block is already `<h2>`.

```
BEFORE:
<h2>Hello world|</h2>

AFTER:
<p>Hello world|</p>
```

**Consensus behavior:** Applying the same heading level that is already active converts the
block back to a paragraph. This is a "toggle" semantic.

**Divergences:**
- ProseMirror's raw `setBlockType` does NOT have built-in toggle logic. It skips blocks
  that already match the target type. Toggle behavior must be implemented by the application
  (check current type, if matching, set to paragraph instead).
- Tiptap's `toggleHeading({ level: N })` has built-in toggle: if the block is already
  heading level N, it converts to paragraph.
- Google Docs and Notion both implement toggle behavior.

### 1.3 Single Block: Change Heading Level

**Action:** Apply heading level 3 to an existing `<h1>`.

```
BEFORE:
<h1>Title|</h1>

AFTER:
<h3>Title|</h3>
```

The heading level changes. This is NOT a toggle -- it converts to the new level.

### 1.4 Multi-Block Selection: Set All to Same Type

**Action:** Select across multiple paragraphs, apply heading level 2.

```
BEFORE:
<p>First [paragraph</p>
<p>Second paragraph</p>
<p>Third] paragraph</p>

AFTER:
<h2>First [paragraph</h2>
<h2>Second paragraph</h2>
<h2>Third] paragraph</h2>
```

All textblocks that overlap the selection are converted. The selection is preserved.

**ProseMirror:** `setBlockType` iterates through all selection ranges, finds textblock nodes
between range boundaries, and applies the type change to each. Blocks already matching the
target type are skipped. Returns false if no applicable textblocks exist.

### 1.5 Multi-Block Selection: Toggle When All Are Same Type

**Action:** Select across multiple `<h2>` blocks, apply heading level 2 (toggle off).

```
BEFORE:
<h2>First [heading</h2>
<h2>Second heading</h2>
<h2>Third] heading</h2>

AFTER:
<p>First [heading</p>
<p>Second heading</p>
<p>Third] heading</p>
```

When all selected blocks are already the target type, toggle them all back to paragraph.

### 1.6 Multi-Block Selection: Mixed Types

**Action:** Select across blocks of different types, apply heading level 2.

```
BEFORE:
<p>A [paragraph</p>
<h2>Already h2</h2>
<h3>Was] h3</h3>

AFTER:
<h2>A [paragraph</h2>
<h2>Already h2</h2>
<h2>Was] h3</h2>
```

All blocks are converted to the target type regardless of their current type.
ProseMirror's `setBlockType` skips blocks already at the target type, but the visual
result is the same since they are already correct.

### 1.7 Edge Case: Block Type in List Items

**Action:** Set block type inside a list item.

```
BEFORE:
<ul>
  <li><p>Item| one</p></li>
  <li><p>Item two</p></li>
</ul>

AFTER:
<ul>
  <li><h2>Item| one</h2></li>
  <li><p>Item two</p></li>
</ul>
```

The textblock inside the list item changes type. The list structure is preserved.

**Note:** Whether this is desirable depends on the editor's schema. ProseMirror schemas
can restrict which node types are valid inside list items. Many editors (Google Docs, Notion)
do NOT allow headings inside list items.

### 1.8 Edge Case: Block Type Inside Blockquote

```
BEFORE:
<blockquote>
  <p>Quoted| text</p>
</blockquote>

AFTER:
<blockquote>
  <h2>Quoted| text</h2>
</blockquote>
```

The textblock inside the blockquote changes. The blockquote wrapper is preserved.

---

## 2. Toggle List

**Commands:** ProseMirror `wrapInList`/`liftListItem`, Tiptap `toggleList`, W3C
`insertOrderedList`/`insertUnorderedList`

### 2.1 Convert Paragraph to Bullet List

**Action:** Toggle bullet list on a paragraph.

```
BEFORE:
<p>Hello| world</p>

AFTER:
<ul>
  <li><p>Hello| world</p></li>
</ul>
```

The paragraph is wrapped in a list item, which is wrapped in a list.

**ProseMirror model:** List items contain paragraphs (or other block content).
The `wrapInList` command computes the wrapping structure needed and applies it.

### 2.2 Convert Multiple Paragraphs to List

**Action:** Select across multiple paragraphs, toggle bullet list.

```
BEFORE:
<p>First [item</p>
<p>Second item</p>
<p>Third] item</p>

AFTER:
<ul>
  <li><p>First [item</p></li>
  <li><p>Second item</p></li>
  <li><p>Third] item</p></li>
</ul>
```

Each paragraph becomes a separate list item within a single list.

### 2.3 Toggle List Off (List Items Back to Paragraphs)

**Action:** Toggle bullet list when cursor is already in a bullet list.

```
BEFORE:
<ul>
  <li><p>First| item</p></li>
  <li><p>Second item</p></li>
  <li><p>Third item</p></li>
</ul>

AFTER:
<p>First| item</p>
<p>Second item</p>
<p>Third item</p>
```

All list items are lifted out of the list and become paragraphs.

**ProseMirror:** Uses `liftListItem` which lifts items out of the list. When the parent
node is not another list, items are lifted entirely out of list context.

**Tiptap:** `toggleList` checks if the current selection is already in the target list
type. If so, it removes the list formatting.

### 2.4 Change List Type (Bullet to Ordered)

**Action:** Toggle ordered list when content is already in a bullet list.

```
BEFORE:
<ul>
  <li><p>First| item</p></li>
  <li><p>Second item</p></li>
</ul>

AFTER:
<ol>
  <li><p>First| item</p></li>
  <li><p>Second item</p></li>
</ol>
```

The list wrapper type changes from `<ul>` to `<ol>`. List items and their content
are preserved.

**Implementation pattern (ProseMirror):** This is typically implemented as:
1. `liftListItem` to remove from existing list
2. `wrapInList` with the new list type

Or, more efficiently, by changing the node type of the list wrapper directly via a
transaction.

**Tiptap:** `toggleList('orderedList', 'listItem')` handles this automatically. If the
selection is in a `bulletList`, it converts to `orderedList` without intermediate lifting.

**Divergence note:** ProseMirror's raw `wrapInList` will fail if content is already
in a list (cannot wrap a list item in another list). The toggle/convert logic must
be implemented at the application level.

### 2.5 Mixed Selection: Some List Items, Some Paragraphs

**Action:** Toggle bullet list when selection spans both list items and paragraphs.

```
BEFORE:
<ul>
  <li><p>Already [a list item</p></li>
</ul>
<p>A paragraph</p>
<p>Another] paragraph</p>

AFTER (consensus approach):
<ul>
  <li><p>Already [a list item</p></li>
  <li><p>A paragraph</p></li>
  <li><p>Another] paragraph</p></li>
</ul>
```

**Behavior:** When some content is already in a list and some is not, the non-list
content is converted to list items and merged into the list. The existing list items
remain as-is.

**Divergence:**
- Some editors (Tiptap with clearNodes approach) first clear all nodes to paragraphs,
  then wrap everything in a new list.
- Other implementations try to extend the existing list to encompass the new items.
- Google Docs extends the existing list.

### 2.6 Partial Selection Within a List

**Action:** Select only middle items of a list, toggle list off.

```
BEFORE:
<ul>
  <li><p>Keep this</p></li>
  <li><p>Remove [this</p></li>
  <li><p>Remove] this</p></li>
  <li><p>Keep this</p></li>
</ul>

AFTER:
<ul>
  <li><p>Keep this</p></li>
</ul>
<p>Remove [this</p>
<p>Remove] this</p>
<ul>
  <li><p>Keep this</p></li>
</ul>
```

The list is split into two separate lists with the lifted items in between.

**ProseMirror:** `liftListItem` handles this by lifting the selected items out,
which automatically splits the parent list.

### 2.7 Nested Lists: Lift Inner List

**Action:** Toggle list off on a nested (indented) list item.

```
BEFORE:
<ul>
  <li>
    <p>Parent item</p>
    <ul>
      <li><p>Nested| item</p></li>
    </ul>
  </li>
</ul>

AFTER (one level of lift):
<ul>
  <li><p>Parent item</p></li>
  <li><p>Nested| item</p></li>
</ul>
```

Lifting a nested item moves it up one level in the list hierarchy. It does NOT
exit the list entirely. To exit completely, lift must be applied again.

**ProseMirror:** `liftListItem` lifts one level at a time. When the parent is another
list, the item becomes a sibling of its former parent. When the parent is NOT a list
(i.e., the item is at the top level), the item exits the list entirely.

### 2.8 Nested Lists: Toggle Off Entire Nested Structure

**Action:** Select all items in a nested list, toggle list off.

```
BEFORE:
<ul>
  <li>
    <p>Parent [item</p>
    <ul>
      <li><p>Child item</p></li>
    </ul>
  </li>
  <li><p>Sibling] item</p></li>
</ul>

AFTER:
<p>Parent [item</p>
<p>Child item</p>
<p>Sibling] item</p>
```

All list structure is removed. Nested items are flattened to paragraphs.

### 2.9 Edge Case: Adjacent Lists Auto-Join

```
BEFORE:
<ul>
  <li><p>Item A</p></li>
</ul>
<p>Convert| me</p>
<ul>
  <li><p>Item B</p></li>
</ul>

AFTER (wrapping middle paragraph):
<ul>
  <li><p>Item A</p></li>
  <li><p>Convert| me</p></li>
  <li><p>Item B</p></li>
</ul>
```

**ProseMirror:** Adjacent lists of the same type are automatically joined after
wrapping operations. This is a property of the transaction system -- when a `wrapInList`
creates a list adjacent to an existing list of the same type, they merge.

---

## 3. Blockquote

**Commands:** ProseMirror `wrapIn`/`lift`, Tiptap `toggleBlockquote`/`setBlockquote`/`unsetBlockquote`

### 3.1 Wrap Single Paragraph in Blockquote

**Action:** Toggle blockquote on a paragraph.

```
BEFORE:
<p>A quoted| passage</p>

AFTER:
<blockquote>
  <p>A quoted| passage</p>
</blockquote>
```

### 3.2 Wrap Multiple Paragraphs in Blockquote

**Action:** Select across multiple paragraphs, toggle blockquote.

```
BEFORE:
<p>First [paragraph</p>
<p>Second paragraph</p>
<p>Third] paragraph</p>

AFTER:
<blockquote>
  <p>First [paragraph</p>
  <p>Second paragraph</p>
  <p>Third] paragraph</p>
</blockquote>
```

All selected paragraphs are wrapped in a single blockquote. They are NOT individually
wrapped.

### 3.3 Unwrap Blockquote (Toggle Off)

**Action:** Toggle blockquote when cursor is inside a blockquote.

```
BEFORE:
<blockquote>
  <p>Quoted| text</p>
  <p>More quoted text</p>
</blockquote>

AFTER:
<p>Quoted| text</p>
<p>More quoted text</p>
```

**ProseMirror:** The `lift` command removes the wrapping blockquote. All content inside
the blockquote is lifted to the parent level.

**Tiptap:** `toggleBlockquote` detects whether the selection is already inside a blockquote
and lifts if so.

### 3.4 Partial Unwrap (Selection Within Blockquote)

**Action:** Select only some paragraphs within a blockquote, toggle off.

```
BEFORE:
<blockquote>
  <p>Keep quoted</p>
  <p>Lift [this</p>
  <p>Lift] this</p>
  <p>Keep quoted</p>
</blockquote>

AFTER:
<blockquote>
  <p>Keep quoted</p>
</blockquote>
<p>Lift [this</p>
<p>Lift] this</p>
<blockquote>
  <p>Keep quoted</p>
</blockquote>
```

The blockquote is split, with the selected paragraphs lifted out.

### 3.5 Nested Blockquotes

**Action:** Toggle blockquote when already inside a blockquote.

```
BEFORE:
<blockquote>
  <p>Outer| quote</p>
</blockquote>

AFTER (wrapIn behavior):
<blockquote>
  <blockquote>
    <p>Outer| quote</p>
  </blockquote>
</blockquote>
```

**Divergence -- this is where implementations differ significantly:**

- **ProseMirror `wrapIn`:** Always wraps. Applying `wrapIn(blockquote)` to content
  already in a blockquote creates a nested blockquote. There is no built-in toggle.
- **Tiptap `toggleBlockquote`:** Toggles. If already in a blockquote, it lifts (unwraps)
  rather than nesting.
- **Google Docs:** Does not support nested blockquotes. Toggle removes the quote.
- **Notion:** Each block is independent. Toggling quote on a quoted block removes it.

**Recommended behavior (consensus):** Toggle semantics. If the selection is already
inside a blockquote, unwrap it. Do not create nested blockquotes unless explicitly
requested.

### 3.6 Blockquote Containing a List

**Action:** Toggle blockquote on content that includes a list.

```
BEFORE:
<p>Intro [text</p>
<ul>
  <li><p>Item one</p></li>
  <li><p>Item] two</p></li>
</ul>

AFTER:
<blockquote>
  <p>Intro [text</p>
  <ul>
    <li><p>Item one</p></li>
    <li><p>Item] two</p></li>
  </ul>
</blockquote>
```

The entire selection including the list is wrapped. The list structure inside the
blockquote is preserved.

### 3.7 Unwrap Blockquote Containing a List

```
BEFORE:
<blockquote>
  <p>Intro| text</p>
  <ul>
    <li><p>Item one</p></li>
  </ul>
</blockquote>

AFTER:
<p>Intro| text</p>
<ul>
  <li><p>Item one</p></li>
</ul>
```

The blockquote wrapper is removed. All inner structure is preserved.

---

## 4. Indent/Outdent

### 4.1 Indent List Item (Sink)

**Action:** Indent a list item (Tab or Cmd/Ctrl+]).

**Precondition:** The item must have a preceding sibling in the same list (cannot indent
the first item).

```
BEFORE:
<ul>
  <li><p>Item one</p></li>
  <li><p>Item| two</p></li>
  <li><p>Item three</p></li>
</ul>

AFTER:
<ul>
  <li>
    <p>Item one</p>
    <ul>
      <li><p>Item| two</p></li>
    </ul>
  </li>
  <li><p>Item three</p></li>
</ul>
```

The indented item becomes a child of the preceding list item. A new nested list is
created inside the preceding item.

**ProseMirror (`sinkListItem`):** Uses `ReplaceAroundStep` to restructure the document.
The item moves into a new `<ul>`/`<ol>` nested inside the preceding `<li>`.

### 4.2 Indent List Item: Preceding Sibling Already Has Nested List

```
BEFORE:
<ul>
  <li>
    <p>Item one</p>
    <ul>
      <li><p>Existing nested</p></li>
    </ul>
  </li>
  <li><p>Item| two</p></li>
</ul>

AFTER:
<ul>
  <li>
    <p>Item one</p>
    <ul>
      <li><p>Existing nested</p></li>
      <li><p>Item| two</p></li>
    </ul>
  </li>
</ul>
```

The item is appended to the existing nested list rather than creating a new one.

### 4.3 Outdent List Item (Lift)

**Action:** Outdent a nested list item (Shift+Tab or Cmd/Ctrl+[).

```
BEFORE:
<ul>
  <li>
    <p>Parent item</p>
    <ul>
      <li><p>Nested| item</p></li>
    </ul>
  </li>
</ul>

AFTER:
<ul>
  <li><p>Parent item</p></li>
  <li><p>Nested| item</p></li>
</ul>
```

The item moves up one level, becoming a sibling of its former parent.

**Children are preserved:** If the lifted item has its own nested list, that nested list
moves with it.

### 4.4 Outdent Top-Level List Item

**Action:** Outdent a list item that is already at the top level.

```
BEFORE:
<ul>
  <li><p>Item| one</p></li>
  <li><p>Item two</p></li>
</ul>

AFTER (ProseMirror/Tiptap):
<p>Item| one</p>
<ul>
  <li><p>Item two</p></li>
</ul>
```

The item exits the list entirely and becomes a paragraph.

**ProseMirror:** `liftListItem` at the top level lifts the item out of the list,
converting it to a paragraph.

**Divergence:**
- **Google Docs / MS Word:** Outdenting a top-level list item does NOT remove it from
  the list. The item stays as a list item at nesting level 0. You must explicitly remove
  the list formatting.
- **ProseMirror / Tiptap:** Outdenting a top-level item removes it from the list entirely.
- **W3C `outdent`:** The spec notes that outdent should never cause a list item to stop
  being a list item (aligned with Google Docs behavior). However, browser implementations
  vary.

### 4.5 Outdent Nested Item with Children

```
BEFORE:
<ul>
  <li>
    <p>Parent</p>
    <ul>
      <li>
        <p>Child|</p>
        <ul>
          <li><p>Grandchild</p></li>
        </ul>
      </li>
    </ul>
  </li>
</ul>

AFTER:
<ul>
  <li><p>Parent</p></li>
  <li>
    <p>Child|</p>
    <ul>
      <li><p>Grandchild</p></li>
    </ul>
  </li>
</ul>
```

The child (with its own nested list) is lifted to be a sibling of Parent. The grandchild
subtree moves with it.

### 4.6 Indent/Outdent in Blockquotes

**Action:** Indent content inside a blockquote (non-list context).

**Divergence:** Behavior varies significantly:
- **Google Docs:** Indent increases the left margin (visual indentation).
- **ProseMirror:** No built-in indent command for non-list content. The `wrapIn` command
  can be used but creates nested blockquotes (see Section 3.5).
- **W3C `indent`:** Wraps content in a `<blockquote>` element.
- **Notion:** Tab key nests the block as a child of the block above (structural nesting,
  not visual indentation).

**Recommended behavior:** For non-list content, indent is either unsupported or creates
visual indentation via margin styles. Do not use nested blockquotes for indentation.

### 4.7 Indent at Top Level (Non-List, Non-Blockquote)

**Action:** Press Tab on a regular paragraph.

```
BEFORE:
<p>Regular| paragraph</p>

AFTER (varies by editor):
```

**Divergences:**
- **Google Docs:** Inserts a tab character or increases indent level.
- **ProseMirror:** No default behavior. Tab is typically consumed by the browser for
  focus navigation unless explicitly handled.
- **Tiptap:** No default behavior for Tab on paragraphs.
- **Notion:** Nests the block under the preceding block.

---

## 5. Enter Key

### 5.1 Split Paragraph in the Middle

**Action:** Press Enter in the middle of a paragraph.

```
BEFORE:
<p>Hello| world</p>

AFTER:
<p>Hello</p>
<p>|world</p>
```

The paragraph splits at the cursor position. Inline marks on the text before the cursor
stay on the first paragraph. The cursor moves to the start of the new paragraph.

**ProseMirror (`splitBlock`):** Deletes any selected content, then splits the parent
block at the cursor depth. Validates that the schema allows splitting.

### 5.2 Enter at End of Paragraph

**Action:** Press Enter at the end of a paragraph.

```
BEFORE:
<p>Hello world|</p>

AFTER:
<p>Hello world</p>
<p>|</p>
```

A new empty paragraph is created after the current one.

### 5.3 Enter at Start of Paragraph

**Action:** Press Enter at the start of a paragraph.

```
BEFORE:
<p>|Hello world</p>

AFTER:
<p></p>
<p>|Hello world</p>
```

An empty paragraph is created before the current one. The cursor stays with the text.

### 5.4 Enter at End of Heading

**Action:** Press Enter at the end of a heading.

```
BEFORE:
<h2>My Heading|</h2>

AFTER:
<h2>My Heading</h2>
<p>|</p>
```

**Consensus behavior:** A new PARAGRAPH (not heading) is created after the heading.
This is the universal behavior across all editors: Google Docs, Notion, ProseMirror
(with proper keymap), Tiptap, Word, etc.

**ProseMirror:** The default `splitBlock` would create another `<h2>`. The standard
keymap uses `splitBlockAs` or a custom command that checks if the cursor is at the end
of a heading and creates a paragraph instead. Many implementations use the "default type"
concept -- the schema defines which node type is the default for a given context.

**Tiptap:** The Heading extension overrides Enter to create a paragraph after a heading
when the cursor is at the end.

### 5.5 Enter in the Middle of a Heading

**Action:** Press Enter in the middle of a heading.

```
BEFORE:
<h2>My He|ading</h2>

AFTER:
<h2>My He</h2>
<h2>|ading</h2>
```

The heading splits into two headings. This is distinct from Enter at the end.

**Divergence:**
- **ProseMirror/Tiptap:** Splits into two headings of the same level.
- **Google Docs:** Splits into two headings of the same level.
- **Notion:** Each block is independent. Enter creates a new block which inherits the type.

### 5.6 Enter in Empty Heading

**Action:** Press Enter in an empty heading.

```
BEFORE:
<h2>|</h2>

AFTER:
<p>|</p>
```

**Consensus:** An empty heading converts to a paragraph on Enter. This is the
"exit heading mode" behavior.

**ProseMirror:** The `liftEmptyBlock` command in the default Enter chain handles this.
When the cursor is in an empty textblock that can be lifted or type-changed, it resets
to the default block type (paragraph). Custom implementations check
`$from.parent.type == heading && textContent.length == 0` and use `setBlockType` to
convert to paragraph.

### 5.7 Split List Item

**Action:** Press Enter in the middle of a list item.

```
BEFORE:
<ul>
  <li><p>Hello| world</p></li>
</ul>

AFTER:
<ul>
  <li><p>Hello</p></li>
  <li><p>| world</p></li>
</ul>
```

The list item splits into two. A new `<li>` is created.

**ProseMirror (`splitListItem`):** Splits the list item at the cursor position within
the paragraph. The new list item contains the content after the cursor.

### 5.8 Enter at End of List Item

**Action:** Press Enter at the end of a list item.

```
BEFORE:
<ul>
  <li><p>Item one|</p></li>
  <li><p>Item two</p></li>
</ul>

AFTER:
<ul>
  <li><p>Item one</p></li>
  <li><p>|</p></li>
  <li><p>Item two</p></li>
</ul>
```

A new empty list item is created between the current and next items.

### 5.9 Enter in Empty List Item (Exit List)

**Action:** Press Enter in an empty list item.

```
BEFORE:
<ul>
  <li><p>Item one</p></li>
  <li><p>|</p></li>
</ul>

AFTER:
<ul>
  <li><p>Item one</p></li>
</ul>
<p>|</p>
```

**Consensus behavior:** An empty list item is removed and the cursor exits the list,
creating a paragraph after it. This is the standard "exit list" gesture.

**Implementation:**
- ProseMirror: The default Enter keymap chains `splitListItem` with `liftEmptyBlock`.
  When the list item is empty, `splitListItem` does not apply, and `liftEmptyBlock`
  kicks in, lifting the empty block out of the list.
- Tiptap: Same behavior via the ListKeymap extension.
- Google Docs: Pressing Enter on an empty bullet removes it and creates a paragraph.
- Notion: Same behavior.

### 5.10 Enter in Empty Nested List Item

**Action:** Press Enter in an empty nested list item.

```
BEFORE:
<ul>
  <li>
    <p>Parent</p>
    <ul>
      <li><p>|</p></li>
    </ul>
  </li>
</ul>

AFTER:
<ul>
  <li><p>Parent</p></li>
  <li><p>|</p></li>
</ul>
```

**Behavior:** The empty nested item is outdented one level (not removed from the list
entirely). It becomes a sibling of the parent item.

**This matches the general principle:** Enter in an empty list item performs a lift/outdent.
If nested, it outdents one level. If already at top level, it exits the list.

### 5.11 Enter in List Item with Nested Children

**Action:** Press Enter at the end of a list item that has a nested sublist.

```
BEFORE:
<ul>
  <li>
    <p>Parent item|</p>
    <ul>
      <li><p>Child item</p></li>
    </ul>
  </li>
</ul>

AFTER:
<ul>
  <li>
    <p>Parent item</p>
    <ul>
      <li><p>|</p></li>
      <li><p>Child item</p></li>
    </ul>
  </li>
</ul>
```

**Note:** The new empty list item is created as the first child of the nested list,
not as a new sibling of the parent. This matches the visual position of the cursor
(the next line is indented).

**Divergence:** Behavior here varies. Some implementations create a new sibling after
the parent item instead. The ProseMirror `splitListItem` command splits the list item,
and the nested list stays with the second part.

### 5.12 Enter in Blockquote

**Action:** Press Enter in the middle of a paragraph inside a blockquote.

```
BEFORE:
<blockquote>
  <p>First| second</p>
</blockquote>

AFTER:
<blockquote>
  <p>First</p>
  <p>|second</p>
</blockquote>
```

The paragraph splits within the blockquote. The blockquote is NOT exited.

### 5.13 Enter in Empty Paragraph Inside Blockquote (Exit Blockquote)

**Action:** Press Enter in an empty paragraph inside a blockquote.

```
BEFORE:
<blockquote>
  <p>Some text</p>
  <p>|</p>
</blockquote>

AFTER:
<blockquote>
  <p>Some text</p>
</blockquote>
<p>|</p>
```

**Consensus:** Pressing Enter in an empty block inside a blockquote exits the blockquote.
The empty paragraph is lifted out and placed after the blockquote.

**ProseMirror:** The `liftEmptyBlock` command handles this. When the empty paragraph
can be lifted out of the blockquote, it is.

---

## 6. Backspace/Delete

### 6.1 Backspace at Start of Paragraph (Merge with Previous)

**Action:** Press Backspace at the start of a paragraph.

```
BEFORE:
<p>First paragraph</p>
<p>|Second paragraph</p>

AFTER:
<p>First paragraph|Second paragraph</p>
```

The two paragraphs merge. The content of the second paragraph is appended to the first.
The cursor is placed at the join point.

**ProseMirror (`joinBackward`):** Finds the "cut" point before the current block, then
attempts to join the two blocks. If the blocks are of compatible types, their content
merges. If not, it tries to move inline content from the current block into the last
child of the preceding structure.

### 6.2 Backspace at Start of Heading (Convert to Paragraph)

**Action:** Press Backspace at the start of a heading (first block in document).

```
BEFORE:
<h2>|My Heading</h2>

AFTER:
<p>|My Heading</p>
```

**Consensus:** Backspace at the start of a heading when there is no preceding block
converts the heading to a paragraph.

**When there IS a preceding block:**

```
BEFORE:
<p>Previous paragraph</p>
<h2>|My Heading</h2>

AFTER:
<p>Previous paragraph|My Heading</p>
```

The heading content merges into the previous paragraph. The heading type is lost.

**ProseMirror:** `joinBackward` merges the heading's content into the preceding paragraph.
The resulting block takes the type of the first (preceding) block.

### 6.3 Backspace at Start of List Item (First Item)

**Action:** Press Backspace at the start of the first list item.

```
BEFORE:
<ul>
  <li><p>|First item</p></li>
  <li><p>Second item</p></li>
</ul>

AFTER:
<p>|First item</p>
<ul>
  <li><p>Second item</p></li>
</ul>
```

The first list item is converted to a paragraph. The remaining items stay as a list.

**ProseMirror:** `joinBackward` at the start of a list item will try to lift the item
out of the list structure. Since there is no preceding list item to merge with, the
item exits the list.

**Tiptap (with ListKeymap):** Pressing backspace at the start of a list item lifts the
content into the preceding context.

### 6.4 Backspace at Start of Non-First List Item

**Action:** Press Backspace at the start of a list item that has a preceding sibling.

```
BEFORE:
<ul>
  <li><p>First item</p></li>
  <li><p>|Second item</p></li>
</ul>

AFTER (Tiptap ListKeymap):
<ul>
  <li><p>First item|Second item</p></li>
</ul>
```

**Tiptap ListKeymap behavior:** The content is lifted into the previous list item,
merging the paragraphs.

**ProseMirror default (without ListKeymap):** `joinBackward` attempts to join with the
preceding block. If list items contain paragraphs, the paragraph of the second item
merges into the paragraph of the first item.

**Divergence:**
- **Without ListKeymap:** Backspace may convert the item to a paragraph (lifting out of
  the list) rather than merging with the previous item.
- **With ListKeymap:** Content merges into the previous list item.
- **Google Docs:** Merges into the previous list item.

### 6.5 Backspace at Start of Nested List Item

**Action:** Press Backspace at the start of a nested list item.

```
BEFORE:
<ul>
  <li>
    <p>Parent</p>
    <ul>
      <li><p>|Nested item</p></li>
    </ul>
  </li>
</ul>

AFTER:
<ul>
  <li><p>Parent</p></li>
  <li><p>|Nested item</p></li>
</ul>
```

The nested item is outdented one level (lifted to become a sibling of the parent).

**Alternative result (merge into parent):**

```
AFTER (some implementations):
<ul>
  <li><p>Parent|Nested item</p></li>
</ul>
```

**Divergence:**
- **ProseMirror default:** May try to join with the parent paragraph, merging text.
- **Tiptap ListKeymap:** Lifts/outdents the nested item.
- **Google Docs:** Outdents the nested item.

### 6.6 Delete at End of Paragraph (Merge with Next)

**Action:** Press Delete at the end of a paragraph.

```
BEFORE:
<p>First paragraph|</p>
<p>Second paragraph</p>

AFTER:
<p>First paragraph|Second paragraph</p>
```

Same as backspace at start of next block: the two paragraphs merge.

**ProseMirror (`joinForward`):** Finds the cut point after the current block and
attempts to join. The inline content of the next block is pulled into the current block.

### 6.7 Delete at End of Paragraph Before a Heading

```
BEFORE:
<p>Paragraph|</p>
<h2>Heading text</h2>

AFTER:
<p>Paragraph|Heading text</p>
```

The heading content is pulled into the paragraph. The heading is removed. The resulting
block is a paragraph (the type of the first block).

### 6.8 Delete at End of Paragraph Before a List

```
BEFORE:
<p>Paragraph|</p>
<ul>
  <li><p>First item</p></li>
  <li><p>Second item</p></li>
</ul>

AFTER:
<p>Paragraph|First item</p>
<ul>
  <li><p>Second item</p></li>
</ul>
```

The first list item's content is merged into the paragraph. The first list item is
removed. Remaining list items stay as a list.

### 6.9 Delete Selection Spanning Multiple Blocks

**Action:** Delete a selection that spans multiple blocks.

```
BEFORE:
<p>First [paragraph</p>
<p>Second paragraph</p>
<p>Third] paragraph</p>

AFTER:
<p>First |paragraph</p>
```

Wait -- more precisely:

```
AFTER:
<p>First | paragraph</p>
```

The selected content is deleted. The remaining content from the first and last blocks
is merged into a single block. The block takes the type of the first block.

**ProseMirror (`deleteSelection`):** Deletes the selected range. The replace operation
automatically merges the remaining prefix of the first block with the remaining suffix
of the last block, using the structure (type) of the first block.

### 6.10 Delete Selection Spanning Block Types

```
BEFORE:
<h2>Head[ing</h2>
<p>Para]graph</p>

AFTER:
<h2>Head|graph</h2>
```

The selection is deleted. The remaining content merges into a single block with the
type of the first block (heading).

### 6.11 Delete Selection Spanning Into/Out of a List

```
BEFORE:
<p>Para[graph</p>
<ul>
  <li><p>First] item</p></li>
  <li><p>Second item</p></li>
</ul>

AFTER:
<p>Para| item</p>
<ul>
  <li><p>Second item</p></li>
</ul>
```

The paragraph absorbs the remaining content of the first list item. The first list
item is removed.

### 6.12 Delete Selection Spanning Multiple List Items

```
BEFORE:
<ul>
  <li><p>First [item</p></li>
  <li><p>Second item</p></li>
  <li><p>Third] item</p></li>
</ul>

AFTER:
<ul>
  <li><p>First | item</p></li>
</ul>
```

The selected content across list items is deleted. The remaining content merges into
the first list item.

### 6.13 Backspace at Start of Blockquote Content

**Action:** Press Backspace at the start of the first paragraph inside a blockquote.

```
BEFORE:
<p>Before the quote</p>
<blockquote>
  <p>|Quoted text</p>
</blockquote>

AFTER:
<p>Before the quote|Quoted text</p>
```

**OR (depending on implementation):**

```
AFTER:
<p>Before the quote</p>
<p>|Quoted text</p>
```

**Divergence:**
- **ProseMirror:** `joinBackward` may merge with the preceding paragraph (pulling content
  out of the blockquote and into the paragraph above).
- **Some editors:** Lift the paragraph out of the blockquote first, then on second
  backspace merge with the preceding paragraph.
- **Google Docs:** Removes the blockquote formatting first.

### 6.14 Backspace at Very Start of Document

**Action:** Press Backspace at position 0 of the document.

```
BEFORE:
<p>|First paragraph</p>

AFTER:
<p>|First paragraph</p>  (no change)
```

No operation. Backspace at the start of the document with no preceding content is a no-op.

**Exception:** If the first block is a heading or other non-paragraph type:

```
BEFORE:
<h2>|Heading</h2>

AFTER:
<p>|Heading</p>
```

Some implementations convert the first block to a paragraph, matching the behavior of
"backspace at start of heading."

---

## 7. Tab in Lists

### 7.1 Tab to Indent List Item

**Action:** Press Tab while cursor is in a list item.

```
BEFORE:
<ul>
  <li><p>Item one</p></li>
  <li><p>Item| two</p></li>
</ul>

AFTER:
<ul>
  <li>
    <p>Item one</p>
    <ul>
      <li><p>Item| two</p></li>
    </ul>
  </li>
</ul>
```

Same as indent (Section 4.1). Tab is the standard keyboard shortcut for indenting list items.

**Keyboard bindings:**
- ProseMirror: `Tab` -> `sinkListItem`, `Shift+Tab` -> `liftListItem`
- Tiptap: Same bindings via the ListItem extension
- Google Docs: `Tab` indents, `Shift+Tab` outdents

### 7.2 Shift+Tab to Outdent List Item

**Action:** Press Shift+Tab in a nested list item.

```
BEFORE:
<ul>
  <li>
    <p>Parent</p>
    <ul>
      <li><p>Nested| item</p></li>
    </ul>
  </li>
</ul>

AFTER:
<ul>
  <li><p>Parent</p></li>
  <li><p>Nested| item</p></li>
</ul>
```

Same as outdent (Section 4.3).

### 7.3 Tab on First List Item (Cannot Indent)

**Action:** Press Tab on the first item in a list (no preceding sibling).

```
BEFORE:
<ul>
  <li><p>|First item</p></li>
  <li><p>Second item</p></li>
</ul>

AFTER:
<ul>
  <li><p>|First item</p></li>
  <li><p>Second item</p></li>
</ul>
```

**No change.** The first item in a list cannot be indented because there is no preceding
sibling to nest under.

**ProseMirror:** `sinkListItem` returns false (command not applicable). The event may
propagate to default Tab behavior.

### 7.4 Tab with Multiple Items Selected

**Action:** Press Tab with selection spanning multiple list items.

```
BEFORE:
<ul>
  <li><p>Item one</p></li>
  <li><p>Item [two</p></li>
  <li><p>Item] three</p></li>
</ul>

AFTER:
<ul>
  <li>
    <p>Item one</p>
    <ul>
      <li><p>Item [two</p></li>
      <li><p>Item] three</p></li>
    </ul>
  </li>
</ul>
```

All selected items are indented together as a group under the preceding item.

### 7.5 Shift+Tab on Top-Level List Item

**Action:** Press Shift+Tab on a top-level list item.

```
BEFORE:
<ul>
  <li><p>Item| one</p></li>
  <li><p>Item two</p></li>
</ul>

AFTER (ProseMirror/Tiptap):
<p>Item| one</p>
<ul>
  <li><p>Item two</p></li>
</ul>
```

The item exits the list entirely.

**Divergence:** Same as Section 4.4. Google Docs does not exit the list on Shift+Tab
at the top level. ProseMirror/Tiptap do.

### 7.6 Tab Outside of Lists

**Action:** Press Tab in a regular paragraph (not in a list).

**Behavior varies:**
- **ProseMirror:** No default handling. Tab may be consumed for focus navigation.
- **Tiptap:** No default handling.
- **Google Docs:** Inserts a tab character.
- **Notion:** Nests the block under the preceding block.

---

## Summary of Key Divergences

| Operation | ProseMirror/Tiptap | Google Docs | Notion |
|-----------|-------------------|-------------|--------|
| Outdent top-level list item | Exits list | Stays as list item | Exits list |
| Backspace at start of heading | Merge with previous OR convert to paragraph | Convert to paragraph | Convert to paragraph |
| Enter at end of heading | New paragraph | New paragraph | New paragraph |
| Nested blockquotes | Possible (wrap creates nesting) | Not supported | Not supported |
| Tab outside lists | No default action | Insert tab character | Nest block |
| Toggle blockquote (already quoted) | Unwrap (Tiptap) / wrap again (raw PM) | Unwrap | Unwrap |
| Backspace at start of list item | Lift out (PM) / merge up (ListKeymap) | Outdent or merge | Outdent |
| Enter in empty heading | Convert to paragraph | Convert to paragraph | New text block |
| Mixed list+paragraph toggle | Varies by implementation | Extend list | Convert all to list |

---

## Appendix A: ProseMirror Default Enter Keymap Chain

The default ProseMirror Enter key binding uses `chainCommands`:

```
Enter: chainCommands(
  newlineInCode,        // Insert \n in code blocks
  createParagraphNear,  // Create paragraph adjacent to non-text blocks
  liftEmptyBlock,       // Lift empty blocks out of parents (exit list/blockquote)
  splitBlock             // Split the current block at cursor
)
```

Commands are tried in order. The first one that returns `true` handles the event.

For list-aware editors, `splitListItem` is typically inserted before `liftEmptyBlock`:

```
Enter: chainCommands(
  newlineInCode,
  createParagraphNear,
  splitListItem(schema.nodes.list_item),
  liftEmptyBlock,
  splitBlock
)
```

## Appendix B: ProseMirror Default Backspace Keymap Chain

```
Backspace: chainCommands(
  deleteSelection,       // Delete non-empty selection
  joinBackward,          // Join with preceding block
  selectNodeBackward     // Select preceding node if unjoinable
)
```

## Appendix C: ProseMirror Default Delete Keymap Chain

```
Delete: chainCommands(
  deleteSelection,      // Delete non-empty selection
  joinForward,          // Join with following block
  selectNodeForward     // Select following node if unjoinable
)
```

## Appendix D: ProseMirror Document Model for Lists

ProseMirror uses a hierarchical model where list items contain block content:

```
bullet_list / ordered_list
  └── list_item
       ├── paragraph (first child, required)
       └── bullet_list / ordered_list (optional, for nesting)
```

Key constraints:
- The first child of a list item is always a paragraph (or equivalent textblock).
- Nested lists are additional children of the list item, after the paragraph.
- A list cannot directly contain another list (must be inside a list item).
- List items cannot be empty -- they must contain at least one paragraph.

This model differs from Notion (flat block list with nesting levels) and some other
editors (CKEditor 5 uses a flat list model with indent attributes).

## Appendix E: Keyboard Shortcut Summary

| Action | Shortcut | ProseMirror Command |
|--------|----------|-------------------|
| Split block | Enter | `splitBlock` |
| Split list item | Enter (in list) | `splitListItem` |
| Join backward | Backspace | `joinBackward` |
| Join forward | Delete | `joinForward` |
| Indent list item | Tab | `sinkListItem` |
| Outdent list item | Shift+Tab | `liftListItem` |
| Indent list item | Ctrl/Cmd+] | `sinkListItem` |
| Outdent list item | Ctrl/Cmd+[ | `liftListItem` |
| Toggle blockquote | Ctrl+Shift+B | `toggleWrap(blockquote)` |
| Set heading 1 | Ctrl+Alt+1 | `setBlockType(heading, {level:1})` |
| Set heading 2 | Ctrl+Alt+2 | `setBlockType(heading, {level:2})` |
| Set paragraph | Ctrl+Alt+0 | `setBlockType(paragraph)` |
| Wrap in list | n/a (toolbar) | `wrapInList` |
| Lift (unwrap) | n/a (toolbar) | `lift` |
