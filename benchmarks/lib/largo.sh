# largo for the benchmark scripts, from one place. Source it, then call `largo_build [features]`: it builds
# logicaffeine-cli in release into the bench target (benchmarks/.logos-bench-target, or BENCH_TARGET_DIR; exported as
# CARGO_TARGET_DIR so the LOGOS programs the scripts build share its dependencies) and sets LARGO to that build's
# binary. A failed build stops the script with cargo's own error; a build that leaves no binary stops it too. There is
# no fallback to any other directory's largo: a binary from an earlier checkout would be measured as this tree's.

LARGO_BENCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_TARGET_DIR="${BENCH_TARGET_DIR:-$LARGO_BENCH_DIR/.logos-bench-target}"

largo_build() {
    local features="${1:-}" log raw="${BENCH_RESULTS_DIR:-$LARGO_BENCH_DIR/results}/raw"
    mkdir -p "$raw"
    log="$raw/largo-build.log"
    if ! cargo build -p logicaffeine-cli --release ${features:+--features "$features"} \
            --manifest-path "$LARGO_BENCH_DIR/../Cargo.toml" > "$log" 2>&1; then
        echo "largo build failed ($log):" >&2
        tail -20 "$log" >&2
        return 1
    fi
    LARGO="$CARGO_TARGET_DIR/release/largo"
    if [ ! -x "$LARGO" ]; then
        echo "cargo built logicaffeine-cli but $LARGO does not exist" >&2
        return 1
    fi
    if ! cargo build -p logicaffeine-jit --release --bins \
            --manifest-path "$LARGO_BENCH_DIR/../Cargo.toml" >> "$log" 2>&1; then
        echo "JIT image tools build failed ($log):" >&2
        tail -20 "$log" >&2
        return 1
    fi
    JIT_IMAGE_BUILD="$CARGO_TARGET_DIR/release/logos-jit-image-build"
    JIT_IMAGE_RUN="$CARGO_TARGET_DIR/release/logos-jit-run"
    # The end-to-end harness fixture replaces Cargo with a stub and exercises
    # the source-run fallback below. A real build must publish both artifacts.
    if [ -z "${FIXTURE_PROGRAMS:-}" ]; then
        if [ ! -x "$JIT_IMAGE_BUILD" ] || [ ! -x "$JIT_IMAGE_RUN" ]; then
            echo "cargo built logicaffeine-jit but its image tools do not exist" >&2
            return 1
        fi
    fi
}
