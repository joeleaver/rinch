#!/usr/bin/env python3
"""Compare two Gungraun runs of `rinch-bench` and write a markdown report.

Each input is the stdout of `cargo bench -p rinch-bench -- --output-format=json`:
one JSON object per line, one line per benchmark. The metric compared is
Callgrind's `Ir` -- instructions executed inside the benchmark function, with
the setup excluded. Two runs of one binary agree to within about 0.03%, so a
difference between base and head past that is the code, not the runner.

    perf_compare.py --head head.jsonl [--base base.jsonl] [--threshold 3.0]
                    [--base-failed] [--accepted] [--note TEXT] [--out report.md]

Exit status: 0 when nothing regressed past the threshold (or the regression was
accepted with the `perf-regression-accepted` label), 1 when something did, 2 on
a malformed input or any unexpected error in this script -- so a crash can never
read as a regression the label would excuse.

A missing or empty base reports the head alone. That passes when the base has
no benchmarks yet; with `--base-failed` (the base has `crates/rinch-bench` and
neither the head's copy nor its own ran) it is a failure like a regression, and
`--accepted` downgrades it the same way. `--base-own-sources` says the base ran
its own copy (`perf_bench_base.sh`, #1036), which the report explains. Whenever
there is a base, the report says how many benchmarks were compared and names
the ones only one side has.
"""

import argparse
import json
import os
import sys

MARKER = "<!-- rinch-perf-bench -->"


def load(path):
    """{benchmark name: Ir} from a Gungraun JSON-lines file, or None."""
    if not path or not os.path.exists(path) or os.path.getsize(path) == 0:
        return None
    runs = {}
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                d = json.loads(line)
                callgrind = d["profiles"][0]["summaries"]["total"]["summary"]["Callgrind"]
                metrics = callgrind["Ir"]["metrics"]
                # `Left` is this run alone; `Both` is [this run, the previous
                # run Gungraun found on disk and compared against].
                ir = (metrics["Left"] if "Left" in metrics else metrics["Both"][0])["Int"]
            except (KeyError, IndexError, TypeError, json.JSONDecodeError) as e:
                print(f"{path}:{n}: not a Gungraun summary line ({e})", file=sys.stderr)
                sys.exit(2)
            # `hot_paths::dom::hover` + id `list_500` -> `dom::hover.list_500`
            group_fn = d["module_path"].split("::", 1)[-1]
            runs[f"{group_fn}.{d['id']}"] = ir
    return runs or None


