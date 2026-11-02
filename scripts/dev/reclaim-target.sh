#!/usr/bin/env bash
# Reclaim disk + inodes from the cargo target/ dir, which grows UNBOUNDED — cargo never
# garbage-collects stale rlibs/test-binaries, so months of feature-flag/profile combos pile
# up (this repo reached 1.7T / 6.1M files, enough to trigger transient ENOSPC under the
# concurrent two-agent build load even with hundreds of GB nominally free).
#
#   scripts/dev/reclaim-target.sh          # SAFE: drop regenerable caches + stale/secondary
#                                      #   profile dirs, NEVER touching the active target/debug
#   scripts/dev/reclaim-target.sh --deep   # ALSO cargo clean the active debug profile (full
#                                      #   rebuild next time) — only when no co-agent is building
#
# What "safe" removes (all regenerated on demand, none is the primary `cargo nextest` path):
#   - target/debug/incremental   compile-speed cache (always safe)
#   - target/test-opt            the Futamura isolated-harness profile
#   - target/llvm-cov-target     coverage-instrumented build
#   - target/{x86_64,aarch64}-*  explicit-target / cross builds
#   - target/wasm32-* target/wasm-release target/server-release   web/wasm builds (dx regens)
#   - target/package target/dx target/doc target/tmp              publish/doc scratch
set -euo pipefail
cd "$(dirname "$0")/../.."

# Never fight a live build for the same files (CLAUDE.md rule 11 / the concurrent-box lesson).
if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then
  echo "a cargo/rustc build is running — refusing to reclaim under it; retry when idle" >&2
  exit 1
fi

before_b=$(df --output=avail -k / | tail -1)
before_i=$(df --output=iused  -k / | tail -1)

SAFE=(
  target/debug/incremental
  target/test-opt
  target/llvm-cov-target
  target/x86_64-unknown-linux-gnu
  target/aarch64-unknown-linux-gnu
  target/wasm32-unknown-unknown
  target/wasm-release
  target/server-release
  target/package
  target/dx
  target/doc
  target/tmp
)
for d in "${SAFE[@]}"; do
  if [ -e "$d" ]; then
    printf 'reclaiming %-42s %s\n' "$d" "$(du -sh "$d" 2>/dev/null | cut -f1)"
    rm -rf "$d"
  fi
done

if [ "${1:-}" = "--deep" ]; then
  echo "deep: cargo clean of the active debug profile (forces a full rebuild)…"
  cargo clean --profile dev 2>/dev/null || rm -rf target/debug
fi

after_b=$(df --output=avail -k / | tail -1)
after_i=$(df --output=iused  -k / | tail -1)
printf '\nreclaimed: %s GiB, %s inodes\n' \
  "$(( (after_b - before_b) / 1024 / 1024 ))" "$(( before_i - after_i ))"
df -h / | tail -1
df -i / | tail -1
