#!/usr/bin/env python3
"""Audit docs/src/guide/component-props.md against the real component structs.

Why this exists (issue #208): the doc is hand-maintained prose + tables, not
generated, and it has drifted from `crates/rinch-components/src/*.rs` before —
most recently, hundreds of rows typed text props as `Option<String>` when the
newcomer-DX change made them bare `String` (empty string = "not set"). A full
file-generator was rejected for this same reason a *human* edit was rejected:
much of the file is hand-written prose (behavioral notes, "Custom Default:"
callouts, worked examples) that no mechanical pass can reproduce faithfully.
This script instead audits: it extracts the real prop list (name + exact type)
for every `#[derive(...)] pub struct X { ... }` in rinch-components that
implements `Component`, and cross-references it against every prop row/mention
in component-props.md, reporting every disagreement for a human to fix.

Usage:
    python3 scripts/audit_component_props_doc.py

Run from the repo root. No cargo, no build — pure source-text parsing (regex
over the Rust struct definitions, regex over the markdown tables). Exits 0
always; this is a report, not a gate — read the sections below and use
judgment, because a few categories of "finding" are expected/benign (see
KNOWN NON-ISSUES below the report).

KNOWN NON-ISSUES you will still see after a clean pass (verified by hand as of
the #208 fix, keep this list updated if you triage a new one):
  - `Option<Callback>` (doc) vs `Option<rinch_core::Callback>` (source) on a
    handful of components (Drawer, DropdownMenuItem, Modal, NavLink,
    Notification) — same type, those files just spell it out fully instead of
    `use`-importing `Callback`. Not a doc bug.
  - The "Callback Types Reference" table at the end of the doc is a 3-column
    reference table (Type/Signature/Used By), not a per-component prop table —
    this script's heading-tracker attributes its rows to whatever `###`
    heading precedes it (BorderlessWindow) and reports them as "not in that
    struct". Ignore.
  - Unit structs (`pub struct Foo;`) with zero fields, documented as
    "**Foo:** No props." — these have no fields to compare against, so they
    show up as an unmatched doc heading rather than a verified match. Ignore
    if the source struct genuinely has no fields.
  - "Components implementing Component but missing from the doc" and "fields
    present in source but missing from a documented table" are real gaps but
    are NOT necessarily #208-scoped type bugs — they're omissions. Check
    whether they're worth an in-scope fix or a separate follow-up issue before
    touching prose/structure.
"""
import json
import os
import re
import sys
from collections import defaultdict

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC_DIR = os.path.join(REPO_ROOT, "crates", "rinch-components", "src")
DOC = os.path.join(REPO_ROOT, "docs", "src", "guide", "component-props.md")

# Structs that are real Rust types in rinch-components but are NOT RSX
# components with a props table of their own (internal data/color-math types,
# or a struct whose sole field is a wrapped sibling component). Skip them.
NON_COMPONENT_STRUCTS = {
    "Hsla", "Hsva", "Rgba",              # color_utils.rs — color math, not a component
    "RenderTreeNodePayload",             # tree.rs — internal payload wrapper type
    "UseTreeOptions", "UseTreeReturn",   # tree.rs — state handles, not props
    "SelectOption",                      # select.rs — plain data row, not a Component
    "ModalRoot",                         # modal.rs — internal `{ modal: Modal }` wrapper, not re-exported
}

FIELD_RE = re.compile(
    r"(?P<doc>(?:\s*///[^\n]*\n)*)"
    r"\s*(?:\#\[[^\]]*\]\s*\n\s*)*"
    r"pub (?P<name>(?:r\#)?[A-Za-z_][A-Za-z0-9_]*)\s*:\s*(?P<type>[^,\n]+?)\s*,",
)


def find_struct_body(text, name):
    pat = re.compile(r"\bpub struct " + re.escape(name) + r"\b[^{;]*\{")
    m = pat.search(text)
    if not m:
        return None
    start = m.end()
    depth = 1
    i = start
    while i < len(text) and depth > 0:
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
        i += 1
    return text[start : i - 1]


def parse_fields(body):
    fields = []
    for m in FIELD_RE.finditer(body):
        name = m.group("name")
        if name.startswith("r#"):
            name = name[2:]
        fields.append({"name": name, "type": m.group("type").strip()})
    return fields


def extract_source():
    """Returns {struct_name: {'file': ..., 'fields': [...], 'is_component': bool}}."""
    results = {}
    component_impls = set()
    for fname in sorted(os.listdir(SRC_DIR)):
        if not fname.endswith(".rs"):
            continue
        with open(os.path.join(SRC_DIR, fname)) as f:
            text = f.read()
        component_impls |= set(re.findall(r"impl Component for (\w+)", text))
        for sname in set(re.findall(r"\bpub struct (\w+)\b[^;{]*\{", text)):
            body = find_struct_body(text, sname)
            if body is None:
                continue
            fields = parse_fields(body)
            if fields:
                results.setdefault(sname, {"file": fname, "fields": fields})
    for sname, info in results.items():
        info["is_component"] = sname in component_impls
    return results, component_impls


