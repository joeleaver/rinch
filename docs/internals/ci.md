# CI and batched merges

Read this page when your task touches: GitHub Actions workflows, which checks a pull request gets, merging reviewed PRs, a red batch, retargeting a stacked PR.

## The model

Two kinds of pull request into `main`:

| | Component PR (any head branch) | Batch PR (head branch `batch/…`) |
|---|---|---|
| `fmt`, `clippy`, `clippy-wasm`, `clippy-android`, `check-android` | run | run |
| `test (unit)`, `test (integration-a-m)`, `test (integration-n-z)`, `test (feature-gated)`, `test-gpu (lavapipe)`, `test-wasm` | skipped | run |
| Perf `instruction-counts` (and its `comment`) | skipped | run, against `main` |
| Documentation `build` (rustdoc, mdBook, UI Zoo, Pages artifact) | skipped | run |
| Documentation `book` (mdBook only) | run when the PR changes `docs/` | skipped |

A push to `main` runs everything a batch does (Perf's push job only seeds its
cache; it measures nothing) and deploys the docs. A manual dispatch of `CI`
(`gh workflow run CI --ref <branch>`) runs every CI job on any branch.

The slow jobs are skipped by a **job-level `if`** on `github.head_ref`, never by
a trigger path or branch filter: a skipped job reports as skipped, while a
filtered-out check never reports, and a required check waiting on it stays
pending for ever. `github.head_ref` is the caller's inside a call from
`base-changed.yml`, so a retarget is classified like the PR's own runs.

Every PR workflow cancels its own superseded run (a push replaces the run of
the push before it). A run on `main` is never cancelled in progress.

## The runner image

