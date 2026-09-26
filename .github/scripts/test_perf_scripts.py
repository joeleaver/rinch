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
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BASE_SCRIPT = os.path.join(HERE, "perf_bench_base.sh")
COMPARE_STEP = os.path.join(HERE, "perf_compare_step.sh")
CHANGES = os.path.join(HERE, "perf_bench_changes.py")
WORKFLOW = os.path.join(HERE, "..", "workflows", "perf.yml")


def workflow_step_env(step_name):
    """{ENV_NAME: expression} of one step's `env:` block in perf.yml."""
    with open(WORKFLOW, encoding="utf-8") as f:
        text = f.read()
    start = text.index(f"      - name: {step_name}\n")
    end = text.find("\n      - name: ", start + 1)
    block = text[start:end if end != -1 else len(text)]
    env = {}
    for m in re.finditer(r"^          ([A-Z_]+): (.+)$", block, re.M):
        env[m.group(1)] = m.group(2).strip()
    return env, block


def compare_env_from_base_outputs(outputs):
    """The Compare step's env, from the base step's outputs, by perf.yml's OWN
    mapping: a typo in the workflow's wiring fails the tests that use this."""
    env, _ = workflow_step_env("Compare")
    out = {}
    for name, expr in env.items():
        m = re.fullmatch(r"\$\{\{ steps\.base\.outputs\.([a-z_]+) \}\}", expr)
        if m:
            out[name] = outputs.get(m.group(1), "")
    return out

