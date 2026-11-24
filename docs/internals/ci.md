# CI and batched merges

Read this page when your task touches: GitHub Actions workflows, which checks a pull request gets, merging reviewed PRs, a red batch, retargeting a stacked PR.

## The model

Two kinds of pull request into `main`:

| | Component PR (any head branch) | Batch PR (head branch `batch/…`) |
|---|---|---|
| `fmt`, `clippy`, `clippy-wasm`, `clippy-android`, `check-android`, `check-macos` | run | run |
| `test (unit)`, `test (integration-a-m)`, `test (integration-n-z)`, `test (feature-gated)`, `test-gpu (lavapipe)`, `test-wasm` | skipped | run |
| Perf `instruction-counts` (and its `comment`) | skipped | skipped, unless it carries the `perf` label: then run, against `main` |
| Documentation `build` (rustdoc, mdBook, UI Zoo, Pages artifact) | skipped | run |
| Documentation `book` (mdBook only) | run when the PR changes `docs/` | skipped |

A push to `main` runs everything a batch does and deploys the docs. It is also
where Perf measures by default: `instruction-counts` compares the pushed commit
(a batch's merge commit) against its first parent, the `main` it landed on, so
each batch's cost is measured once, after it lands (about 40 minutes; each
push's run has its own concurrency group, keyed on the commit, so runs of
successive pushes go in parallel and none is cancelled). There is no label escape hatch on a push: a Perf failure
there is a red `main`, which the lead fixes forward or reverts. The report is
the job summary only. To measure a batch **before** merging it, put the `perf`
label on the batch PR. A manual dispatch of `CI`
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

## The cargo caches

Every cargo cache in the workflows is **`Swatinem/rust-cache`, pinned by commit SHA**, and
holds third-party dependency artifacts only. Do not add an `actions/cache` of `target/` (or of
any directory a workspace crate builds into); `actions/cache` remains only for things that are
not cargo build output (`.deb` packages, the `gungraun-runner` binary).

**Why a cached `target/` is wrong.** Cargo decides that a path crate (every workspace member) is
fresh by comparing mtimes: the crate's dep-info file in its fingerprint directory against every
source file that dep-info lists. Its fingerprint hash covers the compiler, flags, features and
dependency fingerprints, but not the source contents. A checkout writes every source file with
the checkout time as its mtime, and `actions/cache` restores `target/` with tar, which keeps the
mtimes of the run that built it. The cache key (`Cargo.lock`, toolchain) says nothing about the
workspace sources. So:

- an entry built **before** this job's checkout has older artifacts than the sources, and every
  workspace crate rebuilds: correct, and the cached workspace artifacts were dead weight;
- an entry built **after** this job's checkout (a `main` run that built and saved it while this
  job was still installing apt packages) has newer artifacts, and cargo takes a workspace crate
  built from **another commit** as fresh. A crate whose own fingerprint changed for another
  reason is rebuilt against the stale one; a crate that did not is not checked at all.

That is what happened to `clippy` on PR #3017 (job 138257410415) and #3018 (job 138257638967) on
2026-11-24. The job checked out at 07:02:01, spent 35 minutes in `apt-install`, and restored the
cache at 07:37:31 through the restore-key prefix. The entry it got, key
`Linux-cargo-clippy-1.99.0-50d96d98…`, had been saved at 07:30:54 by `main`'s run 45311422899
(main `100412bcd`, checked out at 06:46, a cold build from 07:20). The log printed
`Compiling rinch` but no `Checking rinch-core`, and `rinch-components` failed with E0599 on
`NodeHandle::is_retired`, which the PR had added to `rinch-core` and `100412bcd` lacked. The same
mechanism in `docs.yml` was #1836. The failure is a matter of timing, and it can as easily
**pass** a check wrongly: an unchanged-fingerprint crate that the PR broke is never re-checked.

**What rust-cache does.** Before it saves, it deletes from each profile directory of the target
dir (`.fingerprint/`, `build/`, `deps/`, target-triple directories included) every entry not
named for a package whose manifest lies **outside the workspace root** (`cargo metadata
--all-features`): registry and git dependencies stay, workspace members go. Nothing in the entry
describes workspace source, so no timing can make one look fresh. The alternative, keeping
whole `target/` caches and touching every source after the restore so that workspace crates
always rebuild, was rejected: it caches gigabytes that are never used (members rebuild on every
run anyway), it must catch every file any build script or `include_str!` reads, and it leaves
correctness resting on mtimes again.

**Rules that follow.**

- **One target dir per job, registered as the workspace's.** rust-cache keeps packages outside
  the root it is given, so a separate workspace (an excluded example such as
  `examples/menu-bar-web`) registered with its own target dir would keep the rinch crates it
  reaches by `path`, which lie outside *its* root, and cache them stale. `clippy-wasm` therefore
  builds `menu-bar-web` with `--target-dir target`, inside the main workspace's target, where
  the main workspace's pruning removes them.
- **Only `main` writes** (`save-if: github.event_name == 'push'`; `visual-tests.yml`, which is
  dispatch-only, saves on `refs/heads/main`). A PR or batch run reads `main`'s entry (the
  cache scope falls back to the base branch) and never writes its own, which used to put an
  entry of up to 1.3 GB per job per PR into the repository's 10 GB budget and evict `main`'s.
- Each job has its own `shared-key` (`clippy`, `clippy-wasm`, `clippy-android`, `test-wasm`,
  `android`, `macos`, `test`, `test-gpu`, `docs`, `perf`, `visual`). The key also hashes the
  toolchain, every `Cargo.toml`/`Cargo.lock` and the `CARGO_*`/`RUST*` environment.

**Cost.** None in the steady state: workspace crates were already rebuilt on every run that got
the cache the ordinary way round, which is every run but the unlucky ones. The first run of each
job after this change is cold (no rust-cache entry exists yet), and so is a PR run whose base
`main` has not yet saved for the current lockfile. Measured on `main` (2026-11-24): the two
`clippy` steps took about 4.5 minutes cold (run 45311422899, a cache miss) and 1.4–2.2 minutes
warm.

After this landed, the old `Linux-cargo-*` / `macOS-cargo-*` entries (about 6.7 GB) were deleted
with `gh cache delete`: until they expired they used the 10 GB budget and could evict `main`'s
new entries. A `workflow_dispatch` run of CI never saves, and a runner-image update to its
preinstalled stable toolchain changes the key (rust-cache hashes every installed toolchain)
until `main` saves again.

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
   when it is merged). Carry over each PR's *Performance* notes. Perf then
   runs after the merge, on `main`. If one of them expects a cost, or you want
   the Δ before merging, put the `perf` label on the **batch** PR (and
   `perf-regression-accepted` with it if the cost is accepted).
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
`main`: opened, `synchronize`, reopened (and Perf on its `perf` and
`perf-regression-accepted` labels).

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
