#!/usr/bin/env bash
# Record the PQC + hash whole-primitive benchmark numbers into a citable artifact:
#   benchmarks/results/pqc.json
#
# Runs the three dev benches STRICTLY SEQUENTIALLY (never two suites at once):
#   1. mlkem768_bench  (3 runs — captures run-to-run spread)
#   2. mldsa65_bench   (3 runs)
#   3. uuid_bench      (1 run — 2M iters/algo, internally averaged)
#
# The recorded values are what any document may cite; a document that quotes a range is
# checked to COVER every recorded value.
# Tag the box honestly: --box dev (default) for this machine, --box bench for a
# silenced bench-box run (the publishable one).
#
#   ./scripts/bench/run-pqc-bench.sh [--box dev|bench] [--runs N]
set -euo pipefail
cd "$(dirname "$0")/../.."

BOX="dev"
RUNS=3
while [ $# -gt 0 ]; do
  case "$1" in
    --box) BOX="$2"; shift 2 ;;
    --runs) RUNS="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

TMP="$(mktemp -d /tmp/pqc-bench.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

echo "== ML-KEM-768 bench x$RUNS (sequential) =="
for i in $(seq 1 "$RUNS"); do
  RUSTFLAGS="-C target-cpu=native" cargo test --release -p logicaffeine-tests \
    --test mlkem768_bench -- --ignored --nocapture > "$TMP/mlkem_$i.txt" 2>&1
  grep -E "keygen|encaps|decaps" "$TMP/mlkem_$i.txt" | grep ours || true
done

echo "== ML-DSA-65 bench x$RUNS (sequential) =="
for i in $(seq 1 "$RUNS"); do
  RUSTFLAGS="-C target-cpu=native" cargo test --release -p logicaffeine-tests \
    --test mldsa65_bench -- --ignored --nocapture > "$TMP/mldsa_$i.txt" 2>&1
  grep -E "keygen|sign |verify" "$TMP/mldsa_$i.txt" | grep ours || true
done

echo "== UUID/hash bench x1 =="
RUSTFLAGS="-C target-cpu=native" cargo run --release --example uuid_bench \
  -p logicaffeine-base > "$TMP/uuid.txt" 2>&1

python3 - "$TMP" "$BOX" "$RUNS" <<'EOF'
import json, os, re, subprocess, sys
tmp, box, runs = sys.argv[1], sys.argv[2], int(sys.argv[3])

def cpu_name():
    for line in open('/proc/cpuinfo'):
        if line.startswith('model name'):
            return line.split(':', 1)[1].strip()
    return 'unknown'

ROW = re.compile(r'^\s*(\w+)\s+ours\s+(\d+)\s+ml-\w+\s+(\d+)\s+\(([\d.]+)\S*\)\s+libcrux\s+(\d+)')
def parse_pqc(prefix):
    out = {}
    for i in range(1, runs + 1):
        for line in open(os.path.join(tmp, f'{prefix}_{i}.txt')):
            m = ROW.match(line)
            if m:
                op, ours, port, _ratio, lc = m.groups()
                d = out.setdefault(op, {'ours_ns': [], 'portable_ns': [], 'libcrux_ns': []})
                d['ours_ns'].append(int(ours))
                d['portable_ns'].append(int(port))
                d['libcrux_ns'].append(int(lc))
    return out

HROW = re.compile(r'^(\S[\S ()\-/]*?)\s{2,}ours\s+([\d.]+)\s+M/s\s+ref\s+([\d.]+)\s+M/s\s+([\d.]+)')
hashes = {}
for line in open(os.path.join(tmp, 'uuid.txt')):
    m = HROW.match(line)
    if m:
        name, ours, ref, ratio = m.groups()
        hashes[name.strip()] = {'ours_mops': float(ours), 'ref_mops': float(ref),
                                'ratio': float(ratio)}

date = subprocess.run(['date', '-u', '+%Y-%m-%dT%H:%M:%SZ'],
                      capture_output=True, text=True).stdout.strip()
doc = {
    'meta': {'cpu': cpu_name(), 'date': date, 'box': box, 'runs_per_bench': runs,
             'note': 'ns/op per run; three-way bit-exact gates pass before timing. '
                     'box=dev is a shared interactive machine (expect spread); '
                     'box=bench is the silenced bench box.'},
    'mlkem768': parse_pqc('mlkem'),
    'mldsa65': parse_pqc('mldsa'),
    'hash': hashes,
}
assert doc['mlkem768'] and doc['mldsa65'] and doc['hash'], 'parse failure — no rows found'
out = 'benchmarks/results/pqc.json'
with open(out, 'w') as f:
    json.dump(doc, f, indent=2)
    f.write('\n')
print(f'wrote {out}: mlkem ops={list(doc["mlkem768"])}, mldsa ops={list(doc["mldsa65"])}, '
      f'hash rows={len(hashes)}')
EOF
