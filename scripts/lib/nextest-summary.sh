#!/usr/bin/env bash
# nextest-summary.sh — the release gate's summary, as a pure function of ONE log file.
#
#   scripts/lib/nextest-summary.sh <log>
#
# The full run (scripts/run-all-tests-fast.sh) streams three passes into one log:
#   PASS 1/2  — the main `cargo nextest run`            (its own `Summary [` line)
#   PASS 1b   — the isolated wasm-jit `cargo nextest run` (its own `Summary [` line)
#   PASS 2/2  — `cargo test --doc`                       (cargo-format `test result:` lines)
# Each pass is reported BY NAME from the first `Summary [` line INSIDE ITS OWN SECTION —
# never "the last Summary in the file" (the tiny wasm-jit pass) and never the next pass's
# summary borrowed by a pass that failed to build — the totals sum
# every pass, and the FAILED TESTS block lists TIMEOUT / TERMINATING rows beside FAIL rows
# so a hang can never disappear from the summary. Locked by
# crates/logicaffeine_tests/tests/gate_summary_lock.rs against fixtures cut from real runs.
#
# A nextest pass that printed no `Summary [` line is INCOMPLETE: its row, the totals and the
# stderr line say so, and its figures are the result rows it printed before it stopped.
#
# Exit status: 0 always (this is a reporter; the gate's verdict comes from the pass exit
# codes). Prints `RESULT_FAILED=<n> RESULT_TIMED_OUT=<n> RESULT_INCOMPLETE=<passes>` on stderr
# for callers that want the counts without parsing prose.
set -uo pipefail

LOG="${1:?usage: nextest-summary.sh <log-file>}"
[ -r "$LOG" ] || { echo "nextest-summary.sh: cannot read $LOG" >&2; exit 2; }

# The section a marker opens ends at the NEXT section marker (or EOF).
#
# EVERY pass header the gate prints must be in this pattern. It is the one place that decides which
# pass a line belongs to, and a marker it does not recognise does not open a section — it lands
# inside the PREVIOUS one, which then runs to EOF. That is not hypothetical: ` PASS 3` (the
# Lean-parity verdict gate) was missing here, so everything the gate printed was attributed to the
# doctest pass, and a planted `test result: FAILED. 0 passed; 99 failed;` was reported as
# ` doctests : 3 passed, 99 failed` on a run in which nothing failed. Measured, twice, before and
# after. `H_honesty_extra::the_slicer_recognises_every_pass_marker_the_gate_prints` joins this
# pattern to the headers run-all-tests-fast.sh actually echoes, so a pass added later cannot
# reintroduce the gap.
section() {
  awk -v marker="$1" '
    index($0, marker) && !armed { armed = 1; next }
    armed && /^ PASS [123]|^ PASS 1b/ { exit }
    armed { print }
  ' "$LOG"
}
# A log with no pass markers (a raw nextest log summarised by hand) is one main pass.
if grep -q '^ PASS ' "$LOG"; then MARKED=1; else MARKED=0; fi
pass_text() { # $1 = the pass marker
  if (( MARKED )); then section "$1"
  elif [ "$1" = " PASS 1/2 " ]; then cat "$LOG"
  fi
}
# The first `Summary [` line inside a pass's own text. Bounded to the pass, so a pass that printed
# none is reported as such rather than under the next pass's numbers.
summary_after() {
  pass_text "$1" | awk '
    /^[[:space:]]*Summary \[/ { sub(/^[[:space:]]*Summary \[[^]]*\][[:space:]]*/, ""); print; exit }
  '
}
# Did a pass die before running anything? cargo says so in its own section.
build_failed_in() {
  pass_text "$1" | grep -qE "^error: could not compile|^error: (build failed|could not (compile|document))|error: command .* exited with code"
}
# `N failed` / `N timed out` / `N flaky` / `N tests run` out of a nextest Summary line.
count() { # $1 = summary line, $2 = phrase
  printf '%s\n' "$1" | grep -oE "[0-9]+ $2" | head -1 | awk '{print $1}'
}
num() { printf '%s' "${1:-0}"; }

