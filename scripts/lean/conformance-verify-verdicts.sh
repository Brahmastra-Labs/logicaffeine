#!/usr/bin/env bash
#
# conformance-verify-verdicts.sh — THE SCOREBOARD MUST NOT BE ABLE TO LIE.
#
# `crates/logicaffeine_tests/tests/lean_parity_matrix.rs` checks that every one of the 248
# corpus cases is classified, that no two cases share a decider, that every DIVERGE/OUT carries an
# argument, and that per-tier `TIER_FLOORS` never falls. It reads `manifest.json` and
# `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/*.json` as TEXT. It never runs a single corpus test. So
# `"status": "Native"` is an unverified ASSERTION about a capability, and `TIER_FLOORS` is a ratchet
# on a number nobody measured.
#
# That hole was live and it was lying in BOTH directions at once: cases recorded Native whose test
# FAILS (a capability we claim and do not have — the thing that loses a head-to-head), and cases
# recorded Gap whose test PASSES (free wins nobody was counting, and a floor left artificially low).
# Both were found only by hand-joining a failure list against the verdict JSONs. This script is that
# join, automated, and wired into the baseline so it happens on every run.
#
# WHAT IT DOES
#   1. Discovers every test binary any verdict names — from the verdict files themselves, never a
#      hard-coded list — and locates each as `crates/<crate>/tests/<binary>.rs`.
#   2. LISTS them (`cargo nextest list --message-format json`) for the ground-truth inventory:
#      which tests exist, and which are `#[ignore]`d.
#   3. RUNS them (`--message-format libtest-json-plus`) for the ground-truth result.
#   4. Joins both against the verdicts BY TEST NAME and reports every disagreement.
#
# THE LIES IT REFUSES (any one of them exits 1)
#   NATIVE-BUT-RED      a capability we claim and do not have.           <- the headline
#   GAP-BUT-GREEN       a capability we have and do not count.           <- a free win, and a floor
#                                                                          left too low. Each row
#                       prints the floor edit that banks it, CAPPED at that tier's CAN-BE-NATIVE
#                       column — never `current + free`, which overshoots in any tier that also has
#                       a NATIVE-BUT-RED and would recommend a ratchet resting on a failing test.
#                       MEASURED 2026-09-15: the naive form was wrong in two of thirteen tiers.
#   REFUSED-BUT-RED     the refusal we advertise is not clean.
#   NO-SUCH-TEST        a verdict names a test that does not exist.      <- usually a rename
#   AMBIGUOUS-BINARY    a verdict's bare binary name exists in more than one crate, so the gate
#                       cannot tell the two apart and a case could be decided by a same-named test
#                       in an unrelated crate. MEASURED 2026-09-15: eight test-file stems are
#                       duplicated across this workspace today (none of them a corpus binary), so
#                       this is a live hazard, not a hypothetical.
#   IGNORED-DECIDER     a case whose decider never runs is unclassified.
#   WRONG-TIER          a verdict file classifying a case the corpus puts in ANOTHER tier. The
#                       matrix checks each tier file's case COUNT against its directory, never the
#                       identities, so a misfiled case silently moves a Native from one tier's
#                       floor to another's — and `TIER_FLOORS` is per tier.
#   SHARED-DECIDER      two cases resolving to the SAME (binary, test): one of them is never
#                       actually exercised and reads green for free. `lean_parity_matrix.rs` has a
#                       uniqueness check, but it compares the raw `test` STRING — so a case spelled
#                       `lean_conformance_defects::foo` and one spelled bare `foo` in a tier whose
#                       `test_binary` IS `lean_conformance_defects` are the same decider and it cannot see
#                       it. Resolving first, then checking uniqueness, closes that.
#   NOT-OBSERVED        the binary ran and produced no row for the decider at all.
#   OUT-BOOKED-AS-NATIVE
#                       an `OUT` case — one whose test asserts a CLEAN REFUSAL — recorded `Native`
#                       instead of `Refused`. Every pass/fail column reads OK, because the test
#                       passes and passes for the right reason; only the CLASSIFICATION is wrong,
#                       and the classification is what ratchets. `Native` counts toward
#                       TIER_FLOORS and `Refused` deliberately does not, so booking one this way
#                       claims a capability whose test only ever proved a decline.
#   FLOOR-ABOVE-TRUTH   a TIER_FLOORS entry higher than the number of cases in that tier that can
#                       honestly BE Native — passing, not `Refused`, and not an `OUT` case whose
#                       test proves a refusal. The ratchet resting on a claim, not a measurement.
#
# TWO BUILDS, ONE ANSWER. `list` and `run` are separate cargo invocations, so on a workspace being
# edited underneath them the inventory and the results can come from two source states. That skew
# does not hide: a decider that the inventory has and the run does not becomes NOT-OBSERVED, which is
# a loud finding and not a pass. Under eleven concurrent editors the honest response to a
# NOT-OBSERVED row is to re-run, and the report says so.
#
# RETRIES. Rows arrive in order and the LAST row for a test wins, which is its final status — the
# right answer if a profile ever sets `retries > 0` (`.config/nextest.toml`'s default sets 0).
#
# A CRASH IS NOT A PASS. `grep FAIL` misses an abort, and that cost this campaign a day. nextest
# reports a signal death as an ordinary `{"type":"test","event":"failed"}` row; the only trace is
# text inside that row's `stdout`. So every row is additionally scanned for a crash/timeout
# signature and labelled ABORTED or TIMEOUT. A Gap that kills its process is a different fact from a
# Gap that asserts, and the two must never be laundered into the same cell.
#
# ---- ON FRESHNESS, WHICH IS THE WHOLE POINT ---------------------------------------------------
# The obvious design is to commit a results file and have a Rust lock read it with `include_str!`.
# DO NOT. That file is stale the instant anyone edits kernel source, and NOTHING in the repo can
# tell: it still parses, the lock still reads it, the lock still goes green. It is a photograph of a
# machine that no longer exists, and a green light nobody can distinguish from a measurement — which
# is precisely the dishonesty this script exists to close, rebuilt one layer up.
#
# So: the results are produced in the SAME invocation that reports them, always. The only artifact
# written lands in `logs/` (gitignored — it CANNOT become a committed input), carries a loud
# `_warning`, and is never read back by anything. Three locks keep it that way, all in
# `crates/logicaffeine_tests/tests/H_honesty_extra.rs`:
#   * `the_verdict_gate_is_wired_into_the_baseline` — deleting the gate from run-all-tests-fast.sh
#     is itself a RED test. That is the one staleness that can actually hurt you.
#   * `no_test_reads_a_recorded_verdict_verification_result` — a scan that fails the moment any Rust
#     source reads one of this script's artifacts. The trap is shut before anyone falls in it.
#   * `every_corpus_binary_a_verdict_names_exists_on_disk` — the discovery step, checked without a
#     build, so a renamed binary is loud in milliseconds.
#
# USAGE
#   ./scripts/lean/conformance-verify-verdicts.sh [--cargo-profile P] [--features F] [--json PATH] [--quiet]
#
#   --cargo-profile P  cargo profile to build with. Default `test-opt`, which is what
#                      run-all-tests-fast.sh uses, so this pass reuses that build instead of
#                      making a second one.
#   --features F       workspace features. Default NONE: not one of the 16 corpus binaries needs a
#                      feature, and every feature added is another crate that must build (and Z3
#                      must be installed) before the scoreboard can be checked. The baseline passes
#                      `--features verification` explicitly, because THERE the artifacts already
#                      exist from pass 1 and a different feature set is a different metadata hash
#                      and therefore a second full build.
#   --json PATH        also write the full join as JSON. Default `logs/conformance-verdicts-<ts>.json`
#                      plus a `logs/latest-verdicts.json` symlink. `--json ''` writes nothing.
#   --quiet            suppress the per-tier scoreboard; keep the mismatch lists and the verdict.
#   --keep             keep the temporary nextest JSON (for debugging this script).
#   --print-binaries   print the discovered corpus binaries and exit. No cargo, milliseconds. This
#                      is what `H_honesty_extra::every_corpus_binary_a_verdict_names_exists_on_disk`
#                      cross-checks against its own independent resolver, so the two never drift.
#   --selftest         feed the join synthetic verdicts and synthetic results in which EVERY lie is
#                      present, and fail unless it catches all of them. No cargo, milliseconds. A
#                      lie-detector nobody has lied to is a lie-detector nobody has tested — and
#                      this campaign already lost a ratchet exactly that way (REG-C: the anti-vacuity
#                      detector's own self-test had been failing, so the guard was protecting
#                      nothing while reading green).
#
# EXIT CODES
#   0  every verdict agrees with reality.
#   1  MISMATCH — at least one of the lies above. The report says which.
#   2  COULD NOT COLLECT — the corpus did not build, or a run produced no rows at all. This is
#      deliberately NOT exit 1: "the scoreboard is lying" and "I could not read the scoreboard" are
#      different facts, and reporting a build break as 248 missing tests would be its own lie.
#   3  usage error.
#
# NOTE: this script has no "one suite at a time" guard on purpose — it is invoked from INSIDE
# run-all-tests-fast.sh, which already holds that guard. A second guard here would refuse against
# its own parent.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