def fmt(n):
    return f"{n:,}" if n is not None else "–"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base")
    ap.add_argument("--head", required=True)
    ap.add_argument("--threshold", type=float, default=3.0,
                    help="fail when a benchmark's Ir grows by more than this percentage")
    ap.add_argument("--accepted", action="store_true",
                    help="a regression is reported as a warning instead of failing")
    ap.add_argument("--base-label", default="base")
    ap.add_argument("--head-label", default="head")
    ap.add_argument("--base-failed", action="store_true",
                    help="the base has the benchmarks and they did not run: fail")
    ap.add_argument("--base-own-sources", action="store_true",
                    help="the base ran its own copy of crates/rinch-bench, because "
                         "the head's did not build or run on it (#1036)")
    ap.add_argument("--note", default="", help="an extra line under the table")
    ap.add_argument("--out")
    args = ap.parse_args()

    head = load(args.head)
    if head is None:
        print(f"{args.head}: no benchmark results", file=sys.stderr)
        sys.exit(2)
    base = load(args.base)

    regressed, improved = [], []
    rows = []
    for name in sorted(set(head) | set(base or {})):
        h = head.get(name)
        b = base.get(name) if base else None
        if h is None:
            rows.append(f"| `{name}` | {fmt(b)} | – | – | removed |")
            continue
        if b is None:
            rows.append(f"| `{name}` | – | {fmt(h)} | – | new |")
            continue
        delta = (h - b) * 100.0 / b if b else 0.0
        if delta > args.threshold:
            status = "⚠️ regression" if args.accepted else "❌ regression"
            regressed.append((name, delta))
        elif delta < -args.threshold:
            status = "✅ faster"
            improved.append((name, delta))
        else:
            status = ""
        rows.append(f"| `{name}` | {fmt(b)} | {fmt(h)} | {delta:+.2f}% | {status} |")

    lines = [MARKER, "## Instruction counts (Callgrind `Ir`)", ""]
    if base is None and not args.base:
        lines.append("No base to compare against (not a pull request). Head only.")
    elif base is None and args.base_failed:
        verb = ("accepted by the `perf-regression-accepted` label" if args.accepted
                else "**fails this check**")
        lines.append(
            f"The benchmarks did not build or run on `{args.base_label}`, which has "
            f"them, with either the head's copy of `crates/rinch-bench` or its own, so "
            f"nothing could be compared — {verb}. Usually one of the base's own "
            "benchmarks panics there."
        )
    elif base is None:
        lines.append(
            f"No baseline: the benchmarks did not run on `{args.base_label}` "
            "(it has no `crates/rinch-bench` yet). Head only; nothing is compared."
        )
    elif regressed:
        verb = "accepted by the `perf-regression-accepted` label" if args.accepted else "**fails this check**"
        lines.append(
            f"{len(regressed)} benchmark(s) grew by more than {args.threshold:g}% "
            f"against `{args.base_label}` — {verb}."
        )
    else:
        lines.append(
            f"No benchmark grew by more than {args.threshold:g}% against `{args.base_label}`."
        )
    if base is not None:
        both = sorted(set(head) & set(base))
        head_only = sorted(set(head) - set(base))
        base_only = sorted(set(base) - set(head))
        coverage = f"Compared {len(both)} of {len(set(head) | set(base))} benchmarks."
        if head_only:
            coverage += " Not compared, head only (new): " + ", ".join(f"`{n}`" for n in head_only) + "."
        if base_only:
            coverage += " Not compared, base only (removed): " + ", ".join(f"`{n}`" for n in base_only) + "."
        lines += ["", coverage]
    if base is not None and args.base_own_sources:
        lines += [
            "",
            f"The head's copy of `crates/rinch-bench` did not build or run on "
            f"`{args.base_label}`, so the base ran its own copy. That usually means the "
            "PR adds a benchmark calling an API the same PR introduces. A benchmark "
            "whose scenario this PR changes compares two different scenarios here, so "
            "its Δ is not the library's alone.",
        ]
    lines += [
        "",
        f"| Benchmark | {args.base_label} | {args.head_label} | Δ | |",
        "|---|--:|--:|--:|---|",
        *rows,
        "",
        "Instructions executed by the measured operation only (setup excluded), "
        "under Valgrind. Two runs of one binary agree to within about 0.03%, so a "
        "larger Δ is the code. How to read and reproduce this: "
        "`docs/src/guide/performance.md#ci-regression-job`.",
    ]
    if args.note:
        lines += ["", args.note]
    report = "\n".join(lines) + "\n"

    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(report)
    print(report)

    kind = "warning" if args.accepted else "error"
    for name, delta in regressed:
        print(f"::{kind} title=perf regression::{name}: Ir {delta:+.2f}% "
              f"(threshold {args.threshold:g}%)")
    base_failed = base is None and args.base_failed
    if base_failed:
        print(f"::{kind} title=perf baseline::the benchmarks did not run on the base")
    failed = bool(regressed) or base_failed
    return 1 if failed and not args.accepted else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit as e:
        # argparse's usage error exits 2 as well; keep every non-verdict exit at 2.
        sys.exit(e.code if e.code in (0, 1) else 2)
    except Exception as e:  # noqa: BLE001 -- any crash is exit 2, never 1
        print(f"perf_compare.py: unexpected error: {e!r}", file=sys.stderr)
        sys.exit(2)