# Every nextest failure row, resolved PER TEST rather than per line.
#
# A retried test does not print `FAIL [`; it prints `TRY <n> FAIL [`. The pattern here used to
# anchor the status word to the start of the line, so the whole `aot-cdylib` retry group
# (`retries = 2`, .config/nextest.toml) fell out of this block while still counting in the
# totals above. In the 2026-09-11 baseline that hid two tests that had failed 3/3 —
# `aot_native_corpus::every_benchmark_runs_correctly_on_aot_native` (a real AOT narrowing bug)
# and `hotswap_p17_wasm::scalar_function_builds_to_a_wasm_module` — from the only list anyone
# reads. Same shape as the TIMEOUT rows that vanished before it, in a new row spelling.
#
# The mirror-image mistake is just as unreadable: a test that failed TRY 1 and PASSED on TRY 3
# is FLAKY, not failed, and its earlier `TRY 1 FAIL` rows must NOT be listed. So attempts are
# folded by test identity (everything after the `(n/m)` progress field): a `PASS` on a later
# attempt, or nextest's own `FLAKY n/m` row, retires the identity; otherwise the last attempt's
# row represents it. A test with both TERMINATING and TIMEOUT rows collapses to one line too.
#
# SCOPED TO THE NEXTEST PASSES, and that is the other half of the section-boundary bug. This scrape
# used to read the WHOLE log, while `EXPECTED_NAMED` below is built from the two nextest passes'
# own Summary lines — so a `FAIL [`-shaped row printed by ANY other pass was listed as a failure no
# Summary had counted, and then tripped the reconciliation warning against itself. Measured on a
# hand-cut log in which PASS 3 prints one such row: a fully green run named a failing test and
# reported a 1-test discrepancy. A warning that fires on a green run is a warning people learn to
# scroll past, which costs exactly what it was built to buy.
#
# A log with no pass markers at all (someone summarising a raw nextest log by hand) is scanned
# whole — the scoping is about attributing rows to the right pass, not about refusing to read a log
# that only has one.
# The failure rows of one nextest pass's text on stdin, one per failing TEST (retries folded).
failed_rows() {
  awk '
  {
    line = $0
    s = line; sub(/^[ \t]+/, "", s)
    tryn = 0
    if (s ~ /^TRY [0-9]+ /) { split(s, a, " "); tryn = a[2] + 0; sub(/^TRY [0-9]+ /, "", s) }
    p = index(s, " [");  if (p == 0) next
    split(substr(s, 1, p - 1), b, " ");  status = b[1]
    rest = substr(s, p + 1)
    if (!match(rest, /^\[[^]]*\][ \t]*\([^)]*\)[ \t]*/)) next
    id = substr(rest, RLENGTH + 1)
    if (id == "") next
    if (status == "FLAKY" || (status == "PASS" && tryn > 0)) { recovered[id] = 1; next }
    if (status != "FAIL" && status != "TIMEOUT" && status != "TERMINATING" \
        && status != "SIGKILL" && status != "SIGSEGV" && status != "SIGABRT" && status != "ABORT") next
    if (!(id in seen) || (!plain[id] && (tryn == 0 || tryn >= best[id]))) {
      seen[id] = 1; best[id] = tryn; if (tryn == 0) plain[id] = 1
      sub(/^[ \t]+/, "", line);  row[id] = "   " line
    }
  }
  END { for (id in seen) if (!(id in recovered)) print row[id] }
' | sort
}

# How far a pass got, from its own result rows on stdin: `<finished> <planned> <recovered>`. A
# result row is a status word, its `[duration]` and a numeric `(n/m)` progress field; `finished` is
# the highest `n` (nextest counts a test once, when its final attempt ends), `planned` is the count
# its `Starting N tests` line announced (else the `m` of its rows, else 0), and `recovered` counts
# the tests that failed an attempt and then passed.
pass_progress() {
  awk '
  /^[[:space:]]*Starting [0-9]+ tests? across/ && !planned {
    for (i = 1; i <= NF; i++) if ($i == "Starting") { planned = $(i + 1) + 0; break }
  }
  {
    s = $0; sub(/^[ \t]+/, "", s)
    tryn = 0
    if (s ~ /^TRY [0-9]+ /) { split(s, a, " "); tryn = a[2] + 0; sub(/^TRY [0-9]+ /, "", s) }
    p = index(s, " [");  if (p == 0) next
    split(substr(s, 1, p - 1), b, " ");  status = b[1]
    if (status !~ /^[A-Z]+$/) next
    rest = substr(s, p + 1)
    if (!match(rest, /^\[[^]]*\][ \t]*\([ \t]*[0-9]+\/[0-9]+\)[ \t]*/)) next
    head = substr(rest, 1, RLENGTH); id = substr(rest, RLENGTH + 1)
    sub(/^\[[^]]*\][ \t]*\([ \t]*/, "", head); split(head, nm, "/")
    n = nm[1] + 0; m = nm[2] + 0
    if (n > finished) finished = n
    if (!rows_planned) rows_planned = m
    if (status == "FLAKY" || (status == "PASS" && tryn > 0)) recovered[id] = 1
  }
  END {
    r = 0; for (id in recovered) r++
    printf "%d %d %d", finished, (planned ? planned : rows_planned), r
  }'
}
# Failure rows by what ended them: `<failed> <timed out>`.
row_counts() { # $1 = rows from failed_rows
  printf '%s\n' "$1" | awk '
    NF {
      s = $0; sub(/^[ \t]+/, "", s); sub(/^TRY [0-9]+ /, "", s)
      if (s ~ /^(TIMEOUT|TERMINATING) /) timed++; else failed++
    }
    END { printf "%d %d", failed, timed }'
}