CARGO_PROFILE="test-opt"
FEATURES=""
JSON_OUT="__default__"
QUIET=0
KEEP_WORKDIR=0
PRINT_BINARIES=0
SELFTEST=0

while [ $# -gt 0 ]; do
  case "$1" in
    --cargo-profile) CARGO_PROFILE="${2-}"; shift 2 ;;
    --features)      FEATURES="${2-}";      shift 2 ;;
    --json)          JSON_OUT="${2-}";      shift 2 ;;
    --quiet)         QUIET=1;               shift ;;
    --keep)          KEEP_WORKDIR=1;        shift ;;
    --print-binaries) PRINT_BINARIES=1;     shift ;;
    --selftest)      SELFTEST=1;            shift ;;
    # Print the header comment up to (not including) the `set -uo pipefail` line, so the help can
    # never drift out of range as the header grows.
    -h|--help)       sed -n '2,/^set -uo pipefail/p' "${BASH_SOURCE[0]}" | sed '$d'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 3 ;;
  esac
done

# Z3 header for the `verification` feature (Linux apt path; override if needed) — the same line
# run-all-tests-fast.sh uses, so the two builds are identical and share artifacts.
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

# --print-binaries and --selftest touch no cargo at all; that is what lets the Rust locks run them
# in every baseline for the price of a process spawn.
if [ "$PRINT_BINARIES" = "0" ] && [ "$SELFTEST" = "0" ]; then
  if ! cargo nextest --version >/dev/null 2>&1; then
    echo "ERROR: cargo-nextest not found. Install with:" >&2
    echo '  curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C "$HOME/.cargo/bin"' >&2
    exit 2
  fi
fi

WORKDIR="$(mktemp -d)"
cleanup() { if [ "$KEEP_WORKDIR" = "1" ]; then echo "workdir kept: $WORKDIR" >&2; else rm -rf "$WORKDIR"; fi; }
trap cleanup EXIT

# The join lives in one embedded Python program with two entry points (`discover`, `report`). It is
# written out here rather than kept as a sibling .py so this gate is ONE file: a verifier split
# across two files is a verifier that can be half-deleted.
VERIFY_PY="$WORKDIR/verify.py"
cat > "$VERIFY_PY" <<'PY_EOF'
"""Join the conformance corpus's verdicts against what the corpus tests ACTUALLY did."""
import datetime
import difflib
import json
import os
import re
import sys

# A signal death arrives as an ordinary `failed` row whose only trace is text in `stdout`. Verified
# live on lean_conformance_t00_terms::a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically,
# whose stdout reads "...has overflowed its stack\nfatal runtime error: stack overflow, aborting".
CRASH_RE = re.compile(
    r"SIGABRT|SIGSEGV|SIGILL|SIGBUS|SIGFPE|signal \d+|fatal runtime error"
    r"|has overflowed its stack|Segmentation fault|core dumped|memory allocation of \d+ bytes failed",
    re.I,
)
TIMEOUT_RE = re.compile(r"\btimed out\b|\bTIMEOUT\b|terminate-after|slow-timeout", re.I)

# The corpus, relative to the repository root.
CORPUS = ("crates", "logicaffeine_tests", "tests", "fixtures", "lean_conformance")


def verdict_files(root):
    d = os.path.join(root, *CORPUS, "verdicts")
    if not os.path.isdir(d):
        sys.stderr.write("ERROR: %s does not exist\n" % d)
        sys.exit(1)
    out = []
    for fn in sorted(os.listdir(d)):
        if not fn.endswith(".json"):
            continue
        p = os.path.join(d, fn)
        try:
            out.append((fn, json.load(open(p, encoding="utf-8"))))
        except Exception as e:  # a malformed verdict file is a hard stop, never a silent skip
            sys.stderr.write("ERROR: %s is not valid JSON: %s\n" % (p, e))
            sys.exit(1)
    return out


def resolve(case, tier):
    """(binary, test fn) for a case. `test` is either a bare fn or a qualified `binary::fn`.

    The qualified branch is MANDATORY, not defensive: 11 of the 248 use it today (a case whose
    decider lives in a different binary from its tier's), and a resolver that trusts `test_binary`
    alone reports all 11 as missing tests.
    """
    t = (case.get("test") or "").strip()
    if "::" in t:
        head, fn = t.rsplit("::", 1)
        return head.split("::")[-1], fn
    return tier.get("test_binary", "<none>"), t


def locate_binary(root, binary):
    # EVERY match, never the first. A verdict names a binary by BARE name, and test-binary names are
    # not unique across this workspace (eight duplicated stems, measured), so a first-match resolver
    # would happily satisfy a verdict with a same-named test in an unrelated crate.
    out = []
    for base in ("crates", "apps"):
        b = os.path.join(root, base)
        if not os.path.isdir(b):
            continue
        for d in sorted(os.listdir(b)):
            p = os.path.join(b, d, "tests", binary + ".rs")
            if os.path.isfile(p):
                out.append(os.path.relpath(p, root))
    return out


def all_binaries(root):
    seen = []
    for _fn, tier in verdict_files(root):
        for case in tier.get("cases", []):
            b, _ = resolve(case, tier)
            if b and b not in seen:
                seen.append(b)
    return seen


def cmd_discover(root):
    missing, ambiguous = [], []
    for b in all_binaries(root):
        found = locate_binary(root, b)
        if not found:
            missing.append(b)
        elif len(found) > 1:
            ambiguous.append("%s -> %s" % (b, ", ".join(found)))
        else:
            print(b)
    if missing:
        sys.stderr.write(
            "ERROR: a verdict names a test binary that has no source file:\n  %s\n"
            "  (looked for crates/*/tests/<binary>.rs and apps/*/tests/<binary>.rs)\n"
            % "\n  ".join(missing)
        )
    if ambiguous:
        sys.stderr.write(
            "ERROR: a verdict names a test binary that exists in MORE THAN ONE crate. `--test <name>`\n"
            "       selects by name across the whole workspace, so these are two binaries this gate\n"
            "       cannot tell apart — a case could be decided by a same-named test in an unrelated\n"
            "       crate and read green. Rename one, or qualify the verdict:\n  %s\n"
            % "\n  ".join(ambiguous)
        )
    if missing or ambiguous:
        sys.exit(1)


def read_inventory(path):
    # binary -> {test name: is_ignored}, from `cargo nextest list --message-format json`, PLUS the
    # set of binary-ids each name resolved to. Two suites under one name is not something to merge:
    # merging their test lists is exactly how a verdict gets satisfied by the wrong crate's test.
    data = json.load(open(path, encoding="utf-8"))
    inv, ids = {}, {}
    for suite_id, suite in (data.get("rust-suites") or {}).items():
        name = suite.get("binary-name") or suite_id.split("::")[-1]
        ids.setdefault(name, set()).add(suite_id)
        slot = inv.setdefault(name, {})
        for tname, meta in (suite.get("testcases") or {}).items():
            slot[tname] = bool((meta or {}).get("ignored", False))
    return inv, {n: sorted(v) for n, v in ids.items() if len(v) > 1}


def read_results(path):
    """(binary, test) -> PASS | FAIL | ABORTED | TIMEOUT | IGNORED | UNKNOWN:<event>."""
    out = {}
    odd = []
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            row = json.loads(line)
        except Exception:
            continue
        if row.get("type") != "test":
            continue
        event = row.get("event")
        if event in (None, "started"):
            continue
        name = row.get("name") or ""
        if "$" in name:
            binary_id, tname = name.split("$", 1)
            binary = binary_id.split("::")[-1]
        else:
            binary, tname = "<unknown>", name
        blob = "%s%s" % (row.get("stdout") or "", row.get("stderr") or "")
        if event == "ok":
            status = "PASS"
        elif event == "ignored":
            status = "IGNORED"
        elif event in ("failed", "timeout"):
            status = "FAIL"
        else:
            status = "UNKNOWN:%s" % event
            odd.append(event)
        if status != "PASS" and status != "IGNORED":
            # A crash and an assertion are different facts and must not share a cell.
            if CRASH_RE.search(blob):
                status = "ABORTED"
            elif event == "timeout" or TIMEOUT_RE.search(blob):
                status = "TIMEOUT"
        out[(binary, tname)] = status
    return out, sorted(set(odd))


