# The external solvers the SAT benchmarks race and the proof checkers they verify with, fetched and built
# from source at pinned revisions under $EXT (default <repo>/external/solvers, gitignored). Sourced by
# benchmarks/sat/run-satbench.sh and benchmarks/run-solver-vs-z3.sh after they set ROOT and LOG; every
# builder is a no-op once its binary exists, and appends its output to $LOG.
#
# The pins are the versions benchmarks/results/solvers.json records. A pin moves only together with a
# re-recorded result, so a published number always names the binary that produced it.
#
#   kissat        https://github.com/arminbiere/kissat        rel-4.0.4
#   sadical       https://fmv.jku.at/sadical/sadical.zip      the one published archive (reports `00n`)
#   cadical       https://github.com/arminbiere/cadical       rel-3.0.0
#   cryptominisat https://github.com/msoos/cryptominisat      5.11.21
#   drat-trim     https://github.com/marijnheule/drat-trim    2e3b2dc0ecf938addbd779d42877b6ed69d9a985 (untagged)
#   sr2drat       https://github.com/marijnheule/sr2drat      ca313578e9ce23d37a3950e23a01b098aad5f310 (untagged)

EXT="${SATBENCH_EXT:-$ROOT/external/solvers}"
SOLVER_BUILD_JOBS="${SOLVER_BUILD_JOBS:-4}"
mkdir -p "$EXT"

solver_note() { echo "$*" | tee -a "$LOG" >&2; }

# fetch_tag <dir> <url> <tag>: a shallow clone of one release tag.
fetch_tag() {
  [ -d "$EXT/$1/.git" ] || git clone --quiet --depth 1 --branch "$3" "$2" "$EXT/$1" >>"$LOG" 2>&1
}

# fetch_commit <dir> <url> <sha>: a clone checked out at one commit, for projects that publish no tags.
fetch_commit() {
  [ -d "$EXT/$1/.git" ] || { git clone --quiet "$2" "$EXT/$1" >>"$LOG" 2>&1 && git -C "$EXT/$1" checkout --quiet "$3" >>"$LOG" 2>&1; }
}

KISSAT="$EXT/kissat/build/kissat"
build_kissat() {
  [ -x "$KISSAT" ] && return 0
  solver_note "# fetching+building kissat rel-4.0.4 ..."
  fetch_tag kissat https://github.com/arminbiere/kissat rel-4.0.4 \
    && ( cd "$EXT/kissat" && ./configure && make -j"$SOLVER_BUILD_JOBS" ) >>"$LOG" 2>&1
}

SADICAL="$EXT/sadical/sadical/sadical"
DPRTRIM="$EXT/sadical/sadical/tests/dpr-trim"
build_sadical() {
  [ -x "$SADICAL" ] && [ -x "$DPRTRIM" ] && return 0
  solver_note "# fetching+building sadical + dpr-trim ..."
  ( cd "$EXT" && curl -sL https://fmv.jku.at/sadical/sadical.zip -o sadical.zip \
    && mkdir -p sadical && cd sadical && unzip -oq ../sadical.zip \
    && cd sadical && ./configure.sh && make \
    && gcc -O2 -o tests/dpr-trim tests/dpr-trim.c ) >>"$LOG" 2>&1
}

CADICAL="$EXT/cadical/build/cadical"
build_cadical() {
  [ -x "$CADICAL" ] && return 0
  solver_note "# fetching+building cadical rel-3.0.0 ..."
  fetch_tag cadical https://github.com/arminbiere/cadical rel-3.0.0 \
    && ( cd "$EXT/cadical" && ./configure && make -j"$SOLVER_BUILD_JOBS" ) >>"$LOG" 2>&1
}

