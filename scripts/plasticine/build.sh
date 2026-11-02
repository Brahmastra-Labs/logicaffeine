#!/usr/bin/env bash
# Build the chain INSIDE the container (run via run-in-container.sh). Each step is a separate, logged stage
# so a failure names the layer; nothing here touches git (the tree is tarball-extracted).
set -euo pipefail
ROOT="${1:-/work}"
cd "$ROOT/spatial/pir"
log() { echo "=== [$(date -u +%H:%M:%S)] $*"; }
log "plastiroute (C99, glib, OpenMP)"
( cd plastiroute && make CC=gcc CXX=g++ 2>&1 | tail -5 ) && ls -la plastiroute/plastiroute
log "fmt (tungsten's logging dependency)"
( cd tungsten/fmt && mkdir -p build && cd build && cmake -DFMT_TEST=OFF -DFMT_DOC=OFF .. >/dev/null && make -j8 2>&1 | tail -2 && cp libfmt.a .. )
log "tungsten + ramulator (C++20)"
# The snapshot's Makefile links `-Lfmt -lfmt` but never adds fmt's include dir: hand it to gcc through CPATH.
# `make tungsten` links the snapshot's test stub main.cc (TestDRAMExec, absent on this branch); the flow PIR
# drives builds the simulator LIBRARY objects here and links each app's generated main against them.
# Tungsten master expects its `tbb` submodule built in-tree (`-Itbb/include`, `-L tbb/build/*_release -ltbb`)
# and Intel's header-only `parallelstl` submodule (fetched by fetch.sh). The image's system TBB stands in:
# the include dir exists but is empty (the system headers resolve), and a release dir holding a symlink to
# the system library makes the Makefile's `-L$(wildcard …)` expand to something instead of eating `-ltbb`.
( cd tungsten && mkdir -p tbb/include tbb/build/linux_release && ln -sf /usr/lib/x86_64-linux-gnu/libtbb.so tbb/build/linux_release/libtbb.so \
  && export CPATH="$PWD/fmt/include${CPATH:+:$CPATH}" && make Cpp=g++ CXX=g++ ramulator 2>&1 | tail -3 && { make Cpp=g++ CXX=g++ -j8 all 2>&1 | { grep -E "error|Error|Built|plasticine.o|tungsten$" || true; } | head -20; } )
ls -la tungsten/lib/plasticine.o.OPT
log "plastisim"
( cd plastisim && mkdir -p build && make CC=gcc Cpp=g++ CXX=g++ 2>&1 | tail -3 ) || echo "plastisim: FAILED (fallback oracle only; continuing)"
log "python env for PIR's scripts"
# The pins are installed one by one (cvxpy 1.0.25 has no py3.6 distribution any more; a single failing pin
# must not abort the rest), then cvxpy 1.0.x is taken from the last 1.0 release with scipy < 1.3 (cvxpy 1.0
# imports scipy.misc.logsumexp, removed in scipy 1.3).
( [ -d env ] || virtualenv -p python3.6 env )
for r in $(cat requirements.txt); do env/bin/pip install -q "$r" >/dev/null 2>&1 || echo "  pip: skipped $r"; done
env/bin/pip install -q "cvxpy<1.1" "scipy<1.3" >/dev/null 2>&1 || echo "  pip: cvxpy/scipy fallback failed"
env/bin/python -c "import pandas, numpy, scipy, cvxpy; print('  python env: pandas', pandas.__version__, 'scipy', scipy.__version__, 'cvxpy', cvxpy.__version__)" || echo "  python env: cvxpy unavailable (PIR's cvxpy partitioner will be unusable)"
log "PIR (sbt publishAll) — the Scala compiler"
sbt -batch publishAll 2>&1 | tail -15
log "Spatial pirTest compile"
# The snapshot's regression set includes Capstan-era SPARSE apps (SparseDRAM / Scan / Token) and two apps on
# newer host helpers (FileLUT, writeCSVNow3D, a two-argument `compress`) that this Spatial master lacks; they are set aside (renamed,
# never deleted) so the dense apps — GEMM among them — compile. Idempotent.
for f in $(grep -l 'SparseDRAM\|Scan(\|Token\]\|FileLUT\|writeCSVNow3D' regression/*.scala 2>/dev/null) regression/DRAMCompression.scala; do
    [ -f "$f" ] && mv "$f" "$f.sparse-excluded" && echo "  set aside: $f"
done
( cd .. && sbt -batch "; project pirTest; test:compile" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | { grep -E '\[error\]|\[success\]|Total time|errors? found' || true; } | head -30 )
log "Spatial resources index (the Scala backend — sim.sh — reads resources/files_list from the classpath)"
( cd .. && bash bin/update_resources.sh )
log "Spatial utils + models + emul publishLocal (the generated PIR app's build depends on models 1.1-SNAPSHOT; the Scala backend's simulator on emul)"
( cd .. && sbt -batch "; project utils; publishLocal; project models; publishLocal; project emul; publishLocal" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | { grep -E '\[error\]|\[success\]|published (models|emul)|Total time' || true; } | tail -8 )
log "DONE — next: cd spatial && bin/test -a GEMM -b Tst -p pirTest -T"