def manifest_tiers(root):
    # The corpus manifest is the AUTHORITY on which tier a case belongs to; a verdict file's own
    # `tier` field is a claim. lean_parity_matrix.rs checks each tier file's case COUNT against its
    # directory but never the identities, so a case filed under the wrong tier passes every existing
    # check while moving its Native count — and therefore its floor — to the wrong tier.
    p = os.path.join(root, *CORPUS, "manifest.json")
    out = {}
    try:
        data = json.load(open(p, encoding="utf-8"))
    except (OSError, ValueError):
        return out
    for c in data.get("cases", []):
        seg = [s for s in (c.get("path") or "").split("/") if s.startswith("t") and "_" in s]
        if seg:
            out[c.get("id")] = seg[0]
    return out


def read_tier_floors(root):
    """The TIER_FLOORS ratchet, read as text. Never written — the orchestrator keeps the scoreboard."""
    p = os.path.join(root, "crates", "logicaffeine_tests", "tests", "lean_parity_matrix.rs")
    try:
        src = open(p, encoding="utf-8").read()
    except OSError:
        return {}
    m = re.search(r"const TIER_FLOORS[^=]*=\s*&\[(.*?)\];", src, re.S)
    if not m:
        return {}
    return {k: int(v) for k, v in re.findall(r'\("([^"]+)",\s*(\d+)\)', m.group(1))}


def head_commit(root):
    try:
        head = open(os.path.join(root, ".git", "HEAD"), encoding="utf-8").read().strip()
    except OSError:
        return "unknown"
    if head.startswith("ref:"):
        ref = head.split(" ", 1)[1].strip()
        try:
            return open(os.path.join(root, ".git", ref), encoding="utf-8").read().strip()[:12]
        except OSError:
            return "unknown"
    return head[:12]


# The whole join rests on this table. `Refused` expects PASS, not FAIL: crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/
# README.md defines Refused as "we decline, deliberately and loudly" and its test as one that
# "asserts we refuse cleanly" — so a green Refused test IS the refusal working, and a red one is a
# refusal that leaked. (The fix brief's shorthand lumps Gap and Refused together as "expected to
# fail"; that is right for Gap and backwards for Refused, and the schema is the authority.)
EXPECTED = {"Native": "PASS", "Refused": "PASS", "Gap": "NOT-PASS"}

# The `verdict` field's legal values, from the same schema. An unrecognised one evades
# lean_parity_matrix.rs's "every DIVERGE and OUT states its argument" rule, because that rule keys
# off the exact strings too.
VERDICT_KINDS = ("MUST", "DIVERGE", "OUT")