FAKE_CARGO = textwrap.dedent(r"""
    #!/usr/bin/env python3
    import json, os, sys
    crate = os.path.join("crates", "rinch-bench")
    def lines(p):
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return [l.strip() for l in f if l.strip()]
    if not os.path.isdir(crate):
        print("error: package ID specification `rinch-bench` did not match any packages",
              file=sys.stderr)
        sys.exit(101)
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
    os.makedirs(os.path.join(root, "crates", "rinch-core"), exist_ok=True)
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
                   DIFF_OUT=os.path.join(self.tmp, "base.sources-diff"),
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

    def head_and_compare(self, outputs, accepted=False):
        self.run_head()
        return self.compare(outputs, accepted)

    def compare(self, outputs, accepted=False):
        """Run the Compare step's script as perf.yml runs it; (status, report)."""
        gh_out = os.path.join(self.tmp, "compare_output")
        summary = os.path.join(self.tmp, "summary")
        open(gh_out, "w").close()
        env = dict(os.environ, RUNNER_TEMP=self.tmp, GITHUB_OUTPUT=gh_out,
                   GITHUB_STEP_SUMMARY=summary, HEAD_OUTCOME="success",
                   THRESHOLD="3", ACCEPTED="true" if accepted else "false",
                   BASE_SHA="b" * 40, HEAD_SHA="h" * 40, PR_HEAD_SHA="p" * 40,
                   RUN_URL="https://example.invalid/run",
                   **compare_env_from_base_outputs(outputs))
        p = subprocess.run(["bash", COMPARE_STEP], env=env, capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        with open(gh_out) as f:
            status = int(dict(l.strip().split("=", 1) for l in f if "=" in l)["status"])
        with open(os.path.join(self.tmp, "report.md")) as f:
            return status, f.read()


class BaseStep(Harness):
    def test_same_sources_compare_every_benchmark(self):
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a"], OLD)
        outputs, out = self.run_base()
        self.assertEqual(outputs["ran"], "true")
        self.assertEqual(outputs["sources"], "head")
        code, report = self.head_and_compare(outputs)
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
        code, report = self.head_and_compare(outputs)
        # The existing benches got a Δ -- and a +10% one fails the check.
        self.assertIn("| `dom::hover.list_500` | 1,000 | 1,100 | +10.00% | ❌ regression |", report)
        self.assertIn("| `dom::text_shadow_paint.cold` | – | 550 | – | new |", report)
        self.assertIn("Compared 2 of 3 benchmarks", report)
        self.assertIn("so the base ran its own copy", report)
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
        code, report = self.head_and_compare(outputs)
        self.assertEqual(code, 1, report)
        self.assertIn("did not build or run", report)
        code, _ = self.head_and_compare(outputs, accepted=True)
        self.assertEqual(code, 0)

    def test_a_base_without_the_crate_reports_the_head_alone(self):
        checkout(self.base, ["a"])
        checkout(self.head, ["a", "b"], OLD, needs=["b"])
        outputs, out = self.run_base()
        self.assertEqual((outputs["has_crate"], outputs["ran"]), ("false", "false"))
        code, report = self.head_and_compare(outputs)
        self.assertEqual(code, 0, report)
        self.assertIn("no `crates/rinch-bench` yet", report)


class Compare(Harness):
    def jsonl(self, name, benches):
        """A benchmark run's JSON lines, written where the Compare step reads them."""
        root = os.path.join(self.tmp, "src-" + name)
        checkout(root, [], benches)
        with open(os.path.join(self.tmp, name + ".jsonl"), "w") as f:
            subprocess.run([self.cargo], cwd=root, stdout=f, check=True)

    def test_a_removed_benchmark_is_listed_as_not_compared(self):
        self.jsonl("base", OLD)
        self.jsonl("head", OLD[:1])
        code, report = self.compare({"has_crate": "true", "ran": "true", "sources": "base"})
        self.assertEqual(code, 0, report)
        self.assertIn("Compared 1 of 2 benchmarks", report)
        self.assertIn("base only (removed): `shell::memo_flush.selection_40`", report)

    def test_a_base_that_failed_is_not_compared_whatever_lines_it_left(self):
        # Defence in depth behind the base script's truncation: a base marked
        # failed is no baseline even if its file holds a partial run.
        self.jsonl("base", OLD)
        self.jsonl("head", OLD)
        code, report = self.compare({"has_crate": "true", "ran": "false", "sources": "none"})
        self.assertEqual(code, 1, report)
        self.assertIn("did not build or run", report)

    def test_the_workflow_wires_every_base_output_and_file(self):
        env, _ = workflow_step_env("Compare")
        wired = {m for e in env.values()
                 for m in re.findall(r"steps\.base\.outputs\.([a-z_]+)", e)}
        with open(BASE_SCRIPT) as f:
            written = set(re.findall(r'echo "([a-z_]+)=', f.read())) | {"ran", "sources"}
        self.assertEqual(wired, written)
        base_env, _ = workflow_step_env("Benchmark base")
        self.assertEqual(base_env["OUT"], "${{ runner.temp }}/base.jsonl")
        self.assertEqual(base_env["DIFF_OUT"], "${{ runner.temp }}/base.sources-diff")
        _, head_block = workflow_step_env("Benchmark head")
        self.assertIn('> "$RUNNER_TEMP/head.jsonl"', head_block)
        _, compare_block = workflow_step_env("Compare")
        self.assertIn("bash head/.github/scripts/perf_compare_step.sh", compare_block)


class ScenarioEdits(Harness):
    """The fallback compares two copies of the sources, so a PR that also edits
    an EXISTING scenario must not pass unlabelled (review of #1058, F1)."""

    def test_an_edited_cheaper_scenario_cannot_hide_a_regression(self):
        # The PR adds API `b` + a bench needing it, rewrites hover's and memo's
        # scenarios to do 9% less work, and regresses the library by 10%: the
        # Δs read +0.10%, which alone would pass.
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a", "b"],
                 ["dom::hover.list_500 910", "shell::memo_flush.selection_40 1820",
                  "dom::new.x 5"], needs=["b"], scale=110)
        outputs, _ = self.run_base()
        self.assertEqual((outputs["sources"], outputs["sources_changed"]), ("base", "true"))
        code, report = self.head_and_compare(outputs)
        self.assertIn("+0.10%", report)
        self.assertEqual(code, 1, report)
        self.assertIn("changes existing benchmark sources", report)
        self.assertIn("-dom::hover.list_500 1000", report)
        code, report = self.compare(outputs, accepted=True)
        self.assertEqual(code, 0, report)

    def test_a_purely_additive_change_is_not_a_scenario_edit(self):
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a", "b"], OLD + ["dom::new.x 5"], needs=["b"])
        outputs, _ = self.run_base()
        self.assertEqual((outputs["sources"], outputs["sources_changed"]), ("base", "false"))
        code, report = self.head_and_compare(outputs)
        self.assertEqual(code, 0, report)
        self.assertIn("only adds to it", report)

    def test_the_scenario_edits_are_not_computed_when_the_head_copy_ran(self):
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a"], ["dom::hover.list_500 1"])
        outputs, _ = self.run_base()
        self.assertEqual(outputs["sources"], "head")
        self.assertNotIn("sources_changed", outputs)


class Robustness(Harness):
    """From the review of #1058 (its R2-R4)."""

    def test_paths_with_spaces(self):
        self.base = os.path.join(self.tmp, "ba se")
        self.head = os.path.join(self.tmp, "he ad")
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a", "b"], OLD, needs=["b"])
        outputs, _ = self.run_base()
        self.assertEqual((outputs["ran"], outputs["sources"]), ("true", "base"))

    def test_the_base_tree_keeps_the_head_sources_it_ran(self):
        checkout(self.base, ["a"], OLD)
        checkout(self.head, ["a"], OLD + ["dom::n.x 3"])
        outputs, _ = self.run_base()
        self.assertEqual(outputs["sources"], "head")
        with open(os.path.join(self.base, "crates/rinch-bench/BENCHES")) as f:
            self.assertIn("dom::n.x", f.read())

    def test_a_base_own_copy_panicking_midway_fails_the_check(self):
        # It prints two lines, then panics: the partial file must not be a baseline.
        checkout(self.base, ["a"], OLD, panic=True)
        checkout(self.head, ["a", "b"], OLD, needs=["b"], scale=150)
        outputs, out = self.run_base()
        self.assertEqual((outputs["ran"], outputs["sources"]), ("false", "none"))
        self.assertEqual(os.path.getsize(out), 0)
        code, report = self.head_and_compare(outputs)
        self.assertEqual(code, 1, report)


RUST_BASE = """\
use x::*;

#[library_benchmark]
#[bench::a(setup = setup_a)]
fn a(f: F) -> F {
    measure(f, op_a)
}

pub fn setup_a() -> F {
    let f = F::new("{");
    f.hover(3);
    f
}

library_benchmark_group!(
    name = dom,
    benchmarks = [
        a
    ]
);
"""


class BenchChanges(unittest.TestCase):
    """`perf_bench_changes.py` on Rust-shaped sources."""

    def run_on(self, head_text, base_text=RUST_BASE, extra_head=None):
        with tempfile.TemporaryDirectory() as tmp:
            for side, text in (("base", base_text), ("head", head_text)):
                write(os.path.join(tmp, side, "benches", "hot_paths.rs"), text)
            if extra_head:
                for rel, text in extra_head.items():
                    write(os.path.join(tmp, "head", rel), text)
            p = subprocess.run([sys.executable, CHANGES, os.path.join(tmp, "base"),
                                os.path.join(tmp, "head")],
                               capture_output=True, text=True, check=True)
            return p.stdout

    def test_new_items_a_new_bench_case_and_a_list_entry_are_additions(self):
        head = RUST_BASE.replace(
            "#[bench::a(setup = setup_a)]",
            "#[bench::a(setup = setup_a)]\n#[bench::a_cold(setup = setup_a_cold)]",
        ).replace("        a\n", "        a,\n        b\n") + (
            "\npub fn setup_a_cold() -> F {\n    let f = setup_a();\n    f\n}\n")
        self.assertEqual(self.run_on(head, extra_head={"NEW.md": "anything\n"}), "")

    def test_a_statement_inside_an_existing_fn_is_an_edit(self):
        out = self.run_on(RUST_BASE.replace("    f.hover(3);\n", "    f.hover(3);\n    f.warm();\n"))
        self.assertIn("inserted inside fn setup_a", out)
        self.assertIn("+    f.warm();", out)

    def test_a_rewritten_line_is_an_edit_named_by_its_fn(self):
        out = self.run_on(RUST_BASE.replace("f.hover(3)", "f.hover(2)"))
        self.assertIn("changed in fn setup_a", out)
        self.assertIn("-    f.hover(3);", out)

    def test_a_removed_file_is_an_edit(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(os.path.join(tmp, "base", "src", "lib.rs"), "pub fn x() {}\n")
            os.makedirs(os.path.join(tmp, "head", "src"))
            p = subprocess.run([sys.executable, CHANGES, os.path.join(tmp, "base"),
                                os.path.join(tmp, "head")], capture_output=True, text=True)
        self.assertIn("removed from fn x", p.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
