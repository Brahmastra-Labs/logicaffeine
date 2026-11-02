#!/usr/bin/env bash
# Stage the web app's runtime data bundle.
#
# The wasm build fetches its heavyweight data from /data/* instead of compiling it in
# (native builds still include_str! the same bytes — see ui/data_fetch.rs). This script
# derives that /data directory from the single sources of truth in the repo:
#
#   benchmarks/results/{latest,solvers,latest-codec,latest-interp}.json
#   benchmarks/results/latest-jit-code.json -> the machine code the JIT emitted for each benchmark, disassembled
#   benchmarks/programs/<id>/*            -> bench-sources.json (11 languages per benchmark)
#   benchmarks/profile/flamegraphs.pdf   -> the downloadable CPU-profile chart
#   apps/logicaffeine_web/{privacy,terms}.html
#   crates/logicaffeine_lexicon/assets/lexicon.json
#
# The bundle is derived fresh on every run into a scratch directory and validated there
# — a missing or empty source is a hard failure, the same guarantee include_str! gave at
# compile time — then published into the gitignored output dir, so it can never drift
# from the repo. Publishing writes a file only when its bytes differ and removes only
# files the bundle no longer carries, so a re-stage with nothing new leaves the output
# untouched on disk. The crate's build script runs this for every wasm build and
# `dx serve` watches public/: a run that rewrote unchanged files would trigger the next
# rebuild itself, forever. Run before `dx serve`/`dx build`; CI runs it before the
# deploy build.
#
# OUT_DIR stages the same bundle somewhere else (a check that must not touch the
# checkout's own /data); it must be absent or an empty directory. BENCH_RESULTS_DIR and BENCH_PROFILE_DIR stage a
# development run's results and profile in place of benchmarks/results and benchmarks/profile.
#
# Usage: ./scripts/web/stage-web-data.sh [OUT_DIR]

set -euo pipefail

OUT="${1:-}"
if [ -n "$OUT" ]; then
  if [ -e "$OUT" ] && [ -n "$(ls -A "$OUT")" ]; then
    echo "stage-web-data: $OUT is not empty" >&2
    exit 1
  fi
  mkdir -p "$OUT"
  OUT="$(cd "$OUT" && pwd)"
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

DATA="${OUT:-apps/logicaffeine_web/public/data}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

# The checked-in results. BENCH_RESULTS_DIR stages a development run's results over them (benchmarks/run.sh
# BENCH_RESULTS_DIR=…); a file that run did not produce (it skipped the interpreter, codec or solver phase) stays
# benchmarks/results's, and the script says which.
cp benchmarks/results/latest.json \
   benchmarks/results/solvers.json \
   benchmarks/results/latest-codec.json \
   benchmarks/results/latest-interp.json \
   "$STAGE/"
RESULTS="${BENCH_RESULTS_DIR:-benchmarks/results}"
for f in latest.json solvers.json latest-codec.json latest-interp.json; do
  if [ -f "$RESULTS/$f" ]; then
    cp "$RESULTS/$f" "$STAGE/$f"
  else
    [ "$RESULTS" != benchmarks/results ] && echo "stage-web-data: $f from benchmarks/results ($RESULTS has none)"
  fi
done
cp apps/logicaffeine_web/privacy.html "$STAGE/privacy.html"
cp apps/logicaffeine_web/terms.html "$STAGE/terms.html"
cp crates/logicaffeine_lexicon/assets/lexicon.json "$STAGE/lexicon.json"
# This result may not exist yet, so its paths are quoted: web_data_bundle holds every bare `cp` source to be one the
# bundle requires.
if [ -f "$RESULTS/latest-jit-code.json" ]; then
  cp "$RESULTS/latest-jit-code.json" "$STAGE/latest-jit-code.json"
elif [ -f "benchmarks/results/latest-jit-code.json" ]; then
  cp "benchmarks/results/latest-jit-code.json" "$STAGE/latest-jit-code.json"
else
  echo "stage-web-data: no latest-jit-code.json yet — the LOGOS (JIT) tab shows no machine code"
fi

PROFILE="${BENCH_PROFILE_DIR:-benchmarks/profile}"
if [ -f "$PROFILE/flamegraphs.pdf" ]; then
  cp "$PROFILE/flamegraphs.pdf" "$STAGE/flamegraphs.pdf"
else
  cp benchmarks/profile/flamegraphs.pdf "$STAGE/flamegraphs.pdf"
  echo "stage-web-data: flamegraphs.pdf from benchmarks/profile ($PROFILE has none)"
fi

python3 - "$STAGE" <<'PY'
import json, pathlib, sys

data_dir = pathlib.Path(sys.argv[1])

# bench-sources.json: keyed by the benchmark ids latest.json actually carries, so the
# staged bundle can never desynchronize from the results the page renders.
latest = json.loads((data_dir / "latest.json").read_text())
ids = [b["id"] for b in latest["benchmarks"]]
if not ids:
    sys.exit("stage-web-data: latest.json carries no benchmarks")

langs = {
    "c": "main.c", "cpp": "main.cpp", "rust": "main.rs", "zig": "main.zig",
    "go": "main.go", "java": "Main.java", "js": "main.js", "python": "main.py",
    "ruby": "main.rb", "nim": "main.nim", "logos": "main.lg",
}
sources = {}
for bid in ids:
    prog = pathlib.Path("benchmarks/programs") / bid
    entry = {}
    for lang, fname in langs.items():
        path = prog / fname
        if not path.is_file():
            sys.exit(f"stage-web-data: benchmark '{bid}' is missing {path}")
        text = path.read_text()
        if not text.strip():
            sys.exit(f"stage-web-data: {path} is empty")
        entry[lang] = text
    sources[bid] = entry
(data_dir / "bench-sources.json").write_text(json.dumps(sources))

# Every staged JSON must parse; every staged PDF must carry the %PDF- magic; every
# staged file must be non-empty.
for path in sorted(data_dir.iterdir()):
    if not path.read_bytes().strip():
        sys.exit(f"stage-web-data: staged {path} is empty")
    if path.suffix == ".json":
        json.loads(path.read_text())
    if path.suffix == ".pdf" and not path.read_bytes().startswith(b"%PDF-"):
        sys.exit(f"stage-web-data: staged {path} is not a valid PDF (missing %PDF- header)")

names = ", ".join(p.name for p in sorted(data_dir.iterdir()))
total = sum(p.stat().st_size for p in data_dir.iterdir())
print(f"derived {len(list(data_dir.iterdir()))} files ({total / 1024:.0f} KB): {names}")
print(f"bench-sources.json covers {len(sources)} benchmarks x {len(langs)} languages")
PY

mkdir -p "$DATA"
updated=0
for staged in "$STAGE"/*; do
    name="$(basename "$staged")"
    if ! cmp -s "$staged" "$DATA/$name"; then
        cp "$staged" "$DATA/.$name.tmp"
        mv "$DATA/.$name.tmp" "$DATA/$name"
        updated=$((updated + 1))
    fi
done
removed=0
for published in "$DATA"/* "$DATA"/.[!.]*; do
    [ -e "$published" ] || continue
    if [ ! -e "$STAGE/$(basename "$published")" ]; then
        rm -rf "$published"
        removed=$((removed + 1))
    fi
done
echo "published into $DATA: $updated updated, $removed removed"
