#!/usr/bin/env bash
# Stage the web app's runtime data bundle.
#
# The wasm build fetches its heavyweight data from /data/* instead of compiling it in
# (native builds still include_str! the same bytes — see ui/data_fetch.rs). This script
# derives that /data directory from the single sources of truth in the repo:
#
#   benchmarks/results/{latest,solvers,latest-codec,latest-interp}.json
#   benchmarks/programs/<id>/*            -> bench-sources.json (11 languages per benchmark)
#   apps/logicaffeine_web/{privacy,terms}.html
#   crates/logicaffeine_language/assets/lexicon.json
#
# The output dir is generated fresh on every run and is gitignored, so it can never
# drift from the repo. Every file is validated before the script succeeds — a missing
# or empty source is a hard failure, the same guarantee include_str! gave at compile
# time. Run before `dx serve`/`dx build`; CI runs it before the deploy build.
#
# Usage: ./scripts/stage-web-data.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DATA="apps/logicaffeine_web/public/data"
rm -rf "$DATA"
mkdir -p "$DATA"

cp benchmarks/results/latest.json \
   benchmarks/results/solvers.json \
   benchmarks/results/latest-codec.json \
   benchmarks/results/latest-interp.json \
   "$DATA/"
cp apps/logicaffeine_web/privacy.html "$DATA/privacy.html"
cp apps/logicaffeine_web/terms.html "$DATA/terms.html"
cp crates/logicaffeine_language/assets/lexicon.json "$DATA/lexicon.json"

python3 - "$DATA" <<'PY'
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

# Every staged JSON must parse; every staged file must be non-empty.
for path in sorted(data_dir.iterdir()):
    if not path.read_bytes().strip():
        sys.exit(f"stage-web-data: staged {path} is empty")
    if path.suffix == ".json":
        json.loads(path.read_text())

names = ", ".join(p.name for p in sorted(data_dir.iterdir()))
total = sum(p.stat().st_size for p in data_dir.iterdir())
print(f"staged {len(list(data_dir.iterdir()))} files ({total / 1024:.0f} KB) into {data_dir}: {names}")
print(f"bench-sources.json covers {len(sources)} benchmarks x {len(langs)} languages")
PY