def extract_doc_rows():
    """Returns a list of {component, prop, type, line_no, kind} across the doc."""
    with open(DOC) as f:
        lines = f.readlines()

    doc_rows = []
    current_heading = None
    current_bold = None
    for i, line in enumerate(lines):
        s = line.strip()

        h = re.match(r"^###\s+(.+)$", s)
        if h:
            current_heading = h.group(1).strip()
            current_bold = None
            continue

        b = re.match(r"^\*\*([A-Za-z0-9_]+):\*\*\s*$", s)
        if b:
            current_bold = b.group(1)
            continue

        multi_none = re.match(
            r"^(\*\*[A-Za-z0-9_]+\*\*(?:,\s*\*\*[A-Za-z0-9_]+\*\*)*)\s*:\s*No props\.\s*$", s
        )
        if multi_none:
            for nm in re.findall(r"\*\*([A-Za-z0-9_]+)\*\*", multi_none.group(1)):
                doc_rows.append({"component": nm, "prop": None, "type": None, "line_no": i + 1, "kind": "no-props"})
            continue

        inline = re.match(r"^\*\*([A-Za-z0-9_]+):\*\*\s*(.+)$", s)
        if inline:
            name, rest = inline.group(1), inline.group(2)
            if re.match(r"^no props\.?$", rest.strip(), re.IGNORECASE):
                doc_rows.append({"component": name, "prop": None, "type": None, "line_no": i + 1, "kind": "no-props"})
            else:
                for fm in re.finditer(r"`([a-z_][a-zA-Z0-9_]*)\s*:\s*([^`]+)`", rest):
                    doc_rows.append(
                        {
                            "component": name,
                            "prop": fm.group(1),
                            "type": fm.group(2).strip(),
                            "line_no": i + 1,
                            "kind": "inline",
                        }
                    )
            continue

        if s.startswith("|") and current_heading:
            row = [c.strip() for c in s.strip("|").split("|")]
            if len(row) >= 3 and not re.match(r"^-+$", row[0]) and row[0] != "Prop":
                prop_m = re.match(r"^`([a-zA-Z0-9_]+)`$", row[0])
                if prop_m:
                    type_m = re.match(r"^`(.+)`$", row[1])
                    comp_target = current_bold if current_bold else current_heading
                    if comp_target and "/" in comp_target:
                        comp_target = comp_target.split("/")[0].strip()
                    doc_rows.append(
                        {
                            "component": comp_target,
                            "prop": prop_m.group(1),
                            "type": type_m.group(1) if type_m else row[1],
                            "line_no": i + 1,
                            "kind": "table",
                        }
                    )
    return doc_rows


def main():
    structs, component_impls = extract_source()
    doc_rows = extract_doc_rows()

    by_component = defaultdict(list)
    for r in doc_rows:
        by_component[r["component"]].append(r)

    documented = set(by_component.keys())
    source_components = set(structs.keys()) - NON_COMPONENT_STRUCTS

    mismatches, missing_in_doc, extra_in_doc, missing_components = [], [], [], []

    for comp in sorted(source_components):
        sfields = {f["name"]: f for f in structs[comp]["fields"]}
        if comp not in documented:
            missing_components.append(comp)
            continue
        drows = [r for r in by_component[comp] if r["kind"] != "no-props"]
        dprops = {r["prop"]: r for r in drows}

        for fname, finfo in sfields.items():
            if fname not in dprops:
                missing_in_doc.append((comp, fname, finfo["type"]))
            else:
                doc_t = re.sub(r"\s+", "", dprops[fname]["type"])
                src_t = re.sub(r"\s+", "", finfo["type"])
                if doc_t != src_t:
                    mismatches.append((dprops[fname]["line_no"], comp, fname, dprops[fname]["type"], finfo["type"]))
        for dp, row in dprops.items():
            if dp not in sfields:
                extra_in_doc.append((row["line_no"], comp, dp, row["type"]))

    print(f"Parsed {len(source_components)} RSX components from {SRC_DIR}")
    print(f"Parsed {len(doc_rows)} prop mentions from {os.path.relpath(DOC, REPO_ROOT)}")
    print()

    print(f"=== TYPE MISMATCHES ({len(mismatches)}) ===")
    for ln, comp, field, doc_t, src_t in sorted(mismatches):
        print(f"  line {ln:4} {comp}.{field}: doc says `{doc_t}`  actual `{src_t}`")

    print()
    print(f"=== FIELDS IN SOURCE BUT MISSING FROM DOC TABLE ({len(missing_in_doc)}) ===")
    for comp, field, t in missing_in_doc:
        print(f"  {comp}.{field}: {t}")

    print()
    print(f"=== ROWS IN DOC BUT NOT IN SOURCE STRUCT ({len(extra_in_doc)}) ===")
    for ln, comp, field, doc_t in extra_in_doc:
        print(f"  line {ln:4} {comp}.{field}: doc says `{doc_t}`")

    print()
    print("=== COMPONENTS IMPLEMENTING Component BUT ENTIRELY MISSING FROM THE DOC ===")
    for c in missing_components:
        print(" ", c)

    print()
    print("=== DOC HEADING/BOLD LABELS NOT MATCHED TO ANY SOURCE STRUCT (check for typos) ===")
    for comp in sorted(documented):
        if comp not in source_components and comp not in structs:
            print(" ", comp, "rows:", len(by_component[comp]))

    return 0


if __name__ == "__main__":
    sys.exit(main())
