#!/usr/bin/env bash
# Profile every benchmark on every profiled tier (lib/languages.tsv rows with profile=1: C, LOGOS (Rust), LOGOS (C),
# LOGOS (JIT)) — the flamegraphs, hardware counters and cost buckets the /benchmarks page shows under each benchmark.
#
# Each tier runs the bin/ artifact benchmarks/run.sh built, verified and timed, pinned to the same CPUs, at one size per
# benchmark chosen from the C binary (about PERF_TARGET_SEC of C time), so every tier is profiled on the same n:
#
#   perf record  cycles:u, DWARF call graphs -> flamefold.py -> the trimmed flamegraph tree + the leaf-time buckets
#          (allocation / output / formatting / compute). The sampling rate scales down for a slower tier, so
#          every tier yields about PERF_FREQ x PERF_TARGET_SEC samples. The JIT runs with LOGOS_PERF_MAP=1: perf
#          names its generated code `logos::<function> [jit]`.
#   perf stat    instructions, cycles, branches, branch misses, cache references, cache misses (user space), repeated
#          up to 5 times.
#   objdump(compiled tiers) static call sites to malloc/free/printf/…: a constant, it cannot scale with n.
#   strace (compiled tiers) write calls and memory/write syscall time.
#
# Outputs:
#   $RESULTS/latest-profile.json what the page reads (lib/profile_assemble.py)
#   $RESULTS/profile.json        the per-tier census rows
#   $PROF/tree/<b>_<id>.json     one trimmed flamegraph per tier
#   $PROF/counters/<b>_<id>.csv  perf stat's CSV
#   $PROF/folded/<b>_<id>.folded collapsed stacks;  $PROF/summary/<b>_<id>.json  the buckets
#   $PROF/flamegraphs.pdf        the LOGOS (Rust) flamegraphs, one page each
#
# RESULTS is benchmarks/results and PROF benchmarks/profile, or BENCH_RESULTS_DIR / BENCH_PROFILE_DIR for a
# development run. perf needs kernel.perf_event_paranoid <= 1; the script lowers it (sudo -n) when it can and restores it
# on exit. Knobs: BENCH_ONLY=fib,nbody  PERF_FREQ=2000  PERF_TARGET_SEC=4  NO_RESTORE_PARANOID=1, and the quiet-box
# gate's (lib/suite.sh): BENCH_QUIET BENCH_MAX_LOAD BENCH_QUIET_WAIT BENCH_CPUS.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"
BENCH="$REPO/benchmarks"
PROF="$(realpath -m "${BENCH_PROFILE_DIR:-$BENCH/profile}")"
FOLDED="$PROF/folded"
SUMDIR="$PROF/summary"
RESULTS="$(realpath -m "${BENCH_RESULTS_DIR:-$BENCH/results}")"
mkdir -p "$FOLDED" "$SUMDIR" "$RESULTS"

PERF_FREQ="${PERF_FREQ:-2000}"
PERF_TARGET_SEC="${PERF_TARGET_SEC:-4}"
WL_SYM='std::sys::backtrace::__rust_begin_short_backtrace'

GRN='\033[0;32m'; CYN='\033[0;36m'; YLW='\033[1;33m'; NC='\033[0m'
info(){ echo -e "${CYN}[INFO]${NC} $*" >&2; }
ok(){ echo -e "${GRN}[OK]${NC} $*" >&2; }
warn(){ echo -e "${YLW}[WARN]${NC} $*" >&2; }

command -v perf >/dev/null   || { echo "perf not installed"; exit 1; }
command -v strace >/dev/null || { echo "strace not installed"; exit 1; }
command -v jq >/dev/null     || { echo "jq not installed"; exit 1; }
TIME_BIN=/usr/bin/time; [ -x "$TIME_BIN" ] || { echo "/usr/bin/time missing"; exit 1; }

# ---- enable perf sampling if locked down, restore on exit --------------------
ORIG_PARANOID="$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo 4)"
restore_paranoid(){
  if [ "${NO_RESTORE_PARANOID:-0}" != "1" ] && [ "$ORIG_PARANOID" -ge 2 ] 2>/dev/null; then
    sudo -n sysctl -w kernel.perf_event_paranoid="$ORIG_PARANOID" >/dev/null 2>&1 \
&& info "restored kernel.perf_event_paranoid=$ORIG_PARANOID"
  fi
}
trap restore_paranoid EXIT
if [ "$ORIG_PARANOID" -ge 2 ] 2>/dev/null; then
  sudo -n sysctl -w kernel.perf_event_paranoid=1 >/dev/null 2>&1 \
    && info "temporarily set kernel.perf_event_paranoid=1 (was $ORIG_PARANOID)" \
    || warn "cannot lower perf_event_paranoid ($ORIG_PARANOID); run: sudo sysctl -w kernel.perf_event_paranoid=1"