def cmd_report(root, list_json, run_json, json_out, quiet):
    inventory, collided = read_inventory(list_json)
    results, odd_events = read_results(run_json)
    floors = read_tier_floors(root)
    corpus_tier = manifest_tiers(root)

    rows = []
    for _fn, tier in verdict_files(root):
        tname_tier = tier.get("tier", "<none>")
        for case in tier.get("cases", []):
            binary, test = resolve(case, tier)
            claimed = case.get("status", "<none>")
            want = EXPECTED.get(claimed, "?")
            row = {
                "id": case.get("id", "<none>"),
                "tier": tname_tier,
                "binary": binary,
                "test": test,
                "verdict": case.get("verdict", "<none>"),
                "claimed": claimed,
                "expected": want,
                "observed": None,
                "category": None,
                "note": "",
            }
            if claimed not in EXPECTED:
                # `want` is "?" here, and every comparison below it is then False, so the row used
                # to fall through to the final `else` and be recorded OK — whatever its test did.
                # A one-character typo in a status string ("native", "Gap ") therefore switched the
                # scoreboard's own lie-detector off for that case, silently. That is the exact class
                # of hole this gate exists to close, rebuilt one level up.
                row["observed"] = results.get((binary, test), "<not observed>")
                row["category"] = "MALFORMED-VERDICT"
                row["note"] = ("status %r is not one of %s, so no expectation can be computed for "
                               "this case at all" % (claimed, "/".join(EXPECTED)))
            elif binary in collided:
                row["observed"] = "<ambiguous>"
                row["category"] = "AMBIGUOUS-BINARY"
                row["note"] = "resolves to %s" % ", ".join(collided[binary])
            elif binary not in inventory:
                row["observed"] = "<no such binary>"
                row["category"] = "NO-SUCH-TEST"
                row["note"] = "binary %s was collected by no nextest suite" % binary
            elif test not in inventory[binary]:
                row["observed"] = "<no such test>"
                row["category"] = "NO-SUCH-TEST"
                near = difflib.get_close_matches(test, list(inventory[binary]), n=3, cutoff=0.6)
                row["note"] = "closest in %s: %s" % (binary, ", ".join(near) if near else "(nothing close)")
            elif inventory[binary][test]:
                row["observed"] = "IGNORED"
                row["category"] = "IGNORED-DECIDER"
                row["note"] = "a case whose decider never runs is unclassified, not Native"
            else:
                observed = results.get((binary, test))
                if observed is None:
                    row["observed"] = "<not observed>"
                    row["category"] = "NOT-OBSERVED"
                    row["note"] = ("the binary ran but emitted no row for this test — if the "
                                   "workspace was being edited, `list` and `run` saw two source "
                                   "states; re-run")
                else:
                    row["observed"] = observed
                    passing = observed == "PASS"
                    if want == "PASS" and not passing:
                        row["category"] = "NATIVE-BUT-RED" if claimed == "Native" else "REFUSED-BUT-RED"
                    elif want == "NOT-PASS" and passing:
                        row["category"] = "GAP-BUT-GREEN"
                    else:
                        row["category"] = "OK"
            rows.append(row)

    # Two cases cannot be decided by ONE test. lean_parity_matrix.rs checks this on the raw `test`
    # string, which cannot see that `lean_conformance_defects::foo` and a bare `foo` in a tier whose
    # test_binary is `lean_conformance_defects` name the same function. Resolution happened above, so the
    # check is exact here: whichever of the two is never separately exercised reads green for free.
    shared = {}
    for r in rows:
        if r["category"] == "NO-SUCH-TEST":
            continue
        shared.setdefault((r["binary"], r["test"]), []).append(r["id"])
    shared = {k: v for k, v in shared.items() if len(v) > 1}
    for r in rows:
        if (r["binary"], r["test"]) in shared and r["category"] == "OK":
            r["category"] = "SHARED-DECIDER"
            r["note"] = "also decides %s" % ", ".join(
                i for i in shared[(r["binary"], r["test"])] if i != r["id"])

    # A case filed under the wrong tier is tracked separately from the pass/fail categories, so it
    # can never be masked by one: a case can be BOTH misfiled and red, and both facts matter.
    wrong_tier = [
        (r["id"], r["tier"], corpus_tier[r["id"]])
        for r in rows
        if r["id"] in corpus_tier and corpus_tier[r["id"]] != r["tier"]
    ]

    # An `OUT` case booked as `Native`. Not a pass/fail lie — its test passes, and passes for the
    # right reason — but a CLASSIFICATION lie, and it lands on the one number that ratchets:
    # `Native` counts toward `TIER_FLOORS` and `Refused` deliberately does not, so booking a clean
    # refusal as a native capability lifts that tier's floor by one and claims, in the one place a
    # Lean person reads, a capability whose test only ever proved a decline. Tracked separately
    # from the pass/fail categories for the same reason WRONG-TIER is: a case can be both.
    out_as_native = [
        (r["id"], r["tier"], r["test"], r["observed"])
        for r in rows
        if r["verdict"] == "OUT" and r["claimed"] == "Native"
    ]

    # A verdict file whose own fields are not the ones the schema defines. Tracked per FIELD, not
    # per row, because a case can be malformed twice. A bad `status` has already taken over the
    # row's category above (no expectation exists for it); a bad `verdict` leaves the pass/fail
    # category alone, because it does not change what the test should do — folding the two together
    # would let a typo'd `verdict` mask a NATIVE-BUT-RED, which is the opposite of the point.
    malformed = []
    for r in rows:
        if r["claimed"] not in EXPECTED:
            malformed.append((r["id"], r["tier"], "status", r["claimed"], r))
        if r["verdict"] not in VERDICT_KINDS:
            malformed.append((r["id"], r["tier"], "verdict", r["verdict"], r))

    by_cat = {}
    for r in rows:
        by_cat.setdefault(r["category"], []).append(r)

    # --- per-tier scoreboard -------------------------------------------------------------------
    tiers = []
    for _fn, tier in verdict_files(root):
        t = tier.get("tier", "<none>")
        mine = [r for r in rows if r["tier"] == t]
        claimed_native = sum(1 for r in mine if r["claimed"] == "Native")
        claimed_gap = sum(1 for r in mine if r["claimed"] == "Gap")
        claimed_ref = sum(1 for r in mine if r["claimed"] == "Refused")
        true_pass = sum(1 for r in mine if r["observed"] == "PASS")
        # What a floor can HONESTLY rest on. `lean_parity_matrix.rs`'s own ratchet
        # (`the_capability_floor_never_regresses`) counts cases whose STATUS is `Native` — it does
        # not count passing tests. A `Refused` case is deliberately not Native however green its
        # test is, so counting it here made this check strictly WEAKER than the rule it exists to
        # protect: `t09_metaprogramming` has four passing `Refused` cases, which lifted its
        # "truly passing" to 13 and hid a floor of 10 sitting above only 9 supportable cases.
        # …and an `OUT` case can never be Native however green it is. verdicts/README.md: "`OUT` —
        # genuinely out of scope … The test asserts we **refuse cleanly**; never that we
        # half-work", and "`Refused` — we decline, deliberately and loudly. Correct for `OUT`". A
        # passing OUT test proves a DECLINE, not a capability, so counting it here would let a
        # clean refusal hold up a capability floor. MEASURED 2026-09-15: exactly one case in the
        # corpus is booked that way, and it is holding `t09_metaprogramming`'s floor at 10.
        supportable = sum(
            1 for r in mine
            if r["observed"] == "PASS" and r["claimed"] != "Refused"
            and not (r["verdict"] == "OUT" and r["claimed"] == "Native"))
        bad = sum(1 for r in mine if r["category"] not in ("OK",))
        tiers.append((t, len(mine), claimed_native, claimed_gap, claimed_ref, true_pass,
                      supportable, floors.get(t), bad))

    # The CEILING a tier's floor may honestly reach, by tier. Read by the GAP-BUT-GREEN "bank it"
    # hint below, which must never name a number this one does not support — see the note there.
    supportable_by_tier = {t: sup for (t, _n, _cn, _cg, _cr, _tp, sup, _f, _b) in tiers}

    floor_lies = []
    for (t, _n, _cn, _cg, _cr, _true_pass, supportable, floor, _bad) in tiers:
        if floor is not None and floor > supportable:
            floor_lies.append((t, floor, supportable))

    if not quiet:
        print("  TIER                     cases | claimed: Native  Gap  Refused | TRULY PASSING | CAN BE NATIVE | floor | mismatches")
        for (t, n, cn, cg, cr, tp, sup, floor, bad) in tiers:
            fl = "-" if floor is None else str(floor)
            flag = "" if (floor is None or floor <= sup) else "  <-- FLOOR ABOVE TRUTH"
            print("  %-22s %6d |          %6d %4d %8d | %13d | %13d | %5s | %10d%s"
                  % (t, n, cn, cg, cr, tp, sup, fl, bad, flag))
        tot_n = sum(r["claimed"] == "Native" for r in rows)
        tot_g = sum(r["claimed"] == "Gap" for r in rows)
        tot_r = sum(r["claimed"] == "Refused" for r in rows)
        tot_p = sum(r["observed"] == "PASS" for r in rows)
        tot_s = sum(r["observed"] == "PASS" and r["claimed"] != "Refused" for r in rows)
        tot_b = sum(r["category"] != "OK" for r in rows)
        print("  %-22s %6d |          %6d %4d %8d | %13d | %13d | %5s | %10d"
              % ("TOTAL", len(rows), tot_n, tot_g, tot_r, tot_p, tot_s,
                 sum(v for v in floors.values()) if floors else "-", tot_b))
        print()
        print("  claimed capable (Native + Refused): %d    ACTUALLY passing: %d"
              % (tot_n + tot_r, tot_p))
        print()

    order = [
        ("NATIVE-BUT-RED",
         "a capability we CLAIM and do not have. This is what loses a head-to-head."),
        ("GAP-BUT-GREEN",
         "a capability we HAVE and do not count. Free wins, and a floor left artificially low."),
        ("REFUSED-BUT-RED",
         "the refusal we advertise is not clean — it fails for some other reason."),
        ("NO-SUCH-TEST",
         "a verdict names a test that does not exist. Usually a rename that left the verdict behind."),
        ("AMBIGUOUS-BINARY",
         "the bare binary name resolves to two suites; the gate cannot tell which decides the case."),
        ("IGNORED-DECIDER",
         "the test that decides this case never runs, so the case is unclassified."),
        ("NOT-OBSERVED",
         "the binary ran and emitted no row for this test at all. Re-run: `list` and `run` are two "
         "builds, and on a workspace being edited they can see two source states."),
        ("SHARED-DECIDER",
         "two cases decided by ONE test — whichever is not separately exercised reads green free."),
    ]
    for cat, blurb in order:
        found = by_cat.get(cat, [])
        if not found:
            continue
        print("  == %s == (%d)  %s" % (cat, len(found), blurb))
        for r in found:
            extra = ""
            if cat == "GAP-BUT-GREEN":
                floor = floors.get(r["tier"])
                cn = sum(1 for x in rows if x["tier"] == r["tier"] and x["claimed"] == "Native")
                here = sum(1 for x in found if x["tier"] == r["tier"])
                # The floor this hint names is the tier's CAN-BE-NATIVE column, never
                # `current + free`. Those two differ by exactly the tier's claimed-Native cases
                # that are not passing, and a hint that ignores them tells the orchestrator to
                # bank a floor resting on a RED test — which `the_capability_floor_never_regresses`
                # would wave through (it counts Native STATUSES) and which this gate's own
                # FLOOR-ABOVE-TRUTH would then fire on. MEASURED 2026-09-15: the naive arithmetic
                # overshot in two of the thirteen tiers on the live corpus (t00_terms 17 vs 16,
                # t03_inductives 21 vs 20), i.e. the gate was handing out two bad floors per run.
                sup = supportable_by_tier.get(r["tier"], cn + here)
                short = (cn + here) - sup
                extra = "   [bank it: %s Native %d -> %d (%d free in this tier), floor %s -> %d%s]" % (
                    r["tier"], cn, cn + here, here, floor if floor is not None else "-", sup,
                    "" if short <= 0 else
                    " — %d claimed Native here cannot hold a floor (its test does not pass, or its "
                    "verdict is OUT and a clean refusal is not a capability), so the floor stops "
                    "at %d" % (short, sup))
            print("     %-9s %-20s %s::%s" % (r["id"], r["tier"], r["binary"], r["test"]))
            print("        expected %-9s observed %-16s %s%s"
                  % (r["expected"], r["observed"], r["note"], extra))
        print()

    if wrong_tier:
        print("  == WRONG-TIER == (%d)  a verdict file classifies a case the corpus puts elsewhere."
              % len(wrong_tier))
        print("     The per-tier Native count, and therefore that tier's TIER_FLOORS entry, is")
        print("     being credited to the wrong tier.")
        for (cid, claimed, actual) in wrong_tier:
            print("     %-9s filed under %-22s corpus says %s" % (cid, claimed, actual))
        print()

    if malformed:
        print("  == MALFORMED-VERDICT == (%d)  a verdict field the schema does not define." % len(malformed))
        print("     A status outside Native/Gap/Refused has NO expectation, so the case is")
        print("     unclassified — and before this finding existed it read OK whatever its test did.")
        print("     A verdict outside MUST/DIVERGE/OUT evades the `why` requirement in the matrix.")
        # The row's own CATEGORY is printed, and it is the load-bearing column: a malformed
        # `status` must have taken the category over (no expectation exists), while a malformed
        # `verdict` must have left it exactly as the test decided. Printing it is what lets the
        # self-test tell those two apart — without it, deleting the interception above changes
        # nothing observable and the selftest waves the mutation through. It did, once, measured.
        for (cid, tier, field, value, r) in malformed:
            # `expected` is printed too, and it is load-bearing: for an unrecognised STATUS it must
            # read `?`, because no expectation exists. Without it on the page, changing
            # `EXPECTED.get(claimed, "?")`'s default to a real verdict is a mutation nothing can
            # see — inert only for as long as the interception above happens to run first, which is
            # exactly the kind of "I believe it is inert" that let REG-C sit red for a wave.
            print("     %-9s %-22s %-8s = %-12r expected %-9s observed %-16s category %s"
                  % (cid, tier, field, value, r["expected"], r["observed"], r["category"]))
        print()

    if out_as_native:
        print("  == OUT-BOOKED-AS-NATIVE == (%d)  a clean refusal counted as a capability."
              % len(out_as_native))
        print("     verdicts/README.md: an `OUT` case's test \"asserts we refuse cleanly; never")
        print("     that we half-work\", and the status for a working refusal is `Refused`, which")
        print("     does NOT count toward a floor. `Native` does. Each row below is therefore one")
        print("     rung of a TIER_FLOORS entry resting on a decline.")
        for (cid, tier, test, observed) in out_as_native:
            print("     %-9s %-22s %-16s %s" % (cid, tier, observed, test))
        print()

    if floor_lies:
        print("  == FLOOR-ABOVE-TRUTH == (%d)  a TIER_FLOORS entry the measurements do not support."
              % len(floor_lies))
        print("     `the_capability_floor_never_regresses` counts cases whose STATUS is Native, so")
        print("     a passing `Refused` case can never satisfy a floor however green its test is.")
        for (t, floor, sup) in floor_lies:
            print("     %-22s floor %d, but only %d case(s) in that tier can honestly BE Native"
                  % (t, floor, sup))
        print()

    if odd_events:
        print("  == UNKNOWN NEXTEST EVENTS == %s" % ", ".join(odd_events))
        print("     The libtest JSON shape changed. --message-format-version is pinned at 0.1;")
        print("     re-pin it deliberately rather than letting the join drift.")
        print()

    crashed = [r for r in rows if r["observed"] in ("ABORTED", "TIMEOUT")]
    if crashed:
        print("  == NOT AN ASSERTION FAILURE == (%d)  these deciders CRASHED or timed out." % len(crashed))
        print("     Counted as NOT PASSING wherever they appear above; listed separately because a")
        print("     Gap that kills its process is a different fact from a Gap that asserts.")
        for r in crashed:
            print("     %-9s %-16s %s::%s" % (r["id"], r["observed"], r["binary"], r["test"]))
        print()

    counts = {c: len(by_cat.get(c, [])) for c, _ in order}
    print("SUMMARY: %d native-red, %d gap-green, %d refused-red, %d missing, %d ignored, "
          "%d not-observed, %d shared-decider, %d wrong-tier, %d ambiguous, %d malformed, "
          "%d out-as-native, %d floor-above-truth"
          % (counts["NATIVE-BUT-RED"], counts["GAP-BUT-GREEN"], counts["REFUSED-BUT-RED"],
             counts["NO-SUCH-TEST"], counts["IGNORED-DECIDER"], counts["NOT-OBSERVED"],
             len(shared), len(wrong_tier), counts["AMBIGUOUS-BINARY"], len(malformed),
             len(out_as_native), len(floor_lies)))

    # SHARED-DECIDER contributes one finding per GROUP, not per row, so it is added separately.
    # MALFORMED-VERDICT is likewise counted per FIELD and is absent from `order`, so it cannot be
    # double-counted through `counts`.
    bad_total = (sum(v for k, v in counts.items() if k != "SHARED-DECIDER")
                 + len(shared) + len(wrong_tier) + len(malformed) + len(out_as_native)
                 + len(floor_lies))
    print("VERDICT: %s" % ("OK" if bad_total == 0 else "MISMATCH"))

    if json_out:
        payload = {
            "_warning": (
                "A PHOTOGRAPH, NOT A MEASUREMENT. This file is stale the moment anyone edits kernel "
                "source and nothing can tell. NEVER read it from a test (include_str!, read_to_string) "
                "— that rebuilds the exact dishonesty scripts/lean/conformance-verify-verdicts.sh exists to "
                "close. Re-run the script instead; it is the only honest answer."
            ),
            "generated_at": datetime.datetime.now().astimezone().isoformat(),
            "head": head_commit(root),
            "summary": dict(counts, shared_decider_groups=len(shared),
                            wrong_tier=len(wrong_tier), malformed=len(malformed),
                            out_as_native=len(out_as_native),
                            floor_above_truth=len(floor_lies)),
            "verdict": "OK" if bad_total == 0 else "MISMATCH",
            # The ROWS carry `category`, and an OUT-booked-as-Native case's category is `OK` —
            # its test passes, and passes for the right reason. So a machine reading this file
            # could count the finding in `summary` and never learn WHICH case it is. Named here.
            "out_as_native": [
                {"id": cid, "tier": tier, "test": test, "observed": observed}
                for (cid, tier, test, observed) in out_as_native
            ],
            "tiers": [
                {"tier": t, "cases": n, "claimed_native": cn, "claimed_gap": cg,
                 "claimed_refused": cr, "truly_passing": tp, "can_be_native": sup,
                 "floor": floor, "mismatches": bad}
                for (t, n, cn, cg, cr, tp, sup, floor, bad) in tiers
            ],
            "cases": rows,
        }
        try:
            with open(json_out, "w", encoding="utf-8") as fh:
                json.dump(payload, fh, indent=2, sort_keys=False)
                fh.write("\n")
            print("report : %s" % json_out)
        except OSError as e:
            sys.stderr.write("WARNING: could not write %s: %s\n" % (json_out, e))

    return 0 if bad_total == 0 else 1