MAIN="$(summary_after " PASS 1/2 ")"
WASM="$(summary_after " PASS 1b ")"
DOC_SECTION="$(section " PASS 2/2 ")"
MAIN_ROWS="$(pass_text " PASS 1/2 " | failed_rows)"
WASM_ROWS="$(pass_text " PASS 1b " | failed_rows)"

read -r DOC_PASSED DOC_FAILED DOC_IGNORED < <(
  printf '%s\n' "$DOC_SECTION" | awk '
    /test result:/ {
      for (i = 1; i <= NF; i++) {
        if ($i == "passed;")  passed  += $(i-1)
        if ($i == "failed;")  failed  += $(i-1)
        if ($i == "ignored;") ignored += $(i-1)
      }
    }
    END { printf "%d %d %d", passed, failed, ignored }
  '
)

# The figures of one nextest pass: `<run> <failed> <timed out> <flaky> <incomplete> <planned>`. With
# a Summary line they are nextest's own; without one the pass is incomplete and they are its result
# rows, `planned` being the count it announced. A log with no pass markers holds only the main pass,
# so the wasm-jit pass is absent from it rather than incomplete.
pass_figures() { # $1 = summary line, $2 = failure rows, $3 = pass marker
  if [ -n "$1" ]; then
    printf '%s %s %s %s 0 0' "$(num "$(count "$1" 'tests? run')")" "$(num "$(count "$1" 'failed')")" \
      "$(num "$(count "$1" 'timed out')")" "$(num "$(count "$1" 'flaky')")"
  elif (( ! MARKED )) && [ "$3" != " PASS 1/2 " ]; then
    printf '0 0 0 0 0 0'
  else
    local finished planned recovered failed timed
    read -r finished planned recovered < <(pass_text "$3" | pass_progress)
    read -r failed timed < <(row_counts "$2")
    printf '%s %s %s %s 1 %s' "$finished" "$failed" "$timed" "$recovered" "$planned"
  fi
}
read -r MAIN_RUN MAIN_FAILED MAIN_TIMED MAIN_FLAKY MAIN_INCOMPLETE MAIN_PLANNED < <(pass_figures "$MAIN" "$MAIN_ROWS" " PASS 1/2 ")
read -r WASM_RUN WASM_FAILED WASM_TIMED WASM_FLAKY WASM_INCOMPLETE WASM_PLANNED < <(pass_figures "$WASM" "$WASM_ROWS" " PASS 1b ")

TOTAL_RUN=$(( MAIN_RUN + WASM_RUN + DOC_PASSED + DOC_FAILED ))
TOTAL_FAILED=$(( MAIN_FAILED + WASM_FAILED + DOC_FAILED ))
TOTAL_TIMED=$(( MAIN_TIMED + WASM_TIMED ))
TOTAL_FLAKY=$(( MAIN_FLAKY + WASM_FLAKY ))
TOTAL_INCOMPLETE=$(( MAIN_INCOMPLETE + WASM_INCOMPLETE ))

# A pass with a Summary line reports it. Without one, the row says which of four things happened:
# the pass printed results and stopped (INCOMPLETE, with how far it got and what failed on the way),
# its build failed (cargo said so in its section), it printed nothing at all, or the log holds no
# such pass.
pass_row() { # $1 = summary line, $2 = marker name, $3 = pass noun, $4 = run, $5 = failed, $6 = timed out, $7 = incomplete, $8 = planned
  if [ -n "$1" ]; then printf '%s' "$1"
  elif (( ! $7 )); then printf '%s' "<not in this log: it has no pass markers, so it holds the main pass only>"
  elif (( $4 > 0 )); then
    local of=""; (( $8 > 0 )) && of=" of $8"
    printf '%s' "<INCOMPLETE — no Summary line for $2: the $3 pass stopped after $4${of} tests; $5 failed and $6 timed out among them>"
  elif build_failed_in " $2 "; then
    printf '%s' "<$2 FAILED TO BUILD — no tests ran; the $3 pass never started>"
  else
    printf '%s' "<INCOMPLETE — no Summary line for $2 and no test result: the $3 pass did not finish>"
  fi
}
INCOMPLETE_NAMES=""
(( MAIN_INCOMPLETE )) && INCOMPLETE_NAMES="PASS 1/2"
(( WASM_INCOMPLETE )) && INCOMPLETE_NAMES="${INCOMPLETE_NAMES:+$INCOMPLETE_NAMES, }PASS 1b"
TOTALS_NOTE=""
[ -n "$INCOMPLETE_NAMES" ] && TOTALS_NOTE=" — INCOMPLETE: no Summary line for ${INCOMPLETE_NAMES}; a pass without one counts the result rows it printed"