fi

# The benchmark list is the suite registry (benchmarks/lib/benchmarks.tsv), loaded here with the rest of it; BENCH_ONLY
# narrows it.
. "$BENCH/lib/suite.sh" || exit 1

# Benchmarks whose CLI arg is a linear iteration/size count — safe to scale up
# for perf (more samples) and to double for the scaling check. The rest take a
# small exponent/dimension and stay at their calibrated size.
declare -A LINEAR=()
for b in "${BENCHMARKS[@]}"; do [ "${BENCH_LINEAR[$b]}" = 1 ] && LINEAR[$b]=1; done

calibrated_size(){
  local b="$1" v ov="SIZE_${1}"
  [ -n "${!ov:-}" ] && { echo "${!ov}"; return; }
  if [ -f "$RESULTS/calibrated-sizes.json" ]; then
    v=$(jq -r --arg b "$b" '.benchmarks[$b]["500"].n // empty' "$RESULTS/calibrated-sizes.json" 2>/dev/null)
    [ -n "$v" ] && { echo "$v"; return; }
  fi
  echo "${BENCH_DEV_SIZE[$b]}"
}

# Method B: static call-site census
static_census(){ # <bin> -> "alloc_sites output_sites"
  local dis a o
  dis=$(objdump -d "$1" 2>/dev/null)
  a=$(echo "$dis" | grep -cE 'call.*<(malloc|free|realloc|calloc|_int_malloc|_int_free|__rust_alloc|__rust_dealloc)[@>]')
  o=$(echo "$dis" | grep -ciE 'call.*<(printf|__printf_chk|puts|fwrite|_IO_|vfprintf|.*stdio.*_print|.*float_to_decimal)[@>]')
  echo "${a:-0} ${o:-0}"
}

# Method C: strace syscall accounting -> "write_calls total_syscall_seconds"
strace_acct(){ # <bin> <arg>
  local tmp wr secs; tmp=$(mktemp)
  timeout 120 strace -f -c -e trace=%memory,write "$1" "$2" >/dev/null 2>"$tmp"
  wr=$(awk '$NF=="write"{print $4}' "$tmp"); secs=$(awk '/ total$/{print $2}' "$tmp")
  rm -f "$tmp"
  echo "${wr:-0} ${secs:-0}"
}

# One perf size per benchmark, chosen from the C binary so every tier profiles at the same n and compares exactly:
# scaled toward PERF_TARGET_SEC of C time, never letting C run past the timeout.
perf_size(){ # <bench> <calibrated n>
  local b="$1" cal="$2" t
  t=$(tier_seconds "$b" c "$cal"); [ -z "$t" ] && t=0.05
  awk -v t="$t" -v g="$PERF_TARGET_SEC" 'BEGIN{exit !(t>=g)}' && { echo "$cal"; return; }
  if [ -n "${LINEAR[$b]:-}" ]; then
    awk -v c="$cal" -v t="$t" -v g="$PERF_TARGET_SEC" \
'BEGIN{if(t<0.01)t=0.01;k=int(g/t+0.5);if(k<1)k=1;if(k>150)k=150;printf "%d",c*k}'
    return
  fi
  # Not linear: double n until C runs PERF_TARGET_SEC; a timeout (empty) means the last doubling overshot, so the
  # previous n stands. Safe for the factorial ones.
  local n="$cal" nn tn
  for _ in 1 2 3 4 5; do
    nn=$(awk -v n="$n" 'BEGIN{printf "%d",n*2}')
    tn=$(tier_seconds "$b" c "$nn"); [ -z "$tn" ] && break
    n="$nn"
    awk -v t="$tn" -v g="$PERF_TARGET_SEC" 'BEGIN{exit !(t>=g)}' && break
  done
  echo "$n"
}

# ---- the profiled tiers ------------------------------------------------------------------------------------------
# Every registry language with profile=1 (lib/languages.tsv: C, LOGOS (Rust), LOGOS (C), LOGOS (JIT)), run from the same
# bin/ artifacts benchmarks/run.sh built and timed, pinned to the timed CPUs, all at one size per benchmark chosen from
# the C binary so the tiers compare exactly. The JIT runs with LOGOS_PERF_MAP=1, so perf names its compiled code.
TREEDIR="$PROF/tree"; COUNTDIR="$PROF/counters"; mkdir -p "$TREEDIR" "$COUNTDIR"
largo_build >/dev/null || { echo "largo build failed"; exit 1; }
PROFILED=(); for id in "${LANG_IDS[@]}"; do [ "${LANG_PROFILE[$id]}" = 1 ] && PROFILED+=("$id"); done

