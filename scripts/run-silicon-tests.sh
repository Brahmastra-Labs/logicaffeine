#!/usr/bin/env bash
#
# run-silicon-tests.sh — the post-synthesis gate-level silicon QA locks, run on purpose.
#
# These are the `postsynth_*` sims in crates/logicaffeine_tests/tests/wpe_postsynth_silicon_lock.rs:
# each runs a REAL yosys `synth_xilinx -family xc7` flow and then gate-level simulates the
# resulting ~90-RAMB netlist under iverilog against a faithful RAMB36E1/RAMB18E1 model. They are
# the gold standard that would have caught the two yosys BRAM mis-synthesis bugs before silicon —
# and they cost ~20-30 min each (the seven PHASE-1 localization probes) to ~30-60 min each (the
# four gold-standard result sims). Beside them, fabric_postsynth_lock's: the S1 decode board as
# synthesis builds it, stepped and generating on its serial line, gate by gate (about an hour). And
# d5005_engine_card_lock's co-simulations: the D5005 engine build under Verilator with its memory on
# the card's channels, running a step or the step program decoding the tiny model (600-3,400 s each). And
# hart_soc_lock's boot: Board B's whole computer in RTL booting a program from its flash in lockstep with the
# board model (about 36 min).
#
# They are deliberately EXCLUDED from scripts/run-all-tests-fast.sh (see HEAVY_ONDEMAND_EXCLUDE)
# so the routine baseline stays bounded, exactly as the SAT/coNP research campaign is. That is not
# a loss of coverage: the same file's FAST construction locks run in the baseline every time and
# enforce the exact invariant each silicon fix establishes, in milliseconds, with no yosys —
#   ramb_model_round_trips_a_width1_heap_bit_plane_with_the_cascade_bit_tied_high
#   heap_stream_depth_is_a_power_of_two_so_yosys_needs_no_depth_cascade
#   wide_state_memories_are_lo_hi_split_so_none_maps_to_sdp72
# so a change that reverts a fix goes RED instantly. This script runs the slow gold standard.
#
# The `wpe-postsynth` test-group in .config/nextest.toml serializes them (max-threads = 1), which
# is the discipline the test file's own header asks for, with a per-test backstop derived from the slowest
# test's worst case.
#
# Requires `yosys` and `iverilog` on PATH; the tests skip gracefully (never silently pass) without.
#
# The test set MUST stay in sync with HEAVY_ONDEMAND_EXCLUDE in run-all-tests-fast.sh.
#
# Usage: ./scripts/run-silicon-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export Z3_SYS_Z3_HEADER="${Z3_SYS_Z3_HEADER:-/usr/include/z3.h}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

for tool in yosys iverilog verilator; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "WARNING: $tool not found on PATH — the sims that need it will SKIP (they will not fail)." >&2
  }
done

SILICON_ONLY='(binary(/^(wpe_postsynth_silicon_lock|fabric_postsynth_lock)$/) and test(/^postsynth_/)) or (binary(hart_soc_lock) and test(=the_soc_boots_a_program_from_its_flash)) or (binary(d5005_engine_card_lock) and test(/^(the_engine_runs_a_step_with_its_memory_on_the_card|the_step_program_profiles_its_prompts_last_position_a_job_at_a_time_and_decodes_as_the_machine|a_faulting_step_is_reported_by_the_engines_run|the_step_program_decodes_on_data_lines_as_the_machine_does|the_step_program_decodes_on_secded_lines_as_the_machine_does|the_step_program_decodes_through_a_flip_in_every_secded_line_as_the_machine_does|the_step_program_decodes_on_the_hybrid_engine_as_the_machine_does|the_step_program_decodes_on_the_recurrent_hybrid_engine_as_the_machine_does|the_step_program_decodes_on_the_vectored_hybrid_engine_as_the_machine_does|the_step_program_decodes_on_the_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding|the_step_program_decodes_on_the_recurrent_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding|the_step_program_decodes_on_the_vectored_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding|the_step_program_decodes_three_sequences_at_once_on_the_vectored_hybrid_engine_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_on_four_pairs_a_channel_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_with_the_recurrence_at_a_line_a_clock_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_with_every_unit_on_the_fast_side_as_the_machine_decodes_each|the_greedy_step_program_decodes_six_sequences_at_once_with_its_recurrence_on_four_head_lanes_as_the_machine_decodes_each|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_as_the_machine_decodes_each|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_at_the_cards_clock|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_at_the_small_models_widths|the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_whatever_it_powers_up_holding)$/))'
# The test binaries that selector can reach, each passed to cargo as `--test`, so the run builds only
# them: `-p logicaffeine-tests` alone builds every one of the crate's ~2,300 test binaries whatever `-E`
# selects. ondemand_exclusion_lock holds this list to the selector.
BINARIES='wpe_postsynth_silicon_lock fabric_postsynth_lock d5005_engine_card_lock hart_soc_lock'
TEST_ARGS=()
for bin in $BINARIES; do TEST_ARGS+=(--test "$bin"); done

# Each gate-level sim is single-threaded and long. On a hybrid CPU the efficiency cores run it at about half the rate
# (MEASURED on the heaptile netlist: 14 cycles/s on a 4.4 GHz core, 29 on a 5.7 GHz one), so the run is placed on the
# CPUs above the slowest cpuinfo_max_freq class (on a CPU with one class, nowhere in particular). A speedup only: each
# test's cycle budget and the postsynth wall cap hold at the slowest core's rate.
FAST_CPUS=$(for f in /sys/devices/system/cpu/cpu[0-9]*/cpufreq/cpuinfo_max_freq; do
  c=${f#/sys/devices/system/cpu/cpu}; echo "$(cat "$f") ${c%%/*}"
done 2>/dev/null | sort -k1,1n -k2,2n | awk 'NR==1{m=$1} {f[NR]=$1; c[NR]=$2} END{for (i=1;i<=NR;i++) if (f[i]>m) print c[i]}' | sort -n | paste -sd, -)
if [ -n "$FAST_CPUS" ] && command -v taskset >/dev/null 2>&1; then
  taskset -cp "$FAST_CPUS" $$ >/dev/null   # this shell, and so the cargo exec'd below and every sim it runs
else
  FAST_CPUS=""
fi

echo "Running the post-synthesis gate-level silicon locks and the D5005 card co-simulations (--run-ignored all)${FAST_CPUS:+ on CPUs $FAST_CPUS}."
echo "This is SLOW: 12 gate-level sims at ~20-60 min EACH and 11 co-simulations at 10-60 min each. Expect several hours."
exec cargo nextest run -p logicaffeine-tests --profile full --cargo-profile test-opt --run-ignored all \
  "${TEST_ARGS[@]}" -E "$SILICON_ONLY" "$@"
