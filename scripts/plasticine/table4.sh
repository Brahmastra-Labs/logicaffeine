#!/usr/bin/env bash
# The Spatial PLDI'18 Table 4 gate runner (inside the container, via run-in-container.sh):
#
#   table4.sh PIR <app>...   PIR's mapping of each app onto the 16 x 8 array (resource.csv), no simulation
#   table4.sh Tst <app>...   the whole chain (gate.sh's), then both simulations — the ideal network and the routed
#                            hybrid one — run again on the paper's memory: the generated DDR4 config set aside and
#                            table4/DDR3-1600-1ch.cfg read in its place (runp2p_ddr3.log, runhybrid_ddr3.log beside
#                            the chain's own logs)
#   table4.sh ddr3 <app>...  only those two DDR3 runs, on an app the chain already built
#
# The apps are table4/Table4.scala's, copied into the toolchain's pirTest sources; every run is fresh.
set -euo pipefail
BACKEND="${1:?PIR, Tst or ddr3}"; shift
cd /work/spatial
# The paper's memory, for both networks: Tungsten reads the `script` the chain last copied in (the routed
# network's), so each run copies its own network's script over it and the chain's files are put back after.
ddr3() {
  local gen="gen/Tst_pirTest/$1"
  [ -x "$gen/tungsten/tungsten" ] || { echo "=== $1: no Tungsten build to run on DDR3"; return; }
  ( cd "$gen/tungsten"
    [ -f DDR4-config.cfg.generated ] || cp DDR4-config.cfg DDR4-config.cfg.generated
    cp script script.chain
    cp /table4/DDR3-1600-1ch.cfg DDR4-config.cfg
    for net in p2p hybrid; do
      cp "script_$net" script
      /usr/bin/time -f Runtime:%E ./tungsten > "../log/run${net}_ddr3.log" 2>&1 || true
    done
    cp script.chain script
    cp DDR4-config.cfg.generated DDR4-config.cfg )
  echo "=== [$(date -u +%H:%M:%S)] $1 cycles (generated DDR4, then DDR3-1600 x1; ideal network, then routed)"
  grep -h "Simulation complete\|Caught exception" "$gen/log/runp2p.log" "$gen/log/runhybrid.log" \
    "$gen/log/runp2p_ddr3.log" "$gen/log/runhybrid_ddr3.log" 2>/dev/null || true
}
if [ "$BACKEND" = ddr3 ]; then
  for APP in "$@"; do ddr3 "$APP"; done
  exit 0
fi
export TABLE4_BACKEND="$BACKEND"
cp /table4/Table4.scala pir/regression/Table4.scala
mkdir -p /tmp/logos-cc
cat > /tmp/logos-cc/g++ <<'WRAP'
#!/usr/bin/env bash
exec /usr/bin/g++ "$@" 2>> "${LOGOS_GXX_LOG:-/tmp/logos-gxx.log}"
WRAP
chmod +x /tmp/logos-cc/g++
export PATH="/tmp/logos-cc:$PATH"
for APP in "$@"; do
  GEN="gen/${BACKEND}_pirTest/$APP"
  export LOGOS_GXX_LOG="/work/spatial/gen/${BACKEND}_pirTest/$APP.gxx.log"
  rm -rf "$GEN" "$LOGOS_GXX_LOG"
  echo "=== [$(date -u +%H:%M:%S)] SpatialTest -a $APP -b $BACKEND -p pirTest -T -H"
  pir/env/bin/python -c "
import sys; sys.path.insert(0, 'pir/bin')
import spatial_test
spatial_test.SpatialTest(args=['-a', '$APP', '-b', '$BACKEND', '-p', 'pirTest', '-T', '-H']).run()
" 2>&1 | tail -40
  echo "=== [$(date -u +%H:%M:%S)] $APP resources"
  find "$GEN" -name resource.csv -exec cat {} \; 2>/dev/null | head -20 || true
  if [ "$BACKEND" = Tst ]; then ddr3 "$APP"; fi
done