if [ -d /sys/devices/cpu_core ]; then PMU="cpu_core/"; PMU_END="/u"; else PMU=""; PMU_END=":u"; fi
EVENTS="task-clock"; for e in instructions cycles branches branch-misses cache-references cache-misses; do
  if [ -n "$PMU" ]; then EVENTS+="${EVENTS:+,}${PMU}${e}${PMU_END}"; else EVENTS+="${EVENTS:+,}${e}${PMU_END}"; fi
done

# A run template as `setup && program`: the setup to eval and the program to exec directly under perf.
split_run(){ if [[ "$1" == *" && "* ]]; then SETUP="${1% && *}"; PROGRAM="${1##* && }"; else SETUP=""; PROGRAM="$1"; fi; }

# The frames a tier's program starts at: the flamegraph is re-rooted there.
roots_for(){ # <id> -> ROOTS array
  case "$1" in
    logos_release) ROOTS=(--root "_logos_main [workload]") ;;
    logos_jit) ROOTS=(--root "logicaffeine_compile::ui_bridge::interpret_for_ui_sync_with_args") ;;
    *) ROOTS=(--root main) ;;
  esac
}

# The wall seconds of tier <id> on <bench> at <n>, from one pinned run; empty when it failed or ran past 300 s.
tier_seconds(){ # <bench> <id> <n>
  local SETUP PROGRAM tf t
  split_run "$(suite_run_cmd "$1" "$2" "$3")"
  tf=$(mktemp)
  if ( eval "${SETUP:-true}" && eval "exec $TIME_BIN -f %e -o $tf timeout 300 taskset -c $(suite_cpus) $PROGRAM" ) >/dev/null 2>&1; then
    t=$(tail -1 "$tf")
  fi
  rm -f "$tf"; echo "${t:-}"
}