# ---------------------------------------------------------------------------------------------
# SELFTEST — feed the join every lie at once and fail unless it catches all of them, then feed it
# an honest scoreboard and fail unless it stays quiet. A detector that fires on everything is as
# useless as one that fires on nothing, so both directions are pinned. No cargo, no corpus, no
# network: pure fixtures, milliseconds, which is what lets a Rust lock run it in every baseline.
#
# This exists because REG-C happened: `proof_test_honesty_lock::is_assert_true` stopped matching its
# own synthetic violation, its self-test went red, and the anti-vacuity ratchet was protecting
# nothing while the matrix read green. A guard nobody has lied to is a guard nobody has tested.
# ---------------------------------------------------------------------------------------------

import contextlib
import io
import shutil
import tempfile

_SELFTEST_INVENTORY = {
    "rust-suites": {
        "fakecrate::fake_bin": {
            "binary-name": "fake_bin",
            "testcases": {
                "passes_natively": {"ignored": False},
                "fails_as_a_gap": {"ignored": False},
                "refuses_cleanly": {"ignored": False},
                "claims_but_fails": {"ignored": False},
                "claims_but_crashes": {"ignored": False},
                "secretly_works": {"ignored": False},
                "refusal_is_not_clean": {"ignored": False},
                "decided_by_an_ignored_test": {"ignored": True},
                "listed_but_never_ran": {"ignored": False},
                "times_out_as_a_gap": {"ignored": False},
                "renamed_awayy": {"ignored": False},
                "typoed_status_target": {"ignored": False},
                "typoed_verdict_target": {"ignored": False},
                "out_but_booked_native": {"ignored": False},
            },
        },
        "fakecrate::other_bin": {
            "binary-name": "other_bin",
            "testcases": {"lives_elsewhere": {"ignored": False}},
        },
        # Two suites, ONE binary-name — the live hazard: eight test-file stems are duplicated across
        # this workspace today, so a bare name is not a unique address.
        "crate_a::twin_bin": {
            "binary-name": "twin_bin",
            "testcases": {"which_twin_is_this": {"ignored": False}},
        },
        "crate_b::twin_bin": {
            "binary-name": "twin_bin",
            "testcases": {"which_twin_is_this": {"ignored": False}},
        },
    }
}

