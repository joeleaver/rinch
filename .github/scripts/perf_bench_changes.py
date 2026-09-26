#!/usr/bin/env python3
"""Does the head's copy of `crates/rinch-bench` change any EXISTING scenario?

    perf_bench_changes.py BASE_COPY HEAD_COPY > changes.txt

Used by `perf_bench_base.sh` when the base has to run its own copy of the
benchmark sources (#1036). The shared benchmarks are then compared across two
copies, which is sound only if the head's copy merely ADDS to the base's: a
scenario the PR made cheaper would otherwise hide a real regression. So this
prints every edit that is not a pure addition of new top-level items, and
prints nothing when there is none:

  * a line the head's copy removes or rewrites (trailing whitespace and a
    trailing `,` are ignored, so appending to a list is not a rewrite; blank
    lines are ignored);
  * lines the head's copy inserts INSIDE an existing item (brace depth > 0 at
    the insertion point): a new statement in an existing setup is a new
    scenario, where a new `fn` or a new `#[bench::…]` attribute is not.

It is a proxy, not a parser: braces inside string literals and comments are
skipped crudely. It errs towards reporting a change, which costs a label.
Each report line names the file, the line, and the `fn` it is in.
"""

import os
import re
import sys
from difflib import SequenceMatcher

FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([A-Za-z0-9_$]+)")


def read(path):
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().splitlines()


def significant(lines):
    """[(original index, normalized text)] with blank lines dropped."""
    out = []
    for i, line in enumerate(lines):
        norm = line.rstrip().rstrip(",").rstrip()
        if norm:
            out.append((i, norm))
    return out


def code_only(line):
    line = re.sub(r'"(?:\\.|[^"\\])*"', '""', line)
    line = re.sub(r"'(?:\\.|[^'\\])'", "''", line)
    return line.split("//", 1)[0]


def depth_before(lines, idx):
    depth = 0
    for line in lines[:idx]:
        c = code_only(line)
        depth += c.count("{") - c.count("}")
    return depth


def enclosing_fn(lines, idx):
    for line in reversed(lines[: idx + 1]):
        m = FN.match(line)
        if m:
            return m.group(1)
    return None


def changes(base_dir, head_dir):
    rels = set()
    for root in (base_dir, head_dir):
        for d, _, files in os.walk(root):
            for name in files:
                rels.add(os.path.relpath(os.path.join(d, name), root))
    report = []
    for rel in sorted(rels):
        b_lines = read(os.path.join(base_dir, rel))
        h_lines = read(os.path.join(head_dir, rel))
        b_sig, h_sig = significant(b_lines), significant(h_lines)
        sm = SequenceMatcher(None, [t for _, t in b_sig], [t for _, t in h_sig],
                             autojunk=False)
        for op, i1, i2, j1, j2 in sm.get_opcodes():
            if op == "equal":
                continue
            if op == "insert":
                at = h_sig[j1][0]
                if depth_before(h_lines, at) <= 0:
                    continue  # a new top-level item
                where = f"{rel}:{at + 1}"
                fn = enclosing_fn(h_lines, at)
                what = "inserted inside"
            else:
                at = b_sig[i1][0]
                where = f"{rel}:{at + 1} (base)"
                fn = enclosing_fn(b_lines, at)
                what = "changed in" if op == "replace" else "removed from"
            report.append(f"@@ crates/rinch-bench/{where}: {what} "
                          + (f"fn {fn}" if fn else "a top-level item"))
            report += [f"-{b_lines[i]}" for i, _ in b_sig[i1:i2]]
            report += [f"+{h_lines[j]}" for j, _ in h_sig[j1:j2]]
    return report


if __name__ == "__main__":
    for line in changes(sys.argv[1], sys.argv[2]):
        print(line)