profile_tier(){ # <bench> <id> <perf_n> -> one JSON row on stdout
  local b="$1" id="$2" pn="$3" data maps_before secs freq reps
  secs=$(tier_seconds "$b" "$id" "$pn")
  split_run "$(suite_run_cmd "$b" "$id" "$pn")"
  [ -n "$secs" ] || { warn "  ${LANG_LABEL[$id]}: failed or ran past 300 s at n=$pn"; return 1; }
  freq=$(awk -v f="$PERF_FREQ" -v g="$PERF_TARGET_SEC" -v t="$secs" 'BEGIN{if(t<0.01)t=0.01;x=int(f*g/t);if(x>f)x=f;if(x<97)x=97;print x}')
  reps=$(awk -v t="$secs" 'BEGIN{if(t<0.01)t=0.01;r=int(60/t);if(r>5)r=5;if(r<1)r=1;print r}')
  local -a env=(); [ "$id" = logos_jit ] && env=(LOGOS_PERF_MAP=1)
  data=$(mktemp --suffix=.perfdata)
  maps_before=$(ls /tmp/perf-*.map 2>/dev/null | sort)
  ( eval "${SETUP:-true}" && eval "exec env ${env[*]} timeout 600 taskset -c $(suite_cpus) perf record -e cycles:u -F $freq --call-graph=dwarf -o $data -- $PROGRAM" ) >/dev/null 2>&1
  local -a wl=(); [ "$id" = logos_release ] && wl=(--label-workload "${WL_SYM}=_logos_main [workload]")
  roots_for "$id"
  rm -f "$TREEDIR/${b}_${id}.json"
  perf script -i "$data" 2>/dev/null | python3 "$SCRIPT_DIR/../dev/flamefold.py" \
--folded "$FOLDED/${b}_${id}.folded" --summary "$SUMDIR/${b}_${id}.json" --name "$b" --lang "$id" --n "$pn" \
"${wl[@]}" --tree-json "$TREEDIR/${b}_${id}.json" "${ROOTS[@]}" >/dev/null
  comm -13 <(echo "$maps_before") <(ls /tmp/perf-*.map 2>/dev/null | sort) | xargs -r rm -f
  rm -f "$data"
  [ -s "$TREEDIR/${b}_${id}.json" ] || { warn "  ${LANG_LABEL[$id]}: no profile"; return 1; }
  rm -f "$COUNTDIR/${b}_${id}.csv"
  ( eval "${SETUP:-true}" && eval "exec timeout 900 taskset -c $(suite_cpus) perf stat -x, -r $reps -e $EVENTS -o $COUNTDIR/${b}_${id}.csv -- $PROGRAM" ) >/dev/null 2>&1 \
      || warn "  ${LANG_LABEL[$id]}: no counters (the counting run failed)"
  # A compiled tier's program is one file: count its call sites and its syscalls (at the calibrated size, as strace slows
  # a run). The JIT's program is largo, the same for every benchmark, so it has neither.
  local cal bsites=null osites=null wr=null sys=null ratio=null t1 t2
  cal=$(calibrated_size "$b")
  if [ "$id" != logos_jit ]; then
    read -r bsites osites < <(static_census "$(suite_bin "$b" "$id")")
    read -r wr sys < <(strace_acct "$(suite_bin "$b" "$id")" "$cal")
  fi
  # A linear benchmark doubles its work with n: time n and 2n. A run bound by startup or printing would not double.
  if [ -n "${LINEAR[$b]:-}" ] && ! suite_over_limit "$b" "$id" $((cal * 2)); then
    t1=$(tier_seconds "$b" "$id" "$cal"); t2=$(tier_seconds "$b" "$id" $((cal * 2)))
    [ -n "$t1" ] && [ -n "$t2" ] && ratio=$(awk -v a="$t1" -v c="$t2" 'BEGIN{ if (a > 0) printf "%.2f", c / a; else print "null" }')
  fi
  printf '  %b%-14s%b %6ss  %s  scale x%s\n' "$GRN" "${LANG_LABEL[$id]}" "$NC" "$secs" \
    "$(jq -c '{compute_pct, alloc_pct, output_pct}' "$SUMDIR/${b}_${id}.json")" "$ratio" >&2
  jq -nc --arg b "$b" --arg id "$id" --arg n "$pn" --argjson bs "$bsites" --argjson os "$osites" --arg wr "$wr" --arg sys "$sys" \
    --argjson secs "$secs" --argjson freq "$freq" --argjson reps "$reps" --argjson ratio "$ratio" \
    --slurpfile summary "$SUMDIR/${b}_${id}.json" --slurpfile tree "$TREEDIR/${b}_${id}.json" \
    '{bench:$b, lang:$id, n:$n, seconds:$secs, sample_hz:$freq, stat_repeats:$reps,
A_perf:($summary[0] | {alloc_pct, stdout_pct, fmt_pct, output_pct, compute_pct}
        + {hot_symbol:($tree[0].top_self[0].frame // null), hot_pct:($tree[0].top_self[0].self_pct // null)}),
B_static:{alloc_call_sites:$bs, output_call_sites:$os},
C_strace:{write_calls:($wr|tonumber? // null), mem_write_syscall_seconds:($sys|tonumber? // null),
          scaling_ratio_n_to_2n:$ratio}}'
}

quiet_box_wait || { echo "the machine never went quiet — stop the competing work, or run with BENCH_QUIET=off"; exit 1; }
GATE_AT_START="$QUIET_STATE"

ALL_JSON="[]"
for b in "${BENCHMARKS[@]}"; do
  quiet_box_wait || { echo "the machine went busy and stayed busy before $b"; exit 1; }
  info "Profiling $b …"
  refbin="$(suite_bin "$b" c)"
  [ -x "$refbin" ] || { warn "  no C binary for $b (run benchmarks/run.sh first)"; continue; }
  pn=$(perf_size "$b" "$(calibrated_size "$b")")
  for id in "${PROFILED[@]}"; do
    suite_has "$b" "$id" || continue
    suite_over_limit "$b" "$id" "$pn" && { warn "  ${LANG_LABEL[$id]}: not run at n=$pn — ${LIMIT_REASON["$b $id"]}"; continue; }
    js=$(profile_tier "$b" "$id" "$pn") || continue
    [ -n "$js" ] && ALL_JSON=$(jq -c --argjson r "$js" '. + [$r]' <<<"$ALL_JSON")
  done
done

jq '{generated:"profile-benchmarks.sh", rows:.}' <<<"$ALL_JSON" > "$RESULTS/profile.json"
python3 "$BENCH/lib/profile_assemble.py" --prof "$PROF" --rows "$RESULTS/profile.json" \
  --events "$EVENTS" --cpus "$(suite_cpus)" --gate "$GATE_AT_START" --paranoid "$(cat /proc/sys/kernel/perf_event_paranoid)" \
  > "$RESULTS/latest-profile.json.tmp" || { rm -f "$RESULTS/latest-profile.json.tmp"; echo "profile assembly failed"; exit 1; }
mv "$RESULTS/latest-profile.json.tmp" "$RESULTS/latest-profile.json"
ok "wrote $RESULTS/latest-profile.json"
python3 "$SCRIPT_DIR/../dev/flamegraph_pdf.py" --folded-dir "$FOLDED" --summary-dir "$SUMDIR" --out "$PROF/flamegraphs.pdf" >&2
ok "done — $PROF/flamegraphs.pdf"
