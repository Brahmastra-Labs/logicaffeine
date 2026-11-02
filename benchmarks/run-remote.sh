#!/usr/bin/env bash
# Run the publishable suite on another machine and bring the results home, filed by chip.
#
#   bash benchmarks/run-remote.sh <host> prepare      copy this tree to the machine; install the published run's toolchains
#   bash benchmarks/run-remote.sh <host> start        start run.sh there, detached (it survives this connection)
#   bash benchmarks/run-remote.sh <host> status [id]  where the run is
#   bash benchmarks/run-remote.sh <host> fetch [id]   bring a finished run home and judge it with compare-runs.py
#
# <host> is an ssh destination. On it, everything lives under BENCH_REMOTE_ROOT (default logos-bench, under the login's
# home): the tree (repo/), the pinned toolchains (tools/) and one directory per run (runs/<id>/). Put it on persistent
# storage; on a live-USB system that is a partition of the stick, not the RAM-backed root. lib/remote.sh is the
# machine's half: what it installs, and how a run quiets the machine and gives it back.
#
# fetch files the run under results/machines/<chip>/<id>/ (<chip> from the CPU model the run recorded), compares it
# against the published run in results/ and against that chip's previous run, writes report.md and verdict.json beside
# it, and exits as compare-runs.py does: 0 pass, 1 regression.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MACHINES="$SCRIPT_DIR/results/machines"
REMOTE_ROOT="${BENCH_REMOTE_ROOT:-logos-bench}"
SSH=(ssh -o BatchMode=yes -o ServerAliveInterval=30)

# What never travels: build products, local-only trees, and other machines' archived runs.
EXCLUDES=(
    --exclude=/target/ --exclude=/work/ --exclude=/external/ --exclude=/logs/ --exclude=/tools/ --exclude=/models/
    --exclude=/local/ --exclude=node_modules/ --exclude=target/ --exclude=__pycache__/ --exclude=.DS_Store
    --exclude=/benchmarks/bin/ --exclude=/benchmarks/generated/ --exclude=/benchmarks/asm/
    --exclude=/benchmarks/.logos-bench-target/ --exclude=/benchmarks/results/raw/ --exclude=/benchmarks/results/machines/
)

usage() { sed -n '2,18p' "$0" >&2; exit 2; }

[ $# -ge 2 ] || usage
HOST="$1"
CMD="$2"
shift 2

on_host() { "${SSH[@]}" "$HOST" "bash $REMOTE_ROOT/repo/benchmarks/lib/remote.sh $*"; }

# The directory name a CPU model files under: "Intel(R) Core(TM) i7-4790K CPU @ 4.00GHz" -> intel-core-i7-4790k.
chip_of() {
    python3 - "$1" <<'EOF'
import json, re, sys
cpu = json.load(open(sys.argv[1]))["metadata"].get("cpu") or "unknown-cpu"
cpu = re.sub(r"\((R|TM|tm|r)\)|\bCPU\b|@.*$|\b\d+-Core Processor\b|\bwith .*$", " ", cpu)
print(re.sub(r"[^a-z0-9]+", "-", cpu.lower()).strip("-"))
EOF
}

case "$CMD" in
    prepare)
        "${SSH[@]}" "$HOST" "mkdir -p $REMOTE_ROOT/repo"
        rsync -a --delete "${EXCLUDES[@]}" "$REPO_ROOT/" "$HOST:$REMOTE_ROOT/repo/"
        on_host prepare
        ;;
    start)
        on_host start
        ;;
    status)
        on_host status "${1:-}"
        ;;
    fetch)
        dir=$(on_host path "${1:-}")
        id=$(basename "$dir")
        state=$(on_host status "$id")
        if ! grep -q '^finished ' <<< "$state"; then
            echo "run $id has not finished:" >&2
            echo "$state" >&2
            exit 1
        fi
        staging=$(mktemp -d)
        trap 'rm -rf "$staging"' EXIT
        rsync -a --include='latest*.json' --include=status --include=run.out --include=logs/ --include='logs/**' \
            --exclude='*' "$HOST:$dir/" "$staging/"
        [ -f "$staging/latest.json" ] || { echo "run $id finished without a latest.json:" >&2; echo "$state" >&2; exit 1; }
        dest="$MACHINES/$(chip_of "$staging/latest.json")/$id"
        previous=$(ls -1 "$(dirname "$dest")" 2>/dev/null | grep -v "^$id\$" | sort | tail -1 || true)
        mkdir -p "$dest"
        cp -R "$staging/." "$dest/"
        against=(--against "$SCRIPT_DIR/results")
        [ -n "$previous" ] && against+=(--against "$(dirname "$dest")/$previous")
        echo "fetched $id into ${dest#"$REPO_ROOT/"}"
        rc=0
        python3 "$SCRIPT_DIR/compare-runs.py" "$dest" "${against[@]}" \
            --report "$dest/report.md" --json "$dest/verdict.json" || rc=$?
        exit "$rc"
        ;;
    *)
        usage
        ;;
esac
