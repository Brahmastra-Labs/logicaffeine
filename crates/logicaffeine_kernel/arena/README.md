# Lean Kernel Arena entries

The checkers LOGOS enters in the [Lean Kernel Arena](https://arena.lean-lang.org/), in the Arena's
checker format. `checker.schema.json` is the Arena's `schemas/checker.json`, vendored from
[lean-kernel-arena](https://github.com/leanprover/lean-kernel-arena) at revision
`8139f284045e9f3a9bcf5f86119270b854057aeb`; everything below describes the Arena at that revision.

| file | entry |
|---|---|
| `logos-kernel.yaml` | the dag kernel core, checking each declaration once (`../examples/logos_kernel.rs`) |
| `logos-dual.yaml` | the same core with two complete engines that must agree (`../examples/logos_dual.rs`); declines mathlib, which does not fit its timeout checked twice |

Both entries build with the `arena` profile, `target-cpu=native`, a profile trained on the run line over the Arena's
`init` and `init-prelude` tests, which the build exports first with the Arena's own `lka.py build-test` (from the
Arena's root, four directories above the checker's `src/`), and the kernel's `unchecked-access` feature, with which the dag core reads
its stores by the ids they hand out without a bounds check (every other build of the kernel, and every test, keeps
the checks). The checker
is linked as a position-dependent executable, by the kernel's build script (`cargo:rustc-link-arg-examples=-no-pie`, on
Linux, for the crate's examples alone), so the loader patches none of the binary's own tables at start-up: the Arena
counts every instruction of every test, start-up included. The flag is not in `RUSTFLAGS`, which reach every crate the
build compiles, a procedural macro's shared object among them.
`init-prelude`'s profile is merged at sixty times its weight: `init` alone makes Mathlib's checking the hot path
and leaves the declarations a small test spends its time on cold, which cost the small tests up to 4% more
instructions; weighted, every small test is back within its ceiling and Mathlib costs no more.
`scripts/lean/arena-crown/snapshot.sh` builds the binary every measurement runs with exactly these flags and this
training.

`crates/logicaffeine_tests/tests/arena_entry_lock.rs` holds both files to the schema and to the invariants below.

## Acknowledgements

The core is LOGOS's own code, and it stands on the work of others: the Lean 4 kernel, lean4lean, smalltt,
sokonanoda, nanoda, mathgraph, nano-lean, lean4export and this Arena, credited one by one, with what each gave it, in
the [crate README](../README.md#acknowledgements). Both entries' descriptions credit them too, and say how the
entries are built: the core LOGOS runs (`largo lean check`), in a build tuned for the Arena's instruction count.

## How the Arena builds and runs an entry

- **Submission is a pull request** to lean-kernel-arena adding `checkers/<name>.yaml`; the file
  stem is the checker's name. Pull-request CI builds and runs only the checkers a pull request
  touches, when it touches nothing but `checkers/*.yaml`. The Arena never updates a checker by
  itself: a new version is a pull request bumping `rev`.
- **Machine.** Each checker gets its own job on a Namespace `nscloud-ubuntu-22.04-amd64-8x16`
  runner (8 vCPUs, 16 GB), inside the Arena flake's nix shell (nixpkgs `0ad6f47e`, nixos-26.05):
  rustc and cargo 1.95.0, `llvm-profdata` from `llvmPackages_21`, elan, perf, GNU time. System
  build dependencies go into that `flake.nix`; ours need nothing beyond rustc, cargo,
  `llvm-profdata` and the elan `lka.py build-test` exports `init` and `init-prelude` with. The job has 360 minutes for build, test generation and every test.
- **Build.** `git clone --branch <ref> <url> _build/checkers/<name>/src`, `git checkout <rev>`, then
  the `build` script with `/bin/sh` in `src/`. The network is available: cargo resolves the whole
  workspace's lockfile, git dependencies included, although only the kernel is compiled.
- **Run.** Tests run one at a time. `run` is executed in `src/` with `$IN` set to the test's
  NDJSON file, under `perf stat -e duration_time,task-clock,instructions -- time -- sh -c <run>`.
  Instructions are user-space only (`perf_event_paranoid=2`) and summed over all threads. Exit 0
  accepts, 1 rejects, 2 declines; anything else is an error.
- **Timeouts.** A test's `timeout` kills the whole process group; a timeout or an error scores as
  a decline. The limits (seconds) are derived from the official checker's wall times by
  `set-timeouts.py`: mathlib 5680, navier-stokes-euler 2400, cslib 1800, cedar 940, con-leche 720,
  std 590, init 500, perf/magma-list-deep-n36 390, perf/magma-list-pair-n21 260,
  perf/magma-string-pair-n9 130, perf/app-lam 100, perf/magma-list-deep-n21 80, perf/magma-string-n4
  70, perf/grind-ring-5 and perf/magma-list-pair-n7 60, init-prelude and the other perf tests 50; the
  rest have none. A checker may also declare its own `timeout`, and the smaller limit applies; both
  entries leave it out, so each test's limit is the one in force.
- **Inputs.** The tests are exported with Lean v4.34.1 (cslib with v4.34.0, navier-stokes-euler with
  v4.34.0-rc2, con-leche with v4.33.0, and `bugs/closure-level-subst`, `bugs/pi-closure-context` and
  `corner-cases/large-elim-maybe-prop` with v4.35.0) by lean4export 3.1.0 (`other/level-index-out-of-order`
  and `other/sparse-name-index` record 0.1.0); `results.json` publishes each export's SHA-256.
- **What pull-request CI runs.** Tests marked `skip-on-ci` — init, std, cedar, cslib, con-leche,
  navier-stokes-euler and mathlib — are not run on a pull request. They first run when the merge lands on `master`:
  every push to `master` builds every checker on every test and deploys the site only if every
  checker job succeeds, so a checker job that fails there keeps the whole site from deploying. A
  newer push to `master` cancels the build still running.
- **Rounds.** A round is closed from the results of an existing successful `master` build, with
  nothing measured again, so an entry is in a round only once it is merged and a `master` build
  containing it has succeeded.
- **Rank.** Checkers sort by bad tests not rejected, then good tests not accepted, then mathlib
  instructions (only if mathlib was accepted; otherwise after every checker that was), then
  declines. The site shows instructions as time at 6.0 G instructions per second.
- **Threads.** `threads: N` reserves N CPU slots for a checker that runs N workers; the maintainer
  asks for at most 4. Both entries are single-threaded and declare none.
- **Version.** `version` if given, else `<ref> (<first 7 of rev>)`, or the short revision alone
  when `ref` is `master` or `main`.

## Submitting

Nothing here is pushed or submitted automatically. In order:

1. **The pinned commit is public.** It is a commit of `https://github.com/Brahmastra-Labs/logicaffeine`
   containing this crate, `logicaffeine-base` and a `Cargo.lock` in step with every workspace manifest
   (both builds are `--locked`).
2. **Pin it.** Replace `SUBMISSION_REF` and `SUBMISSION_REV` with its tag and full 40-character
   commit hash, together.
3. **Rehearse against it** (below): the pull-request tests with `--ref`/`--rev`, then `--heavy`,
   each correct, with wall time well inside every timeout and peak memory well inside 16 GB.
4. **The Arena has not moved.** The rehearsal refuses a checker schema that differs from
   `checker.schema.json`. If the Arena changed, re-read its README and `lka.py`, then re-pin the
   revision here, in `crates/logicaffeine_tests/tests/arena_entry_lock.rs` and in the rehearsal.
5. **Open the pull request** from a fork of lean-kernel-arena, adding the pinned
   `checkers/logos-kernel.yaml` (and `checkers/logos-dual.yaml`) and nothing else. Open it as a
   draft until it should be merged; the maintainer merges ready pull requests quickly. Pull
   request text should be written by a person, and anything an agent wrote marked as such: the
   maintainer has asked for exactly that.
6. **Read the pull request's CI.** The `Checker: logos-kernel` job's log has the build and every
   verdict; the built site is the `report.html` artifact.

## Rehearsing

`scripts/lean/arena-crown/rehearse-submission.py` runs the Arena's own `lka.py`, at the pinned revision,
on one entry: schema validation, `build-checker` with the entry's build script in the Arena's
directory layout, then `run` over the tests pull-request CI runs, each with its own timeout. It
builds what the public repository would contain (the root manifest and lockfile and every workspace
member's files as the next commit would hold them: tracked or trackable, less those deleted), with rustc
1.95.0 and an empty cargo home, strips cargo and rustc settings from
the environment, and stages the tests from the vendored corpus (`tests/fixtures/arena_corpus`), so no
Lean export is rebuilt but the `init` and `init-prelude` the entry's build exports itself, with `lka.py`'s
interpreter and the elan on the rehearsing shell's `PATH`; those two run and are judged as well.

```bash
rustup toolchain install 1.95.0 --profile minimal --component llvm-tools   # once

# this checkout's published workspace (its workspace members' files as the next commit would hold them)
python3 scripts/lean/arena-crown/rehearse-submission.py --work /tmp/arena-rehearsal/logos-kernel

# the public commit, exactly as submitted
python3 scripts/lean/arena-crown/rehearse-submission.py --work /tmp/arena-rehearsal/logos-kernel \
    --ref <tag> --rev <commit>

# also the skip-on-ci corpora, from their local exports
python3 scripts/lean/arena-crown/rehearse-submission.py --work /tmp/arena-rehearsal/logos-kernel --heavy
```

The skip-on-ci exports are not published. `scripts/lean/arena-crown/build-arena-inputs.py` rebuilds them
the way `lka.py build-test` does, from source tarballs, outside the repository, and keeps an export
only if its SHA-256 is the one `results.json` publishes; `vendor-corpus.py` then records them as the
corpus's heavy cases.

The build and every test share 16 GB and no swap (`--memory`), inside a transient systemd scope:
the runner's memory, so a checker that would be killed there is killed here, and a runaway
cannot push the rest of the machine into the OOM killer.

`--work` must lie outside every cargo workspace: cargo reads `.cargo/config.toml` from each parent
directory, and a developer's local linker or rustflags would otherwise leak into the build. Where
this user may not count events (`perf_event_paranoid` above 2), a stand-in for `perf` runs each
test unmeasured by perf, so lka.py still records wall clock and peak RSS but no instructions;
`scripts/lean/arena-crown/instr.sh` counts them the way the Arena does.

## Protocol

`logos_kernel [--check sem|classic] <file>` and `logos_dual <file>` (or standard input) exit 0 when
every declaration is admitted, 1 on a rejection, 2 on a decline; the reason is written to standard
error.