Every workflow runs on **`ubuntu-24.04`**, pinned rather than `ubuntu-latest` (#1901). On 2026-10-25
`ubuntu-latest` moved to Ubuntu 26.04 with no change on our side, and main went red: the
transitional `libgdk-pixbuf2.0-dev` was gone, and `libspa` 0.8's bindgen output lost
`spa_pod_builder`'s fields. Moving to a new image is its own PR with a green batch
(#1902 tracks 26.04). The apt cache key carries `ImageOS`, so a new image starts with a
cold `.deb` cache.

## The `test` shards

The four `test (…)` shards together run exactly what the single `test` job ran
before. The first three select `--workspace` and differ only in targets, so
cargo unifies features as `cargo test --workspace` always did (a test alive only
because some example turns its feature on stays alive). **Do not shard by
package** (`-p`, `--exclude`): that changes the unification and kills such tests
silently. The integration shards split by the first letter of the target name,
`[a-m]*` and its exact complement `[!a-m]*`, so a new test file lands in exactly
one of them whatever it is called. A new feature-gated combination goes in the
`feature-gated` shard. `ci.yml` has the full reasoning.

## The GPU pixel tests (`test-gpu (lavapipe)`)

The `#[ignore]`d fixtures that need a GPU adapter (they render Vello scenes and
the compositor's passes through wgpu and read the pixels back) run in the
`test-gpu (lavapipe)` job, on Mesa's software Vulkan driver, so a runner with no
GPU has an adapter (#1865 D9). It runs where the `test` shards do. Its check
step finds lavapipe's ICD file, puts it in `VK_ICD_FILENAMES` and fails if
`vulkaninfo` shows no `llvmpipe` device; `WGPU_BACKEND=vulkan` keeps wgpu off
Mesa's GL, which the fixtures honour because they build their instance with
`InstanceDescriptor::from_env_or_default()` (a new GPU fixture should too). With
no lavapipe every fixture then fails on its `.expect("a GPU adapter")`.

Each `cargo test` in the job states the **exact** number of tests it passes,
and the job fails on any other number: a filter that matches nothing, or a
fixture whose `cfg` went false, prints `0 passed` and exits 0. **A PR that adds
or removes a GPU fixture changes the count in `ci.yml`** (or the next batch goes
red saying so). To run the job's tests locally on lavapipe:

```bash
export WGPU_BACKEND=vulkan VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
cargo test -p rinch --features gpu,video --lib -- --ignored gpu_ --skip gpu_hover_probe --test-threads=1
cargo test -p rinch --features gpu,video --test gpu_gradient_mask_1798 -- --ignored --test-threads=1
cargo test -p rinch-dom --no-default-features --test background_tiling_embed_tests -- --ignored gpu_ --test-threads=1
```

(The ICD file is `lvp_icd.x86_64.json` on some Mesa packages.) Leave
`VK_ICD_FILENAMES` out to run them on the host's own GPU. Not in the job:
`a_later_window_presents_with_the_shared_device`, which opens a window, and
`gpu_hover_probe`, a timing printout.

## The batch procedure

1. Review each component PR as usual; its fast checks must be green.
2. Make the batch branch from the current `main`:

   ```bash
   git fetch origin
   git switch -c batch/2026-10-22-1 origin/main
   ```

   Name it `batch/YYYY-MM-DD-N`. The `batch/` prefix is what turns the full
   suite on; anything else is a component PR.
3. Merge each reviewed PR's head into it, keeping one merge commit per PR:

   ```bash
   for pr in 1801 1803 1807; do
     git fetch origin "pull/$pr/head:pr-$pr"
     git merge --no-ff "pr-$pr" -m "Merge #$pr into batch"
   done
   ```

   Resolve conflicts once, here, in the merge that meets them.
4. Push and open one PR titled `batch: …` listing the PRs it carries, one
   `Refs #N` line each (not `Fixes`: each component PR closes its own issues
   when it is merged). Carry over each PR's *Performance* notes, and put the
   `perf-regression-accepted` label on the **batch** PR if one of them accepts a
   cost.
5. When it is green, merge the batch PR with a **merge commit** (never squash or
   rebase). The component PRs' head commits are then in `main`, so GitHub marks
   each of them merged and closes the issues they fix. Delete the batch branch.

A component PR pushed to after it was batched is no longer the commit in the
batch: drop it from the batch or rebuild the batch.

## A red batch

A red batch blocks everything behind it, so it is the top priority. Find the
PR that broke it (the failing test's area, or bisect the batch's merge
commits), then either fix it on that PR's branch and rebuild the batch, or
drop that PR (rebuild the batch without its merge) and send it back to its
author. Do not merge a batch with a red check.

## Retargets and other trigger traps

`ci.yml`, `perf.yml` and `docs.yml` run on `pull_request` events whose base is
`main`: opened, `synchronize`, reopened (and Perf on its label).

- **A base change to `main`** fires only `edited`, which those three do not
  list. `base-changed.yml` listens for `edited` and calls them as reusable
  workflows when `github.event.changes.base` is present. Those checks are named
  `ci / test (unit)`, `perf / instruction-counts`, `docs / build`, …; the same PR
  pushed to afterwards gets the plain names. A title or body edit shows three
  skipped checks named `ci`, `perf` and `docs` and runs nothing. Do not
  "simplify" this into `edited` plus a job-level `if:` inside `ci.yml`: a body
  edit would then start a run whose jobs are skipped under the real checks'
  names, and the newest check of a name is the one the merge box shows, so a red
  `test` would read as passed (#448).
- **Retarget a stacked PR before its base branch is deleted.** Deleting the
  parent's branch closes the child instead of retargeting it, and a PR whose
  base branch is gone cannot be reopened. Change the base first
  (`gh api -X PATCH repos/OWNER/REPO/pulls/N -f base=main`).
- **A PR that conflicts with its base gets no run of any workflow**: GitHub has
  no merge commit to run on. Check `gh pr view N --json mergeable,mergeStateStatus`.
- **Re-running a run does not move it to a newer `main`**; push to the branch
  to test against the current base.

"No checks" is never a pass: confirm a run exists for the PR's head commit.

**Feature-gated test files use `#![cfg(feature = "…")]`, never `[[test]] required-features`.** The integration shards select targets with `--test '<glob>'`, and cargo refuses a target named that way whose required features are off (batch #1793 failed on it). Name the feature combination in the `feature-gated` shard instead.