echo " nextest  : $(pass_row "$MAIN" "PASS 1/2" main "$MAIN_RUN" "$MAIN_FAILED" "$MAIN_TIMED" "$MAIN_INCOMPLETE" "$MAIN_PLANNED")"
echo " wasm-jit : $(pass_row "$WASM" "PASS 1b" wasm-jit "$WASM_RUN" "$WASM_FAILED" "$WASM_TIMED" "$WASM_INCOMPLETE" "$WASM_PLANNED")"
echo " doctests : ${DOC_PASSED} passed, ${DOC_FAILED} failed, ${DOC_IGNORED} ignored"
echo " totals   : ${TOTAL_RUN} tests run, ${TOTAL_FAILED} failed, ${TOTAL_TIMED} timed out, ${TOTAL_FLAKY} flaky${TOTALS_NOTE}"

named() { printf '%s' "$1" | grep -c '[^[:space:]]' || true; }
MAIN_NAMED=$(named "$MAIN_ROWS")
WASM_NAMED=$(named "$WASM_ROWS")

# One row per failing test across both passes: a test that failed in both is one failure, listed
# once, under the row the main pass printed. Every failure is listed — a capped list names a subset
# and reads as the whole.
FAILED_ROWS="$(printf '%s\n%s\n' "$MAIN_ROWS" "$WASM_ROWS" | awk '
  NF {
    s = $0; sub(/^[ \t]+/, "", s); sub(/^TRY [0-9]+ /, "", s)
    p = index(s, " ["); rest = substr(s, p + 1)
    if (!match(rest, /^\[[^]]*\][ \t]*\([^)]*\)[ \t]*/)) next
    id = substr(rest, RLENGTH + 1)
    if (!(id in seen)) { seen[id] = 1; print }
  }')"
if [ -n "$FAILED_ROWS" ]; then
  echo
  echo " FAILED TESTS (nextest — FAIL and TIMEOUT rows):"
  printf '%s\n' "$FAILED_ROWS"
fi

# Each pass with a Summary line must name, in its own section, exactly as many failing tests as the
# Summary counted. The rows are scraped and the counts are nextest's own arithmetic, so a mismatch
# means the scrape missed a row shape, and the warning says by how much. Per pass, because a test
# failing in both passes is counted by each and listed once. A pass with no Summary line has no
# count to reconcile: its figures ARE its rows.
reconcile() { # $1 = pass name, $2 = named, $3 = counted failed, $4 = counted timed out, $5 = incomplete
  (( $5 )) && return
  local expected=$(( $3 + $4 ))
  if (( $2 != expected )); then
    local gap=$(( expected - $2 )); [ "$gap" -lt 0 ] && gap=$(( -gap ))
    echo
    echo " ⚠ RECONCILIATION ($1): the pass counted ${expected} failing test(s)" \
         "($3 failed + $4 timed out) but its section names $2 — ${gap} unaccounted for."
    echo "   The failure extraction in scripts/lib/nextest-summary.sh is incomplete for some row" \
         "shape in this log. Fix it there; do not read the list above as the whole truth."
  fi
}
reconcile "PASS 1/2" "$MAIN_NAMED" "$MAIN_FAILED" "$MAIN_TIMED" "$MAIN_INCOMPLETE"
reconcile "PASS 1b" "$WASM_NAMED" "$WASM_FAILED" "$WASM_TIMED" "$WASM_INCOMPLETE"
if (( DOC_FAILED > 0 )); then
  echo
  echo " FAILED DOCTESTS:"
  printf '%s\n' "$DOC_SECTION" | grep -E '^test .* \.\.\. FAILED$' | sed 's/^/   /' | sort -u | head -100
fi

echo "RESULT_FAILED=${TOTAL_FAILED} RESULT_TIMED_OUT=${TOTAL_TIMED} RESULT_INCOMPLETE=${TOTAL_INCOMPLETE}" >&2
exit 0