CRYPTOMINISAT="$EXT/cryptominisat/build/cryptominisat5"
build_cryptominisat() {
  [ -x "$CRYPTOMINISAT" ] && return 0
  solver_note "# fetching+building cryptominisat 5.11.21 ..."
  fetch_tag cryptominisat https://github.com/msoos/cryptominisat 5.11.21 \
    && ( mkdir -p "$EXT/cryptominisat/build" && cd "$EXT/cryptominisat/build" \
         && cmake -DCMAKE_BUILD_TYPE=Release -DENABLE_PYTHON_INTERFACE=OFF -DCMAKE_BUILD_RPATH_USE_ORIGIN=ON .. \
         && make -j"$SOLVER_BUILD_JOBS" cryptominisat5-bin ) >>"$LOG" 2>&1
}

DRATTRIM="$EXT/drat-trim/drat-trim"
LRATCHECK="$EXT/drat-trim/lrat-check"
build_drattrim() {
  [ -x "$DRATTRIM" ] && [ -x "$LRATCHECK" ] && return 0
  solver_note "# fetching+building drat-trim + lrat-check ..."
  fetch_commit drat-trim https://github.com/marijnheule/drat-trim 2e3b2dc0ecf938addbd779d42877b6ed69d9a985 \
    && ( cd "$EXT/drat-trim" && gcc -O2 -o drat-trim drat-trim.c && gcc -O2 -o lrat-check lrat-check.c ) >>"$LOG" 2>&1
}

SR2DRAT="$EXT/sr2drat/sr2drat"
build_sr2drat() {
  [ -x "$SR2DRAT" ] && return 0
  solver_note "# fetching+building sr2drat (SR → DRAT) ..."
  fetch_commit sr2drat https://github.com/marijnheule/sr2drat ca313578e9ce23d37a3950e23a01b098aad5f310 \
    && ( cd "$EXT/sr2drat" && gcc -D_GNU_SOURCE -std=gnu99 -O2 sr2drat.c -o sr2drat ) >>"$LOG" 2>&1
}

# solver_answers <binary>: the binary runs and decides a one-variable formula both ways, printing the competition's
# `s SATISFIABLE` and `s UNSATISFIABLE` lines. A binary that is present but cannot run (a library it was linked
# against has moved, say) answers neither, and every row it raced would read as an error.
solver_answers() {
  local bin=$1 out
  [ -x "$bin" ] || return 1
  printf 'p cnf 1 1\n1 0\n' > "$EXT/.answers-sat.cnf"
  printf 'p cnf 1 2\n1 0\n-1 0\n' > "$EXT/.answers-unsat.cnf"
  out=$(timeout 20 "$bin" "$EXT/.answers-sat.cnf" 2>/dev/null || true)
  grep -qx 's SATISFIABLE' <<<"$out" || return 1
  out=$(timeout 20 "$bin" "$EXT/.answers-unsat.cnf" 2>/dev/null || true)
  grep -qx 's UNSATISFIABLE' <<<"$out"
}

# require_solvers <name>…: build each named solver, then refuse — naming every one still missing, and every raced
# solver that does not answer — rather than let a run publish a table with a competitor silently absent.
require_solvers() {
  local name missing=()
  for name in "$@"; do
    "build_$name" || true
  done
  for name in "$@"; do
    case "$name" in
      kissat) solver_answers "$KISSAT" || missing+=("kissat ($KISSAT)") ;;
      sadical) { solver_answers "$SADICAL" && [ -x "$DPRTRIM" ]; } || missing+=("sadical ($SADICAL)") ;;
      cadical) solver_answers "$CADICAL" || missing+=("cadical ($CADICAL)") ;;
      cryptominisat) solver_answers "$CRYPTOMINISAT" || missing+=("cryptominisat ($CRYPTOMINISAT)") ;;
      drattrim) { [ -x "$DRATTRIM" ] && [ -x "$LRATCHECK" ]; } || missing+=("drat-trim ($DRATTRIM)") ;;
      sr2drat) [ -x "$SR2DRAT" ] || missing+=("sr2drat ($SR2DRAT)") ;;
      *) missing+=("$name (no builder)") ;;
    esac
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    solver_note "ERROR: external solver missing or not answering a one-variable formula (delete its build to rebuild it) — see $LOG: ${missing[*]}"
    return 1
  fi
}
