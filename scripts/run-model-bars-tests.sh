#!/usr/bin/env bash
#
# run-model-bars-tests.sh — the decode engine's real-checkpoint accuracy bars, run on purpose.
#
# Each walks a corpus of prompts through the engine model at every knob point it is held to, the corpus one batch,
# and compares its greedy continuations and its logits after each prompt with llama.cpp's runs of the same checkpoint
# (benchmarks/models/{qwen3.5-0.8b,llama3.2-1b}/golden-{,long-}{f32,cpu}.json; for the 27B,
# benchmarks/silicon/d5005/goldens/qwen3.8-27b/, whose provenance names the checkpoint and its SHA-256):
#
#   logicaffeine-tests::d5005_engine_lock   qwen3_5_0_8b_meets_every_knob_points_bar
#   logicaffeine-tests::d5005_engine_lock   qwen3_5_0_8b_meets_every_knob_points_bar_on_long_prompts
#   logicaffeine-tests::d5005_engine_lock   llama3_2_1b_meets_every_knob_points_bar
#   logicaffeine-tests::d5005_engine_lock   llama3_2_1b_meets_every_knob_points_bar_on_long_prompts
#   logicaffeine-tests::d5005_engine_lock   qwen3_8_27b_{precise,two_word_int8,one_word,fastest}_point_meets_its_bar
#   logicaffeine-tests::d5005_engine_lock   qwen3_8_27b_{precise,two_word_int8,one_word,fastest}_point_meets_its_bar_on_long_prompts
#   logicaffeine-tests::d5005_engine_lock   qwen3_5_0_8b_fastest_point_flushed_a_window_behind_meets_its_bar{,_on_long_prompts}
#   logicaffeine-tests::d5005_engine_lock   qwen3_8_27b_fastest_point_flushed_a_window_behind_meets_its_bar{,_on_long_prompts}
#
# MEASURED scale: Qwen3.5-0.8B over 32 short prompts and 8 long ones (291 to 834 tokens), at four knob points, each
# a whole forward of the model per token on the CPU in the engine's exact arithmetic — about seven minutes a knob
# point on a box carrying other sessions, so each test runs past the baseline's 20-minute default and its 30-minute
# backstop.
#
# WHY THEY ARE NOT IN THE ROUTINE BASELINE. They are receipts that a knob point is as accurate as llama.cpp's own
# quantization, not locks on the arithmetic: every number they rest on is held bit for bit by the fast locks —
# d5005_numerics_lock, d5005_rtl_lock, and d5005_engine_lock's tiny-model bars, invariances and flush, which still
# run in the baseline every time. They are excluded by `MODEL_BARS_EXCLUDE` in run-all-tests-fast.sh, and each is
# `#[ignore]`d, so a run that names the binary without this runner's `--run-ignored all` leaves them out.
#
# The test set MUST stay in sync with MODEL_BARS_EXCLUDE in run-all-tests-fast.sh
# (enforced by crates/logicaffeine_tests/tests/ondemand_exclusion_lock.rs).
#
# Requires the checkpoints under models/ and their goldens; the runner names each checkpoint in LOGOS_REQUIRE_TOOLS,
# so a missing one fails the run instead of passing it as a skip.
#
# Usage: ./scripts/run-model-bars-tests.sh [extra nextest args…]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
command -v cargo >/dev/null 2>&1 || source "$HOME/.cargo/env"

export LOGOS_REQUIRE_TOOLS="${LOGOS_REQUIRE_TOOLS:+$LOGOS_REQUIRE_TOOLS,}checkpoint:qwen3.5-0.8b,checkpoint:qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf,checkpoint:llama3.2-1b"

MODEL_BARS_ONLY='binary(d5005_engine_lock) and test(/^(qwen3|llama3)_/)'
# The test binaries that selector can reach, each passed to cargo as `--test`, so the run builds only them.
# ondemand_exclusion_lock holds this list to the selector.
BINARIES='d5005_engine_lock'
TEST_ARGS=()
for bin in $BINARIES; do TEST_ARGS+=(--test "$bin"); done

echo "Running the decode engine's real-checkpoint bars (about seven minutes a knob point); they run one at a time."
exec cargo nextest run -p logicaffeine-tests --profile research --cargo-profile test-opt --run-ignored all -j 1 \
  "${TEST_ARGS[@]}" -E "$MODEL_BARS_ONLY" "$@"
