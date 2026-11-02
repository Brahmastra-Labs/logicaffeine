#!/usr/bin/env bash
# The CREDIBILITY GATE (run inside the container via run-in-container.sh): the UNMODIFIED chain must run
# Spatial's own GEMM regression app end to end — genpir → gentst → maketst → runp2p → runproute → runhybrid —
# and report PCU/PMU/AG utilization + Tungsten cycles. Until this reproduces the published Table-4 shape, no
# row of ours goes through the chain. Usage: gate.sh [app] (default GEMM_0)
set -euo pipefail
APP="${1:-GEMM_0}"
cd /work/spatial
# ★ THE HARNESS DRAINS A BUILD'S STDERR ONLY AFTER THE BUILD EXITS. `utils.process.Subprocess.block` reads stdout
# line by line and leaves stderr in its pipe until the child is done, so a Tungsten build writing past the pipe's
# 64 KB deadlocks with every g++ asleep in pipe_write. Every generated context includes the toolchain's
# `sparsity/templates/Helpers.h`, whose narrowing warning costs ~860 bytes per file: a few hundred contexts —
# four sections of a composed LOGOS program — are past it. So g++ runs through a wrapper that keeps its
# diagnostics in the app's log directory; a failed compile still fails `make`, which the harness reads.
mkdir -p /tmp/logos-cc
cat > /tmp/logos-cc/g++ <<'WRAP'
#!/usr/bin/env bash
exec /usr/bin/g++ "$@" 2>> "${LOGOS_GXX_LOG:-/tmp/logos-gxx.log}"
WRAP
chmod +x /tmp/logos-cc/g++
export PATH="/tmp/logos-cc:$PATH"
export LOGOS_GXX_LOG="/work/spatial/gen/Tst_pirTest/$APP.gxx.log"
rm -f "$LOGOS_GXX_LOG"
# A ledger row is a FRESH run: the harness caches passes per app, so a stale generated dir would replay an
# old Spatial-to-PIR output against new sources. Remove it before every run.
rm -rf "gen/Tst_pirTest/$APP"
echo "=== [$(date -u +%H:%M:%S)] SpatialTest -a $APP -b Tst -p pirTest -T -H"
# spatial/bin/test (this Spatial branch) expects a `main()` that pir/bin/spatial_test.py (this PIR branch) does
# not have; the class it wraps is driven directly with the same flags — no toolchain source is edited.
pir/env/bin/python -c "
import sys; sys.path.insert(0, 'pir/bin')
import spatial_test
spatial_test.SpatialTest(args=['-a', '$APP', '-b', 'Tst', '-p', 'pirTest', '-T', '-H']).run()
" 2>&1 | tail -80
echo "=== [$(date -u +%H:%M:%S)] summary"
# The table holds a row for every app this toolchain has run, so its tail need not reach this one: the app's own row
# comes first, then the tail.
bin/log gen/Tst_pirTest -m PCU,PMU,DAG,MC,VPCU,VPMU > /tmp/logos-summary.txt 2>&1 || true
awk -v row="Tst $APP " 'index($0, row) == 1' /tmp/logos-summary.txt
tail -20 /tmp/logos-summary.txt
echo "=== tungsten cycle lines"
grep -rh "Simulation complete" "gen/Tst_pirTest/$APP" 2>/dev/null | head -5 || true
find "gen/Tst_pirTest/$APP" -name 'resource.csv' -o -name 'summary.csv' 2>/dev/null | head -5