# `renamed_away` is deliberately absent from the inventory while `renamed_awayy` is present, so the
# NO-SUCH-TEST row must also produce a close-match suggestion — the common cause is a rename.
_SELFTEST_RESULTS = [
    {"type": "test", "event": "started", "name": "fakecrate::fake_bin$passes_natively"},
    {"type": "test", "event": "ok", "name": "fakecrate::fake_bin$passes_natively"},
    {"type": "test", "event": "failed", "name": "fakecrate::fake_bin$fails_as_a_gap",
     "stdout": "assertion failed: the gap is open"},
    {"type": "test", "event": "ok", "name": "fakecrate::fake_bin$refuses_cleanly"},
    {"type": "test", "event": "failed", "name": "fakecrate::fake_bin$claims_but_fails",
     "stdout": "assertion `left == right` failed"},
    {"type": "test", "event": "failed", "name": "fakecrate::fake_bin$claims_but_crashes",
     "stdout": "\nthread has overflowed its stack\nfatal runtime error: stack overflow, aborting\n"},
    {"type": "test", "event": "ok", "name": "fakecrate::fake_bin$secretly_works"},
    {"type": "test", "event": "failed", "name": "fakecrate::fake_bin$refusal_is_not_clean",
     "stdout": "panicked: the refusal leaked"},
    {"type": "test", "event": "failed", "name": "fakecrate::fake_bin$times_out_as_a_gap",
     "stdout": "test timed out after 1800s (terminate-after)"},
    {"type": "test", "event": "ok", "name": "fakecrate::other_bin$lives_elsewhere"},
    {"type": "test", "event": "failed", "name": "fakecrate::fake_bin$typoed_status_target",
     "stdout": "assertion failed: the capability is missing"},
    {"type": "test", "event": "failed", "name": "fakecrate::fake_bin$typoed_verdict_target",
     "stdout": "assertion failed: the capability is missing"},
    {"type": "test", "event": "ok", "name": "fakecrate::fake_bin$out_but_booked_native"},
    {"type": "suite", "event": "failed", "passed": 5, "failed": 7},
]

_SELFTEST_CASES_DISHONEST = [
    ("F-01", "MUST", "Native", "passes_natively"),
    ("F-02", "MUST", "Gap", "fails_as_a_gap"),
    ("F-03", "OUT", "Refused", "refuses_cleanly"),
    ("F-04", "MUST", "Native", "claims_but_fails"),
    ("F-05", "MUST", "Native", "claims_but_crashes"),
    ("F-06", "MUST", "Gap", "secretly_works"),
    ("F-07", "OUT", "Refused", "refusal_is_not_clean"),
    ("F-08", "MUST", "Native", "renamed_away"),
    ("F-09", "MUST", "Native", "decided_by_an_ignored_test"),
    ("F-10", "MUST", "Native", "listed_but_never_ran"),
    ("F-11", "MUST", "Native", "other_bin::lives_elsewhere"),
    ("F-12", "MUST", "Gap", "times_out_as_a_gap"),
    # F-13 names the SAME test as F-01. One of the two is never separately exercised and reads
    # green for free; lean_parity_matrix.rs's string-level uniqueness check cannot see this when the
    # two spellings differ, which is why the gate re-checks it AFTER resolution.
    ("F-13", "MUST", "Native", "fake_bin::passes_natively"),
    ("F-14", "MUST", "Native", "twin_bin::which_twin_is_this"),
    # F-15's status is `native`, not `Native`. Before MALFORMED-VERDICT existed, EXPECTED.get()
    # returned "?" for it, every comparison against "?" was False, and the row fell through to the
    # final `else` and was recorded OK — a red test reading green because of one capital letter.
    ("F-15", "MUST", "native", "typoed_status_target"),
    # F-16's verdict is `must`, not `MUST`. That is not a pass/fail lie, so its category must stay
    # exactly what the test did — but it escapes lean_parity_matrix.rs's "every DIVERGE and OUT
    # states its argument" rule, which keys off the exact strings, so it is still a finding.
    ("F-16", "must", "Gap", "typoed_verdict_target"),
    # F-17 names a whole BINARY no suite produced — a deleted or renamed test file, as against
    # F-08's renamed test inside a binary that still exists. Measured: without this case the
    # missing-binary branch was never exercised, and deleting it changed nothing the selftest could
    # see. Two different causes, two different notes, and both must fire.
    ("F-17", "MUST", "Gap", "ghost_bin::vanished"),
    # F-18's test PASSES, and passes for the right reason — it proves a clean refusal. But its
    # status is `Native`, so a decline is booked as a capability and takes a rung of the tier's
    # floor with it. Nothing in the pass/fail join can see this: expected PASS, observed PASS.
    ("F-18", "OUT", "Native", "out_but_booked_native"),
]

# The honest twin: every verdict matches what the fixture actually did. FOUR cases truly pass — but
# one of them (F-03) is `Refused`, which can never count toward a floor, so only THREE can honestly
# be Native. A floor of 3 is exactly supported and must NOT trip FLOOR-ABOVE-TRUTH; a floor of 4,
# which the passing-test count alone would appear to justify, MUST trip it. That gap is the whole
# finding: `t09_metaprogramming` has four passing `Refused` cases hiding a floor of 10 above only 9
# supportable ones.
_SELFTEST_CASES_HONEST = [
    ("F-01", "MUST", "Native", "passes_natively"),
    ("F-02", "MUST", "Gap", "fails_as_a_gap"),
    ("F-03", "OUT", "Refused", "refuses_cleanly"),
    ("F-04", "MUST", "Gap", "claims_but_fails"),
    ("F-05", "MUST", "Gap", "claims_but_crashes"),
    ("F-06", "MUST", "Native", "secretly_works"),
    ("F-07", "MUST", "Gap", "refusal_is_not_clean"),
    ("F-11", "MUST", "Native", "other_bin::lives_elsewhere"),
    ("F-12", "MUST", "Gap", "times_out_as_a_gap"),
]


def _selftest_root(tmp, cases, floor, misfiled=None):
    vdir = os.path.join(tmp, *CORPUS, "verdicts")
    os.makedirs(vdir)
    # The manifest fixture: every case lives in t99_fake except `misfiled`, which the corpus puts
    # somewhere else while the verdict file still claims it.
    manifest = {
        "case_count": len(cases),
        "cases": [
            {"id": i, "tier": "99 . fake", "title": i, "expect": "translate",
             "path": "cases/%s/%s_x.lean" % ("t98_other" if i == misfiled else "t99_fake", i),
             "guards": ["g"]}
            for (i, _v, _st, _t) in cases
        ],
    }
    with open(os.path.join(tmp, *CORPUS, "manifest.json"), "w", encoding="utf-8") as fh:
        json.dump(manifest, fh)
    payload = {
        "tier": "t99_fake",
        "test_binary": "fake_bin",
        "cases": [
            {"id": i, "verdict": v, "status": st, "test": t, "why": "x" * 50}
            for (i, v, st, t) in cases
        ],
    }
    with open(os.path.join(vdir, "t99_fake.json"), "w", encoding="utf-8") as fh:
        json.dump(payload, fh)
    mdir = os.path.join(tmp, "crates", "logicaffeine_tests", "tests")
    os.makedirs(mdir, exist_ok=True)
    with open(os.path.join(mdir, "lean_parity_matrix.rs"), "w", encoding="utf-8") as fh:
        fh.write("const TIER_FLOORS: &[(&str, usize)] = &[\n    (\"t99_fake\", %d),\n];\n" % floor)
    lj = os.path.join(tmp, "inventory.json")
    with open(lj, "w", encoding="utf-8") as fh:
        json.dump(_SELFTEST_INVENTORY, fh)
    rj = os.path.join(tmp, "results.jsonl")
    with open(rj, "w", encoding="utf-8") as fh:
        for row in _SELFTEST_RESULTS:
            fh.write(json.dumps(row) + "\n")
    return lj, rj


