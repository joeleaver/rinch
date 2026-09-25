#!/usr/bin/env python3
"""Self-tests for the Perf workflow's scripts (issue #1036).

Run: `python3 .github/scripts/test_perf_scripts.py` (the `Perf` workflow runs
it before it builds anything). `perf_bench_base.sh` is driven with a FAKE
`cargo` that stands in for `cargo bench -p rinch-bench`: it "builds" only when
every API the bench crate's `NEEDS` file names is listed in the checkout's
`API` file, and then prints one Gungraun JSON line per entry of the bench
crate's `BENCHES` file. That is the whole of what the base step cares about --
whether a given copy of `crates/rinch-bench` builds and runs on a given
library -- so the fallback logic can be tested without Valgrind or a build.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BASE_SCRIPT = os.path.join(HERE, "perf_bench_base.sh")
COMPARE = os.path.join(HERE, "perf_compare.py")

FAKE_CARGO = textwrap.dedent(r"""
    #!/usr/bin/env python3
    import json, os, sys
    crate = os.path.join("crates", "rinch-bench")
    def lines(p):
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return [l.strip() for l in f if l.strip()]
    with open("Cargo.lock", "a") as f:
        f.write("# touched by a cargo run\n")
    api = set(lines("API"))
    missing = [n for n in lines(os.path.join(crate, "NEEDS")) if n not in api]
    if missing:
        print(f"error[E0425]: cannot find function `{missing[0]}`", file=sys.stderr)
        sys.exit(101)
    scale = int((lines("SCALE") or ["100"])[0])
    for entry in lines(os.path.join(crate, "BENCHES")):
        name, ir = entry.split()
        group_fn, bench_id = name.rsplit(".", 1)
        print(json.dumps({
            "module_path": "hot_paths::" + group_fn,
            "id": bench_id,
            "profiles": [{"summaries": {"total": {"summary": {"Callgrind": {
                "Ir": {"metrics": {"Left": {"Int": int(ir) * scale // 100}}}}}}}}],
        }))
    if os.path.exists(os.path.join(crate, "PANIC")):
        print("thread 'main' panicked", file=sys.stderr)
        sys.exit(101)
""").lstrip()

OLD = ["dom::hover.list_500 1000", "shell::memo_flush.selection_40 2000"]


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def checkout(root, api, benches=None, needs=(), panic=False, scale=None):
    """A fake checkout: the library's API, and optionally `crates/rinch-bench`."""
    write(os.path.join(root, "API"), "\n".join(api) + "\n")
    write(os.path.join(root, "Cargo.lock"), "# lockfile of " + os.path.basename(root) + "\n")
    if scale is not None:
        write(os.path.join(root, "SCALE"), f"{scale}\n")
    if benches is not None:
        crate = os.path.join(root, "crates", "rinch-bench")
        write(os.path.join(crate, "BENCHES"), "\n".join(benches) + "\n")
        write(os.path.join(crate, "NEEDS"), "".join(n + "\n" for n in needs))
        if panic:
            write(os.path.join(crate, "PANIC"), "")


class Harness(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="perf-self-test-")
        self.base = os.path.join(self.tmp, "base")
        self.head = os.path.join(self.tmp, "head")
        self.bin = os.path.join(self.tmp, "bin")
        os.makedirs(self.bin)
        self.cargo = os.path.join(self.bin, "cargo")
        write(self.cargo, FAKE_CARGO)
        os.chmod(self.cargo, 0o755)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def run_base(self):
        out = os.path.join(self.tmp, "base.jsonl")
        gh_out = os.path.join(self.tmp, "gh_output")
        open(gh_out, "w").close()
        env = dict(os.environ, BASE_DIR=self.base, HEAD_DIR=self.head, OUT=out,
                   CARGO=self.cargo, GITHUB_OUTPUT=gh_out, BENCH_ARGS="")
        p = subprocess.run(["bash", BASE_SCRIPT], env=env, capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        with open(gh_out) as f:
            outputs = dict(l.strip().split("=", 1) for l in f if "=" in l)
        return outputs, out

    def run_head(self):
        out = os.path.join(self.tmp, "head.jsonl")
        with open(out, "w") as f:
            subprocess.run([self.cargo], cwd=self.head, stdout=f, check=True)
        return out

    def compare(self, base_out, head_out, outputs, *extra):
        args = [sys.executable, COMPARE, "--head", head_out, "--base", base_out,
                "--base-label", "main", "--head-label", "merge"]
        if outputs.get("ran") != "true" and outputs.get("has_crate") == "true":
            args.append("--base-failed")
        if outputs.get("sources") == "base":
            args.append("--base-own-sources")
        p = subprocess.run(args + list(extra), capture_output=True, text=True)
        return p.returncode, p.stdout


class BaseStep(Harness):
    def test_same_sources_compare_every_benchmark(self):
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a"], OLD)
        outputs, out = self.run_base()
        self.assertEqual(outputs["ran"], "true")
        self.assertEqual(outputs["sources"], "head")
        code, report = self.compare(out, self.run_head(), outputs)
        self.assertEqual(code, 0, report)
        self.assertIn("Compared 2 of 2 benchmarks", report)

    def test_a_new_bench_calling_a_new_api_still_compares_the_old_ones(self):
        # The #1020 shape: the PR adds `clear_cache` AND a bench that calls it.
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a", "clear_cache"],
                 OLD + ["dom::text_shadow_paint.cold 500"], needs=["clear_cache"],
                 scale=110)
        outputs, out = self.run_base()
        self.assertEqual(outputs["has_crate"], "true")
        self.assertEqual(outputs["ran"], "true")
        self.assertEqual(outputs["sources"], "base")
        code, report = self.compare(out, self.run_head(), outputs)
        # The existing benches got a Δ -- and a +10% one fails the check.
        self.assertIn("| `dom::hover.list_500` | 1,000 | 1,100 | +10.00% | ❌ regression |", report)
        self.assertIn("| `dom::text_shadow_paint.cold` | – | 500 | – | new |", report)
        self.assertIn("Compared 2 of 3 benchmarks", report)
        self.assertIn("its own copy of `crates/rinch-bench`", report)
        self.assertEqual(code, 1, report)

    def test_the_fallback_restores_the_bases_lockfile(self):
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a", "b"], OLD, needs=["b"])
        outputs, _ = self.run_base()
        self.assertEqual(outputs["sources"], "base")
        with open(os.path.join(self.base, "Cargo.lock")) as f:
            # The base's own lockfile plus exactly ONE run's touch: the
            # fallback's. The head-sources attempt's touch was undone.
            self.assertEqual(f.read(), "# lockfile of base\n# touched by a cargo run\n")

    def test_a_head_bench_that_panics_on_the_base_falls_back(self):
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a"], OLD, panic=True)
        outputs, out = self.run_base()
        self.assertEqual((outputs["ran"], outputs["sources"]), ("true", "base"))
        with open(out) as f:
            self.assertEqual(len(f.readlines()), 2)

    def test_a_base_that_runs_neither_copy_fails_the_check(self):
        # The base's own copy is broken too: nothing ran, and that must not pass.
        checkout(self.base, ["a"], OLD, needs=["gone"])
        checkout(self.head, ["a", "b"], OLD, needs=["b"])
        outputs, out = self.run_base()
        self.assertEqual((outputs["has_crate"], outputs["ran"]), ("true", "false"))
        self.assertEqual(outputs["sources"], "none")
        self.assertEqual(os.path.getsize(out), 0)
        code, report = self.compare(out, self.run_head(), outputs)
        self.assertEqual(code, 1, report)
        self.assertIn("did not build or run", report)
        code, _ = self.compare(out, self.run_head(), outputs, "--accepted")
        self.assertEqual(code, 0)

    def test_a_base_without_the_crate_reports_the_head_alone(self):
        checkout(self.base, ["a"])
        checkout(self.head, ["a", "b"], OLD, needs=["b"])
        outputs, out = self.run_base()
        self.assertEqual((outputs["has_crate"], outputs["ran"]), ("false", "false"))
        code, report = self.compare(out, self.run_head(), outputs)
        self.assertEqual(code, 0, report)
        self.assertIn("no `crates/rinch-bench` yet", report)


class Compare(Harness):
    def jsonl(self, name, benches):
        root = os.path.join(self.tmp, name)
        checkout(root, [], benches)
        out = os.path.join(self.tmp, name + ".jsonl")
        with open(out, "w") as f:
            subprocess.run([self.cargo], cwd=root, stdout=f, check=True)
        return out

    def test_a_removed_benchmark_is_listed_as_not_compared(self):
        base = self.jsonl("b", OLD)
        head = self.jsonl("h", OLD[:1])
        code, report = self.compare(base, head, {"ran": "true", "sources": "base"})
        self.assertEqual(code, 0, report)
        self.assertIn("Compared 1 of 2 benchmarks", report)
        self.assertIn("base only (removed): `shell::memo_flush.selection_40`", report)


if __name__ == "__main__":
    unittest.main(verbosity=2)