def _run_report(cases, floor, misfiled=None):
    tmp = tempfile.mkdtemp(prefix="conformance-selftest-")
    try:
        lj, rj = _selftest_root(tmp, cases, floor, misfiled)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = cmd_report(tmp, lj, rj, None, False)
        return code, buf.getvalue()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def cmd_selftest():
    failures = []

    def want(cond, msg):
        if not cond:
            failures.append(msg)

    # --- the dishonest scoreboard: every lie must be caught -----------------------------------
    code, out = _run_report(_SELFTEST_CASES_DISHONEST, 9, misfiled="F-02")
    expected = ("SUMMARY: 2 native-red, 1 gap-green, 1 refused-red, 2 missing, 1 ignored, "
                "1 not-observed, 1 shared-decider, 1 wrong-tier, 1 ambiguous, 2 malformed, "
                "1 out-as-native, 1 floor-above-truth")
    summary = [l for l in out.splitlines() if l.startswith("SUMMARY:")]
    want(summary and summary[0] == expected,
         "the join did not catch every planted lie.\n    expected %r\n    got      %r"
         % (expected, summary[0] if summary else "<no SUMMARY line>"))
    want(code == 1, "a scoreboard full of lies must exit 1, got %d" % code)
    want("VERDICT: MISMATCH" in out, "a scoreboard full of lies must print VERDICT: MISMATCH")
    # The crash and the timeout must be told apart from an ordinary assertion failure.
    want("F-05" in out and "ABORTED" in out,
         "a decider that dies on a signal must be labelled ABORTED, not merely failed")
    want("F-12" in out and "TIMEOUT" in out,
         "a decider that is terminated must be labelled TIMEOUT, not merely failed")
    # A qualified `binary::test` must resolve to the OTHER binary, not be reported missing.
    want("F-11" not in out.split("== NO-SUCH-TEST ==")[-1].split("==")[0] if "== NO-SUCH-TEST ==" in out else True,
         "a qualified binary::test verdict must resolve, not be reported as a missing test")
    want("renamed_awayy" in out,
         "a missing test must suggest the closest name — the usual cause is a rename")
    want("F-17" in out and "was collected by no nextest suite" in out,
         "a verdict naming a whole BINARY that no suite produced must be reported, and reported as "
         "a different fact from a renamed test inside a binary that still exists")
    want("F-13" in out and "SHARED-DECIDER" in out,
         "two cases resolving to one test must be reported, whatever spelling each used")
    # The bank-it hint names the CAN-BE-NATIVE ceiling, not `current + free`. In this fixture TEN
    # cases claim Native and only four of the tier's cases can honestly hold a floor, so the hint
    # must say 4 and say WHY — a hint that said 11 would be telling the orchestrator to bank a
    # floor resting on tests that failed, went missing, were ignored, never ran, or (F-18) proved
    # only a clean refusal.
    want("bank it: t99_fake Native 10 -> 11 (1 free in this tier), floor 9 -> 4 — 7 claimed "
         "Native here cannot hold a floor (its test does not pass, or its verdict is OUT and a "
         "clean refusal is not a capability), so the floor stops at 4" in out,
         "a GAP-BUT-GREEN row must print the floor edit that banks it, capped at what the tier's "
         "measurements actually support:\n%s" % out)

    # --- the honest twin: nothing may fire ----------------------------------------------------
    code2, out2 = _run_report(_SELFTEST_CASES_HONEST, 3)
    want(code2 == 0, "an honest scoreboard must exit 0, got %d" % code2)
    want("VERDICT: OK" in out2, "an honest scoreboard must print VERDICT: OK")
    want("F-14" in out and "AMBIGUOUS-BINARY" in out,
         "a bare binary name resolving to two suites must be refused, not merged")
    want("F-02" in out and "WRONG-TIER" in out,
         "a case filed under a tier the corpus does not put it in must be reported")
    want("MALFORMED-VERDICT" in out and "F-15" in out and "F-16" in out,
         "a status or verdict the schema does not define must be reported, not waved through")
    want("'native'" in out and "'must'" in out,
         "the malformed report must quote the offending VALUE, or nobody can see the typo")
    # THE decisive property, and the one a first attempt at this self-test missed: an unclassifiable
    # STATUS must take the row's category over. Without that, `EXPECTED.get(claimed, "?")` yields
    # "?", every comparison against it is False, and the row falls through to the final `else` and
    # is recorded OK — a red test reading green because of one capital letter. Measured: deleting
    # the interception left the SUMMARY counts and the finding list byte-identical, so only the
    # CATEGORY column can kill that mutation.
    malformed_rows = [l for l in out.splitlines() if "category " in l and (" F-15" in l or " F-16" in l)]
    want(any("F-15" in l and "category MALFORMED-VERDICT" in l for l in malformed_rows),
         "a case whose status is outside the schema must be categorised MALFORMED-VERDICT, never "
         "OK: %r" % malformed_rows)
    # …and its mirror: a typo'd `verdict` does NOT change what the test should do, so it must leave
    # the pass/fail category exactly alone. A blanket override would let it mask a NATIVE-BUT-RED.
    want(any("F-16" in l and "category OK" in l for l in malformed_rows),
         "a malformed `verdict` must not take over the row's pass/fail category: %r" % malformed_rows)
    # An unclassifiable STATUS has no expectation, and the row must say so rather than borrow one.
    want(any("F-15" in l and "expected ?" in l for l in malformed_rows),
         "a case with an unrecognised status must report `expected ?` — giving it a real "
         "expectation is how the whole join quietly starts judging it: %r" % malformed_rows)
    want(any("F-16" in l and "expected NOT-PASS" in l for l in malformed_rows),
         "…while a case with a good status keeps the expectation its status earns: %r" % malformed_rows)
    want("SUMMARY: 0 native-red, 0 gap-green, 0 refused-red, 0 missing, 0 ignored, "
         "0 not-observed, 0 shared-decider, 0 wrong-tier, 0 ambiguous, 0 malformed, "
         "0 out-as-native, 0 floor-above-truth" in out2,
         "an honest scoreboard must report nothing:\n%s" % out2)
    # ...and a floor one rung above what the fixture actually proves must still trip.
    code3, out3 = _run_report(_SELFTEST_CASES_HONEST, 4)
    want(code3 == 1 and "FLOOR-ABOVE-TRUTH" in out3,
         "a floor of 4 must trip FLOOR-ABOVE-TRUTH. FOUR cases pass, so a check that counted "
         "passing TESTS would wave this through — but one of them is `Refused` and can never be "
         "Native, so the matrix's own ratchet supports only THREE. Counting the passing Refused "
         "case is how a live floor trap stayed invisible for a whole wave.")
    want("can honestly BE Native" in out3,
         "the FLOOR-ABOVE-TRUTH row must say what it counted, or the next reader repeats the error")

    # --- a finding must be able to fail the gate ON ITS OWN -----------------------------------
    # Printing a finding and exiting 0 is a gate that reports and does not gate. Measured: dropping
    # `len(malformed)` from the exit total left the dishonest fixture exiting 1 anyway — the other
    # nine findings carried it — so only a scoreboard whose ONLY lie is this one can pin it. Every
    # finding added from here on owes the same three lines.
    code4, out4 = _run_report(_SELFTEST_CASES_HONEST + [("F-15", "MUST", "native", "typoed_status_target")], 3)
    want(code4 == 1, "an otherwise-honest scoreboard with ONE malformed verdict must exit 1, got %d" % code4)
    want("VERDICT: MISMATCH" in out4,
         "a malformed verdict alone must flip the verdict — a finding that cannot fail the gate by "
         "itself is decoration")
    want("1 malformed" in out4, "the count must reach the summary line:\n%s" % out4)

    # …and the clean case keeps the short form, so the suffix is a finding rather than noise.
    _code5, out5 = _run_report(
        [("F-01", "MUST", "Native", "passes_natively"), ("F-06", "MUST", "Gap", "secretly_works")], 1)
    want("bank it: t99_fake Native 1 -> 2 (1 free in this tier), floor 1 -> 2]" in out5,
         "with every claimed Native green, the hint is `current + free` and carries no caveat:\n%s"
         % out5)

    # THE INVARIANT, checked structurally over every fixture rather than by reading one string: a
    # bank-it hint may never name a floor above its own tier's CAN BE NATIVE column. Those two
    # numbers are computed by different code paths from the same rows, so a change that breaks one
    # and not the other is caught here without anyone having to re-derive the arithmetic by hand.
    for label, page in (("dishonest", out), ("honest", out2), ("clean-bank", out5)):
        ceiling = {}
        for line in page.splitlines():
            cols = [c.strip() for c in line.split("|")]
            if len(cols) == 6 and cols[0].split()[:1] and not cols[0].startswith("TIER"):
                head = cols[0].split()
                # cols[3] is CAN BE NATIVE. cols[4] is the FLOOR — reading that one instead
                # makes the check compare the hint against the very ratchet it is proposing to
                # move, which is no check at all. Caught by this fixture the first time it ran.
                try:
                    ceiling[head[0]] = int(cols[3])
                except (IndexError, ValueError):
                    pass
        for line in page.splitlines():
            m = re.search(r"\[bank it: (\S+) Native \d+ -> \d+ \(\d+ free in this tier\), "
                          r"floor \S+ -> (\d+)", line)
            if not m:
                continue
            tier, proposed = m.group(1), int(m.group(2))
            want(tier in ceiling,
                 "%s fixture: a bank-it hint names tier %r, which has no scoreboard row" % (label, tier))
            want(proposed <= ceiling.get(tier, -1),
                 "%s fixture: the bank-it hint proposes a floor of %d for %s, above the %s that "
                 "tier's CAN BE NATIVE column supports. Banking it would put a ratchet on a case "
                 "whose test does not pass." % (label, proposed, tier, ceiling.get(tier)))


    want("F-18" in out and "OUT-BOOKED-AS-NATIVE" in out,
         "an `OUT` case booked `Native` must be caught. Its test PASSES and passes for the right "
         "reason, so every pass/fail column reads OK — the only thing wrong is the classification, "
         "and the classification is what ratchets")

    # …and it must fail the gate BY ITSELF. Every other finding in the dishonest fixture would
    # carry the exit code, so only a scoreboard whose ONLY lie is this one can pin it.
    code6, out6 = _run_report(
        _SELFTEST_CASES_HONEST + [("F-18", "OUT", "Native", "out_but_booked_native")], 3)
    want(code6 == 1 and "OUT-BOOKED-AS-NATIVE" in out6 and "1 out-as-native" in out6,
         "an otherwise-honest scoreboard with ONE `OUT` case booked Native must exit 1 and say so; "
         "a finding that cannot fail the gate by itself is decoration:\n%s" % out6)
    # …and it must NOT be allowed to hold up a floor: with F-18 excluded from CAN BE NATIVE, the
    # tier still supports only 3, so a floor of 3 stays honest and a floor of 4 still trips.
    want("floor 3, but only" not in out6,
         "F-18 passes but cannot be Native, so a floor of 3 must remain exactly supported:\n%s" % out6)
    code7, out7 = _run_report(
        _SELFTEST_CASES_HONEST + [("F-18", "OUT", "Native", "out_but_booked_native")], 4)
    want("FLOOR-ABOVE-TRUTH" in out7,
         "a passing `OUT`/`Native` case must NOT be able to lift a floor — if it could, the "
         "corpus's one such case would go on holding t09_metaprogramming at 10:\n%s" % out7)

    if failures:
        print("SELFTEST: FAILED (%d)" % len(failures))
        for f in failures:
            print("  * %s" % f)
        return 1
    print("SELFTEST: OK — every planted lie caught, honest scoreboard stayed silent")
    return 0


def main():
    mode = sys.argv[1]
    root = sys.argv[2] if len(sys.argv) > 2 else "."
    if mode == "discover":
        cmd_discover(root)
        return 0
    if mode == "report":
        return cmd_report(root, sys.argv[3], sys.argv[4], sys.argv[5] or None, sys.argv[6] == "1")
    if mode == "selftest":
        return cmd_selftest()
    sys.stderr.write("unknown mode %r\n" % mode)
    return 3


if __name__ == "__main__":
    sys.exit(main())
PY_EOF

# ---- 1. DISCOVERY -----------------------------------------------------------------------------
# The set of binaries is DERIVED from the verdicts, never hard-coded. A verdict's `test` field is
# either a bare fn name (decided by its tier's `test_binary`) or a qualified `binary::fn` — 11 of
# the 248 use the qualified form today, and a resolver that trusts `test_binary` alone mis-reports
# every one of them as missing. A name that resolves to no source file is a hard COULD-NOT-COLLECT
# rather than 248 phantom failures.
if [ "$SELFTEST" = "1" ]; then
  python3 "$VERIFY_PY" selftest
  exit $?
fi

BINARIES_FILE="$WORKDIR/binaries.txt"
if ! python3 "$VERIFY_PY" discover "$REPO_ROOT" > "$BINARIES_FILE" 2>"$WORKDIR/discover.err"; then
  cat "$WORKDIR/discover.err" >&2
  exit 2
fi
mapfile -t BINARIES < "$BINARIES_FILE"
if [ "${#BINARIES[@]}" -eq 0 ]; then
  echo "ERROR: no corpus test binaries discovered from crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/*.json" >&2
  exit 2
fi
if [ "$PRINT_BINARIES" = "1" ]; then
  printf '%s\n' "${BINARIES[@]}"
  exit 0
fi

# Target selection, not a filterset: `--test <name>` restricts what cargo BUILDS. A `-E 'binary(..)'`
# filterset would still compile every test target in the selected packages — several hundred of them
# in logicaffeine-tests alone — and then throw the results away.
TARGET_ARGS=()
for b in "${BINARIES[@]}"; do TARGET_ARGS+=(--test "$b"); done

FEATURE_ARGS=()
[ -n "$FEATURES" ] && FEATURE_ARGS+=(--features "$FEATURES")

echo "-- conformance verdict verification -------------------------------------------"
echo "   binaries : ${#BINARIES[@]}  (${BINARIES[*]})"
echo "   profile  : $CARGO_PROFILE   features: ${FEATURES:-<none>}"
echo "   nextest  : $(cargo nextest --version 2>/dev/null | head -1)"
echo

# ---- 2. INVENTORY -----------------------------------------------------------------------------
# What tests EXIST and which are `#[ignore]`d. That is a different question from what passed, and
# the runner is the wrong place to ask it: nextest emits no row for a test it filtered out, so an
# ignored decider and a deleted decider look identical in a run stream. `list` tells them apart,
# which is what makes IGNORED-DECIDER and NO-SUCH-TEST two distinct findings instead of one blur.
# cargo's stderr is TEED, not swallowed: the first invocation on a cold target dir is a multi-minute
# build, and a gate that prints nothing for five minutes inside a baseline log is indistinguishable
# from a hung one. Progress goes to the terminal live; the copy is kept only to quote on failure.
LIST_JSON="$WORKDIR/inventory.json"
{ cargo nextest list --workspace "${FEATURE_ARGS[@]}" --cargo-profile "$CARGO_PROFILE" \
    "${TARGET_ARGS[@]}" --message-format json > "$LIST_JSON"; } 2> >(tee "$WORKDIR/list.err" >&2)
LIST_EXIT=$?
if [ "$LIST_EXIT" -ne 0 ] || [ ! -s "$LIST_JSON" ]; then
  echo "ERROR: could not LIST the corpus binaries (cargo nextest list exit $LIST_EXIT)." >&2
  echo "       This is a BUILD failure, not a scoreboard failure — reporting it as 248 missing" >&2
  echo "       tests would be its own lie. Fix the build and re-run." >&2
  echo "       (cargo's diagnostics are above — they were streamed live, not swallowed.)" >&2
  exit 2
fi

# ---- 3. RUN -----------------------------------------------------------------------------------
# NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 is MANDATORY: without it nextest hard-errors with "libtest
# JSON output is an experimental feature and must be enabled with ...". `--message-format-version`
# is PINNED so a nextest upgrade cannot silently reshape the rows under us and turn a real join into
# a green report about nothing; the report says so loudly if an unrecognised event appears. Pinned
# against cargo-nextest 0.9.137.
#
# nextest's own exit code is IGNORED on purpose. 100 (tests failed) is the EXPECTED state while any
# Gap is open — that is what a Gap IS — so a gate keyed off it would be red forever and stop being
# read. The verdict comes from the JSON alone. A build failure is caught above; a run that produces
# no rows is caught here and in the join.
RUN_JSON="$WORKDIR/results.jsonl"
{ NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 \
  cargo nextest run --workspace "${FEATURE_ARGS[@]}" --cargo-profile "$CARGO_PROFILE" \
    "${TARGET_ARGS[@]}" --no-fail-fast \
    --status-level none --final-status-level none \
    --message-format libtest-json-plus --message-format-version 0.1 \
    > "$RUN_JSON"; } 2> >(tee "$WORKDIR/run.err" >&2)
RUN_EXIT=$?
if [ ! -s "$RUN_JSON" ]; then
  echo "ERROR: the corpus run produced no output at all (cargo nextest run exit $RUN_EXIT)." >&2
  echo "       (cargo's diagnostics are above — they were streamed live, not swallowed.)" >&2
  exit 2
fi

# ---- 4. JOIN AND REPORT -----------------------------------------------------------------------
if [ "$JSON_OUT" = "__default__" ]; then
  mkdir -p "$REPO_ROOT/logs"
  JSON_OUT="$REPO_ROOT/logs/conformance-verdicts-$(date +%Y%m%d-%H%M%S).json"
fi

python3 "$VERIFY_PY" report "$REPO_ROOT" "$LIST_JSON" "$RUN_JSON" "$JSON_OUT" "$QUIET"
VERDICT_EXIT=$?

if [ -n "$JSON_OUT" ] && [ -f "$JSON_OUT" ] \
   && [ "$(cd "$(dirname "$JSON_OUT")" && pwd)" = "$REPO_ROOT/logs" ]; then
  ln -sfn "$(basename "$JSON_OUT")" "$REPO_ROOT/logs/latest-verdicts.json" 2>/dev/null || true
fi

exit "$VERDICT_EXIT"
